use serde_json::{json, Map, Value};

use crate::config::model::{
    CoreKind, Outbound, Protocol, RealityOptions, TlsOptions, Transport,
};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct PoolListener {
    pub config_id: i64,
    pub http_port: u16,
    pub socks_port: u16,
}

#[derive(Debug, Clone)]
pub enum PoolLayout {
    /// One SOCKS bridge inbound per configuration; Rust rotates across them.
    Internal { bridge_ports: Vec<u16> },
    /// One HTTP and one SOCKS listener per configuration, exposed directly.
    Upstream { listeners: Vec<PoolListener> },
}

const API_TAG: &str = "api";
const DIRECT_TAG: &str = "direct";

/// Temporary single-proxy configuration used by the delay tester.
pub fn test_config(
    core: CoreKind,
    outbound: &Outbound,
    inbound_port: u16,
    api_port: u16,
    secret: &str,
) -> AppResult<Value> {
    let tag = "proxy";
    let mut value = match core {
        CoreKind::SingBox => json!({
            "log": {"level": "error", "timestamp": true},
            "experimental": {"clash_api": {
                "external_controller": format!("127.0.0.1:{api_port}"),
                "secret": secret
            }},
            "dns": {"servers": [{"type": "local", "tag": "dns-local"}]},
            "inbounds": [{
                "type": "socks",
                "tag": "in-test",
                "listen": "127.0.0.1",
                "listen_port": inbound_port
            }],
            "outbounds": [sing_box_outbound(outbound, tag)?, {"type": "direct", "tag": DIRECT_TAG}],
            "route": {
                "default_domain_resolver": "dns-local",
                "rules": [{"inbound": ["in-test"], "outbound": tag}],
                "final": DIRECT_TAG
            }
        }),
        CoreKind::Xray => json!({
            "log": {"loglevel": "warning"},
            "stats": {},
            "api": {"tag": API_TAG, "services": ["StatsService"]},
            "inbounds": [
                api_inbound(api_port),
                {
                    "tag": "in-test",
                    "listen": "127.0.0.1",
                    "port": inbound_port,
                    "protocol": "socks",
                    "settings": {"auth": "noauth", "udp": false}
                }
            ],
            "outbounds": [xray_outbound(outbound, tag)?],
            "routing": {"rules": [
                {"type": "field", "inboundTag": [API_TAG], "outboundTag": API_TAG},
                {"type": "field", "inboundTag": ["in-test"], "outboundTag": tag}
            ]}
        }),
    };
    prune_nulls(&mut value);
    Ok(value)
}

/// Configuration for the local pool. All listeners bind to loopback only.
pub fn pool_config(
    core: CoreKind,
    configs: &[(i64, Outbound)],
    layout: &PoolLayout,
    api_port: u16,
    secret: &str,
) -> AppResult<Value> {
    if configs.is_empty() {
        return Err(AppError::msg("the pool needs at least one configuration"));
    }
    let mut value = match core {
        CoreKind::SingBox => sing_box_pool(configs, layout, api_port, secret)?,
        CoreKind::Xray => xray_pool(configs, layout, api_port)?,
    };
    prune_nulls(&mut value);
    Ok(value)
}

fn sing_box_pool(
    configs: &[(i64, Outbound)],
    layout: &PoolLayout,
    api_port: u16,
    secret: &str,
) -> AppResult<Value> {
    let mut inbounds = Vec::new();
    let mut rules = Vec::new();
    let mut outbounds = Vec::new();
    for (index, (id, outbound)) in configs.iter().enumerate() {
        let proxy = format!("proxy-{id}");
        outbounds.push(sing_box_outbound(outbound, &proxy)?);
        match layout {
            PoolLayout::Internal { bridge_ports } => {
                let port = *bridge_ports
                    .get(index)
                    .ok_or("bridge port list is shorter than the pool")?;
                let tag = format!("bridge-{id}");
                inbounds.push(json!({
                    "type": "socks", "tag": tag, "listen": "127.0.0.1", "listen_port": port
                }));
                rules.push(json!({"inbound": [tag], "outbound": proxy}));
            }
            PoolLayout::Upstream { listeners } => {
                let listener = listeners
                    .get(index)
                    .ok_or("listener list is shorter than the pool")?;
                let http_tag = format!("http-{id}");
                let socks_tag = format!("socks-{id}");
                inbounds.push(json!({
                    "type": "http", "tag": http_tag, "listen": "127.0.0.1",
                    "listen_port": listener.http_port
                }));
                inbounds.push(json!({
                    "type": "socks", "tag": socks_tag, "listen": "127.0.0.1",
                    "listen_port": listener.socks_port
                }));
                rules.push(json!({"inbound": [http_tag], "outbound": proxy}));
                rules.push(json!({"inbound": [socks_tag], "outbound": proxy}));
            }
        }
    }
    outbounds.push(json!({"type": "direct", "tag": DIRECT_TAG}));
    Ok(json!({
        "log": {"level": "error", "timestamp": true},
        "experimental": {"clash_api": {
            "external_controller": format!("127.0.0.1:{api_port}"),
            "secret": secret
        }},
        "dns": {"servers": [{"type": "local", "tag": "dns-local"}]},
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {
            "default_domain_resolver": "dns-local",
            "rules": rules,
            "final": DIRECT_TAG
        }
    }))
}

