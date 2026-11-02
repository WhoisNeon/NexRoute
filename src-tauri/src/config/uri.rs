use std::collections::{BTreeMap, HashMap};

use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::Value;
use url::Url;

use super::model::{Outbound, Protocol, RealityOptions, TlsOptions, Transport};

#[derive(Debug, Clone)]
pub struct ParsedUri {
    pub outbound: Outbound,
}

const SS_METHODS: &[&str] = &[
    "aes-128-gcm",
    "aes-192-gcm",
    "aes-256-gcm",
    "chacha20-ietf-poly1305",
    "chacha20-poly1305",
    "xchacha20-ietf-poly1305",
    "xchacha20-poly1305",
    "2022-blake3-aes-128-gcm",
    "2022-blake3-aes-256-gcm",
    "2022-blake3-chacha20-poly1305",
];

pub fn is_ss_method(method: &str) -> bool {
    SS_METHODS.iter().any(|m| m.eq_ignore_ascii_case(method))
}

/// Parse one share link. The original URI is kept by the caller for storage.
pub fn parse_uri(raw: &str) -> Result<ParsedUri, String> {
    let raw = raw.trim().trim_start_matches('\u{feff}').trim();
    if raw.is_empty() {
        return Err("empty URI".into());
    }
    let lower = raw.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("vless://") {
        return parse_vless(raw, &raw[raw.len() - rest.len()..]);
    }
    if let Some(rest) = lower.strip_prefix("vmess://") {
        return parse_vmess(raw, &raw[raw.len() - rest.len()..]);
    }
    if let Some(rest) = lower.strip_prefix("ss://") {
        return parse_shadowsocks(&raw[raw.len() - rest.len()..]);
    }
    if lower.starts_with("socks://")
        || lower.starts_with("socks5://")
        || lower.starts_with("socks5h://")
    {
        return parse_socks(raw);
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return parse_http(raw);
    }
    Err(format!(
        "unsupported scheme: {}",
        raw.split("://").next().unwrap_or(raw)
    ))
}

fn split_fragment(input: &str) -> (&str, Option<String>) {
    match input.split_once('#') {
        Some((body, name)) => (body, Some(decode(name))),
        None => (input, None),
    }
}

fn decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().to_string()
}

