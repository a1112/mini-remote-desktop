use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use quinn::{Endpoint, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Clone, Debug)]
pub struct QuicServerAdvert {
    pub addr: String,
    pub server_name: String,
    pub cert_der_base64: String,
}

#[derive(Clone, Debug)]
pub struct QuicAu {
    pub payload: Vec<u8>,
    pub tx_unix_us: u64,
}

fn unix_time_us() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_micros().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

pub fn start_quic_sender(
    bind_addr: SocketAddr,
) -> Result<(QuicServerAdvert, mpsc::Sender<QuicAu>)> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = rcgen::generate_simple_self_signed(vec!["agent-rust".to_string()])
        .context("quic: generate self-signed cert failed")?;
    let cert_der = cert.cert.der().to_vec();
    let key_der = cert.key_pair.serialize_der();

    let cert_chain = vec![CertificateDer::from(cert_der.clone())];
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key_der));
    let server_config = ServerConfig::with_single_cert(cert_chain, key)
        .context("quic: build server config failed")?;
    let endpoint =
        Endpoint::server(server_config, bind_addr).context("quic: bind endpoint failed")?;
    let local_addr = endpoint
        .local_addr()
        .context("quic: read local addr failed")?;
    let advertise_addr = std::env::var("AGENT_QUIC_ADVERTISE_ADDR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("127.0.0.1:{}", local_addr.port()));

    let advert = QuicServerAdvert {
        addr: advertise_addr,
        server_name: "agent-rust".to_string(),
        cert_der_base64: BASE64.encode(cert_der),
    };
    let queue = std::env::var("AGENT_QUIC_QUEUE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(64)
        .clamp(1, 512);
    let (tx, mut rx) = mpsc::channel::<QuicAu>(queue);

    tokio::spawn(async move {
        let mut seq: u64 = 0;
        let wire_debug = std::env::var("AGENT_QUIC_WIRE_DEBUG")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let mut wire_debug_left = 8usize;
        while let Some(incoming) = endpoint.accept().await {
            match incoming.await {
                Ok(conn) => {
                    info!(remote = %conn.remote_address(), "quic sender connected");
                    match conn.open_uni().await {
                        Ok(mut stream) => {
                            while let Some(frame) = rx.recv().await {
                                seq = seq.saturating_add(1);
                                let len = frame.payload.len() as u32;
                                if wire_debug && wire_debug_left > 0 {
                                    info!(
                                        seq,
                                        len,
                                        hash = format!("{:016x}", fnv1a64(frame.payload.as_slice())),
                                        tx_unix_us = frame.tx_unix_us,
                                        "quic wire tx frame"
                                    );
                                    wire_debug_left -= 1;
                                }
                                if stream.write_u32(len).await.is_err()
                                    || stream.write_u64(seq).await.is_err()
                                    || stream
                                        .write_u64(if frame.tx_unix_us == 0 {
                                            unix_time_us()
                                        } else {
                                            frame.tx_unix_us
                                        })
                                        .await
                                        .is_err()
                                    || stream.write_all(frame.payload.as_slice()).await.is_err()
                                {
                                    warn!("quic sender stream write failed, waiting for reconnect");
                                    break;
                                }
                            }
                            let _ = stream.finish();
                            if rx.is_closed() {
                                break;
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "quic sender open uni stream failed");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "quic sender accept failed");
                }
            }
        }
        drop(endpoint);
    });

    Ok((advert, tx))
}
