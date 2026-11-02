use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::pool::rotation::RotationState;

pub mod http;
pub mod socks5;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HEAD_BYTES: usize = 64 * 1024;

pub struct Gateway {
    pub rotation: Arc<RotationState>,
}

/// Open a tunnel through one internal SOCKS bridge. Retries the next bridges
/// when a bridge itself is unreachable, and feeds the health state.
pub async fn pick_and_connect(
    gateway: &Gateway,
    host: &str,
    port: u16,
) -> std::io::Result<TcpStream> {
    if host.is_empty() || port == 0 {
        return Err(invalid_input("invalid target"));
    }
    let candidates = gateway.rotation.candidates();
    if candidates.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "no live bridges in the pool",
        ));
    }
    let attempts = candidates.len().min(3);
    let mut last_error = None;
    for bridge in candidates.into_iter().take(attempts) {
        match open_via_bridge(bridge.port, host, port).await {
            Ok(stream) => {
                gateway.rotation.note_success(bridge.config_id);
                return Ok(stream);
            }
            Err(error) => {
                gateway.rotation.note_failure(bridge.config_id);
                last_error = Some(error);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "upstream failed")))
}

/// Minimal SOCKS5 client used to reach the core's internal bridge inbound.
pub async fn open_via_bridge(
    bridge_port: u16,
    host: &str,
    port: u16,
) -> std::io::Result<TcpStream> {
    let target_host = host.to_string();
    let future = async move {
        let mut stream = TcpStream::connect(("127.0.0.1", bridge_port)).await?;
        stream.set_nodelay(true)?;
        stream.write_all(&[5, 1, 0]).await?;
        let mut greeting = [0u8; 2];
        stream.read_exact(&mut greeting).await?;
        if greeting != [5, 0] {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "bridge rejected SOCKS5 authentication",
            ));
        }

        let mut request = vec![5u8, 1, 0];
        match target_host.parse::<IpAddr>() {
            Ok(IpAddr::V4(ip)) => {
                request.push(1);
                request.extend_from_slice(&ip.octets());
            }
            Ok(IpAddr::V6(ip)) => {
                request.push(4);
                request.extend_from_slice(&ip.octets());
            }
            Err(_) => {
                if target_host.len() > 255 {
                    return Err(invalid_input("target host is too long"));
                }
                request.push(3);
                request.push(target_host.len() as u8);
                request.extend_from_slice(target_host.as_bytes());
            }
        }
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await?;

        let mut reply = [0u8; 4];
        stream.read_exact(&mut reply).await?;
        if reply[0] != 5 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid SOCKS5 reply from bridge",
            ));
        }
        let address_len = match reply[3] {
            1 => 4,
            4 => 16,
            3 => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await?;
                len[0] as usize
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid SOCKS5 address type from bridge",
                ))
            }
        };
        let mut ignore = vec![0u8; address_len + 2];
        stream.read_exact(&mut ignore).await?;
        if reply[1] != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("upstream SOCKS5 error {}", reply[1]),
            ));
        }
        Ok(stream)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, future)
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "upstream connect timed out"))?
}

pub(crate) fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.to_string())
}

pub(crate) async fn read_head(stream: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut buffer = Vec::with_capacity(2048);
    loop {
        if let Some(position) = find_bytes(&buffer, b"\r\n\r\n") {
            let end = position + 4;
            let rest = buffer.split_off(end);
            return Ok((buffer, rest));
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(invalid_input("request headers are too large"));
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed before sending headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

pub(crate) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Parse `host:port` or `[v6]:port` as used by the CONNECT method.
pub(crate) fn parse_authority(input: &str) -> Result<(String, u16), String> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| "invalid IPv6 authority".to_string())?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| "missing port".to_string())?
            .parse::<u16>()
            .map_err(|_| "invalid port".to_string())?;
        if host.is_empty() || port == 0 {
            return Err("invalid authority".into());
        }
        return Ok((host.to_string(), port));
    }
    let (host, port) = input
        .rsplit_once(':')
        .ok_or_else(|| "missing port".to_string())?;
    let port = port.parse::<u16>().map_err(|_| "invalid port".to_string())?;
    if host.is_empty() || port == 0 {
        return Err("invalid authority".into());
    }
    Ok((host.to_string(), port))
}

/// Rewrite an absolute-form proxy request into an origin-form request.
/// shortcut: one request per connection (`Connection: close`), so HTTP
/// upgrade/keep-alive semantics are not preserved on plain HTTP.
pub(crate) fn rewrite_forward_request(head: &[u8]) -> Result<(String, u16, Vec<u8>), String> {
    let text = std::str::from_utf8(head).map_err(|_| "request headers are not UTF-8")?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or("empty request")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing method")?;
    let target = parts.next().ok_or("missing request URI")?;
    let version = parts.next().unwrap_or("HTTP/1.1");
    let url = url::Url::parse(target).map_err(|_| "proxy requests must use an absolute URI")?;
    if url.scheme() != "http" {
        return Err("HTTPS requests must use CONNECT".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "request URI has no host".to_string())?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let mut path = url.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }

    let mut headers: Vec<String> = Vec::new();
    let mut has_host = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let key = line
            .split_once(':')
            .map(|(k, _)| k.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(
            key.as_str(),
            "proxy-connection" | "proxy-authorization" | "connection" | "keep-alive"
        ) {
            continue;
        }
        if key == "host" {
            has_host = true;
        }
        headers.push(line.to_string());
    }
    if !has_host {
        let host_value = if port == 80 {
            host.clone()
        } else {
            format!("{host}:{port}")
        };
        headers.push(format!("Host: {host_value}"));
    }
    headers.push("Connection: close".into());

    let mut rewritten = format!("{method} {path} {version}\r\n");
    for header in headers {
        rewritten.push_str(&header);
        rewritten.push_str("\r\n");
    }
    rewritten.push_str("\r\n");
    Ok((host, port, rewritten.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authorities() {
        assert_eq!(
            parse_authority("example.com:443").unwrap(),
            ("example.com".into(), 443)
        );
        assert_eq!(
            parse_authority("[2001:db8::1]:8443").unwrap(),
            ("2001:db8::1".into(), 8443)
        );
        assert!(parse_authority("example.com").is_err());
        assert!(parse_authority(":443").is_err());
    }

    #[test]
    fn rewrites_absolute_form_without_leaking_proxy_headers() {
        let head = b"GET http://example.com/hello?q=1 HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\nProxy-Authorization: Basic abc\r\nUser-Agent: test\r\n\r\n";
        let (host, port, rewritten) = rewrite_forward_request(head).unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        let text = String::from_utf8(rewritten).unwrap();
        assert!(text.starts_with("GET /hello?q=1 HTTP/1.1\r\n"));
        assert!(!text.to_lowercase().contains("proxy-"));
        assert!(text.contains("Connection: close"));
        assert!(text.contains("Host: example.com"));
    }

    #[test]
    fn refuses_https_absolute_form() {
        let head = b"GET https://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert!(rewrite_forward_request(head).is_err());
    }
}