fn parse_vless(raw: &str, body: &str) -> Result<ParsedUri, String> {
    let (body, fragment) = split_fragment(body);
    let url = Url::parse(&format!("vless://{body}")).map_err(|e| format!("invalid vless URI: {e}"))?;
    if url.password().is_some() {
        return Err("vless URI must not contain a password".into());
    }
    let uuid = decode(url.username());
    if uuid.is_empty() {
        return Err("vless URI is missing its user id".into());
    }
    uuid::Uuid::parse_str(&uuid).map_err(|_| "vless user id is not a valid UUID")?;
    let server = url
        .host_str()
        .ok_or("vless URI is missing a host")?
        .to_string();
    let port = url.port().unwrap_or(443);

    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
        .collect();
    let get = |key: &str| params.get(key).map(String::as_str).unwrap_or("");

    let security = get("security").to_ascii_lowercase();
    if !security.is_empty() && !matches!(security.as_str(), "tls" | "reality" | "none") {
        return Err(format!("unsupported vless security: {security}"));
    }

    let sni = first_non_empty(&[get("sni"), get("servername"), get("peer")]);
    let fingerprint = first_non_empty(&[get("fp"), get("fingerprint")]);
    let alpn = split_csv(get("alpn"));
    let insecure = matches!(get("allowinsecure"), "1" | "true" | "yes");
    let flow = match get("flow") {
        "" | "none" => None,
        flow => Some(flow.to_string()),
    };

    let tls = if security == "tls"
        || security == "reality"
        || (security.is_empty() && sni.is_some())
    {
        let reality = if security == "reality" {
            let public_key = get("pbk");
            if public_key.is_empty() {
                return Err("vless reality link is missing pbk (public key)".into());
            }
            Some(RealityOptions {
                public_key: public_key.to_string(),
                short_id: none_if_empty(get("sid")),
                spider_x: none_if_empty(get("spx")),
            })
        } else {
            None
        };
        Some(TlsOptions {
            server_name: sni,
            insecure,
            alpn,
            fingerprint,
            reality,
        })
    } else {
        None
    };

    let host_header = get("host");
    let path = get("path");
    let transport = match get("type").to_ascii_lowercase().as_str() {
        "" | "tcp" | "raw" => {
            if get("headertype").eq_ignore_ascii_case("http") {
                return Err("vless TCP with HTTP header obfuscation is not supported".into());
            }
            Some(Transport::Tcp)
        }
        "ws" => {
            let mut headers = BTreeMap::new();
            if !host_header.is_empty() {
                headers.insert("Host".to_string(), host_header.to_string());
            }
            Some(Transport::Ws {
                path: Some(if path.is_empty() { "/" } else { path }.to_string()),
                headers,
            })
        }
        "grpc" => {
            let service =
                first_non_empty(&[get("servicename"), get("service")]).unwrap_or_default();
            Some(Transport::Grpc {
                service_name: none_if_empty(&service),
            })
        }
        "http" | "h2" => Some(Transport::Http {
            path: Some(if path.is_empty() { "/" } else { path }.to_string()),
            host: none_if_empty(host_header),
        }),
        other => {
            return Err(format!(
                "unsupported vless transport: {other} (supported: tcp, ws, grpc, http)"
            ))
        }
    };

    let name = fragment
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));
    let outbound = Outbound {
        protocol: Protocol::Vless,
        server,
        port,
        name,
        uuid: Some(uuid),
        flow,
        alter_id: None,
        security: None,
        method: None,
        password: None,
        username: None,
        plugin: None,
        plugin_opts: None,
        tls,
        transport,
    };
    outbound
        .validate_for(super::model::CoreKind::SingBox)
        .map_err(|e| format!("{e} ({raw})"))?;
    Ok(ParsedUri { outbound })
}

fn parse_vmess(raw: &str, body: &str) -> Result<ParsedUri, String> {
    let (body, fragment) = split_fragment(body);
    let decoded = decode_base64(body.trim())?;
    let json = String::from_utf8(decoded).map_err(|_| "vmess payload is not UTF-8")?;
    let value: Value =
        serde_json::from_str(&json).map_err(|e| format!("invalid vmess JSON: {e}"))?;
    let obj = value.as_object().ok_or("vmess payload is not an object")?;

    let text = |key: &str| -> String {
        match obj.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    };
    let server = text("add");
    if server.is_empty() {
        return Err("vmess link is missing its address".into());
    }
    let port: u16 = text("port")
        .parse()
        .map_err(|_| "vmess link has an invalid port")?;
    if port == 0 {
        return Err("vmess port must be between 1 and 65535".into());
    }
    let uuid = text("id");
    uuid::Uuid::parse_str(&uuid).map_err(|_| "vmess user id is not a valid UUID")?;
    let alter_id: u32 = match text("aid").as_str() {
        "" => 0,
        aid => aid.parse().map_err(|_| "vmess alterId is not a number")?,
    };
    let security = match text("scy").as_str() {
        "" => "auto".to_string(),
        scy => scy,
    };
    let network = text("net").to_ascii_lowercase();
    let header_type = text("type").to_ascii_lowercase();
    let host = text("host");
    let path = text("path");
    let sni = text("sni");
    let tls_enabled = text("tls").eq_ignore_ascii_case("tls") || !sni.is_empty();
    if text("tls").eq_ignore_ascii_case("reality") {
        return Err("vmess reality links are not supported; use VLESS".into());
    }
    let alpn = split_csv(&text("alpn"));
    let fingerprint = none_if_empty(&text("fp"));

    let transport = match network.as_str() {
        "" | "tcp" | "raw" => {
            if header_type == "http" {
                return Err("vmess TCP with HTTP header obfuscation is not supported".into());
            }
            Some(Transport::Tcp)
        }
        "ws" => {
            let mut headers = BTreeMap::new();
            if !host.is_empty() {
                headers.insert("Host".to_string(), host.clone());
            }
            Some(Transport::Ws {
                path: Some(if path.is_empty() { "/" } else { path.as_str() }.to_string()),
                headers,
            })
        }
        "grpc" => Some(Transport::Grpc {
            service_name: none_if_empty(&path),
        }),
        "http" | "h2" => Some(Transport::Http {
            path: Some(if path.is_empty() { "/" } else { path.as_str() }.to_string()),
            host: none_if_empty(&host),
        }),
        other => return Err(format!("unsupported vmess transport: {other}")),
    };

    let name = fragment
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            let ps = text("ps");
            if ps.trim().is_empty() {
                format!("{server}:{port}")
            } else {
                ps
            }
        });

    let outbound = Outbound {
        protocol: Protocol::Vmess,
        server,
        port,
        name,
        uuid: Some(uuid),
        flow: None,
        alter_id: Some(alter_id),
        security: Some(security),
        method: None,
        password: None,
        username: None,
        plugin: None,
        plugin_opts: None,
        tls: tls_enabled.then(|| TlsOptions {
            server_name: none_if_empty(&sni).or_else(|| none_if_empty(&host)),
            insecure: false,
            alpn,
            fingerprint,
            reality: None,
        }),
        transport,
    };
    outbound
        .validate_for(super::model::CoreKind::SingBox)
        .map_err(|e| format!("{e} ({raw})"))?;
    Ok(ParsedUri { outbound })
}

