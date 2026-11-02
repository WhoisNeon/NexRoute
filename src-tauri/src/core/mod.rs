use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::process::{Child, Command};
use uuid::Uuid;

use crate::config::model::CoreKind;
use crate::error::{AppError, AppResult};

pub mod api;
pub mod gen;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    pub id: CoreKind,
    pub label: String,
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SidecarPaths {
    pub sing_box: Option<PathBuf>,
    pub xray: Option<PathBuf>,
}

impl SidecarPaths {
    pub fn resolve() -> Self {
        Self {
            sing_box: find_sidecar("sing-box"),
            xray: find_sidecar("xray"),
        }
    }

    pub fn path(&self, core: CoreKind) -> AppResult<&Path> {
        match core {
            CoreKind::SingBox => self.sing_box.as_deref(),
            CoreKind::Xray => self.xray.as_deref(),
        }
        .ok_or_else(|| {
            AppError::msg(format!(
                "{} sidecar is missing. Release builds bundle it; development builds need \
                 `node scripts/fetch-sidecars.mjs`.",
                core.label()
            ))
        })
    }

    pub async fn info(&self) -> Vec<CoreInfo> {
        let mut infos = Vec::new();
        for (core, path) in [
            (CoreKind::SingBox, self.sing_box.as_deref()),
            (CoreKind::Xray, self.xray.as_deref()),
        ] {
            let version = match path {
                Some(path) => api::sidecar_version(path).await,
                None => None,
            };
            infos.push(CoreInfo {
                id: core,
                label: core.label().to_string(),
                available: path.is_some(),
                version,
                path: path.map(|p| p.display().to_string()),
            });
        }
        infos
    }
}

/// Lives next to the executable when bundled; falls back to the repository's
/// binaries directory (target-triple suffixed) during development and CI.
fn find_sidecar(name: &str) -> Option<PathBuf> {
    let exe_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&exe_name));
            candidates.push(dir.join("binaries").join(&exe_name));
        }
    }
    let dev_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    candidates.push(dev_dir.join(&exe_name));
    if let Ok(entries) = std::fs::read_dir(&dev_dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with(&format!("{name}-"))
                && !file_name.ends_with(".sha256")
                && entry.path().is_file()
            {
                candidates.push(entry.path());
            }
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub struct CoreProcess {
    child: Child,
    pub dir: PathBuf,
}

impl CoreProcess {
    pub async fn start(
        kind: CoreKind,
        binary: &Path,
        config: &Value,
        work_root: &Path,
        label: &str,
    ) -> AppResult<Self> {
        let dir = work_root.join(format!("{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("config.json");
        std::fs::write(&config_path, serde_json::to_vec_pretty(config)?)?;

        let log_path = dir.join("core.log");
        let log_out = std::fs::File::create(&log_path)?;
        let log_err = log_out.try_clone()?;

        let mut command = Command::new(binary);
        command
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_out))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = command.spawn().map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            AppError::msg(format!(
                "could not start {} ({}): {e}",
                binary.display(),
                kind.label()
            ))
        })?;

        Ok(Self { child, dir })
    }

    pub async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_has_platform_extension() {
        let expected = format!("sing-box{}", std::env::consts::EXE_SUFFIX);
        assert!(expected.starts_with("sing-box"));
    }
}