fn xray_pool(
    configs: &[(i64, Outbound)],
    layout: &PoolLayout,
    api_port: u16,
) -> AppResult<Value> {
    let mut inbounds = vec![api_inbound(api_port)];
    let mut rules = vec![json!({
        "type": "field", "inboundTag": [API_TAG], "outboundTag": API_TAG
    })];
    let mut outbounds = Vec::new();
    for (index, (id, outbound)) in configs.iter().enumerate() {
        let proxy = format!("proxy-{id}");
        outbounds.push(xray_outbound(outbound, &proxy)?);
        match layout {
            PoolLayout::Internal { bridge_ports } => {
                let port = *bridge_ports
                    .get(index)
                    .ok_or("bridge port list is shorter than the pool")?;
                let tag = format!("bridge-{id}");
                inbounds.push(json!({
                    "tag": tag, "listen": "127.0.0.1", "port": port,
                    "protocol": "socks",
                    "settings": {"auth": "noauth", "udp": false}
                }));
                rules.push(json!({
                    "type": "field", "inboundTag": [tag], "outboundTag": proxy
                }));
            }
            PoolLayout::Upstream { listeners } => {
                let listener = listeners
                    .get(index)
                    .ok_or("listener list is shorter than the pool")?;
                let http_tag = format!("http-{id}");
                let socks_tag = format!("socks-{id}");
                inbounds.push(json!({
                    "tag": http_tag, "listen": "127.0.0.1", "port": listener.http_port,
                    "protocol": "http", "settings": {"timeout": 300}
                }));
                inbounds.push(json!({
                    "tag": socks_tag, "listen": "127.0.0.1", "port": listener.socks_port,
                    "protocol": "socks",
                    "settings": {"auth": "noauth", "udp": false}
                }));
                rules.push(json!({
                    "type": "field", "inboundTag": [http_tag], "outboundTag": proxy
                }));
                rules.push(json!({
                    "type": "field", "inboundTag": [socks_tag], "outboundTag": proxy
                }));
            }
        }
    }
    Ok(json!({
        "log": {"loglevel": "warning"},
        "stats": {},
        "api": {"tag": API_TAG, "services": ["StatsService"]},
        "inbounds": inbounds,
        "outbounds": outbounds,
        "routing": {"rules": rules}
    }))
}

fn api_inbound(port: u16) -> Value {
    json!({
        "tag": API_TAG,
        "listen": "127.0.0.1",
        "port": port,
        "protocol": "dokodemo-door",
        "settings": {"address": "127.0.0.1"}
    })
}

fn sing_box_outbound(outbound: &Outbound, tag: &str) -> AppResult<Value> {
    let tls = outbound.tls.as_ref().map(sing_box_tls);
    let transport = outbound.transport.as_ref().and_then(sing_box_transport);
    let mut value = match outbound.protocol {
        Protocol::Vless => json!({
            "type": "vless",
            "tag": tag,
            "server": outbound.server,
            "server_port": outbound.port,
            "uuid": outbound.uuid,
            "flow": outbound.flow,
            "tls": tls,
            "transport": transport
        }),
        Protocol::Vmess => json!({
            "type": "vmess",
            "tag": tag,
            "server": outbound.server,
            "server_port": outbound.port,
            "uuid": outbound.uuid,
            "alter_id": outbound.alter_id.unwrap_or(0),
            "security": outbound.security.clone().unwrap_or_else(|| "auto".into()),
            "tls": tls,
            "transport": transport
        }),
        Protocol::Shadowsocks => json!({
            "type": "shadowsocks",
            "tag": tag,
            "server": outbound.server,
            "server_port": outbound.port,
            "method": outbound.method,
            "password": outbound.password,
            "plugin": outbound.plugin,
            "plugin_opts": outbound.plugin_opts
        }),
        Protocol::Socks => json!({
            "type": "socks",
            "tag": tag,
            "server": outbound.server,
            "server_port": outbound.port,
            "version": "5",
            "username": outbound.username,
            "password": outbound.password
        }),
        Protocol::Http => json!({
            "type": "http",
            "tag": tag,
            "server": outbound.server,
            "server_port": outbound.port,
            "username": outbound.username,
            "password": outbound.password,
            "tls": tls
        }),
    };
    prune_nulls(&mut value);
    Ok(value)
}

