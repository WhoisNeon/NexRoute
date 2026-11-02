use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use tauri::AppHandle;

use crate::config::settings::Settings;
use crate::config::store::Store;
use crate::core::SidecarPaths;
use crate::error::AppResult;
use crate::pool::PoolManager;

#[derive(Clone)]
pub struct AppState {
    pub app: AppHandle,
    pub store: Arc<Store>,
    pub settings: Arc<RwLock<Settings>>,
    pub paths: SidecarPaths,
    pub pool: Arc<PoolManager>,
    pub testing: Arc<AtomicBool>,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(
        app: AppHandle,
        store: Store,
        settings: Settings,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            app,
            store: Arc::new(store),
            settings: Arc::new(RwLock::new(settings)),
            paths: SidecarPaths::resolve(),
            pool: Arc::new(PoolManager::default()),
            testing: Arc::new(AtomicBool::new(false)),
            data_dir,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: Settings) -> AppResult<()> {
        self.store.save_settings(&settings)?;
        if let Ok(mut current) = self.settings.write() {
            *current = settings;
        }
        Ok(())
    }

    pub fn work_root(&self) -> AppResult<PathBuf> {
        let root = self.data_dir.join("work");
        std::fs::create_dir_all(&root)?;
        Ok(root)
    }
}
