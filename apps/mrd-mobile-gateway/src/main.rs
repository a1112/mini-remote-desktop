//! Foreground mobile gateway: run in the interactive Windows session for desktop capture.

use anyhow::{Context, Result};
use std::{env, net::SocketAddr};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let bind: SocketAddr = env::var("MRD_MOBILE_GATEWAY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9534".to_string())
        .parse()
        .context("invalid MRD_MOBILE_GATEWAY_BIND")?;
    let router = mrd_mobile_gateway::router();
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .context("bind mobile gateway")?;
    tracing::info!("mobile gateway listening on {}", listener.local_addr()?);
    if !bind.ip().is_loopback() {
        let port = listener.local_addr()?.port();
        tokio::spawn(async move {
            run_discovery(port).await;
        });
    }
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("mobile gateway stopped")
}

async fn run_discovery(port: u16) {
    let socket = match tokio::net::UdpSocket::bind("0.0.0.0:9535").await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!("LAN discovery unavailable: {error}");
            return;
        }
    };
    let name = env::var("COMPUTERNAME").unwrap_or_else(|_| "Rdesk-PC".to_string());
    let mut buffer = [0u8; 64];
    loop {
        let Ok((count, peer)) = socket.recv_from(&mut buffer).await else {
            break;
        };
        let std::net::IpAddr::V4(ip) = peer.ip() else {
            continue;
        };
        let octets = ip.octets();
        let private = ip.is_loopback()
            || octets[0] == 10
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            || (octets[0] == 192 && octets[1] == 168);
        if !private {
            continue;
        }
        if let Some(reply) = mrd_mobile_gateway::discovery_reply(&buffer[..count], &name, port) {
            let _ = socket.send_to(reply.as_bytes(), peer).await;
        }
    }
}
