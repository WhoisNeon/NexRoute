use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::config::model::{CoreKind, Outbound, PoolMode};
use crate::config::settings::{Rotation, Settings};
use crate::config::store::{ConfigRecord, Store};
use crate::core::gen::{self, PoolLayout, PoolListener};
use crate::core::{api, CoreProcess, SidecarPaths};
use crate::error::{AppError, AppResult};
use crate::pool::rotation::{Bridge, RotationState};
use crate::proxy::Gateway;

const DEFAULT_HTTP_PORT: u16 = 8911;
const DEFAULT_SOCKS_PORT: u16 = 8912;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolEndpoint {
    pub config_id: i64,
    pub name: String,
    pub protocol: String,
    pub http: String,
    pub socks: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedConfig {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatus {
    pub running: bool,
    pub mode: Option<PoolMode>,
    pub core: Option<CoreKind>,
    pub http_endpoint: Option<String>,
    pub socks_endpoint: Option<String>,
    pub endpoints: Vec<PoolEndpoint>,
    pub alive_count: usize,
    pub skipped: Vec<SkippedConfig>,
    pub rotation: Rotation,
    pub started_at: Option<String>,
}

impl PoolStatus {
    pub fn stopped(rotation: Rotation) -> Self {
        Self {
            running: false,
            mode: None,
            core: None,
            http_endpoint: None,
            socks_endpoint: None,
            endpoints: Vec::new(),
            alive_count: 0,
            skipped: Vec::new(),
            rotation,
            started_at: None,
        }
    }
}

struct PoolRuntime {
    mode: PoolMode,
    core: CoreKind,
    rotation: Arc<RotationState>,
    http_port: Option<u16>,
    socks_port: Option<u16>,
    endpoints: Vec<PoolEndpoint>,
    skipped: Vec<SkippedConfig>,
    started_at: String,
    process: CoreProcess,
    tasks: Vec<JoinHandle<()>>,
    alive_count: usize,
}

impl PoolRuntime {
    fn status(&self) -> PoolStatus {
        PoolStatus {
            running: true,
            mode: Some(self.mode),
            core: Some(self.core),
            http_endpoint: self.http_port.map(|p| format!("http://127.0.0.1:{p}")),
            socks_endpoint: self.socks_port.map(|p| format!("socks5://127.0.0.1:{p}")),
            endpoints: self.endpoints.clone(),
            alive_count: self.alive_count,
            skipped: self.skipped.clone(),
            rotation: self.rotation.strategy(),
            started_at: Some(self.started_at.clone()),
        }
    }
}

#[derive(Default)]
pub struct PoolManager {
    runtime: Mutex<Option<PoolRuntime>>,
}

impl PoolManager {
    pub async fn status(&self, rotation: Rotation) -> PoolStatus {
        match self.runtime.lock().await.as_ref() {
            Some(runtime) => runtime.status(),
            None => PoolStatus::stopped(rotation),
        }
    }

    pub async fn is_running(&self) -> bool {
        self.runtime.lock().await.is_some()
    }

    pub async fn stop(&self) {
        let mut guard = self.runtime.lock().await;
        if let Some(mut runtime) = guard.take() {
            for task in runtime.tasks.drain(..) {
                task.abort();
            }
            runtime.process.stop().await;
        }
    }

    pub async fn set_strategy(&self, rotation: Rotation) {
        if let Some(runtime) = self.runtime.lock().await.as_ref() {
            runtime.rotation.set_strategy(rotation);
        }
    }

    /// Reflect test outcomes in a running pool. Dead configurations stop
    /// receiving new connections; live bridges recover automatically.
    pub async fn apply_test_results(&self, results: &[(i64, bool)]) {
        if let Some(runtime) = self.runtime.lock().await.as_mut() {
            for (id, alive) in results {
                if !alive {
                    runtime.rotation.remove(*id);
                }
            }
            if runtime.mode == PoolMode::Internal {
                runtime.alive_count = runtime.rotation.len();
            }
        }
    }

    pub async fn start(
        &self,
        store: &Store,
        settings: &Settings,
        paths: &SidecarPaths,
        work_root: &Path,
        mode: PoolMode,
    ) -> AppResult<PoolStatus> {
        let mut guard = self.runtime.lock().await;
        if guard.is_some() {
            return Err(AppError::msg("the proxy pool is already running"));
        }
        let binary = paths.path(settings.core)?.to_path_buf();
        let alive = store.alive()?;
        if alive.is_empty() {
            return Err(AppError::msg(
                "no alive configurations. Run Test All before starting the pool.",
            ));
        }

        let mut usable: Vec<ConfigRecord> = Vec::new();
        let mut skipped: Vec<SkippedConfig> = Vec::new();
        for record in alive {
            match record.outbound.validate_for(settings.core) {
                Ok(()) => usable.push(record),
                Err(reason) => skipped.push(SkippedConfig {
                    name: record.outbound.display_name(),
                    reason,
                }),
            }
        }
        if usable.is_empty() {
            return Err(AppError::msg(format!(
                "no alive configuration is supported by {}. Resolve the skipped entries first.",
                settings.core.label()
            )));
        }

        let mut used = HashSet::new();
        let api_port = reserve_port(&mut used)?;
        let secret = Uuid::new_v4().simple().to_string();
        let layout = match mode {
            PoolMode::Internal => {
                let mut bridge_ports = Vec::new();
                for _ in &usable {
                    bridge_ports.push(reserve_port(&mut used)?);
                }
                PoolLayout::Internal { bridge_ports }
            }
            PoolMode::Upstream => {
                let mut listeners = Vec::new();
                for record in &usable {
                    listeners.push(PoolListener {
                        config_id: record.id,
                        http_port: reserve_port(&mut used)?,
                        socks_port: reserve_port(&mut used)?,
                    });
                }
                PoolLayout::Upstream { listeners }
            }
        };

        // Bind the Rust listeners before starting the core so a collision is
        // reported as a pool failure rather than a silent half-start.
        let (http_listener, socks_listener, http_port, socks_port) = if mode == PoolMode::Internal {
            let http = bind_gateway(DEFAULT_HTTP_PORT).await?;
            let socks = bind_gateway(DEFAULT_SOCKS_PORT).await?;
            let http_port = http.local_addr()?.port();
            let socks_port = socks.local_addr()?.port();
            (Some(http), Some(socks), Some(http_port), Some(socks_port))
        } else {
            (None, None, None, None)
        };

        let outbounds: Vec<(i64, crate::config::model::Outbound)> = usable
            .iter()
            .map(|record| (record.id, record.outbound.clone()))
            .collect();
        let config = gen::pool_config(settings.core, &outbounds, &layout, api_port, &secret)?;
        let probe_port = match &layout {
            PoolLayout::Internal { bridge_ports } => bridge_ports.first().copied(),
            PoolLayout::Upstream { listeners } => listeners.first().map(|l| l.http_port),
        }
        .ok_or("pool has no listeners")?;

        let mut process =
            CoreProcess::start(settings.core, &binary, &config, work_root, "pool").await?;
        if let Err(error) = api::wait_ready(
            settings.core,
            &binary,
            api_port,
            probe_port,
            Some(&secret),
        )
        .await
        {
            process.stop().await;
            return Err(AppError::msg(format!(
                "{} failed to start: {error}",
                settings.core.label()
            )));
        }

        let bridges = match &layout {
            PoolLayout::Internal { bridge_ports } => usable
                .iter()
                .zip(bridge_ports.iter())
                .map(|(record, port)| Bridge {
                    config_id: record.id,
                    port: *port,
                })
                .collect(),
            PoolLayout::Upstream { .. } => Vec::new(),
        };
        let rotation = Arc::new(RotationState::new(settings.rotation, bridges));

        let mut tasks = Vec::new();
        if let (Some(http), Some(socks)) = (http_listener, socks_listener) {
            let gateway = Arc::new(Gateway {
                rotation: rotation.clone(),
            });
            tasks.push(tokio::spawn(crate::proxy::http::run(
                http,
                gateway.clone(),
            )));
            tasks.push(tokio::spawn(crate::proxy::socks5::run(socks, gateway)));
        }

        let endpoints = match &layout {
            PoolLayout::Internal { .. } => Vec::new(),
            PoolLayout::Upstream { listeners } => listeners
                .iter()
                .zip(usable.iter())
                .map(|(listener, record)| PoolEndpoint {
                    config_id: record.id,
                    name: record.outbound.display_name(),
                    protocol: record.outbound.protocol.label().to_string(),
                    http: format!("http://127.0.0.1:{}", listener.http_port),
                    socks: format!("socks5://127.0.0.1:{}", listener.socks_port),
                })
                .collect(),
        };

        let runtime = PoolRuntime {
            mode,
            core: settings.core,
            rotation,
            http_port,
            socks_port,
            endpoints,
            skipped,
            started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            process,
            tasks,
            alive_count: usable.len(),
        };
        let status = runtime.status();
        *guard = Some(runtime);
        Ok(status)
    }
}

fn reserve_port(used: &mut HashSet<u16>) -> AppResult<u16> {
    for _ in 0..64 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        drop(listener);
        if used.insert(port) {
            return Ok(port);
        }
    }
    Err(AppError::msg("could not allocate a free local port"))
}

async fn bind_gateway(preferred: u16) -> AppResult<TcpListener> {
    match TcpListener::bind(("127.0.0.1", preferred)).await {
        Ok(listener) => Ok(listener),
        Err(_) => TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(AppError::from),
    }
}

/// Parse a list of share links, returning every valid one with its original URI.
pub fn parse_all(links: &[String]) -> (Vec<(String, Outbound)>, Vec<(String, String)>) {
    let mut parsed = Vec::new();
    let mut invalid = Vec::new();
    for link in links {
        match crate::config::uri::parse_uri(link) {
            Ok(item) => parsed.push((link.clone(), item.outbound)),
            Err(error) => invalid.push((link.clone(), error)),
        }
    }
    (parsed, invalid)
}
