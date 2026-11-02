use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::config::model::Outbound;
use crate::config::settings::Settings;
use crate::config::store::ConfigRecord;
use crate::core::gen;
use crate::core::{api, CoreProcess};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const MAX_CONCURRENT_TESTS: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestProgress {
    total: usize,
    done: usize,
    alive: usize,
    dead: usize,
    finished: bool,
}

struct Counters {
    done: AtomicUsize,
    alive: AtomicUsize,
    dead: AtomicUsize,
}

/// Start a background batch. Only one batch may run at a time.
pub fn start(state: AppState, records: Vec<ConfigRecord>) -> AppResult<usize> {
    if records.is_empty() {
        return Err(AppError::msg("no configurations selected"));
    }
    if state.testing.swap(true, Ordering::SeqCst) {
        return Err(AppError::msg("a test run is already in progress"));
    }
    let total = records.len();
    tauri::async_runtime::spawn(async move {
        run_batch(state, records, total).await;
    });
    Ok(total)
}

async fn run_batch(state: AppState, records: Vec<ConfigRecord>, total: usize) {
    let settings = state.settings();
    let counters = Arc::new(Counters {
        done: AtomicUsize::new(0),
        alive: AtomicUsize::new(0),
        dead: AtomicUsize::new(0),
    });
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_TESTS));
    let mut set = JoinSet::new();

    for record in records {
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };
        let state = state.clone();
        let settings = settings.clone();
        let counters = counters.clone();
        set.spawn(async move {
            let _permit = permit;
            test_and_store(state, settings, record, counters, total).await;
        });
    }
    while set.join_next().await.is_some() {}
    state.testing.store(false, Ordering::SeqCst);
    let _ = state.app.emit(
        "test-progress",
        TestProgress {
            total,
            done: counters.done.load(Ordering::SeqCst),
            alive: counters.alive.load(Ordering::SeqCst),
            dead: counters.dead.load(Ordering::SeqCst),
            finished: true,
        },
    );
}

async fn test_and_store(
    state: AppState,
    settings: Settings,
    record: ConfigRecord,
    counters: Arc<Counters>,
    total: usize,
) {
    let id = record.id;
    let outcome = test_proxy(&state, &settings, &record.outbound).await;
    let (alive, delay, error) = match outcome {
        Ok(delay) => (true, Some(delay), None),
        Err(error) => (false, None, Some(error)),
    };

    if let Err(error) = state
        .store
        .set_result(id, alive, delay, error.as_deref())
    {
        eprintln!("nexroute: could not save test result for {id}: {error}");
    }

    if let Ok(mut rows) = state.store.by_ids(&[id]) {
        if let Some(record) = rows.pop() {
            let _ = state.app.emit("test-result", record.view());
        }
    }

    state.pool.apply_test_results(&[(id, alive)]).await;

    if alive {
        counters.alive.fetch_add(1, Ordering::SeqCst);
    } else {
        counters.dead.fetch_add(1, Ordering::SeqCst);
    }
    let done = counters.done.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state.app.emit(
        "test-progress",
        TestProgress {
            total,
            done,
            alive: counters.alive.load(Ordering::SeqCst),
            dead: counters.dead.load(Ordering::SeqCst),
            finished: done >= total,
        },
    );
}

/// Spawn a temporary core with one local SOCKS inbound, then issue a real HTTP
/// request through it. The measured duration is the full request round trip.
async fn test_proxy(
    state: &AppState,
    settings: &Settings,
    outbound: &Outbound,
) -> Result<i64, String> {
    outbound
        .validate_for(settings.core)
        .map_err(|error| format!("{}: {error}", settings.core.label()))?;
    let binary = state
        .paths
        .path(settings.core)
        .map_err(|error| error.to_string())?
        .to_path_buf();

    let work_root = state.work_root().map_err(|error| error.to_string())?;
    let mut used = HashSet::new();
    let inbound_port = free_port(&mut used)?;
    let api_port = free_port(&mut used)?;
    let secret = Uuid::new_v4().simple().to_string();
    let config = gen::test_config(
        settings.core,
        outbound,
        inbound_port,
        api_port,
        &secret,
    )
    .map_err(|error| error.to_string())?;

    let mut process = CoreProcess::start(settings.core, &binary, &config, &work_root, "test")
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = api::wait_ready(
        settings.core,
        &binary,
        api_port,
        inbound_port,
        Some(&secret),
    )
    .await
    {
        process.stop().await;
        return Err(error);
    }

    let result = async {
        let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{inbound_port}"))
            .map_err(|error| format!("could not build the test client: {error}"))?;
        let client = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_millis(settings.timeout_ms))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(concat!("NexRoute/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("could not build the test client: {error}"))?;

        let started = Instant::now();
        let response = client
            .get(&settings.test_url)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    format!("timed out after {} ms", settings.timeout_ms)
                } else {
                    format!("request failed: {error}")
                }
            })?;
        let status = response.status();
        let _ = response.bytes().await.map_err(|error| {
            if error.is_timeout() {
                format!("timed out while reading the response after {} ms", settings.timeout_ms)
            } else {
                format!("response failed: {error}")
            }
        })?;
        let delay = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
        if !acceptable(status.as_u16()) {
            return Err(format!("unexpected HTTP status {status}"));
        }
        Ok(delay)
    }
    .await;

    process.stop().await;
    result
}

fn acceptable(status: u16) -> bool {
    (200..400).contains(&status)
}

fn free_port(used: &mut HashSet<u16>) -> Result<u16, String> {
    for _ in 0..64 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("could not reserve a test port: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        drop(listener);
        if used.insert(port) {
            return Ok(port);
        }
    }
    Err("could not reserve a test port".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_success_and_redirects_only() {
        assert!(acceptable(200));
        assert!(acceptable(204));
        assert!(acceptable(301));
        assert!(!acceptable(199));
        assert!(!acceptable(404));
        assert!(!acceptable(500));
    }
}
