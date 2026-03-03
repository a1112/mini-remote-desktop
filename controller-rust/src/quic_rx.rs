use crate::webrtc::peer::VideoFrame;
use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use quinn::{ClientConfig, Endpoint};
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info, warn};

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn looks_like_annexb(buf: &[u8]) -> bool {
    buf.len() >= 4
        && ((buf[0] == 0 && buf[1] == 0 && buf[2] == 1)
            || (buf[0] == 0 && buf[1] == 0 && buf[2] == 0 && buf[3] == 1))
}

fn is_probable_single_nal(buf: &[u8]) -> bool {
    if buf.is_empty() {
        return false;
    }
    let nal_type = buf[0] & 0x1f;
    (1..=23).contains(&nal_type)
}

fn avcc_to_annexb_with_len(buf: &[u8], len_size: usize) -> Option<Vec<u8>> {
    if len_size == 0 || len_size > 4 || buf.len() <= len_size {
        return None;
    }
    let mut i = 0usize;
    let mut out = Vec::with_capacity(buf.len() + 64);
    let mut nals = 0usize;
    while i + len_size <= buf.len() {
        let mut n = 0usize;
        for b in &buf[i..i + len_size] {
            n = (n << 8) | (*b as usize);
        }
        i += len_size;
        if n == 0 || i + n > buf.len() {
            return None;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&buf[i..i + n]);
        i += n;
        nals += 1;
    }
    if i == buf.len() && nals > 0 {
        Some(out)
    } else {
        None
    }
}

fn contains_idr_annexb(buf: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 4 < buf.len() {
        let sc_len = if i + 3 < buf.len() && buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            3
        } else if i + 4 < buf.len()
            && buf[i] == 0
            && buf[i + 1] == 0
            && buf[i + 2] == 0
            && buf[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };
        let hdr = i + sc_len;
        if hdr < buf.len() && (buf[hdr] & 0x1f) == 5 {
            return true;
        }
        i = hdr.saturating_add(1);
    }
    false
}

fn to_annexb_if_needed(buf: &[u8]) -> Bytes {
    // Already Annex-B start code prefixed.
    if looks_like_annexb(buf) {
        return Bytes::copy_from_slice(buf);
    }

    // Try AVCC length-prefixed NAL units (len-size 4/2/1 are common).
    for len_size in [4usize, 2, 1] {
        if let Some(v) = avcc_to_annexb_with_len(buf, len_size) {
            return Bytes::from(v);
        }
    }

    // Some encoders return a raw single NALU without start code.
    if is_probable_single_nal(buf) {
        let mut out = Vec::with_capacity(buf.len() + 4);
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(buf);
        return Bytes::from(out);
    }

    Bytes::copy_from_slice(buf)
}

#[derive(Clone, Debug)]
pub struct QuicConnectInfo {
    pub addr: String,
    pub server_name: String,
    pub cert_der_base64: String,
}

pub async fn connect_quic_receiver(
    info: &QuicConnectInfo,
) -> Result<Arc<Mutex<mpsc::Receiver<VideoFrame>>>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let addr: SocketAddr = info
        .addr
        .parse()
        .with_context(|| format!("parse quic addr failed: {}", info.addr))?;
    let cert_der = BASE64
        .decode(info.cert_der_base64.as_bytes())
        .context("decode quic cert base64 failed")?;

    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(cert_der))
        .context("add quic root cert failed")?;
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .context("build quic client tls config failed")?,
    ));

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().context("parse client bind failed")?)
        .context("create quic client endpoint failed")?;
    endpoint.set_default_client_config(client_config);

    let conn = endpoint
        .connect(addr, &info.server_name)
        .context("quic connect setup failed")?
        .await
        .context("quic connect failed")?;
    info!(remote = %addr, "quic receiver connected");

    let rx_queue = std::env::var("MRD_QUIC_RX_QUEUE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0 && *v <= 1024)
        .unwrap_or(8);
    let (tx, rx) = mpsc::channel::<VideoFrame>(rx_queue);
    tokio::spawn(async move {
        let wire_debug = std::env::var("MRD_QUIC_WIRE_DEBUG")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let mut wire_debug_left = 8usize;
        let dump_dir = std::env::var("MRD_QUIC_DUMP_DIR").ok();
        let mut dump_left = 3usize;
        loop {
            let mut stream = match conn.accept_uni().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "quic accept uni stream failed");
                    break;
                }
            };
            // New stream may start in the middle of a GOP; wait for IDR to avoid
            // feeding undecodable inter frames that inflate startup latency.
            let mut waiting_for_keyframe = true;
            loop {
                let len = match stream.read_u32().await {
                    Ok(v) => v as usize,
                    Err(_) => break,
                };
                let seq = match stream.read_u64().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let tx_unix_us = match stream.read_u64().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut buf = vec![0_u8; len];
                if let Err(e) = stream.read_exact(&mut buf).await {
                    error!(error = %e, "quic read frame payload failed");
                    break;
                }
                if wire_debug && wire_debug_left > 0 {
                    info!(
                        seq,
                        len,
                        tx_unix_us,
                        hash = format!("{:016x}", fnv1a64(&buf)),
                        "quic wire rx frame"
                    );
                    wire_debug_left -= 1;
                }
                let payload = to_annexb_if_needed(&buf);
                if let Some(dir) = &dump_dir {
                    if dump_left > 0 {
                        let idx = 4 - dump_left;
                        let _ = std::fs::create_dir_all(dir);
                        let raw_path = format!("{dir}/quic_rx_{idx}_seq{seq}_raw.h264");
                        let annexb_path = format!("{dir}/quic_rx_{idx}_seq{seq}_annexb.h264");
                        let _ = std::fs::write(raw_path, &buf);
                        let _ = std::fs::write(annexb_path, payload.as_ref());
                        dump_left -= 1;
                    }
                }
                let is_keyframe = contains_idr_annexb(payload.as_ref());
                if waiting_for_keyframe {
                    if !is_keyframe {
                        continue;
                    }
                    waiting_for_keyframe = false;
                    info!(seq, "quic receiver synchronized on keyframe");
                }
                let frame = VideoFrame {
                    data: payload,
                    timestamp: seq,
                    is_keyframe,
                    sequence: seq,
                    tx_unix_us,
                };
                if tx.try_send(frame).is_err() {
                    // drop when decoder path is temporarily saturated
                }
            }
        }
        drop(endpoint);
    });

    Ok(Arc::new(Mutex::new(rx)))
}