fn sing_box_tls(tls: &TlsOptions) -> Value {
    let reality = tls.reality.as_ref();
    let fingerprint = tls
        .fingerprint
        .clone()
        .or_else(|| reality.map(|_| "chrome".to_string()));
    let mut value = json!({
        "enabled": true,
        "server_name": tls.server_name,
        "insecure": tls.insecure,
        "alpn": if tls.alpn.is_empty() { Value::Null } else { json!(tls.alpn) },
        "utls": fingerprint.map(|f| json!({"enabled": true, "fingerprint": f})),
        "reality": reality.map(sing_box_reality)
    });
    prune_nulls(&mut value);
    value
}

fn sing_box_reality(reality: &RealityOptions) -> Value {
    json!({
        "enabled": true,
        "public_key": reality.public_key,
        "short_id": reality.short_id,
        "spider_x": reality.spider_x
    })
}

fn sing_box_transport(transport: &Transport) -> Option<Value> {
    match transport {
        Transport::Tcp => None,
        Transport::Ws { path, headers } => Some(json!({
            "type": "ws",
            "path": path.clone().unwrap_or_else(|| "/".into()),
            "headers": if headers.is_empty() { Value::Null } else { json!(headers) }
        })),
        Transport::Grpc { service_name } => Some(json!({
            "type": "grpc",
            "service_name": service_name
        })),
        Transport::Http { path, host } => Some(json!({
            "type": "http",
            "path": path.clone().unwrap_or_else(|| "/".into()),
            "host": host.clone().map(|h| vec![h])
        })),
    }
}

fn xray_outbound(outbound: &Outbound, tag: &str) -> AppResult<Value> {
    let stream = xray_stream(outbound);
    let protocol = outbound.protocol.as_str();
    let mut settings = match outbound.protocol {
        Protocol::Vless => json!({
            "vnext": [{
                "address": outbound.server,
                "port": outbound.port,
                "users": [{
                    "id": outbound.uuid,
                    "encryption": "none",
                    "flow": outbound.flow
                }]
            }]
        }),
        Protocol::Vmess => json!({
            "vnext": [{
                "address": outbound.server,
                "port": outbound.port,
                "users": [{
                    "id": outbound.uuid,
                    "alterId": outbound.alter_id.unwrap_or(0),
                    "security": outbound.security.clone().unwrap_or_else(|| "auto".into())
                }]
            }]
        }),
        Protocol::Shadowsocks => json!({
            "servers": [{
                "address": outbound.server,
                "port": outbound.port,
                "method": outbound.method,
                "password": outbound.password
            }]
        }),
        Protocol::Socks => json!({
            "servers": [{
                "address": outbound.server,
                "port": outbound.port,
                "users": credentials(outbound)
            }]
        }),
        Protocol::Http => json!({
            "servers": [{
                "address": outbound.server,
                "port": outbound.port,
                "users": credentials(outbound)
            }]
        }),
    };
    prune_nulls(&mut settings);
    let mut value = json!({
        "tag": tag,
        "protocol": protocol,
        "settings": settings,
        "streamSettings": stream
    });
    prune_nulls(&mut value);
    Ok(value)
}

fn credentials(outbound: &Outbound) -> Value {
    match (&outbound.username, &outbound.password) {
        (Some(user), password) => json!([{
            "user": user,
            "pass": password.clone().unwrap_or_default()
        }]),
        _ => Value::Null,
    }
}

