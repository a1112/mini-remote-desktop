//! Foreground mobile gateway: run in the interactive Windows session for desktop capture.

use anyhow::{bail, Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::generate_simple_self_signed;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::{env, net::SocketAddr};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let bind: SocketAddr = env::var("MRD_MOBILE_GATEWAY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9534".to_string())
        .parse()
        .context("invalid MRD_MOBILE_GATEWAY_BIND")?;
    let pairing_token = env::var("MRD_MOBILE_GATEWAY_PAIRING_TOKEN")
        .context("MRD_MOBILE_GATEWAY_PAIRING_TOKEN must be set")?;
    if pairing_token.as_bytes().len() < 32 {
        bail!("MRD_MOBILE_GATEWAY_PAIRING_TOKEN must contain at least 32 bytes");
    }
    let router = mrd_mobile_gateway::router();
    let configured_cert = env::var("MRD_MOBILE_GATEWAY_CERT_PEM").ok();
    let configured_key = env::var("MRD_MOBILE_GATEWAY_KEY_PEM").ok();
    let (cert_chain, key_der) = match (configured_cert, configured_key) {
        (Some(cert), Some(key)) => {
            let mut cert_reader = Cursor::new(cert.as_bytes());
            let cert_chain = rustls_pemfile::certs(&mut cert_reader)
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("parse mobile gateway TLS certificate")?
                .into_iter()
                .map(|certificate| certificate.to_vec())
                .collect::<Vec<_>>();
            let mut key_reader = Cursor::new(key.as_bytes());
            let key_der = rustls_pemfile::private_key(&mut key_reader)
                .context("parse mobile gateway TLS private key")?
                .map(|key| key.secret_der().to_vec())
                .context("mobile gateway TLS private key is missing")?;
            if cert_chain.is_empty() {
                bail!("MRD_MOBILE_GATEWAY_CERT_PEM contains no certificates");
            }
            (cert_chain, key_der)
        }
        (None, None) => {
            let certificate = generate_simple_self_signed(vec!["localhost".to_string()])
                .context("generate mobile gateway TLS certificate")?;
            (
                vec![certificate.cert.der().to_vec()],
                certificate.key_pair.serialize_der(),
            )
        }
        _ => {
            bail!("MRD_MOBILE_GATEWAY_CERT_PEM and MRD_MOBILE_GATEWAY_KEY_PEM must be set together")
        }
    };
    let fingerprint = Sha256::digest(&cert_chain[0]);
    let fingerprint = fingerprint
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let tls = RustlsConfig::from_der(cert_chain, key_der)
        .await
        .context("build mobile gateway TLS configuration")?;
    tracing::info!("mobile gateway listening with TLS on {}", bind);
    if !bind.ip().is_loopback() {
        let port = bind.port();
        let fingerprint = fingerprint.clone();
        tokio::spawn(async move {
            run_discovery(port, &fingerprint).await;
        });
    }
    axum_server::bind_rustls(bind, tls)
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("mobile gateway stopped")
}

async fn run_discovery(port: u16, fingerprint: &str) {
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
        if let Some(reply) =
            mrd_mobile_gateway::discovery_reply(&buffer[..count], &name, port, fingerprint)
        {
            let _ = socket.send_to(reply.as_bytes(), peer).await;
        }
    }
}
