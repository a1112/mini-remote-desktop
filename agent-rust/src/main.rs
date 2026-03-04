mod capture_policy;
mod capture_runtime;
mod encoder_policy;
mod encoder_runtime;
mod input_injector;
mod net_adapt;
mod nvenc_native;
mod profile;
mod quic_tx;
mod rtp_send;
mod runtime_stats;

use crate::capture_policy::{CaptureBackend, choose_backend};
use crate::capture_runtime::{
    RawFrame, build_frame_capturer, detect_input_resolution, resize_rgba_fast, sleep_until,
};
use crate::encoder_policy::{VideoEncoderBackend, choose_encoder_backend};
use crate::encoder_runtime::{build_video_encoder, encode_rgba_frame, request_keyframe};
use crate::input_injector::InputInjector;
use crate::net_adapt::NetAdaptController;
use crate::nvenc_native::{NativeEncodePath, NativeNvencPipeline, NativeNvencTexturePipeline};
#[cfg(windows)]
use crate::capture_runtime::WgcWindowCapturer;
use crate::profile::apply_capture_profile;
use crate::quic_tx::{QuicAu, QuicServerAdvert, start_quic_sender};
use crate::rtp_send::{RtpH264Sender, RtpH264SenderConfig};
use crate::runtime_stats::{RuntimeStats, spawn_rtcp_feedback_loop, spawn_stats_panel};
use agent_rust::load_config;
use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{error, info, warn};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc::ice_transport::ice_candidate_pair::RTCIceCandidatePair;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::{
    RTCPFeedback, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
    TYPE_RTCP_FB_TRANSPORT_CC,
};
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use common_control_proto::ChannelClass;

#[derive(Default)]
struct SessionState {
    sessions: HashMap<String, SessionEntry>,
}

