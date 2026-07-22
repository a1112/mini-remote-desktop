use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use quinn::{ClientConfig, Endpoint};
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct AudioQuicConnectInfo {
    pub addr: String,
    pub server_name: String,
    pub cert_der_base64: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub sequence: u64,
    pub capture_unix_us: u64,
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_samples: u16,
    pub payload: Vec<u8>,
}

pub async fn connect_audio_quic_receiver(
    info: &AudioQuicConnectInfo,
) -> Result<Arc<Mutex<mpsc::Receiver<AudioFrame>>>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let addr: SocketAddr = info
        .addr
        .parse()
        .with_context(|| format!("parse audio quic addr failed: {}", info.addr))?;
    let cert_der = BASE64
        .decode(info.cert_der_base64.as_bytes())
        .context("decode audio quic cert base64 failed")?;

    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(cert_der))
        .context("add audio quic root cert failed")?;
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .context("build audio quic client tls config failed")?,
    ));

    let mut endpoint = Endpoint::client(
        "0.0.0.0:0"
            .parse()
            .context("parse audio client bind failed")?,
    )
    .context("create audio quic client endpoint failed")?;
    endpoint.set_default_client_config(client_config);

    let conn = endpoint
        .connect(addr, &info.server_name)
        .context("audio quic connect setup failed")?
        .await
        .context("audio quic connect failed")?;
    info!(remote = %addr, codec = %info.codec, sample_rate = info.sample_rate, channels = info.channels, "audio quic receiver connected");

    let rx_queue = std::env::var("MRD_AUDIO_RX_QUEUE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(128)
        .clamp(16, 1024);
    let (tx, rx) = mpsc::channel::<AudioFrame>(rx_queue);

    tokio::spawn(async move {
        loop {
            let mut stream = match conn.accept_uni().await {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "audio quic accept uni stream failed");
                    break;
                }
            };
            loop {
                let len = match stream.read_u32().await {
                    Ok(v) => v as usize,
                    Err(_) => break,
                };
                let sequence = match stream.read_u64().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let capture_unix_us = match stream.read_u64().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let codec = match stream.read_u8().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let sample_rate = match stream.read_u32().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let channels = match stream.read_u16().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let frame_samples = match stream.read_u16().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut payload = vec![0_u8; len];
                if let Err(e) = stream.read_exact(&mut payload).await {
                    error!(error = %e, "audio quic read frame payload failed");
                    break;
                }
                if tx
                    .try_send(AudioFrame {
                        sequence,
                        capture_unix_us,
                        codec,
                        sample_rate,
                        channels,
                        frame_samples,
                        payload,
                    })
                    .is_err()
                {
                    // Drop stale audio when playback is saturated.
                }
            }
        }
        drop(endpoint);
    });

    Ok(Arc::new(Mutex::new(rx)))
}
