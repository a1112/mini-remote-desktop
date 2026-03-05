use crate::quic_tx::QuicAu;
use anyhow::{Context, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
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

fn encode_wire_header(len: u32, seq: u64, tx_us: u64) -> [u8; 20] {
    let mut hdr = [0_u8; 20];
    hdr[..4].copy_from_slice(&len.to_be_bytes());
    hdr[4..12].copy_from_slice(&seq.to_be_bytes());
    hdr[12..20].copy_from_slice(&tx_us.to_be_bytes());
    hdr
}

fn encode_wire_packet(
    scratch: &mut Vec<u8>,
    payload: &[u8],
    seq: u64,
    tx_us: u64,
) {
    scratch.clear();
    scratch.reserve(20 + payload.len());
    append_wire_packet(scratch, payload, seq, tx_us);
}

fn append_wire_packet(scratch: &mut Vec<u8>, payload: &[u8], seq: u64, tx_us: u64) {
    let header = encode_wire_header(payload.len() as u32, seq, tx_us);
    scratch.extend_from_slice(&header);
    scratch.extend_from_slice(payload);
}

fn default_batch_limits_by_mode(mode: &str) -> (usize, usize) {
    match mode {
        "latency" | "latency_first" => (1, 256 * 1024),
        "throughput" | "max" | "throughput_first" => (8, 1024 * 1024),
        _ => (4, 512 * 1024),
    }
}

fn load_batch_limits() -> (usize, usize) {
    let mode = std::env::var("AGENT_FPS_MODE")
        .ok()
        .unwrap_or_else(|| "balanced".to_string())
        .trim()
        .to_ascii_lowercase();
    let (default_frames, default_bytes) = default_batch_limits_by_mode(&mode);
    let max_frames = std::env::var("AGENT_WEBTRANSPORT_BATCH_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default_frames)
        .clamp(1, 64);
    let max_bytes = std::env::var("AGENT_WEBTRANSPORT_BATCH_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default_bytes)
        .clamp(4 * 1024, 8 * 1024 * 1024);
    (max_frames, max_bytes)
}

fn encode_wire_batch_packets(scratch: &mut Vec<u8>, frames: &[QuicAu], seq_start: u64) -> u64 {
    scratch.clear();
    let reserve: usize = frames.iter().map(|f| 20 + f.payload.len()).sum();
    scratch.reserve(reserve);
    let mut seq = seq_start;
    for frame in frames {
        append_wire_packet(
            scratch,
            frame.payload.as_slice(),
            seq,
            if frame.tx_unix_us == 0 {
                unix_time_us()
            } else {
                frame.tx_unix_us
            },
        );
        seq = seq.saturating_add(1);
    }
    seq
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
    let mut scratch = Vec::with_capacity(
        std::env::var("AGENT_WEBTRANSPORT_PACKET_RESERVE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(64 * 1024)
            .clamp(1024, 8 * 1024 * 1024),
    );
    let (max_batch_frames, max_batch_bytes) = load_batch_limits();
    let mut frame_batch: Vec<QuicAu> = Vec::with_capacity(max_batch_frames);
    let mut pending_frame: Option<QuicAu> = None;
    let mut seq: u64 = 0;

    loop {
        let first = if let Some(frame) = pending_frame.take() {
            frame
        } else {
            match rx.recv().await {
                Some(frame) => frame,
                None => break,
            }
        };
        frame_batch.clear();
        frame_batch.push(first);
        let mut batch_wire_bytes = 20 + frame_batch[0].payload.len();
        while frame_batch.len() < max_batch_frames {
            match rx.try_recv() {
                Ok(frame) => {
                    let wire_bytes = 20 + frame.payload.len();
                    if batch_wire_bytes.saturating_add(wire_bytes) > max_batch_bytes {
                        pending_frame = Some(frame);
                        break;
                    }
                    batch_wire_bytes = batch_wire_bytes.saturating_add(wire_bytes);
                    frame_batch.push(frame);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        // Reuse the existing wire format for compatibility:
        // u32 len + u64 seq + u64 tx_unix_us + payload.
        seq = encode_wire_batch_packets(&mut scratch, frame_batch.as_slice(), seq.saturating_add(1));
        if stream.write_all(scratch.as_slice()).await.is_err() {
            warn!(
                len = scratch.len() as u64,
                batch = frame_batch.len(),
                seq,
                "webtransport stream write failed, waiting for reconnect"
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quic_tx::QuicAu;

    fn parse_wire_packet(pkt: &[u8]) -> Option<(usize, u64, u64, &[u8])> {
        if pkt.len() < 20 {
            return None;
        }
        let len = u32::from_be_bytes(pkt[..4].try_into().ok()?) as usize;
        if pkt.len() < 20 + len {
            return None;
        }
        let seq = u64::from_be_bytes(pkt[4..12].try_into().ok()?);
        let tx_us = u64::from_be_bytes(pkt[12..20].try_into().ok()?);
        Some((20 + len, seq, tx_us, &pkt[20..20 + len]))
    }

    #[test]
    fn wire_header_layout_is_stable() {
        let hdr = encode_wire_header(0x0102_0304, 0x0102_0304_0506_0708, 0x1112_1314_1516_1718);
        assert_eq!(&hdr[..4], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(
            &hdr[4..12],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(
            &hdr[12..20],
            &[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18]
        );
    }

    #[test]
    fn wire_packet_layout_is_stable() {
        let mut scratch = Vec::new();
        let payload = [0xAA, 0xBB, 0xCC];
        encode_wire_packet(&mut scratch, &payload, 7, 9);
        let pkt = scratch.as_slice();
        assert_eq!(pkt.len(), 23);
        assert_eq!(&pkt[..4], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(&pkt[4..12], &[0, 0, 0, 0, 0, 0, 0, 7]);
        assert_eq!(&pkt[12..20], &[0, 0, 0, 0, 0, 0, 0, 9]);
        assert_eq!(&pkt[20..], &payload);
    }

    #[test]
    fn wire_batch_layout_is_stable() {
        let frames = vec![
            QuicAu {
                payload: vec![0xAA, 0xBB],
                tx_unix_us: 101,
            },
            QuicAu {
                payload: vec![0xCC],
                tx_unix_us: 102,
            },
        ];
        let mut scratch = Vec::new();
        let next_seq = encode_wire_batch_packets(&mut scratch, &frames, 7);
        assert_eq!(next_seq, 9);

        let (used_a, seq_a, tx_a, payload_a) = parse_wire_packet(&scratch).expect("packet a");
        assert_eq!(seq_a, 7);
        assert_eq!(tx_a, 101);
        assert_eq!(payload_a, [0xAA, 0xBB]);

        let (used_b, seq_b, tx_b, payload_b) =
            parse_wire_packet(&scratch[used_a..]).expect("packet b");
        assert_eq!(seq_b, 8);
        assert_eq!(tx_b, 102);
        assert_eq!(payload_b, [0xCC]);
        assert_eq!(used_a + used_b, scratch.len());
    }

    #[test]
    fn batch_limits_follow_fps_mode() {
        unsafe {
            std::env::set_var("AGENT_FPS_MODE", "latency");
            std::env::remove_var("AGENT_WEBTRANSPORT_BATCH_FRAMES");
            std::env::remove_var("AGENT_WEBTRANSPORT_BATCH_BYTES");
        }
        assert_eq!(load_batch_limits(), (1, 256 * 1024));
        unsafe {
            std::env::set_var("AGENT_FPS_MODE", "throughput");
        }
        assert_eq!(load_batch_limits(), (8, 1024 * 1024));
    }
}