fn parse_shadowsocks(body: &str) -> Result<ParsedUri, String> {
    let (body, fragment) = split_fragment(body);
    let (main, query) = match body.split_once('?') {
        Some((m, q)) => (m, q),
        None => (body, ""),
    };

    let (credentials, host_port) = match main.rsplit_once('@') {
        Some((creds, host_port)) => {
            let decoded = decode(creds);
            // SIP002: base64url userinfo. Plain user:pass is tolerated too.
            let creds = if decoded.contains(':') {
                decoded
            } else {
                match decode_base64(&decoded) {
                    Ok(bytes) => String::from_utf8(bytes)
                        .map_err(|_| "shadowsocks credentials are not UTF-8")?,
                    Err(_) => return Err("invalid shadowsocks credentials".into()),
                }
            };
            (creds, host_port.trim_end_matches('/').to_string())
        }
        None => {
            let decoded = String::from_utf8(decode_base64(main)?)
                .map_err(|_| "shadowsocks payload is not UTF-8")?;
            let (creds, host_port) = decoded
                .rsplit_once('@')
                .ok_or("invalid shadowsocks URI: missing host")?;
            (creds.to_string(), host_port.trim_end_matches('/').to_string())
        }
    };

    let (method, password) = credentials
        .split_once(':')
        .ok_or("shadowsocks URI is missing method or password")?;
    let method = method.trim().to_ascii_lowercase();
    if !is_ss_method(&method) {
        return Err(format!("unsupported shadowsocks method: {method}"));
    }
    if password.is_empty() {
        return Err("shadowsocks URI is missing its password".into());
    }
    let (server, port) = parse_host_port(&host_port)?;

    let mut plugin = None;
    let mut plugin_opts = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key.eq_ignore_ascii_case("plugin") && !value.is_empty() {
            let value = value.to_string();
            let (name, opts) = value.split_once(';').unwrap_or((&value, ""));
            plugin = none_if_empty(name);
            plugin_opts = none_if_empty(opts);
        }
    }

    let name = fragment
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));
    let outbound = Outbound {
        protocol: Protocol::Shadowsocks,
        server,
        port,
        name,
        uuid: None,
        flow: None,
        alter_id: None,
        security: None,
        method: Some(method),
        password: Some(password.to_string()),
        username: None,
        plugin,
        plugin_opts,
        tls: None,
        transport: None,
    };
    Ok(ParsedUri { outbound })
}

