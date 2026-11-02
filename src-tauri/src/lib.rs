pub mod commands;
pub mod config;
pub mod core;
pub mod error;
pub mod pool;
pub mod proxy;
pub mod state;
pub mod tester;

use std::sync::atomic::Ordering;

use tauri::Manager;

use crate::error::AppError;
use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| AppError::msg(format!("no app data directory: {e}")))?;
            std::fs::create_dir_all(&data_dir)?;
            let work = data_dir.join("work");
            if work.exists() {
                // Previous runs are allowed to leave temporary cores behind on
                // a crash; nothing is running yet at this point.
                let _ = std::fs::remove_dir_all(&work);
            }
            std::fs::create_dir_all(&work)?;

            let store = crate::config::store::Store::open(&data_dir.join("nexroute.db"))?;
            let settings = store.settings()?;
            let state = AppState::new(app.handle().clone(), store, settings, data_dir);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_configs,
            commands::import_uris,
            commands::import_subscription,
            commands::delete_configs,
            commands::delete_dead,
            commands::test_configs,
            commands::test_all,
            commands::get_settings,
            commands::update_settings,
            commands::start_pool,
            commands::stop_pool,
            commands::get_pool_status,
            commands::get_core_info,
            commands::get_app_info,
        ])
        .build(tauri::generate_context!())
        .expect("could not build the NexRoute window")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let Some(state) = app.try_state::<AppState>() else {
                    return;
                };
                if state.testing.load(Ordering::SeqCst) {
                    state.testing.store(false, Ordering::SeqCst);
                }
                tauri::async_runtime::block_on(async {
                    state.pool.stop().await;
                });
            }
        });
}
