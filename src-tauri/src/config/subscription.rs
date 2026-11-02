use base64::Engine;
use reqwest::Client;
use url::Url;

use crate::error::{AppError, AppResult};

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Fetch a subscription and return its plain-text share links.
/// Supports plain lists and base64-wrapped lists, which are both common.
pub async fn fetch_subscription(url: &str) -> AppResult<String> {
    let parsed = Url::parse(url).map_err(|e| AppError::msg(format!("invalid subscription URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::msg("subscription URL must use http or https"));
    }
    if parsed.host_str().unwrap_or_default().is_empty() {
        return Err(AppError::msg("subscription URL must include a host"));
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .user_agent(concat!("NexRoute/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;
    let response = client.get(parsed).send().await?;
    if !response.status().is_success() {
        return Err(AppError::msg(format!(
            "subscription returned HTTP {}",
            response.status()
        )));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_BODY_BYTES as u64 {
            return Err(AppError::msg("subscription is larger than 8 MiB"));
        }
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::msg("subscription is larger than 8 MiB"));
    }
    let body = String::from_utf8_lossy(&bytes).to_string();
    Ok(decode_subscription(&body))
}

/// Subscription providers either return links verbatim or a single base64 blob.
pub fn decode_subscription(body: &str) -> String {
    let trimmed = body.trim().trim_start_matches('\u{feff}');
    if trimmed.to_ascii_lowercase().contains("://") {
        return trimmed.to_string();
    }
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return String::new();
    }
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    for engine in engines {
        if let Ok(bytes) = engine.decode(&compact) {
            if let Ok(decoded) = String::from_utf8(bytes) {
                return decoded;
            }
        }
    }
    trimmed.to_string()
}

pub fn links_from_text(input: &str) -> Vec<String> {
    input
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_subscription() {
        let payload = "vless://6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70@example.com:443#One\nsocks5://127.0.0.1:1080#Two";
        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
        let decoded = decode_subscription(&encoded);
        assert_eq!(links_from_text(&decoded).len(), 2);
        assert_eq!(decode_subscription(payload), payload);
    }

    #[tokio::test]
    async fn rejects_non_http_subscription_urls() {
        let error = fetch_subscription("file:///tmp/list").await.unwrap_err();
        assert!(error.to_string().contains("http"));
    }
}
