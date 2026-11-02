use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{pick_and_connect, Gateway, HANDSHAKE_TIMEOUT};

const SOCKS_VERSION: u8 = 5;
const NO_AUTH: u8 = 0;
const CMD_CONNECT: u8 = 1;

pub async fn run(listener: TcpListener, gateway: Arc<Gateway>) {
    loop {
        match listener.accept().await {
            Ok((client, _)) => {
                let gateway = gateway.clone();
                tokio::spawn(async move {
                    let _ = handle(client, gateway).await;
                });
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }
}

async fn handle(mut client: TcpStream, gateway: Arc<Gateway>) -> std::io::Result<()> {
    let _ = client.set_nodelay(true);
    let target = match tokio::time::timeout(HANDSHAKE_TIMEOUT, negotiate(&mut client)).await {
        Ok(Ok(target)) => target,
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            let _ = client
                .write_all(&[SOCKS_VERSION, 1, 0, 1, 0, 0, 0, 0, 0, 0])
                .await;
            return Ok(());
        }
    };

    let (host, port) = target;
    match pick_and_connect(&gateway, &host, port).await {
        Ok(mut upstream) => {
            client
                .write_all(&[SOCKS_VERSION, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await?;
            let _ = client.flush().await;
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            Ok(())
        }
        Err(_) => {
            client
                .write_all(&[SOCKS_VERSION, 1, 0, 1, 0, 0, 0, 0, 0, 0])
                .await?;
            Ok(())
        }
    }
}

/// Perform the SOCKS5 no-auth negotiation and return the CONNECT target.
async fn negotiate(client: &mut TcpStream) -> std::io::Result<(String, u16)> {
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await?;
    if greeting[0] != SOCKS_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unsupported SOCKS version",
        ));
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&NO_AUTH) {
        client.write_all(&[SOCKS_VERSION, 0xFF]).await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "client requires authentication",
        ));
    }
    client.write_all(&[SOCKS_VERSION, NO_AUTH]).await?;

    let mut header = [0u8; 4];
    client.read_exact(&mut header).await?;
    if header[0] != SOCKS_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid SOCKS5 request",
        ));
    }
    if header[1] != CMD_CONNECT {
        client
            .write_all(&[SOCKS_VERSION, 7, 0, 1, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "only CONNECT is supported",
        ));
    }

    let mut host = String::new();
    match header[3] {
        1 => {
            let mut octets = [0u8; 4];
            client.read_exact(&mut octets).await?;
            host = std::net::Ipv4Addr::from(octets).to_string();
        }
        4 => {
            let mut octets = [0u8; 16];
            client.read_exact(&mut octets).await?;
            host = std::net::Ipv6Addr::from(octets).to_string();
        }
        3 => {
            let mut length = [0u8; 1];
            client.read_exact(&mut length).await?;
            let mut bytes = vec![0u8; length[0] as usize];
            client.read_exact(&mut bytes).await?;
            host = String::from_utf8(bytes).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "SOCKS5 domain is not UTF-8",
                )
            })?;
        }
        _ => {
            client
                .write_all(&[SOCKS_VERSION, 8, 0, 1, 0, 0, 0, 0, 0, 0])
                .await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported SOCKS5 address type",
            ));
        }
    }
    let mut port_bytes = [0u8; 2];
    client.read_exact(&mut port_bytes).await?;
    let port = u16::from_be_bytes(port_bytes);
    if host.is_empty() || port == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid SOCKS5 target",
        ));
    }
    Ok((host, port))
}