fn xray_stream(outbound: &Outbound) -> Value {
    if outbound.protocol == Protocol::Shadowsocks || outbound.protocol == Protocol::Socks {
        return Value::Null;
    }
    let network = match outbound.transport.as_ref() {
        None | Some(Transport::Tcp) => "tcp",
        Some(Transport::Ws { .. }) => "ws",
        Some(Transport::Grpc { .. }) => "grpc",
        Some(Transport::Http { .. }) => "http",
    };
    let mut stream = Map::new();
    stream.insert("network".into(), json!(network));
    if let Some(tls) = &outbound.tls {
        if let Some(reality) = &tls.reality {
            stream.insert("security".into(), json!("reality"));
            stream.insert(
                "realitySettings".into(),
                json!({
                    "serverName": tls.server_name,
                    "fingerprint": tls.fingerprint.clone().or_else(|| Some("chrome".into())),
                    "publicKey": reality.public_key,
                    "shortId": reality.short_id,
                    "spiderX": reality.spider_x
                }),
            );
        } else {
            stream.insert("security".into(), json!("tls"));
            stream.insert(
                "tlsSettings".into(),
                json!({
                    "serverName": tls.server_name,
                    "allowInsecure": tls.insecure,
                    "alpn": if tls.alpn.is_empty() { Value::Null } else { json!(tls.alpn) },
                    "fingerprint": tls.fingerprint
                }),
            );
        }
    }
    match outbound.transport.as_ref() {
        Some(Transport::Ws { path, headers }) => {
            stream.insert(
                "wsSettings".into(),
                json!({
                    "path": path.clone().unwrap_or_else(|| "/".into()),
                    "headers": if headers.is_empty() { Value::Null } else { json!(headers) }
                }),
            );
        }
        Some(Transport::Grpc { service_name }) => {
            stream.insert("grpcSettings".into(), json!({"serviceName": service_name}));
        }
        Some(Transport::Http { path, host }) => {
            stream.insert(
                "httpSettings".into(),
                json!({
                    "path": path.clone().unwrap_or_else(|| "/".into()),
                    "host": host.clone().map(|h| vec![h])
                }),
            );
        }
        _ => {}
    }
    let mut value = Value::Object(stream);
    prune_nulls(&mut value);
    value
}

fn prune_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, inner| !inner.is_null());
            for inner in map.values_mut() {
                prune_nulls(inner);
            }
        }
        Value::Array(items) => {
            for item in items {
                prune_nulls(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Protocol, RealityOptions, TlsOptions, Transport};

    fn vless() -> Outbound {
        Outbound {
            protocol: Protocol::Vless,
            server: "edge.example.com".into(),
            port: 443,
            name: "edge".into(),
            uuid: Some("6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70".into()),
            flow: Some("xtls-rprx-vision".into()),
            alter_id: None,
            security: None,
            method: None,
            password: None,
            username: None,
            plugin: None,
            plugin_opts: None,
            tls: Some(TlsOptions {
                server_name: Some("edge.example.com".into()),
                fingerprint: Some("chrome".into()),
                reality: Some(RealityOptions {
                    public_key: "pk".into(),
                    short_id: Some("ab".into()),
                    spider_x: None,
                }),
                ..Default::default()
            }),
            transport: Some(Transport::Tcp),
        }
    }

    #[test]
    fn sing_box_test_config_has_api_and_route() {
        let config = test_config(CoreKind::SingBox, &vless(), 18080, 19090, "s").unwrap();
        assert_eq!(config["outbounds"][0]["type"], "vless");
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(config["route"]["rules"][0]["outbound"], "proxy");
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"],
            "127.0.0.1:19090"
        );
        assert_eq!(config["outbounds"][0]["tls"]["reality"]["public_key"], "pk");
    }

    #[test]
    fn xray_test_config_uses_vnext_and_reality() {
        let config = test_config(CoreKind::Xray, &vless(), 18080, 19090, "s").unwrap();
        assert_eq!(config["outbounds"][0]["protocol"], "vless");
        assert_eq!(
            config["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"],
            "xtls-rprx-vision"
        );
        assert_eq!(config["outbounds"][0]["streamSettings"]["security"], "reality");
    }

    #[test]
    fn internal_pool_has_one_bridge_per_config() {
        let config = pool_config(
            CoreKind::SingBox,
            &[(1, vless()), (2, vless())],
            &PoolLayout::Internal {
                bridge_ports: vec![20001, 20002],
            },
            21000,
            "s",
        )
        .unwrap();
        assert_eq!(config["inbounds"].as_array().unwrap().len(), 2);
        assert_eq!(config["route"]["rules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn upstream_pool_exposes_http_and_socks_per_config() {
        let config = pool_config(
            CoreKind::Xray,
            &[(1, vless())],
            &PoolLayout::Upstream {
                listeners: vec![PoolListener {
                    config_id: 1,
                    http_port: 22001,
                    socks_port: 22002,
                }],
            },
            21000,
            "s",
        )
        .unwrap();
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 3);
        assert!(inbounds.iter().any(|i| i["protocol"] == "http"));
        assert!(inbounds.iter().all(|i| i["listen"] == "127.0.0.1"));
    }

    #[test]
    fn no_listener_ever_binds_wildcard() {
        let config = pool_config(
            CoreKind::SingBox,
            &[(1, vless())],
            &PoolLayout::Upstream {
                listeners: vec![PoolListener {
                    config_id: 1,
                    http_port: 22001,
                    socks_port: 22002,
                }],
            },
            21000,
            "s",
        )
        .unwrap();
        let text = serde_json::to_string(&config).unwrap();
        assert!(!text.contains("0.0.0.0"));
    }
}
