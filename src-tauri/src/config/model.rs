use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    #[serde(rename = "vless")]
    Vless,
    #[serde(rename = "vmess")]
    Vmess,
    #[serde(rename = "ss")]
    Shadowsocks,
    #[serde(rename = "socks")]
    Socks,
    #[serde(rename = "http")]
    Http,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vless => "vless",
            Self::Vmess => "vmess",
            Self::Shadowsocks => "ss",
            Self::Socks => "socks",
            Self::Http => "http",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vless => "VLESS",
            Self::Vmess => "VMess",
            Self::Shadowsocks => "Shadowsocks",
            Self::Socks => "SOCKS5",
            Self::Http => "HTTP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoreKind {
    SingBox,
    Xray,
}

impl CoreKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "Xray-core",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PoolMode {
    Internal,
    Upstream,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealityOptions {
    pub public_key: String,
    #[serde(default)]
    pub short_id: Option<String>,
    #[serde(default)]
    pub spider_x: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TlsOptions {
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub alpn: Vec<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub reality: Option<RealityOptions>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    Tcp,
    Ws {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Grpc {
        #[serde(default)]
        service_name: Option<String>,
    },
    Http {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        host: Option<String>,
    },
}

impl Transport {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Ws { .. } => "ws",
            Self::Grpc { .. } => "grpc",
            Self::Http { .. } => "http",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outbound {
    pub protocol: Protocol,
    pub server: String,
    pub port: u16,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub flow: Option<String>,
    #[serde(default)]
    pub alter_id: Option<u32>,
    #[serde(default)]
    pub security: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default)]
    pub plugin_opts: Option<String>,
    #[serde(default)]
    pub tls: Option<TlsOptions>,
    #[serde(default)]
    pub transport: Option<Transport>,
}

impl Outbound {
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}:{}", self.server, self.port)
        } else {
            self.name.clone()
        }
    }

    pub fn transport_kind(&self) -> &'static str {
        self.transport.as_ref().map(Transport::kind).unwrap_or("tcp")
    }

    /// Reject protocol/feature combinations the selected core does not implement.
    pub fn validate_for(&self, core: CoreKind) -> Result<(), String> {
        if self.server.trim().is_empty() {
            return Err("server is empty".into());
        }
        if self.port == 0 {
            return Err("port must be between 1 and 65535".into());
        }

        match self.protocol {
            Protocol::Vless | Protocol::Vmess => {
                let uuid = self.uuid.as_deref().unwrap_or_default();
                if uuid.is_empty() {
                    return Err("missing user id".into());
                }
                if let Some(flow) = self.flow.as_deref() {
                    if !flow.is_empty() {
                        if self.protocol != Protocol::Vless {
                            return Err("flow is only valid for VLESS".into());
                        }
                        if flow != "xtls-rprx-vision" {
                            return Err(format!("unsupported VLESS flow: {flow}"));
                        }
                        let tls = self.tls.as_ref().ok_or("VLESS flow requires TLS")?;
                        if tls.reality.is_none() && !tls.insecure {
                            if tls.server_name.is_none() || tls.server_name.as_deref() == Some("") {
                                return Err("VLESS flow requires TLS with a server name".into());
                            }
                        }
                        if !matches!(self.transport, None | Some(Transport::Tcp)) {
                            return Err("VLESS flow is only supported over TCP".into());
                        }
                    }
                }
            }
            Protocol::Shadowsocks => {
                if self.method.as_deref().unwrap_or_default().is_empty() {
                    return Err("shadowsocks method is missing".into());
                }
                if self.password.as_deref().unwrap_or_default().is_empty() {
                    return Err("shadowsocks password is missing".into());
                }
                if let Some(plugin) = self.plugin.as_deref() {
                    if !plugin.is_empty() {
                        if core == CoreKind::Xray {
                            return Err(
                                "Xray-core does not support Shadowsocks SIP003 plugins; use sing-box"
                                    .into(),
                            );
                        }
                        if !matches!(plugin, "obfs-local" | "simple-obfs" | "v2ray-plugin") {
                            return Err(format!("unsupported shadowsocks plugin: {plugin}"));
                        }
                    }
                }
            }
            Protocol::Socks | Protocol::Http => {}
        }

        if let Some(tls) = &self.tls {
            if let Some(reality) = &tls.reality {
                if reality.public_key.trim().is_empty() {
                    return Err("reality public key is missing".into());
                }
                if tls.server_name.as_deref().unwrap_or_default().is_empty() {
                    return Err("reality requires a server name (sni)".into());
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Outbound {
        Outbound {
            protocol: Protocol::Vless,
            server: "example.com".into(),
            port: 443,
            name: "test".into(),
            uuid: Some("6f1c2c4e-1f6c-4a1d-9a2e-1b3c4d5e6f70".into()),
            flow: None,
            alter_id: None,
            security: None,
            method: None,
            password: None,
            username: None,
            plugin: None,
            plugin_opts: None,
            tls: Some(TlsOptions {
                server_name: Some("example.com".into()),
                ..Default::default()
            }),
            transport: None,
        }
    }

    #[test]
    fn vision_requires_tcp() {
        let mut out = base();
        out.flow = Some("xtls-rprx-vision".into());
        out.transport = Some(Transport::Ws {
            path: Some("/ws".into()),
            headers: BTreeMap::new(),
        });
        assert!(out.validate_for(CoreKind::SingBox).is_err());
    }

    #[test]
    fn ss_plugins_are_sing_box_only() {
        let mut out = base();
        out.protocol = Protocol::Shadowsocks;
        out.uuid = None;
        out.method = Some("aes-128-gcm".into());
        out.password = Some("pw".into());
        out.plugin = Some("obfs-local".into());
        assert!(out.validate_for(CoreKind::SingBox).is_ok());
        assert!(out.validate_for(CoreKind::Xray).is_err());
    }
}