struct SessionEntry {
    pc: Arc<RTCPeerConnection>,
    running: Arc<AtomicBool>,
    _injector: Arc<InputInjector>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionTransport {
    WebRtc,
    Quic,
}

impl SessionTransport {
    fn parse(v: Option<&str>) -> Self {
        match v.unwrap_or("webrtc").to_ascii_lowercase().as_str() {
            "quic" => SessionTransport::Quic,
            _ => SessionTransport::WebRtc,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            SessionTransport::WebRtc => "webrtc",
            SessionTransport::Quic => "quic",
        }
    }
}

const CAPTURE_TS_MAGIC: &[u8; 4] = b"TSU1";

fn h264_debug_budget() -> usize {
    std::env::var("AGENT_H264_DEBUG_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(24)
        .clamp(1, 5000)
}

fn nvenc_recreate_on_force_idr_enabled_from(raw: Option<&str>) -> bool {
    match raw.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if matches!(v.as_str(), "0" | "false" | "off" | "no") => false,
        Some(v) if matches!(v.as_str(), "1" | "true" | "on" | "yes") => true,
        _ => true,
    }
}

fn nvenc_recreate_on_force_idr_enabled() -> bool {
    nvenc_recreate_on_force_idr_enabled_from(
        std::env::var("AGENT_NVENC_RECREATE_ON_FORCE_IDR").ok().as_deref(),
    )
}

fn should_recreate_nvenc_on_force_idr(
    selected_transport: SessionTransport,
    encoder_backend: VideoEncoderBackend,
    keyframe_requested: bool,
) -> bool {
    keyframe_requested
        && selected_transport == SessionTransport::WebRtc
        && encoder_backend == VideoEncoderBackend::Nvenc
        && nvenc_recreate_on_force_idr_enabled()
}

fn unix_time_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn pack_capture_ts_au(bytes: Vec<u8>, capture_start_us: u64, with_header: bool) -> Arc<[u8]> {
    if !with_header {
        return Arc::<[u8]>::from(bytes);
    }
    let mut out = Vec::with_capacity(12 + bytes.len());
    out.extend_from_slice(CAPTURE_TS_MAGIC);
    out.extend_from_slice(&capture_start_us.to_be_bytes());
    out.extend_from_slice(&bytes);
    Arc::<[u8]>::from(out)
}

fn unpack_capture_ts_au(buf: &[u8]) -> (u64, &[u8]) {
    if buf.len() >= 12 && &buf[..4] == CAPTURE_TS_MAGIC {
        let mut ts = [0_u8; 8];
        ts.copy_from_slice(&buf[4..12]);
        (u64::from_be_bytes(ts), &buf[12..])
    } else {
        (0, buf)
    }
}

type WsWrite = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "agent_rust=info,tokio=warn,webrtc=warn".to_string()),
        )
        .init();

    let mut cfg = load_config(&PathBuf::from("config.json"));
    if cfg.device_name == "Rust Agent" {
        if let Ok(host) = std::env::var("COMPUTERNAME") {
            if !host.trim().is_empty() {
                cfg.device_name = format!("{host} - Rust Agent");
            }
        }
    }

    info!(ws_url = %cfg.ws_url, "connecting to signaling server");
    info!(
        fps = cfg.capture.fps,
        backend = %cfg.capture.backend,
        encoder = %cfg.capture.encoder,
        allow_fallback = cfg.capture.allow_fallback,
        allow_encoder_fallback = cfg.capture.allow_encoder_fallback,
        strict_gpu_direct = cfg.capture.strict_gpu_direct,
        "capture configuration"
    );

    let capture_cfg = Arc::new(Mutex::new(cfg.capture.clone()));

    let (ws, _) = connect_async(&cfg.ws_url)
        .await
        .with_context(|| format!("connect signaling failed: {}", cfg.ws_url))?;

    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));
    let session = Arc::new(Mutex::new(SessionState::default()));
    let mut ws_read_failed = false;

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "websocket read error");
                ws_read_failed = true;
                break;
            }
        };

        if !msg.is_text() {
            continue;
        }

        let text = msg.into_text().context("ws message not text")?;
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let typ = v["type"].as_str().unwrap_or("");
        let action = v["action"].as_str().unwrap_or("");

        if typ == "system" && action == "connected" {
            let reg = json!({
                "type":"device",
                "action":"register",
                "payload":{
                    "type":"agent-rust",
                    "name": cfg.device_name,
                    "protocolVersion": 2,
                    "transports": ["webrtc", "quic"],
                    "capabilities": {
                        "protocols": ["webrtc", "quic"],
                        "platforms": ["windows"],
                        "codecs": ["h264"],
                        "features": ["multi-end-compat", "capability-negotiation"]
                    }
                }
            });
            ws_send_json(&write, &reg).await?;
            info!(device_name = %cfg.device_name, "registered with signaling server");
            continue;
        }

        if typ == "webrtc" && action == "offer" {
            let payload = &v["payload"];
            let controller_id = payload["controllerId"].as_str().unwrap_or("").to_string();
            let requested_transport = SessionTransport::parse(payload["transport"].as_str());
            let controller_caps = payload
                .get("capabilities")
                .filter(|val| val.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let offer_type = payload["offer"]["type"]
                .as_str()
                .unwrap_or("offer")
                .to_string();
            let offer_sdp = payload["offer"]["sdp"].as_str().unwrap_or("").to_string();
            if controller_id.is_empty() || offer_sdp.is_empty() {
                warn!("received invalid offer payload");
                continue;
            }

            let selected_transport = match requested_transport {
                SessionTransport::WebRtc => SessionTransport::WebRtc,
                SessionTransport::Quic => SessionTransport::Quic,
            };
            info!(
                requested_transport = requested_transport.as_str(),
                selected_transport = selected_transport.as_str(),
                controller_caps = %controller_caps,
                "session transport negotiated"
            );

            let max_clients = std::env::var("AGENT_MAX_CLIENTS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(4)
                .max(1);
            let session_running = Arc::new(AtomicBool::new(true));
            let old_entry = {
                let mut s = session.lock().await;
                if !s.sessions.contains_key(&controller_id) && s.sessions.len() >= max_clients {
                    warn!(
                        controller_id = %controller_id,
                        max_clients,
                        active_clients = s.sessions.len(),
                        "rejecting offer: max client limit reached"
                    );
                    None
                } else {
                    s.sessions.remove(&controller_id)
                }
            };
            if old_entry.is_none() {
                let s = session.lock().await;
                if !s.sessions.contains_key(&controller_id) && s.sessions.len() >= max_clients {
                    let err_msg = json!({
                        "type": "webrtc",
                        "action": "error",
                        "payload": {
                            "controllerId": controller_id,
                            "message": format!("max clients reached ({max_clients})"),
                        }
                    });
                    let _ = ws_send_json(&write, &err_msg).await;
                    continue;
                }
            }
            if let Some(entry) = old_entry {
                entry.running.store(false, Ordering::SeqCst);
                let pc = entry.pc;
                if let Err(e) = pc.close().await {
                    warn!(error = %e, "failed to close previous peer connection");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }

            let mut quic_advert: Option<QuicServerAdvert> = None;
            let mut quic_tx: Option<tokio::sync::mpsc::Sender<QuicAu>> = None;
            if selected_transport == SessionTransport::Quic {
                let bind_addr: std::net::SocketAddr = "0.0.0.0:0"
                    .parse()
                    .context("parse quic bind addr failed")?;
                let (advert, tx) = start_quic_sender(bind_addr)?;
                quic_advert = Some(advert);
                quic_tx = Some(tx);
            }

            let injector = Arc::new(InputInjector::new());
            let pc =
                create_peer_connection(write.clone(), controller_id.clone(), injector.clone())
                    .await?;
            let effective_capture_cfg = { capture_cfg.lock().await.clone() };
            attach_video_track_with_policy(
                pc.clone(),
                &effective_capture_cfg,
                session_running.clone(),
                selected_transport,
                quic_tx,
            )
            .await?;

            pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?)
                .await
                .context("set remote offer failed")?;

            let answer = pc
                .create_answer(None)
                .await
                .context("create answer failed")?;
            pc.set_local_description(answer.clone())
                .await
                .context("set local answer failed")?;

            let msg = json!({
                "type": "webrtc",
                "action": "answer",
                "payload": {
                    "answer": { "type": offer_type.replace("offer", "answer"), "sdp": answer.sdp },
                    "controllerId": controller_id.clone(),
                    "selectedTransport": selected_transport.as_str(),
                    "quic": quic_advert.as_ref().map(|q| json!({
                        "addr": q.addr,
                        "serverName": q.server_name,
                        "certDerBase64": q.cert_der_base64,
                    })),
                    "agentCapabilities": {
                        "protocols": ["webrtc", "quic"],
                        "platforms": ["windows"],
                        "codecs": ["h264"],
                        "features": ["multi-end-compat", "capability-negotiation"]
                    }
                }
            });
            ws_send_json(&write, &msg).await?;
            info!("WebRTC answer sent");

            let mut s = session.lock().await;
            s.sessions.insert(
                controller_id,
                SessionEntry {
                    pc,
                    running: session_running,
                    _injector: injector,
                },
            );
            continue;
        }

        if typ == "control" && action == "updateCapture" {
            let patch = v["payload"]["capture"].clone();
            let controller_id = v["payload"]["controllerId"].as_str().unwrap_or("").to_string();
            if let Err(e) = apply_capture_patch(&capture_cfg, &patch).await {
                warn!(error = %e, "apply capture update failed");
            } else {
                info!(controller_id = %controller_id, patch = %patch, "capture settings updated");
            }
            let entries = {
                let mut s = session.lock().await;
                let all: Vec<SessionEntry> = s.sessions.drain().map(|(_, v)| v).collect();
                all
            };
            for entry in entries {
                entry.running.store(false, Ordering::SeqCst);
                if let Err(e) = entry.pc.close().await {
                    warn!(error = %e, "failed to close peer connection after updateCapture");
                }
            }
            continue;
        }

        if typ == "webrtc" && action == "iceCandidate" {
            let candidate = &v["payload"]["candidate"];
            if candidate.is_null() {
                continue;
            }
            let controller_id = v["payload"]["controllerId"].as_str().unwrap_or("").to_string();
            let cand: webrtc::ice_transport::ice_candidate::RTCIceCandidateInit =
                serde_json::from_value(candidate.clone()).context("parse remote ice failed")?;
            let target_pc = {
                let s = session.lock().await;
                if controller_id.is_empty() {
                    s.sessions.values().next().map(|e| e.pc.clone())
                } else {
                    s.sessions.get(&controller_id).map(|e| e.pc.clone())
                }
            };
            if let Some(pc) = target_pc {
                if let Err(e) = pc.add_ice_candidate(cand).await {
                    warn!(error = %e, controller_id = %controller_id, "failed to add remote ice candidate");
                }
            } else {
                warn!(controller_id = %controller_id, "no active session for incoming ICE candidate");
            }
        }
    }

    let had_active_session = {
        let s = session.lock().await;
        !s.sessions.is_empty()
    };
    if had_active_session {
        warn!(
            ws_read_failed = ws_read_failed,
            "signaling stream ended while session active, entering grace period"
        );
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    let entries = {
        let mut s = session.lock().await;
        s.sessions.drain().map(|(_, v)| v).collect::<Vec<_>>()
    };
    for entry in entries {
        entry.running.store(false, Ordering::SeqCst);
        if let Err(e) = entry.pc.close().await {
            warn!(error = %e, "failed to close peer connection on shutdown");
        }
    }

    Ok(())
}

async fn apply_capture_patch(
    capture_cfg: &Arc<Mutex<agent_rust::CaptureConfig>>,
    patch: &Value,
) -> Result<()> {
    let mut cfg = capture_cfg.lock().await;
    if let Some(v) = patch.get("targetWidth").and_then(|v| v.as_u64()) {
        cfg.target_width = v as u32;
    }
    if let Some(v) = patch.get("targetHeight").and_then(|v| v.as_u64()) {
        cfg.target_height = v as u32;
    }
    if let Some(v) = patch.get("bitrateKbps").and_then(|v| v.as_u64()) {
        let br = (v as u32).max(100);
        cfg.bitrate_kbps = br;
        if cfg.max_bitrate_kbps < br {
            cfg.max_bitrate_kbps = br;
        }
    }
    if let Some(v) = patch.get("backend").and_then(|v| v.as_str()) {
        cfg.backend = v.to_ascii_lowercase();
    }
    if let Some(v) = patch.get("encoder").and_then(|v| v.as_str()) {
        cfg.encoder = v.to_ascii_lowercase();
    }
    if let Some(v) = patch.get("windowMode").and_then(|v| v.as_str()) {
        match v.to_ascii_lowercase().as_str() {
            "auto" => {
                unsafe {
                    std::env::remove_var("AGENT_WGC_WINDOW_HWND");
                }
            }
            "foreground" => {
                #[cfg(windows)]
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
                    let hwnd = GetForegroundWindow();
                    if !hwnd.0.is_null() {
                        std::env::set_var(
                            "AGENT_WGC_WINDOW_HWND",
                            format!("{:?}", hwnd.0 as isize),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn create_peer_connection(
    ws_write: Arc<Mutex<WsWrite>>,
    controller_id: String,
    injector: Arc<InputInjector>,
) -> Result<Arc<RTCPeerConnection>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut se = SettingEngine::default();
    se.set_srtp_protection_profiles(vec![
        SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
    ]);
    se.set_include_loopback_candidate(true);
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_setting_engine(se)
        .build();

    let pc = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .context("new peer connection failed")?,
    );

    {
        let ws_write = ws_write.clone();
        let controller_id = controller_id.clone();
        pc.on_ice_candidate(Box::new(move |cand| {
            let ws_write = ws_write.clone();
            let controller_id = controller_id.clone();
            Box::pin(async move {
                if let Some(c) = cand {
                    if let Ok(cjson) = c.to_json() {
                        let msg = json!({
                            "type": "webrtc",
                            "action": "iceCandidate",
                            "payload": {
                                "targetDeviceId": controller_id,
                                "candidate": cjson
                            }
                        });
                        if let Err(e) = ws_send_json(&ws_write, &msg).await {
                            warn!(error = %e, "failed to send local ICE candidate");
                        }
                    }
                }
            })
        }));
    }

    pc.on_peer_connection_state_change(Box::new(|s: RTCPeerConnectionState| {
        info!(state = %s, "peer connection state changed");
        Box::pin(async {})
    }));
    pc.on_ice_connection_state_change(Box::new(move |s: RTCIceConnectionState| {
        info!(state = %s, "ice connection state changed");
        Box::pin(async {})
    }));
    {
        let injector = injector.clone();
        pc.on_data_channel(Box::new(move |dc| {
            let injector = injector.clone();
            Box::pin(async move {
                let label = dc.label().to_string();
                let class = match label.as_str() {
                    "ctrl_rt" => Some(ChannelClass::Realtime),
                    "ctrl_rel" => Some(ChannelClass::Reliable),
                    _ => None,
                };
                if class.is_none() {
                    info!(label = %label, "received non-control data channel");
                    return;
                }
                let class = class.unwrap_or(ChannelClass::Reliable);
                info!(label = %label, class = ?class, "control data channel bound");
                let injector = injector.clone();
                dc.on_message(Box::new(move |msg| {
                    let injector = injector.clone();
                    Box::pin(async move {
                        if let Err(e) = injector.push_raw(class, &msg.data).await {
                            warn!(error = %e, "failed to decode/queue control frame");
                        }
                    })
                }));
            })
        }));
    }
    pc.sctp()
        .transport()
        .ice_transport()
        .on_selected_candidate_pair_change(Box::new(move |p: RTCIceCandidatePair| {
            info!(pair = %p, "selected ICE candidate pair changed");
            Box::pin(async {})
        }));

    Ok(pc)
}

async fn attach_video_track_with_policy(
    pc: Arc<RTCPeerConnection>,
    capture_cfg: &agent_rust::CaptureConfig,
    session_running: Arc<AtomicBool>,
    selected_transport: SessionTransport,
    quic_tx: Option<tokio::sync::mpsc::Sender<QuicAu>>,
) -> Result<()> {
    let mut effective_cfg = capture_cfg.clone();
    let with_capture_ts_header = selected_transport == SessionTransport::Quic;
    apply_capture_profile(&mut effective_cfg);
    if selected_transport == SessionTransport::WebRtc {
        // WebRTC path favors stable decoder bootstrap over ultra-aggressive RTP burst mode.
        // Keep QUIC tuning unchanged.
        effective_cfg.rtp_use_manual_packetizer = false;
        effective_cfg.max_fps_mode = false;
        info!(
            rtp_use_manual_packetizer = effective_cfg.rtp_use_manual_packetizer,
            max_fps_mode = effective_cfg.max_fps_mode,
            "applied WebRTC-safe media send policy"
        );
    }
    if effective_cfg.tier_limit_enable {
        info!(
            tier_ladder_fps = %format!(
                "{}/{}/{}/{}/{}",
                effective_cfg.tier_fps_l1,
                effective_cfg.tier_fps_l2,
                effective_cfg.tier_fps_l3,
                effective_cfg.tier_fps_l4,
                effective_cfg.tier_fps_l5
            ),
            tier_ladder_bitrate_kbps = %format!(
                "{}/{}/{}/{}/{}",
                effective_cfg.tier_bitrate_kbps_l1,
                effective_cfg.tier_bitrate_kbps_l2,
                effective_cfg.tier_bitrate_kbps_l3,
                effective_cfg.tier_bitrate_kbps_l4,
                effective_cfg.tier_bitrate_kbps_l5
            ),
            selected_fps = effective_cfg.fps,
            selected_bitrate_kbps = effective_cfg.bitrate_kbps,
            "multi-tier limits applied"
        );
    }

    let (encoder_backend, logs) = choose_encoder_backend(&effective_cfg);
    for line in logs {
        info!("{}", line);
    }
    let forced_backend = std::env::var("AGENT_CAPTURE_BACKEND_FORCE")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty());
    let requested_backend = forced_backend
        .as_deref()
        .unwrap_or(&capture_cfg.backend)
        .to_ascii_lowercase();
    let mut backend_cfg = capture_cfg.clone();
    if let Some(force) = forced_backend.as_deref() {
        backend_cfg.backend = force.to_string();
        info!(forced_backend = force, "capture backend forced by env");
    }
    let (backend, logs) = if encoder_backend == VideoEncoderBackend::Nvenc
        && matches!(requested_backend.as_str(), "auto" | "dxgi")
    {
        (
            CaptureBackend::Dxgi,
            vec!["capture backend selected: dxgi (native nvenc path bypass probe)".to_string()],
        )
    } else {
        choose_backend(&backend_cfg)
    };
    for line in logs {
        info!("{}", line);
    }

    let codec_cap = RTCRtpCodecCapability {
        mime_type: "video/H264".to_string(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=64001f"
            .to_string(),
        rtcp_feedback: vec![
            RTCPFeedback {
                typ: TYPE_RTCP_FB_NACK.to_string(),
                parameter: "".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_NACK.to_string(),
                parameter: "pli".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_CCM.to_string(),
                parameter: "fir".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
                parameter: "".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
                parameter: "".to_string(),
            },
        ],
    };

    let use_manual_packetizer = effective_cfg.rtp_use_manual_packetizer;
    let sample_track = if use_manual_packetizer {
        None
    } else {
        Some(Arc::new(TrackLocalStaticSample::new(
            codec_cap.clone(),
            "video".to_string(),
            "rust-agent".to_string(),
        )))
    };
    let rtp_track = if use_manual_packetizer {
        Some(Arc::new(TrackLocalStaticRTP::new(
            codec_cap,
            "video".to_string(),
            "rust-agent".to_string(),
        )))
    } else {
        None
    };
    let track: Arc<dyn TrackLocal + Send + Sync> = if let Some(t) = &rtp_track {
        t.clone()
    } else if let Some(t) = &sample_track {
        t.clone()
    } else {
        return Err(anyhow!("invalid track mode"));
    };

    let sender = pc
        .add_track(track)
        .await
        .context("add local video track failed")?;

    let enable_network_adapt = effective_cfg.network_adapt_enable;
    let adapt_min_fps = effective_cfg.min_fps.max(1);
    let adapt_max_fps = effective_cfg
        .fps
        .max(1)
        .clamp(adapt_min_fps, effective_cfg.max_fps.max(1));
    let adapt = Arc::new(NetAdaptController::new(
        adapt_min_fps,
        adapt_max_fps,
        effective_cfg.fps.max(1),
        effective_cfg.network_adapt_floor_bitrate_kbps.max(100),
        effective_cfg
            .network_adapt_ceiling_bitrate_kbps
            .max(effective_cfg.network_adapt_floor_bitrate_kbps.max(100)),
        effective_cfg.bitrate_kbps.max(100),
        effective_cfg.tier_limit_enable,
        [
            effective_cfg.tier_fps_l1,
            effective_cfg.tier_fps_l2,
            effective_cfg.tier_fps_l3,
            effective_cfg.tier_fps_l4,
            effective_cfg.tier_fps_l5,
        ],
        [
            effective_cfg.tier_bitrate_kbps_l1,
            effective_cfg.tier_bitrate_kbps_l2,
            effective_cfg.tier_bitrate_kbps_l3,
            effective_cfg.tier_bitrate_kbps_l4,
            effective_cfg.tier_bitrate_kbps_l5,
        ],
    ));
    let stats = Arc::new(RuntimeStats::new(
        adapt.current_fps(),
        adapt.current_bitrate_kbps(),
    ));
    stats
        .tier_level
        .store(adapt.current_tier_level(), Ordering::Relaxed);
    stats
        .tier_reason_code
        .store(adapt.tier_reason_code(), Ordering::Relaxed);
    stats
        .tier_switch_count
        .store(adapt.tier_switch_count(), Ordering::Relaxed);
    // Force an initial IDR so decoder bootstrap does not depend on transport timing.
    let keyframe_request = Arc::new(AtomicBool::new(true));

    spawn_rtcp_feedback_loop(
        sender.clone(),
        keyframe_request.clone(),
        adapt.clone(),
        stats.clone(),
        enable_network_adapt,
        effective_cfg.force_idr_on_pli,
    );
    spawn_stats_panel(
        stats.clone(),
        adapt.clone(),
        effective_cfg.stats_interval_ms,
        session_running.clone(),
    );

    if encoder_backend == VideoEncoderBackend::Nvenc && backend == CaptureBackend::Dxgi {
        let (input_w, input_h) = detect_input_resolution()?;
        let target_w = if effective_cfg.target_width > 0 {
            effective_cfg.target_width
        } else {
            input_w
        };
        let target_h = if effective_cfg.target_height > 0 {
            effective_cfg.target_height
        } else {
            input_h
        };
        let native_init = async {
            let mut last_err: Option<anyhow::Error> = None;
            for attempt in 0..30 {
                match NativeNvencPipeline::new(target_w, target_h, &effective_cfg) {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        let msg = e.to_string();
                        let duplicate_output = msg.contains("DuplicateOutput")
                            || msg.contains("0x887A0022")
                            || msg.contains("desktop duplication unavailable");
                        last_err = Some(e);
                        if duplicate_output && attempt < 29 {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                            continue;
                        }
                        break;
                    }
                }
            }
            Err(last_err.unwrap_or_else(|| anyhow!("native nvenc init failed")))
        };
        match native_init.await {
            Ok(mut native) => {
                info!(
                    input_w,
                    input_h,
                    target_w,
                    target_h,
                    fps = effective_cfg.fps.max(1),
                    strict_gpu_direct = effective_cfg.strict_gpu_direct,
                    adapter = %native.adapter_summary(),
                    "native NVENC pipeline attached"
                );
                let queue_depth = effective_cfg.queue_depth.clamp(1, 64) as usize;
                let block_queue = effective_cfg.queue_strategy == "block";
                let (encoded_tx, mut encoded_rx) =
                    tokio::sync::mpsc::channel::<Arc<[u8]>>(queue_depth);
                let keyframe_request2 = keyframe_request.clone();
                let stats_encode = stats.clone();
                let session_running_encode = session_running.clone();
                let effective_cfg_encode = effective_cfg.clone();
                let selected_transport_encode = selected_transport;
                let idr_interval_frames =
                    effective_cfg.fps.max(1) * effective_cfg.idr_interval_sec.max(1);
                std::thread::spawn(move || {
                    let mut encoded_frames: u32 = 0;
                    let strict_gpu_direct = effective_cfg.strict_gpu_direct;
                    while session_running_encode.load(Ordering::SeqCst) {
                        let keyframe_requested = keyframe_request2.swap(false, Ordering::Relaxed);
                        let interval_force = idr_interval_frames > 0
                            && encoded_frames > 0
                            && encoded_frames.is_multiple_of(idr_interval_frames);
                        let force_idr = keyframe_requested || interval_force;
                        if should_recreate_nvenc_on_force_idr(
                            selected_transport_encode,
                            encoder_backend,
                            keyframe_requested,
                        ) {
                            match NativeNvencPipeline::new(
                                target_w,
                                target_h,
                                &effective_cfg_encode,
                            ) {
                                Ok(v) => {
                                    native = v;
                                    info!("recreated native NVENC pipeline on keyframe request");
                                }
                                Err(e) => warn!(
                                    error = %e,
                                    "failed to recreate native NVENC pipeline on keyframe request"
                                ),
                            }
                        }
                        match native.encode_next(force_idr) {
                            Ok(Some(v)) if !v.bytes.is_empty() => {
                                encoded_frames = encoded_frames.saturating_add(1);
                                stats_encode
                                    .encoded_au_total
                                    .fetch_add(1, Ordering::Relaxed);
                                match v.path {
                                    NativeEncodePath::DirectTexture => {
                                        stats_encode
                                            .native_direct_frames
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                    NativeEncodePath::CopyResource => {
                                        stats_encode
                                            .native_copy_frames
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                    NativeEncodePath::ScaleBlt => {
                                        stats_encode
                                            .native_scale_frames
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                let path_stats = native.path_stats();
                                stats_encode
                                    .native_direct_register_failures
                                    .store(path_stats.direct_register_failures, Ordering::Relaxed);
                                stats_encode
                                    .native_acquire_ok
                                    .store(path_stats.acquire_ok, Ordering::Relaxed);
                                stats_encode
                                    .native_acquire_timeout
                                    .store(path_stats.acquire_timeout, Ordering::Relaxed);
                                stats_encode
                                    .native_acquire_errors
                                    .store(path_stats.acquire_errors, Ordering::Relaxed);
                                let encoded = pack_capture_ts_au(
                                    v.bytes,
                                    v.capture_start_us,
                                    with_capture_ts_header,
                                );
                                if block_queue {
                                    let _ = encoded_tx.blocking_send(encoded);
                                } else {
                                    let _ = encoded_tx.try_send(encoded);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                error!(error = %e, "native NVENC encode failed");
                                if strict_gpu_direct {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                });

                if selected_transport == SessionTransport::Quic {
                    let quic_sender = quic_tx
                        .as_ref()
                        .cloned()
                        .ok_or_else(|| anyhow!("quic transport selected but quic sender missing"))?;
                    let stats_send = stats.clone();
                    let session_running_send = session_running.clone();
                    tokio::spawn(spawn_send_loop_quic(
                        quic_sender,
                        encoded_rx,
                        stats_send,
                        session_running_send,
                    ));
                } else if let Some(track) = rtp_track.clone() {
                    let sender = RtpH264Sender::new(
                        track,
                        &RtpH264SenderConfig {
                            fps: effective_cfg.fps.max(1),
                            mtu: effective_cfg.rtp_mtu,
                            frame_pacing_enable: effective_cfg.frame_pacing_enable,
                            frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
                        },
                    );
                    tokio::spawn(spawn_send_loop_rtp(
                        sender,
                        encoded_rx,
                        adapt,
                        stats,
                        enable_network_adapt,
                        effective_cfg.max_fps_mode,
                        effective_cfg.idle_repeat_fps,
                        session_running.clone(),
                    ));
                } else if let Some(track) = sample_track.clone() {
                    let fps = effective_cfg.fps.max(1);
                    let stats_send = stats.clone();
                    let repeat_last = effective_cfg.max_fps_mode;
                    let idle_repeat_fps = effective_cfg.idle_repeat_fps.max(1);
                    let session_running_send = session_running.clone();
                    tokio::spawn(spawn_send_loop_sample(
                        track,
                        encoded_rx,
                        fps,
                        stats_send,
                        repeat_last,
                        idle_repeat_fps,
                        session_running_send,
                    ));
                }
                return Ok(());
            }
            Err(e) => {
                if effective_cfg.strict_gpu_direct || !effective_cfg.allow_encoder_fallback {
                    return Err(anyhow!(
                        "native nvenc init failed and fallback disabled: {e}"
                    ));
                }
                warn!(error = %e, "native NVENC init failed, using fallback");
            }
        }
    }

    if encoder_backend == VideoEncoderBackend::Nvenc && backend == CaptureBackend::Wgc {
        #[cfg(windows)]
        {
            let mut wgc = WgcWindowCapturer::new()?;
            let first = wgc.capture_gpu_frame(Duration::from_millis(250))?;
            let input_w = first.width;
            let input_h = first.height;
            let target_w = if effective_cfg.target_width > 0 {
                effective_cfg.target_width
            } else {
                input_w
            };
            let target_h = if effective_cfg.target_height > 0 {
                effective_cfg.target_height
            } else {
                input_h
            };
            let native_init = NativeNvencTexturePipeline::new(
                wgc.device(),
                wgc.context(),
                target_w,
                target_h,
                &effective_cfg,
            );
            match native_init {
                Ok(mut native) => {
                    info!(
                        input_w,
                        input_h,
                        target_w,
                        target_h,
                        fps = effective_cfg.fps.max(1),
                        strict_gpu_direct = effective_cfg.strict_gpu_direct,
                        "WGC native NVENC texture pipeline attached"
                    );
                    let queue_depth = effective_cfg.queue_depth.clamp(1, 64) as usize;
                    let block_queue = effective_cfg.queue_strategy == "block";
                    let (encoded_tx, mut encoded_rx) =
                        tokio::sync::mpsc::channel::<Arc<[u8]>>(queue_depth);
                    let keyframe_request2 = keyframe_request.clone();
                    let stats_encode = stats.clone();
                    let session_running_encode = session_running.clone();
                    let effective_cfg_encode = effective_cfg.clone();
                    let selected_transport_encode = selected_transport;
                    let idr_interval_frames =
                        effective_cfg.fps.max(1) * effective_cfg.idr_interval_sec.max(1);
                    std::thread::spawn(move || {
                        let mut encoded_frames: u32 = 0;
                        let strict_gpu_direct = effective_cfg.strict_gpu_direct;
                        while session_running_encode.load(Ordering::SeqCst) {
                            let keyframe_requested =
                                keyframe_request2.swap(false, Ordering::Relaxed);
                            let interval_force = idr_interval_frames > 0
                                && encoded_frames > 0
                                && encoded_frames.is_multiple_of(idr_interval_frames);
                            let force_idr = keyframe_requested || interval_force;
                            if should_recreate_nvenc_on_force_idr(
                                selected_transport_encode,
                                encoder_backend,
                                keyframe_requested,
                            ) {
                                match NativeNvencTexturePipeline::new(
                                    wgc.device(),
                                    wgc.context(),
                                    target_w,
                                    target_h,
                                    &effective_cfg_encode,
                                ) {
                                    Ok(v) => {
                                        native = v;
                                        info!(
                                            "recreated WGC native NVENC texture pipeline on keyframe request"
                                        );
                                    }
                                    Err(e) => warn!(
                                        error = %e,
                                        "failed to recreate WGC native NVENC texture pipeline on keyframe request"
                                    ),
                                }
                            }
                            let capture = wgc.capture_gpu_frame(Duration::from_millis(120));
                            let capture_start_us = capture
                                .as_ref()
                                .map(|f| f.capture_start_us)
                                .unwrap_or(0);
                            let encoded_res = capture
                                .and_then(|frame| native.encode_texture(&frame.texture, force_idr));
                            match encoded_res {
                                Ok(Some(v)) if !v.bytes.is_empty() => {
                                    encoded_frames = encoded_frames.saturating_add(1);
                                    stats_encode
                                        .encoded_au_total
                                        .fetch_add(1, Ordering::Relaxed);
                                    match v.path {
                                        NativeEncodePath::DirectTexture => {
                                            stats_encode
                                                .native_direct_frames
                                                .fetch_add(1, Ordering::Relaxed);
                                        }
                                        NativeEncodePath::CopyResource => {
                                            stats_encode
                                                .native_copy_frames
                                                .fetch_add(1, Ordering::Relaxed);
                                        }
                                        NativeEncodePath::ScaleBlt => {
                                            stats_encode
                                                .native_scale_frames
                                                .fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    let path_stats = native.path_stats();
                                    stats_encode.native_direct_register_failures.store(
                                        path_stats.direct_register_failures,
                                        Ordering::Relaxed,
                                    );
                                    stats_encode
                                        .native_acquire_ok
                                        .store(path_stats.acquire_ok, Ordering::Relaxed);
                                    stats_encode
                                        .native_acquire_timeout
                                        .store(path_stats.acquire_timeout, Ordering::Relaxed);
                                    stats_encode
                                        .native_acquire_errors
                                        .store(path_stats.acquire_errors, Ordering::Relaxed);
                                    let encoded = pack_capture_ts_au(
                                        v.bytes,
                                        if capture_start_us == 0 {
                                            v.capture_start_us
                                        } else {
                                            capture_start_us
                                        },
                                        with_capture_ts_header,
                                    );
                                    if block_queue {
                                        let _ = encoded_tx.blocking_send(encoded);
                                    } else {
                                        let _ = encoded_tx.try_send(encoded);
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    error!(error = %e, "WGC native NVENC encode failed");
                                    if strict_gpu_direct {
                                        break;
                                    }
                                    std::thread::sleep(Duration::from_millis(2));
                                }
                            }
                        }
                    });

                    if selected_transport == SessionTransport::Quic {
                        let quic_sender = quic_tx
                            .as_ref()
                            .cloned()
                            .ok_or_else(|| anyhow!("quic transport selected but quic sender missing"))?;
                        let stats_send = stats.clone();
                        let session_running_send = session_running.clone();
                        tokio::spawn(spawn_send_loop_quic(
                            quic_sender,
                            encoded_rx,
                            stats_send,
                            session_running_send,
                        ));
                    } else if let Some(track) = rtp_track.clone() {
                        let sender = RtpH264Sender::new(
                            track,
                            &RtpH264SenderConfig {
                                fps: effective_cfg.fps.max(1),
                                mtu: effective_cfg.rtp_mtu,
                                frame_pacing_enable: effective_cfg.frame_pacing_enable,
                                frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
                            },
                        );
                        tokio::spawn(spawn_send_loop_rtp(
                            sender,
                            encoded_rx,
                            adapt,
                            stats,
                            enable_network_adapt,
                            effective_cfg.max_fps_mode,
                            effective_cfg.idle_repeat_fps,
                            session_running.clone(),
                        ));
                    } else if let Some(track) = sample_track.clone() {
                        let fps = effective_cfg.fps.max(1);
                        let stats_send = stats.clone();
                        let repeat_last = effective_cfg.max_fps_mode;
                        let idle_repeat_fps = effective_cfg.idle_repeat_fps.max(1);
                        let session_running_send = session_running.clone();
                        tokio::spawn(spawn_send_loop_sample(
                            track,
                            encoded_rx,
                            fps,
                            stats_send,
                            repeat_last,
                            idle_repeat_fps,
                            session_running_send,
                        ));
                    }
                    return Ok(());
                }
                Err(e) => {
                    if effective_cfg.strict_gpu_direct || !effective_cfg.allow_encoder_fallback {
                        return Err(anyhow!(
                            "wgc native nvenc init failed and fallback disabled: {e}"
                        ));
                    }
                    warn!(error = %e, "WGC native NVENC init failed, using fallback");
                }
            }
        }
        #[cfg(not(windows))]
        {
            warn!("WGC native NVENC path requires Windows build; using fallback pipeline");
        }
    }

    let fps = effective_cfg
        .fps
        .clamp(effective_cfg.min_fps.max(1), effective_cfg.max_fps.max(1));
    let frame_ms = (1000.0 / fps as f64).max(1.0).round() as u64;
    let frame_duration = Duration::from_millis(frame_ms);
    let allow_encoder_fallback = effective_cfg.allow_encoder_fallback;
    let block_queue = effective_cfg.queue_strategy == "block";
    let running = Arc::new(AtomicBool::new(true));
    let latest = Arc::new(std::sync::Mutex::new(None::<RawFrame>));
    let queue_depth = effective_cfg.queue_depth.clamp(1, 64) as usize;
    let (encoded_tx, mut encoded_rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(queue_depth);

    {
        let running = running.clone();
        let latest = latest.clone();
        let target_width = effective_cfg.target_width;
        let target_height = effective_cfg.target_height;
        let session_running_capture = session_running.clone();
        std::thread::spawn(move || {
            let mut capturer = match build_frame_capturer(backend) {
                Ok(v) => v,
                Err(e) => {
                    error!(error = %e, "capture initialization failed");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let mut next_tick = Instant::now();
            while running.load(Ordering::Relaxed) && session_running_capture.load(Ordering::SeqCst)
            {
                match capturer.capture() {
                    Ok((mut rgba, mut width, mut height)) => {
                        if target_width > 0
                            && target_height > 0
                            && (target_width != width || target_height != height)
                        {
                            if let Some((resized, rw, rh)) =
                                resize_rgba_fast(&rgba, width, height, target_width, target_height)
                            {
                                rgba = resized;
                                width = rw;
                                height = rh;
                            }
                        }
                        if let Ok(mut slot) = latest.lock() {
                            *slot = Some(RawFrame {
                                rgba,
                                width,
                                height,
                                capture_start_us: unix_time_us(),
                            });
                        }
                    }
                    Err(e) => error!(error = %e, "capture frame failed"),
                }
                next_tick += frame_duration;
                sleep_until(next_tick);
            }
        });
    }

    {
        let running = running.clone();
        let latest = latest.clone();
        let encode_cfg = effective_cfg.clone();
        let keyframe_request2 = keyframe_request.clone();
        let adapt2 = adapt.clone();
        let stats_encode = stats.clone();
        let session_running_encode = session_running.clone();
        let idr_interval_frames = fps.max(1) * effective_cfg.idr_interval_sec.max(1);
        std::thread::spawn(move || {
            let mut encoder = match build_video_encoder(
                fps,
                &encode_cfg,
                encoder_backend,
                allow_encoder_fallback,
            ) {
                Ok(e) => e,
                Err(e) => {
                    error!(error = %e, "H264 encoder initialization failed");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let mut encoded_frames: u32 = 0;

            while running.load(Ordering::Relaxed) && session_running_encode.load(Ordering::SeqCst) {
                let frame = match latest.lock() {
                    Ok(mut slot) => slot.take(),
                    Err(_) => None,
                };
                let Some(frame) = frame else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let interval_force = idr_interval_frames > 0
                    && encoded_frames > 0
                    && encoded_frames.is_multiple_of(idr_interval_frames);
                if keyframe_request2.swap(false, Ordering::Relaxed) || interval_force {
                    request_keyframe(&mut encoder);
                }

                let target_bitrate_kbps = if enable_network_adapt {
                    Some(adapt2.current_bitrate_kbps())
                } else {
                    None
                };

                let encoded = match encode_rgba_frame(
                    &mut encoder,
                    &frame.rgba,
                    frame.width,
                    frame.height,
                    target_bitrate_kbps,
                    enable_network_adapt,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(error = %e, "H264 encode failed");
                        continue;
                    }
                };
                if encoded.is_empty() {
                    continue;
                }
                encoded_frames = encoded_frames.saturating_add(1);
                stats_encode
                    .encoded_au_total
                    .fetch_add(1, Ordering::Relaxed);
                let encoded =
                    pack_capture_ts_au(encoded, frame.capture_start_us, with_capture_ts_header);
                if block_queue {
                    let _ = encoded_tx.blocking_send(encoded);
                } else {
                    let _ = encoded_tx.try_send(encoded);
                }
            }
        });
    }

    if selected_transport == SessionTransport::Quic {
        let quic_sender = quic_tx
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("quic transport selected but quic sender missing"))?;
        let stats_send = stats.clone();
        let session_running_send = session_running.clone();
        tokio::spawn(spawn_send_loop_quic(
            quic_sender,
            encoded_rx,
            stats_send,
            session_running_send,
        ));
    } else if let Some(track) = rtp_track {
        let sender = RtpH264Sender::new(
            track,
            &RtpH264SenderConfig {
                fps,
                mtu: effective_cfg.rtp_mtu,
                frame_pacing_enable: effective_cfg.frame_pacing_enable,
                frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
            },
        );
        tokio::spawn(spawn_send_loop_rtp(
            sender,
            encoded_rx,
            adapt,
            stats,
            enable_network_adapt,
            effective_cfg.max_fps_mode,
            effective_cfg.idle_repeat_fps,
            session_running.clone(),
        ));
    } else if let Some(track) = sample_track {
        let stats_send = stats.clone();
        let repeat_last = effective_cfg.max_fps_mode;
        let idle_repeat_fps = effective_cfg.idle_repeat_fps.max(1);
        let session_running_send = session_running.clone();
        tokio::spawn(async move {
            let mut last_encoded: Option<Arc<[u8]>> = None;
            let mut last_sps: Option<Vec<u8>> = None;
            let mut last_pps: Option<Vec<u8>> = None;
            let h264_debug = std::env::var("AGENT_H264_DEBUG")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let mut h264_debug_left = h264_debug_budget();
            let mut next_due = Instant::now();
            while session_running_send.load(Ordering::SeqCst) {
                wait_until_due(next_due).await;
                let mut got_fresh = false;
                while let Ok(encoded) = encoded_rx.try_recv() {
                    update_h264_param_cache(encoded.as_ref(), &mut last_sps, &mut last_pps);
                    last_encoded = Some(encoded);
                    got_fresh = true;
                }
                if !got_fresh
                    && last_encoded.is_some()
                    && let Ok(Some(v)) =
                        tokio::time::timeout(Duration::from_millis(2), encoded_rx.recv()).await
                {
                    update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
                    last_encoded = Some(v);
                    got_fresh = true;
                }
                let encoded = if let Some(v) = last_encoded.as_ref() {
                    v.clone()
                } else {
                    match encoded_rx.recv().await {
                        Some(v) => {
                            update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
                            last_encoded = Some(v.clone());
                            got_fresh = true;
                            v
                        }
                        None => break,
                    }
                };
                let send_fps = if got_fresh || !repeat_last {
                    fps
                } else {
                    idle_repeat_fps
                };
                let send_gap = Duration::from_millis((1000.0 / send_fps as f64).max(1.0) as u64);
                next_due = advance_send_deadline(next_due, send_gap, Instant::now());
                let au_for_send = if let Some(patched) =
                    patch_h264_au_with_cached_params(encoded.as_ref(), &last_sps, &last_pps)
                {
                    Arc::<[u8]>::from(patched)
                } else {
                    encoded.clone()
                };
                if h264_debug && h264_debug_left > 0 {
                    let nals = parse_annexb_nals_view(au_for_send.as_ref());
                    let nal_types: Vec<u8> = nals.iter().map(|n| n.nal_type).collect();
                    let has_sps = nal_types.contains(&7);
                    let has_pps = nal_types.contains(&8);
                    let has_idr = nal_types.contains(&5);
                    let take = au_for_send.len().min(12);
                    let mut head = String::new();
                    for b in &au_for_send[..take] {
                        use std::fmt::Write as _;
                        let _ = write!(&mut head, "{:02X} ", b);
                    }
                    info!(
                        au_bytes = au_for_send.len(),
                        has_sps,
                        has_pps,
                        has_idr,
                        nal_types = ?nal_types,
                        head = %head.trim_end(),
                        "h264 sample au debug"
                    );
                    h264_debug_left -= 1;
                }
                let sample = Sample {
                    data: Bytes::copy_from_slice(au_for_send.as_ref()),
                    duration: send_gap,
                    ..Default::default()
                };
                if let Err(e) = track.write_sample(&sample).await {
                    error!(error = %e, "sample write failed");
                    running.store(false, Ordering::Relaxed);
                    break;
                }
                stats_send.sent_au_total.fetch_add(1, Ordering::Relaxed);
                stats_send.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
                if got_fresh {
                    stats_send
                        .unique_sent_au_total
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    stats_send
                        .repeated_sent_au_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                if !repeat_last {
                    last_encoded = None;
                }
            }
        });
    }

    Ok(())
}

async fn spawn_send_loop_sample(
    track: Arc<TrackLocalStaticSample>,
    mut encoded_rx: tokio::sync::mpsc::Receiver<Arc<[u8]>>,
    fps: u32,
    stats_send: Arc<RuntimeStats>,
    repeat_last: bool,
    idle_repeat_fps: u32,
    session_running_send: Arc<AtomicBool>,
) {
    let mut last_encoded: Option<Arc<[u8]>> = None;
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let h264_debug = std::env::var("AGENT_H264_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut h264_debug_left = h264_debug_budget();
    let mut next_due = Instant::now();
    while session_running_send.load(Ordering::SeqCst) {
        wait_until_due(next_due).await;
        let mut got_fresh = false;
        while let Ok(encoded) = encoded_rx.try_recv() {
            update_h264_param_cache(encoded.as_ref(), &mut last_sps, &mut last_pps);
            last_encoded = Some(encoded);
            got_fresh = true;
        }
        if !got_fresh
            && last_encoded.is_some()
            && let Ok(Some(v)) = tokio::time::timeout(Duration::from_millis(2), encoded_rx.recv()).await
        {
            update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
            last_encoded = Some(v);
            got_fresh = true;
        }
        let encoded = if let Some(v) = last_encoded.as_ref() {
            v.clone()
        } else {
            match encoded_rx.recv().await {
                Some(v) => {
                    update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
                    last_encoded = Some(v.clone());
                    got_fresh = true;
                    v
                }
                None => break,
            }
        };
        let send_fps = if got_fresh || !repeat_last {
            fps
        } else {
            idle_repeat_fps
        };
        let send_gap = Duration::from_millis((1000.0 / send_fps as f64).max(1.0) as u64);
        next_due = advance_send_deadline(next_due, send_gap, Instant::now());
        let au_for_send = if let Some(patched) =
            patch_h264_au_with_cached_params(encoded.as_ref(), &last_sps, &last_pps)
        {
            Arc::<[u8]>::from(patched)
        } else {
            encoded.clone()
        };
        if h264_debug && h264_debug_left > 0 {
            let nals = parse_annexb_nals_view(au_for_send.as_ref());
            let nal_types: Vec<u8> = nals.iter().map(|n| n.nal_type).collect();
            let has_sps = nal_types.contains(&7);
            let has_pps = nal_types.contains(&8);
            let has_idr = nal_types.contains(&5);
            let take = au_for_send.len().min(12);
            let mut head = String::new();
            for b in &au_for_send[..take] {
                use std::fmt::Write as _;
                let _ = write!(&mut head, "{:02X} ", b);
            }
            info!(
                au_bytes = au_for_send.len(),
                has_sps,
                has_pps,
                has_idr,
                nal_types = ?nal_types,
                head = %head.trim_end(),
                "h264 sample au debug"
            );
            h264_debug_left -= 1;
        }
        let sample = Sample {
            data: Bytes::copy_from_slice(au_for_send.as_ref()),
            duration: send_gap,
            ..Default::default()
        };
        if let Err(e) = track.write_sample(&sample).await {
            error!(error = %e, "sample write failed");
            break;
        }
        stats_send.sent_au_total.fetch_add(1, Ordering::Relaxed);
        stats_send.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
        if got_fresh {
            stats_send
                .unique_sent_au_total
                .fetch_add(1, Ordering::Relaxed);
        } else {
            stats_send
                .repeated_sent_au_total
                .fetch_add(1, Ordering::Relaxed);
        }
        if !repeat_last {
            last_encoded = None;
        }
    }
}

async fn spawn_send_loop_rtp(
    mut sender: RtpH264Sender,
    mut encoded_rx: tokio::sync::mpsc::Receiver<Arc<[u8]>>,
    adapt: Arc<NetAdaptController>,
    stats: Arc<RuntimeStats>,
    enable_network_adapt: bool,
    repeat_last_au_on_idle: bool,
    idle_repeat_fps: u32,
    session_running: Arc<AtomicBool>,
) {
    let mut next_due = Instant::now();
    let mut next_recover_tick = Instant::now();
    let mut last_encoded: Option<Arc<[u8]>> = None;
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let h264_debug = std::env::var("AGENT_H264_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut h264_debug_left = h264_debug_budget();
    let mut consecutive_send_errors: u32 = 0;
    while session_running.load(Ordering::SeqCst) {
        if enable_network_adapt && Instant::now() >= next_recover_tick {
            if let Some((fps_v, br_v)) = adapt.tick_recover() {
                info!(
                    target_fps = fps_v,
                    target_bitrate_kbps = br_v,
                    "network adapt recovered"
                );
            }
            next_recover_tick = Instant::now() + Duration::from_secs(1);
        }

        let target_fps = adapt.current_fps().max(1);
        let target_bitrate = adapt.current_bitrate_kbps().max(100);
        stats.target_fps.store(target_fps, Ordering::Relaxed);
        stats
            .target_bitrate_kbps
            .store(target_bitrate, Ordering::Relaxed);

        let idle_repeat_fps = idle_repeat_fps.max(1);
        wait_until_due(next_due).await;

        let mut got_fresh = false;
        while let Ok(encoded) = encoded_rx.try_recv() {
            update_h264_param_cache(encoded.as_ref(), &mut last_sps, &mut last_pps);
            last_encoded = Some(encoded);
            got_fresh = true;
        }
        if !got_fresh
            && last_encoded.is_some()
            && let Ok(Some(v)) =
                tokio::time::timeout(Duration::from_millis(2), encoded_rx.recv()).await
        {
            update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
            last_encoded = Some(v);
            got_fresh = true;
        }
        if last_encoded.is_none() {
            match encoded_rx.recv().await {
                Some(v) => {
                    update_h264_param_cache(v.as_ref(), &mut last_sps, &mut last_pps);
                    last_encoded = Some(v);
                    got_fresh = true;
                }
                None => break,
            }
        }
        let Some(encoded) = (if let Some(v) = last_encoded.as_ref() {
            Some(v.clone())
        } else {
            None
        }) else {
            continue;
        };
        let send_fps = if got_fresh || !repeat_last_au_on_idle {
            target_fps
        } else {
            idle_repeat_fps
        };
        let frame_gap = Duration::from_millis((1000.0 / send_fps as f64).max(1.0) as u64);
        next_due = advance_send_deadline(next_due, frame_gap, Instant::now());
        let au_for_send = if let Some(patched) =
            patch_h264_au_with_cached_params(encoded.as_ref(), &last_sps, &last_pps)
        {
            Arc::<[u8]>::from(patched)
        } else {
            encoded.clone()
        };
        if h264_debug && h264_debug_left > 0 {
            let nals = parse_annexb_nals_view(au_for_send.as_ref());
            let nal_types: Vec<u8> = nals.iter().map(|n| n.nal_type).collect();
            let has_sps = nal_types.contains(&7);
            let has_pps = nal_types.contains(&8);
            let has_idr = nal_types.contains(&5);
            let take = au_for_send.len().min(12);
            let mut head = String::new();
            for b in &au_for_send[..take] {
                use std::fmt::Write as _;
                let _ = write!(&mut head, "{:02X} ", b);
            }
            info!(
                au_bytes = au_for_send.len(),
                has_sps,
                has_pps,
                has_idr,
                nal_types = ?nal_types,
                head = %head.trim_end(),
                "h264 rtp au debug"
            );
            h264_debug_left -= 1;
        }
        if let Err(e) = sender.send_access_unit(au_for_send.as_ref()).await {
            consecutive_send_errors = consecutive_send_errors.saturating_add(1);
            warn!(
                error = %e,
                consecutive_send_errors,
                "RTP write failed, retrying"
            );
            // During ICE/DTLS startup, writes can transiently fail.
            // Keep session alive and retry instead of tearing media loop down.
            tokio::time::sleep(Duration::from_millis(5)).await;
            if consecutive_send_errors >= 400 {
                error!("too many consecutive RTP send failures, stopping RTP loop");
                break;
            }
            continue;
        }
        consecutive_send_errors = 0;
        stats.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
        stats.sent_au_total.fetch_add(1, Ordering::Relaxed);
        if got_fresh {
            stats.unique_sent_au_total.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.repeated_sent_au_total.fetch_add(1, Ordering::Relaxed);
        }
        if !repeat_last_au_on_idle {
            last_encoded = None;
        }
    }
}

async fn spawn_send_loop_quic(
    quic_sender: tokio::sync::mpsc::Sender<QuicAu>,
    mut encoded_rx: tokio::sync::mpsc::Receiver<Arc<[u8]>>,
    stats: Arc<RuntimeStats>,
    session_running: Arc<AtomicBool>,
) {
    let quic_debug = std::env::var("AGENT_QUIC_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut debug_left = 8usize;
    let max_au_bytes = std::env::var("AGENT_QUIC_MAX_AU_BYTES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1_500_000)
        .clamp(64 * 1024, 8 * 1024 * 1024);
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let mut dropped = 0_u64;
    while session_running.load(Ordering::SeqCst) {
        let encoded = match encoded_rx.recv().await {
            Some(v) => v,
            None => break,
        };
        let (capture_start_us, payload) = unpack_capture_ts_au(encoded.as_ref());
        let mut out = payload.to_vec();
        update_h264_param_cache(&out, &mut last_sps, &mut last_pps);
        if let Some(patched) = patch_h264_au_with_cached_params(&out, &last_sps, &last_pps) {
            out = patched;
        }
        if quic_debug && debug_left > 0 {
            let take = out.len().min(12);
            let mut head = String::new();
            for b in &out[..take] {
                use std::fmt::Write as _;
                let _ = write!(&mut head, "{:02X} ", b);
            }
            let nal_types: Vec<u8> = parse_annexb_nals_view(&out)
                .iter()
                .map(|n| n.nal_type)
                .collect();
            info!(
                au_bytes = out.len(),
                head = %head.trim_end(),
                nal_types = ?nal_types,
                "quic debug access-unit"
            );
            debug_left -= 1;
        }
        if out.len() > max_au_bytes {
            dropped = dropped.saturating_add(1);
            stats.quic_au_dropped.fetch_add(1, Ordering::Relaxed);
            if dropped.is_multiple_of(60) {
                warn!(
                    dropped,
                    au_bytes = out.len(),
                    max_au_bytes,
                    "quic dropped oversized access-unit"
                );
            }
            continue;
        }
        let out = Arc::<[u8]>::from(out);
        let quic_au = QuicAu {
            payload: out.clone(),
            tx_unix_us: if capture_start_us == 0 {
                unix_time_us()
            } else {
                capture_start_us
            },
        };
        match quic_sender.try_send(quic_au) {
            Ok(()) => {
                stats.sent_au_total.fetch_add(1, Ordering::Relaxed);
                stats.unique_sent_au_total.fetch_add(1, Ordering::Relaxed);
                stats.quic_au_sent.fetch_add(1, Ordering::Relaxed);
                stats
                    .quic_bytes_sent
                    .fetch_add(out.len() as u64, Ordering::Relaxed);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                dropped = dropped.saturating_add(1);
                stats.quic_au_dropped.fetch_add(1, Ordering::Relaxed);
                if dropped.is_multiple_of(120) {
                    warn!(dropped, "quic sender saturated, dropping stale frames");
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                error!("quic sender channel closed");
                break;
            }
        }
    }
}

fn patch_h264_au_with_cached_params(
    au: &[u8],
    sps: &Option<Vec<u8>>,
    pps: &Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if sps.is_none() || pps.is_none() {
        return None;
    }
    let nals = parse_annexb_nals_view(au);
    if nals.is_empty() {
        return None;
    }
    let has_idr = nals.iter().any(|n| n.nal_type == 5);
    let has_sps = nals.iter().any(|n| n.nal_type == 7);
    let has_pps = nals.iter().any(|n| n.nal_type == 8);
    if !has_idr || (has_sps && has_pps) {
        return None;
    }
    let mut out = Vec::with_capacity(au.len() + sps.as_ref().map_or(0, |v| v.len()) + pps.as_ref().map_or(0, |v| v.len()));
    if let Some(v) = sps {
        out.extend_from_slice(v);
    }
    if let Some(v) = pps {
        out.extend_from_slice(v);
    }
    out.extend_from_slice(au);
    Some(out)
}

fn update_h264_param_cache(au: &[u8], sps: &mut Option<Vec<u8>>, pps: &mut Option<Vec<u8>>) {
    for n in parse_annexb_nals_view(au) {
        if n.nal_type == 7 {
            *sps = Some(n.bytes.to_vec());
        } else if n.nal_type == 8 {
            *pps = Some(n.bytes.to_vec());
        }
    }
}

struct AnnexbNalView<'a> {
    nal_type: u8,
    bytes: &'a [u8],
}

fn parse_annexb_nals_view(buf: &[u8]) -> Vec<AnnexbNalView<'_>> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 < buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            starts.push((i, 3usize));
            i += 3;
            continue;
        }
        if i + 4 < buf.len() && buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 0 && buf[i + 3] == 1 {
            starts.push((i, 4usize));
            i += 4;
            continue;
        }
        i += 1;
    }
    if starts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(starts.len());
    for (idx, (sc, sclen)) in starts.iter().enumerate() {
        let start = sc + sclen;
        let end = if idx + 1 < starts.len() { starts[idx + 1].0 } else { buf.len() };
        if start >= end || end > buf.len() {
            continue;
        }
        let nal = &buf[*sc..end];
        out.push(AnnexbNalView {
            nal_type: buf[start] & 0x1f,
            bytes: nal,
        });
    }
    out
}

async fn ws_send_json(ws: &Arc<Mutex<WsWrite>>, v: &Value) -> Result<()> {
    let text = v.to_string();
    let mut w = ws.lock().await;
    w.send(Message::Text(text))
        .await
        .map_err(|e| anyhow!("ws send failed: {e}"))
}

fn advance_send_deadline(prev_due: Instant, gap: Duration, now: Instant) -> Instant {
    let next = prev_due + gap;
    if next < now { now } else { next }
}

async fn wait_until_due(deadline: Instant) {
    // On Windows, short tokio::sleep durations are often rounded by coarse timer
    // granularity (~15.6ms). Keep the final short wait in cooperative/yield-spin
    // mode so high-fps pacing is not collapsed to ~64fps.
    const COARSE_SLEEP_GUARD: Duration = Duration::from_millis(12);
    const YIELD_SPIN_THRESHOLD: Duration = Duration::from_micros(200);
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remain = deadline - now;
        if remain > COARSE_SLEEP_GUARD {
            tokio::time::sleep(remain - COARSE_SLEEP_GUARD).await;
        } else if remain > YIELD_SPIN_THRESHOLD {
            tokio::task::yield_now().await;
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_send_deadline_keeps_constant_cadence_when_not_late() {
        let now = Instant::now();
        let prev = now + Duration::from_millis(20);
        let gap = Duration::from_millis(16);
        let next = advance_send_deadline(prev, gap, now);
        assert_eq!(next, prev + gap);
    }

    #[test]
    fn advance_send_deadline_catches_up_when_late() {
        let now = Instant::now();
        let prev = now - Duration::from_millis(50);
        let gap = Duration::from_millis(16);
        let next = advance_send_deadline(prev, gap, now);
        assert_eq!(next, now);
    }

    #[tokio::test]
    async fn wait_until_due_preserves_sub_10ms_deadline() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(4);
        wait_until_due(deadline).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(12),
            "wait_until_due overslept short deadline: elapsed={elapsed:?}"
        );
    }

    #[test]
    fn nvenc_recreate_env_parse_defaults_to_enabled() {
        assert!(nvenc_recreate_on_force_idr_enabled_from(None));
        assert!(nvenc_recreate_on_force_idr_enabled_from(Some("1")));
        assert!(nvenc_recreate_on_force_idr_enabled_from(Some("true")));
        assert!(nvenc_recreate_on_force_idr_enabled_from(Some("yes")));
    }

    #[test]
    fn nvenc_recreate_env_parse_allows_disable() {
        assert!(!nvenc_recreate_on_force_idr_enabled_from(Some("0")));
        assert!(!nvenc_recreate_on_force_idr_enabled_from(Some("false")));
        assert!(!nvenc_recreate_on_force_idr_enabled_from(Some("off")));
        assert!(!nvenc_recreate_on_force_idr_enabled_from(Some("no")));
    }

    #[test]
    fn recreate_policy_only_for_webrtc_nvenc_with_external_keyframe_request() {
        assert!(should_recreate_nvenc_on_force_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::Nvenc,
            true,
        ));
        assert!(!should_recreate_nvenc_on_force_idr(
            SessionTransport::Quic,
            VideoEncoderBackend::Nvenc,
            true,
        ));
        assert!(!should_recreate_nvenc_on_force_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::OpenH264,
            true,
        ));
        assert!(!should_recreate_nvenc_on_force_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::Nvenc,
            false,
        ));
    }
}
