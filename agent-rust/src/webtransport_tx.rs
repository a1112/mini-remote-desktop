use crate::quic_tx::QuicAu;
use anyhow::{Context, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, SendStream, ServerConfig};

#[derive(Clone, Debug)]
pub struct WebTransportAdvert {
    pub url: String,
    pub alpn: String,
    pub cert_fingerprint_sha256: Vec<u8>,
}

fn unix_time_us() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_micros().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

fn default_sans() -> Vec<String> {
    if let Ok(raw) = std::env::var("AGENT_WEBTRANSPORT_SAN") {
        let v: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ]
}

fn advertise_url(port: u16) -> String {
    if let Ok(v) = std::env::var("AGENT_WEBTRANSPORT_URL") {
        let s = v.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let host = std::env::var("AGENT_WEBTRANSPORT_ADVERTISE_HOST")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let path = std::env::var("AGENT_WEBTRANSPORT_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/mrd".to_string());
    format!("https://{}:{}{}", host, port, path)
}

async fn handle_incoming_session(
    rx: &mut mpsc::Receiver<QuicAu>,
    incoming: IncomingSession,
) -> Result<bool> {
    let session_request = incoming
        .await
        .context("webtransport incoming await failed")?;
    let path = session_request.path().to_string();
    if path != "/mrd" && path != "/" {
        warn!(path = %path, "webtransport request path not allowed");
        session_request.not_found().await;
        return Ok(false);
    }

    let connection = session_request
        .accept()
        .await
        .context("webtransport session accept failed")?;
    info!(remote = %connection.remote_address(), "webtransport client connected");

    let mut stream = open_send_stream(&connection).await?;

    while let Some(frame) = rx.recv().await {
        let len = frame.payload.len() as u32;
        let tx_us = if frame.tx_unix_us == 0 {
            unix_time_us()
        } else {
            frame.tx_unix_us
        };
        // Reuse the existing wire format for compatibility:
        // u32 len + u64 seq + u64 tx_unix_us + payload
        // Here seq is monotonic timestamp-based surrogate.
        let seq = tx_us;
        if stream.write_u32(len).await.is_err()
            || stream.write_u64(seq).await.is_err()
            || stream.write_u64(tx_us).await.is_err()
            || stream.write_all(frame.payload.as_ref()).await.is_err()
        {
            warn!("webtransport stream write failed, waiting for reconnect");
            return Ok(false);
        }
    }
    Ok(true)
}

async fn open_send_stream(connection: &wtransport::Connection) -> Result<SendStream> {
    let mut last_error = String::new();
    for _ in 0..20 {
        match connection.open_uni().await {
            Ok(opening) => match opening.await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    last_error = format!("open_uni failed: {e}");
                }
            },
            Err(e) => {
                last_error = format!("open_uni init failed: {e}");
            }
        }

        match connection.open_bi().await {
            Ok(opening) => match opening.await {
                Ok((send_stream, _recv_stream)) => return Ok(send_stream),
                Err(e) => {
                    last_error = format!("open_bi failed: {e}");
                }
            },
            Err(e) => {
                last_error = format!("open_bi init failed: {e}");
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(anyhow::anyhow!(
        "webtransport open stream failed after retries: {}",
        last_error
    ))
}

pub fn start_webtransport_sender(
    bind_addr: SocketAddr,
) -> Result<(WebTransportAdvert, mpsc::Sender<QuicAu>)> {
    let sans = default_sans();
    let identity =
        Identity::self_signed(sans).context("webtransport self-signed identity failed")?;
    let cert_hash = identity
        .certificate_chain()
        .as_slice()
        .first()
        .map(|c| c.hash().as_ref().to_vec())
        .context("webtransport certificate hash missing")?;

    let cfg = ServerConfig::builder()
        .with_bind_address(SocketAddr::new(
            match bind_addr.ip() {
                IpAddr::V4(v4) => IpAddr::V4(v4),
                IpAddr::V6(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            },
            bind_addr.port(),
        ))
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let endpoint = Endpoint::server(cfg).context("webtransport endpoint bind failed")?;
    let local_port = endpoint
        .local_addr()
        .context("webtransport read local addr failed")?
        .port();

    let advert = WebTransportAdvert {
        url: advertise_url(local_port),
        alpn: std::env::var("AGENT_WEBTRANSPORT_ALPN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "h3".to_string()),
        cert_fingerprint_sha256: cert_hash,
    };

    let queue = std::env::var("AGENT_WEBTRANSPORT_QUEUE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(64)
        .clamp(1, 512);
    let (tx, mut rx) = mpsc::channel::<QuicAu>(queue);

    tokio::spawn(async move {
        loop {
            let incoming = endpoint.accept().await;
            match handle_incoming_session(&mut rx, incoming).await {
                Ok(true) => break,
                Ok(false) => continue,
                Err(e) => {
                    error!(error = %e, "webtransport session loop failed");
                }
            }
            if rx.is_closed() {
                break;
            }
        }
    });

    Ok((advert, tx))
}
