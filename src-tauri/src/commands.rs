use serde::Serialize;
use tauri::{Emitter, State};

use crate::config::model::PoolMode;
use crate::config::settings::Settings;
use crate::config::store::{ConfigView, Counts, ImportOutcome, InvalidUri, ListQuery};
use crate::config::subscription::{fetch_subscription, links_from_text};
use crate::core::CoreInfo;
use crate::error::{AppError, AppResult};
use crate::pool::{self, PoolStatus};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigsResponse {
    pub configs: Vec<ConfigView>,
    pub counts: Counts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub os: String,
    pub arch: String,
}

#[tauri::command]
pub fn list_configs(state: State<'_, AppState>, query: ListQuery) -> AppResult<ConfigsResponse> {
    Ok(ConfigsResponse {
        configs: state.store.list(&query)?,
        counts: state.store.counts()?,
    })
}

#[tauri::command]
pub fn import_uris(state: State<'_, AppState>, text: String) -> AppResult<ImportOutcome> {
    let outcome = import_links(&state, &links_from_text(&text))?;
    Ok(outcome)
}

#[tauri::command]
pub async fn import_subscription(
    state: State<'_, AppState>,
    url: String,
) -> AppResult<ImportOutcome> {
    let body = fetch_subscription(&url).await?;
    let links = links_from_text(&body);
    if links.is_empty() {
        return Err(AppError::msg(
            "the subscription did not contain any proxy links",
        ));
    }
    import_links(&state, &links)
}

fn import_links(state: &AppState, links: &[String]) -> AppResult<ImportOutcome> {
    if links.is_empty() {
        return Err(AppError::msg("no proxy links were provided"));
    }
    let (parsed, invalid) = pool::parse_all(links);
    let (added, duplicates) = state.store.import(&parsed)?;
    let outcome = ImportOutcome {
        added,
        duplicates,
        invalid: invalid
            .into_iter()
            .map(|(input, error)| InvalidUri { input, error })
            .collect(),
    };
    if added > 0 {
        let _ = state.app.emit("configs-changed", ());
    }
    Ok(outcome)
}

#[tauri::command]
pub async fn delete_configs(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    if state.pool.is_running().await {
        return Err(AppError::msg(
            "stop the proxy pool before deleting configurations",
        ));
    }
    let deleted = state.store.delete(&ids)?;
    if deleted > 0 {
        let _ = state.app.emit("configs-changed", ());
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn delete_dead(state: State<'_, AppState>) -> AppResult<usize> {
    if state.pool.is_running().await {
        return Err(AppError::msg(
            "stop the proxy pool before deleting configurations",
        ));
    }
    let deleted = state.store.delete_dead()?;
    if deleted > 0 {
        let _ = state.app.emit("configs-changed", ());
    }
    Ok(deleted)
}

#[tauri::command]
pub fn test_configs(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let records = state.store.by_ids(&ids)?;
    crate::tester::start(state.inner().clone(), records)
}

#[tauri::command]
pub fn test_all(state: State<'_, AppState>) -> AppResult<usize> {
    let records = state.store.all()?;
    crate::tester::start(state.inner().clone(), records)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> AppResult<Settings> {
    settings.validate()?;
    let running = state.pool.is_running().await;
    let current = state.settings();
    if running && current.core != settings.core {
        return Err(AppError::msg(
            "stop the proxy pool before switching the core",
        ));
    }
    state.save_settings(settings.clone())?;
    if running {
        state.pool.set_strategy(settings.rotation).await;
    }
    Ok(settings)
}

#[tauri::command]
pub async fn start_pool(
    state: State<'_, AppState>,
    mode: PoolMode,
) -> AppResult<PoolStatus> {
    let settings = state.settings();
    let root = state.work_root()?;
    let status = state
        .pool
        .start(&state.store, &settings, &state.paths, &root, mode)
        .await?;
    let _ = state.app.emit("pool-status", &status);
    Ok(status)
}

#[tauri::command]
pub async fn stop_pool(state: State<'_, AppState>) -> AppResult<PoolStatus> {
    state.pool.stop().await;
    let status = state.pool.status(state.settings().rotation).await;
    let _ = state.app.emit("pool-status", &status);
    Ok(status)
}

#[tauri::command]
pub async fn get_pool_status(state: State<'_, AppState>) -> AppResult<PoolStatus> {
    Ok(state.pool.status(state.settings().rotation).await)
}

#[tauri::command]
pub async fn get_core_info(state: State<'_, AppState>) -> AppResult<Vec<CoreInfo>> {
    Ok(state.paths.info().await)
}

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.data_dir.display().to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}
