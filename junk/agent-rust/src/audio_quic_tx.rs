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

#[derive(Clone, Debug)]
pub struct AudioQuicServerAdvert {
    pub addr: String,
    pub server_name: String,
    pub cert_der_base64: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug)]
pub struct AudioQuicPacket {
    pub sequence: u64,
    pub capture_unix_us: u64,
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_samples: u16,
    pub payload: Vec<u8>,
}

fn unix_time_us() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_micros().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

pub fn start_audio_quic_sender(
    bind_addr: SocketAddr,
) -> Result<(AudioQuicServerAdvert, mpsc::Sender<AudioQuicPacket>)> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = rcgen::generate_simple_self_signed(vec!["agent-rust-audio".to_string()])
        .context("audio quic: generate self-signed cert failed")?;
    let cert_der = cert.cert.der().to_vec();
    let key_der = cert.key_pair.serialize_der();

    let cert_chain = vec![CertificateDer::from(cert_der.clone())];
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key_der));
    let server_config = ServerConfig::with_single_cert(cert_chain, key)
        .context("audio quic: build server config failed")?;
    let endpoint =
        Endpoint::server(server_config, bind_addr).context("audio quic: bind endpoint failed")?;
    let local_addr = endpoint
        .local_addr()
        .context("audio quic: read local addr failed")?;
    let advertise_addr = std::env::var("AGENT_AUDIO_QUIC_ADVERTISE_ADDR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("127.0.0.1:{}", local_addr.port()));

    let advert = AudioQuicServerAdvert {
        addr: advertise_addr,
        server_name: "agent-rust-audio".to_string(),
        cert_der_base64: BASE64.encode(cert_der),
        codec: "opus".to_string(),
        sample_rate: 48_000,
        channels: 2,
    };

    let queue = std::env::var("AGENT_AUDIO_QUIC_QUEUE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(128)
        .clamp(8, 1024);
    let (tx, mut rx) = mpsc::channel::<AudioQuicPacket>(queue);

    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            match incoming.await {
                Ok(conn) => {
                    info!(remote = %conn.remote_address(), "audio quic sender connected");
                    match conn.open_uni().await {
                        Ok(mut stream) => {
                            while let Some(pkt) = rx.recv().await {
                                let len = pkt.payload.len() as u32;
                                if stream.write_u32(len).await.is_err()
                                    || stream.write_u64(pkt.sequence).await.is_err()
                                    || stream
                                        .write_u64(if pkt.capture_unix_us == 0 {
                                            unix_time_us()
                                        } else {
                                            pkt.capture_unix_us
                                        })
                                        .await
                                        .is_err()
                                    || stream.write_u8(pkt.codec).await.is_err()
                                    || stream.write_u32(pkt.sample_rate).await.is_err()
                                    || stream.write_u16(pkt.channels).await.is_err()
                                    || stream.write_u16(pkt.frame_samples).await.is_err()
                                    || stream.write_all(pkt.payload.as_slice()).await.is_err()
                                {
                                    warn!(
                                        "audio quic sender stream write failed, waiting for reconnect"
                                    );
                                    break;
                                }
                            }
                            let _ = stream.finish();
                            if rx.is_closed() {
                                break;
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "audio quic sender open uni stream failed");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "audio quic sender accept failed");
                }
            }
        }
        drop(endpoint);
    });

    Ok((advert, tx))
}