fn parse_socks(raw: &str) -> Result<ParsedUri, String> {
    let (body, fragment) = split_fragment(raw);
    let url = Url::parse(body).map_err(|e| format!("invalid SOCKS URI: {e}"))?;
    let server = url
        .host_str()
        .ok_or("SOCKS URI is missing a host")?
        .to_string();
    let port = url.port().unwrap_or(1080);
    let (username, password) = credentials_from_url(&url);
    let name = fragment
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));
    Ok(ParsedUri {
        outbound: Outbound {
            protocol: Protocol::Socks,
            server,
            port,
            name,
            uuid: None,
            flow: None,
            alter_id: None,
            security: None,
            method: None,
            password,
            username,
            plugin: None,
            plugin_opts: None,
            tls: None,
            transport: None,
        },
    })
}

fn parse_http(raw: &str) -> Result<ParsedUri, String> {
    let (body, fragment) = split_fragment(raw);
    let url = Url::parse(body).map_err(|e| format!("invalid HTTP proxy URI: {e}"))?;
    let server = url
        .host_str()
        .ok_or("HTTP proxy URI is missing a host")?
        .to_string();
    let port = url.port().unwrap_or(80);
    let (username, password) = credentials_from_url(&url);
    let tls = (url.scheme() == "https").then(|| TlsOptions {
        server_name: Some(server.clone()),
        ..Default::default()
    });
    let name = fragment
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"));
    Ok(ParsedUri {
        outbound: Outbound {
            protocol: Protocol::Http,
            server,
            port,
            name,
            uuid: None,
            flow: None,
            alter_id: None,
            security: None,
            method: None,
            password,
            username,
            plugin: None,
            plugin_opts: None,
            tls,
            transport: None,
        },
    })
}

fn credentials_from_url(url: &Url) -> (Option<String>, Option<String>) {
    if url.username().is_empty() {
        return (None, None);
    }
    let user = decode(url.username());
    if let Some(pass) = url.password() {
        return (
            none_if_empty(&user),
            none_if_empty(&decode(pass)),
        );
    }
    // Some clients put base64(user:pass) in the userinfo field.
    if !user.contains(':') {
        if let Ok(bytes) = decode_base64(&user) {
            if let Ok(joined) = String::from_utf8(bytes) {
                if let Some((u, p)) = joined.split_once(':') {
                    return (none_if_empty(u), none_if_empty(p));
                }
            }
        }
    }
    (none_if_empty(&user), None)
}

