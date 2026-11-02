use std::path::Path;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::process::Command;

use crate::config::model::CoreKind;

/// Wait until the core answers on its control API (sing-box) or at least
/// accepts connections on its first listener (Xray). The real-delay test is
/// always the authoritative health check; logs are never parsed.
pub async fn wait_ready(
    core: CoreKind,
    binary: &Path,
    api_port: u16,
    probe_port: u16,
    secret: Option<&str>,
) -> Result<(), String> {
    match core {
        CoreKind::SingBox => wait_for_clash_api(api_port, secret).await,
        CoreKind::Xray => {
            wait_for_tcp(probe_port).await?;
            // Xray's supported control plane is gRPC, reached through its own
            // CLI. A failed stats query does not make the core unusable, so it
            // is only a best-effort readiness signal.
            let _ = query_xray_stats(binary, api_port).await;
            Ok(())
        }
    }
}

async fn wait_for_clash_api(port: u16, secret: Option<&str>) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://127.0.0.1:{port}/version");
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let mut request = client.get(&url);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        if let Ok(response) = request.send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(
                "sing-box did not start within 12s; check that the sidecar is executable".into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

async fn wait_for_tcp(port: u16) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("the core did not open its local listener within 12s".into());
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

async fn query_xray_stats(binary: &Path, api_port: u16) -> Result<(), String> {
    let mut command = Command::new(binary);
    command
        .arg("api")
        .arg("statsquery")
        .arg(format!("--server=127.0.0.1:{api_port}"))
        .arg("--pattern=")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(3), command.output())
        .await
        .map_err(|_| "Xray API query timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Best-effort version string for diagnostics in Settings.
pub async fn sidecar_version(binary: &Path) -> Option<String> {
    let mut command = Command::new(binary);
    command.arg("--version").kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Environment"))
        .map(str::to_string)
}
