use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::model::CoreKind;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    RoundRobin,
    Random,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub test_url: String,
    pub timeout_ms: u64,
    pub core: CoreKind,
    pub rotation: Rotation,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            test_url: "https://www.gstatic.com/generate_204".into(),
            timeout_ms: 2000,
            core: CoreKind::SingBox,
            rotation: Rotation::RoundRobin,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> AppResult<()> {
        let url = Url::parse(&self.test_url)
            .map_err(|e| AppError::msg(format!("invalid test URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AppError::msg("test URL must use http or https"));
        }
        if url.host_str().unwrap_or_default().is_empty() {
            return Err(AppError::msg("test URL must include a host"));
        }
        if !(200..=60_000).contains(&self.timeout_ms) {
            return Err(AppError::msg(
                "timeout must be between 200 and 60000 milliseconds",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_settings() {
        let mut settings = Settings::default();
        assert!(settings.validate().is_ok());
        settings.test_url = "file:///etc/passwd".into();
        assert!(settings.validate().is_err());
        settings.test_url = "https://example.com/204".into();
        settings.timeout_ms = 50;
        assert!(settings.validate().is_err());
        settings.timeout_ms = 2000;
        assert!(settings.validate().is_ok());
    }
}
