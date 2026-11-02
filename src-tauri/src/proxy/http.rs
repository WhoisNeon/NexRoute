use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{
    parse_authority, pick_and_connect, read_head, rewrite_forward_request, Gateway,
    HANDSHAKE_TIMEOUT,
};

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
    let (head, leftover) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_head(&mut client))
        .await
    {
        Ok(Ok(parts)) => parts,
        Ok(Err(error)) => {
            write_error(&mut client, 400, &error.to_string()).await;
            return Ok(());
        }
        Err(_) => {
            write_error(&mut client, 408, "request timeout").await;
            return Ok(());
        }
    };

    let text = String::from_utf8_lossy(&head);
    let request_line = text.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = match parse_authority(target) {
            Ok(target) => target,
            Err(error) => {
                write_error(&mut client, 400, &error).await;
                return Ok(());
            }
        };
        let mut upstream = match pick_and_connect(&gateway, &host, port).await {
            Ok(stream) => stream,
            Err(error) => {
                write_error(&mut client, 502, &format!("upstream unavailable: {error}")).await;
                return Ok(());
            }
        };
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let _ = client.flush().await;
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        return Ok(());
    }

    let (host, port, rewritten) = match rewrite_forward_request(&head) {
        Ok(parts) => parts,
        Err(error) => {
            write_error(&mut client, 400, &error).await;
            return Ok(());
        }
    };
    let mut upstream = match pick_and_connect(&gateway, &host, port).await {
        Ok(stream) => stream,
        Err(error) => {
            write_error(
                &mut client,
                502,
                &format!("upstream unavailable: {error}"),
            )
            .await;
            return Ok(());
        }
    };
    upstream.write_all(&rewritten).await?;
    if !leftover.is_empty() {
        upstream.write_all(&leftover).await?;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

async fn write_error(client: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        400 => "Bad Request",
        408 => "Request Timeout",
        _ => "Bad Gateway",
    };
    let body = format!("{message}\n");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = client.write_all(response.as_bytes()).await;
    let _ = client.shutdown().await;
}