fn parse_host_port(input: &str) -> Result<(String, u16), String> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or("invalid IPv6 address in URI")?;
        let port = tail
            .strip_prefix(':')
            .ok_or("URI is missing a port")?
            .parse::<u16>()
            .map_err(|_| "invalid port in URI")?;
        return Ok((host.to_string(), port));
    }
    let (host, port) = input.rsplit_once(':').ok_or("URI is missing a port")?;
    if host.is_empty() {
        return Err("URI is missing a host".into());
    }
    let port = port
        .trim_end_matches('/')
        .parse::<u16>()
        .map_err(|_| "invalid port in URI")?;
    if port == 0 {
        return Err("port must be between 1 and 65535".into());
    }
    Ok((host.to_string(), port))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .trim_end_matches('=')
        .to_string();
    if cleaned.is_empty() {
        return Err("empty base64 payload".into());
    }
    let engines = [
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
    ];
    for engine in engines {
        if let Ok(bytes) = engine.decode(cleaned.as_bytes()) {
            return Ok(bytes);
        }
    }
    Err("invalid base64 payload".into())
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn first_non_empty<'a>(values: &[&'a str]) -> Option<String> {
    values
        .iter()
        .find(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
}

fn none_if_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::CoreKind;

    #[test]
    fn parses_vless_ws_tls() {
        let uri = "vless://6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70@example.com:443?security=tls&sni=cdn.example.com&type=ws&path=%2Fray&host=cdn.example.com&alpn=h2%2Chttp%2F1.1#Tokyo%20Edge";
        let parsed = parse_uri(uri).unwrap();
        let out = parsed.outbound;
        assert_eq!(out.protocol, Protocol::Vless);
        assert_eq!(out.name, "Tokyo Edge");
        assert_eq!(out.server, "example.com");
        assert_eq!(out.port, 443);
        assert_eq!(out.transport_kind(), "ws");
        let tls = out.tls.unwrap();
        assert_eq!(tls.server_name.unwrap(), "cdn.example.com");
        assert_eq!(tls.alpn, vec!["h2", "http/1.1"]);
        out.validate_for(CoreKind::Xray).unwrap();
    }

    #[test]
    fn parses_vless_reality() {
        let uri = "vless://6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70@1.2.3.4:443?security=reality&sni=www.example.com&fp=chrome&pbk=abc123&sid=deadbeef&flow=xtls-rprx-vision";
        let out = parse_uri(uri).unwrap().outbound;
        let tls = out.tls.unwrap();
        assert_eq!(tls.reality.unwrap().public_key, "abc123");
        out.validate_for(CoreKind::SingBox).unwrap();
        out.validate_for(CoreKind::Xray).unwrap();
    }

    #[test]
    fn parses_vmess() {
        let json = r#"{"v":"2","ps":"Vmess Node","add":"vm.example.com","port":"8443","id":"6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70","aid":"0","scy":"auto","net":"ws","type":"none","host":"vm.example.com","path":"/ws","tls":"tls","sni":"vm.example.com"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(json);
        let out = parse_uri(&format!("vmess://{encoded}")).unwrap().outbound;
        assert_eq!(out.protocol, Protocol::Vmess);
        assert_eq!(out.name, "Vmess Node");
        assert_eq!(out.port, 8443);
        assert_eq!(out.transport_kind(), "ws");
        assert!(out.tls.is_some());
    }

    #[test]
    fn parses_shadowsocks_sip002_and_legacy() {
        let creds = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("aes-256-gcm:p@ss");
        let out = parse_uri(&format!(
            "ss://{creds}@ss.example.com:8388?plugin=obfs-local%3Bobfs%3Dhttp#SS%20Node"
        ))
        .unwrap()
        .outbound;
        assert_eq!(out.protocol, Protocol::Shadowsocks);
        assert_eq!(out.method.as_deref(), Some("aes-256-gcm"));
        assert_eq!(out.password.as_deref(), Some("p@ss"));
        assert_eq!(out.plugin.as_deref(), Some("obfs-local"));
        assert_eq!(out.plugin_opts.as_deref(), Some("obfs=http"));
        assert!(out.validate_for(CoreKind::Xray).is_err());

        let legacy = base64::engine::general_purpose::STANDARD
            .encode("aes-128-gcm:secret@legacy.example.com:443");
        let out = parse_uri(&format!("ss://{legacy}#Legacy")).unwrap().outbound;
        assert_eq!(out.server, "legacy.example.com");
        assert_eq!(out.password.as_deref(), Some("secret"));
    }

    #[test]
    fn parses_socks_and_http() {
        let socks = parse_uri("socks5://user:pass@127.0.0.1:1080#Local").unwrap().outbound;
        assert_eq!(socks.protocol, Protocol::Socks);
        assert_eq!(socks.username.as_deref(), Some("user"));
        assert_eq!(socks.password.as_deref(), Some("pass"));

        let http = parse_uri("http://user:pass@proxy.example.com:3128").unwrap().outbound;
        assert_eq!(http.protocol, Protocol::Http);
        assert_eq!(http.port, 3128);
    }

    #[test]
    fn rejects_unsupported_scheme() {
        assert!(parse_uri("trojan://pass@example.com:443").is_err());
        assert!(parse_uri("ss://not-base64").is_err());
        assert!(parse_uri("vless://not-a-uuid@example.com:443").is_err());
    }
}
