mod audio_control;
mod capture_policy;
mod capture_runtime;
mod clipboard;
mod control_plane;
mod encoder_policy;
mod encoder_runtime;
mod file_ops;
mod file_transfer;
mod input_injector;
mod net_adapt;
mod nvenc_native;
mod profile;
mod quic_tx;
mod rclone_mount;
mod rtp_send;
mod runtime_stats;
mod security;
mod webdav_client;
mod webdav_mount;
mod webtransport_tx;

use crate::capture_policy::{CaptureBackend, choose_backend};
#[cfg(windows)]
use crate::capture_runtime::WgcWindowCapturer;
use crate::capture_runtime::{
    RawFrame, build_frame_capturer, detect_input_resolution, resize_rgba_fast, sleep_until,
};
use crate::encoder_policy::{VideoEncoderBackend, choose_encoder_backend};
use crate::encoder_runtime::{build_video_encoder, encode_rgba_frame, request_keyframe};
use crate::input_injector::InputInjector;
use crate::net_adapt::NetAdaptController;
use crate::nvenc_native::{NativeEncodePath, NativeNvencPipeline, NativeNvencTexturePipeline};
use crate::profile::apply_capture_profile;
use crate::quic_tx::{QuicAu, QuicServerAdvert, start_quic_sender};
use crate::rtp_send::{RtpH264Sender, RtpH264SenderConfig, TX_UNIX_US_EXT_URI};
use crate::runtime_stats::{RuntimeStats, spawn_rtcp_feedback_loop, spawn_stats_panel};
use crate::webtransport_tx::{WebTransportAdvert, start_webtransport_sender};
use agent_rust::load_config;
use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use common_control_proto::ChannelClass;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{error, info, warn};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc::ice_transport::ice_candidate_pair::RTCIceCandidatePair;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpHeaderExtensionCapability, RTPCodecType,
};
use webrtc::rtp_transceiver::{
    RTCPFeedback, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
    TYPE_RTCP_FB_TRANSPORT_CC,
};
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

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
    WebTransport,
}

impl SessionTransport {
    fn parse(v: Option<&str>) -> Self {
        match v.unwrap_or("webrtc").to_ascii_lowercase().as_str() {
            "quic" => SessionTransport::Quic,
            "webtransport" => SessionTransport::WebTransport,
            _ => SessionTransport::WebRtc,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            SessionTransport::WebRtc => "webrtc",
            SessionTransport::Quic => "quic",
            SessionTransport::WebTransport => "webtransport",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

impl VideoCodec {
    fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("h264").trim().to_ascii_lowercase().as_str() {
            "hevc" | "h265" => VideoCodec::Hevc,
            "av1" => VideoCodec::Av1,
            _ => VideoCodec::H264,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::Av1 => "av1",
        }
    }
}

fn parse_transport_priority(raw: &str) -> Vec<SessionTransport> {
    raw.split(',')
        .filter_map(|s| match s.trim().to_ascii_lowercase().as_str() {
            "webrtc" => Some(SessionTransport::WebRtc),
            "quic" => Some(SessionTransport::Quic),
            "webtransport" => Some(SessionTransport::WebTransport),
            _ => None,
        })
        .collect()
}

fn parse_codec_priority(raw: &str) -> Vec<VideoCodec> {
    raw.split(',')
        .filter_map(|s| {
            let codec = VideoCodec::parse(Some(s.trim()));
            if s.trim().is_empty() {
                None
            } else {
                Some(codec)
            }
        })
        .collect()
}

fn controller_supports_codec(controller_caps: &Value, codec: VideoCodec) -> bool {
    let wanted = codec.as_str();
    let Some(codecs) = controller_caps.get("codecs").and_then(|v| v.as_array()) else {
        return codec == VideoCodec::H264;
    };
    if codecs.is_empty() {
        return codec == VideoCodec::H264;
    }
    codecs
        .iter()
        .filter_map(|v| v.as_str())
        .any(|v| v.eq_ignore_ascii_case(wanted))
}

fn select_codec_by_strategy(
    selected_transport: SessionTransport,
    controller_caps: &Value,
) -> VideoCodec {
    let force = std::env::var("AGENT_CODEC_FORCE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| VideoCodec::parse(Some(v.as_str())));
    if let Some(codec) = force {
        if selected_transport == SessionTransport::WebRtc && codec != VideoCodec::H264 {
            return VideoCodec::H264;
        }
        return codec;
    }
    let priority_raw = std::env::var("AGENT_CODEC_PRIORITY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "av1,hevc,h264".to_string());
    let mut priority = parse_codec_priority(&priority_raw);
    if priority.is_empty() {
        priority = vec![VideoCodec::H264];
    }
    if selected_transport == SessionTransport::WebRtc {
        return VideoCodec::H264;
    }
    for codec in priority {
        if controller_supports_codec(controller_caps, codec) {
            return codec;
        }
    }
    VideoCodec::H264
}

fn transport_allowed_by_env(transport: SessionTransport) -> bool {
    let key = match transport {
        SessionTransport::WebRtc => "AGENT_TRANSPORT_ENABLE_WEBRTC",
        SessionTransport::Quic => "AGENT_TRANSPORT_ENABLE_QUIC",
        SessionTransport::WebTransport => "AGENT_TRANSPORT_ENABLE_WEBTRANSPORT",
    };
    std::env::var(key)
        .ok()
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

fn controller_supports_transport(controller_caps: &Value, transport: SessionTransport) -> bool {
    let wanted = transport.as_str();
    let Some(protocols) = controller_caps.get("protocols").and_then(|v| v.as_array()) else {
        // Backward compatibility: old clients might not provide capabilities.
        return true;
    };
    if protocols.is_empty() {
        return true;
    }
    protocols
        .iter()
        .filter_map(|v| v.as_str())
        .any(|v| v.eq_ignore_ascii_case(wanted))
}

fn select_transport_by_strategy(
    requested: SessionTransport,
    controller_caps: &Value,
    concurrent_clients: usize,
) -> SessionTransport {
    let auto = std::env::var("AGENT_TRANSPORT_AUTO_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let multi_client_upgrade_at = std::env::var("AGENT_TRANSPORT_MULTI_CLIENT_UPGRADE_AT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 64);
    let default_priority = "webtransport,quic,webrtc".to_string();
    let priority = std::env::var("AGENT_TRANSPORT_AUTO_PRIORITY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(default_priority);
    let mut order = Vec::new();
    order.push(requested);
    if auto {
        let priority_list = parse_transport_priority(&priority);
        if concurrent_clients >= multi_client_upgrade_at && requested == SessionTransport::WebRtc {
            order.extend(
                priority_list
                    .iter()
                    .copied()
                    .filter(|v| *v != SessionTransport::WebRtc),
            );
            order.push(SessionTransport::WebRtc);
        } else {
            order.extend(priority_list);
        }
    } else {
        order.extend([
            SessionTransport::WebRtc,
            SessionTransport::Quic,
            SessionTransport::WebTransport,
        ]);
    }

    let mut dedup = Vec::new();
    for t in order {
        if dedup.contains(&t) {
            continue;
        }
        dedup.push(t);
    }

    for candidate in dedup {
        if transport_allowed_by_env(candidate)
            && controller_supports_transport(controller_caps, candidate)
        {
            return candidate;
        }
    }

    SessionTransport::WebRtc
}

const CAPTURE_TS_MAGIC: &[u8; 4] = b"TSU1";
static ACTIVE_STREAM_SESSIONS: AtomicU32 = AtomicU32::new(0);
static SHARED_ENCODED_HUB: OnceLock<Arc<SharedEncodedHub>> = OnceLock::new();

struct SharedEncodedHub {
    tx: tokio::sync::broadcast::Sender<Arc<[u8]>>,
    fps_ref: Arc<AtomicU32>,
}

fn decrement_active_stream_sessions() {
    loop {
        let cur = ACTIVE_STREAM_SESSIONS.load(Ordering::Relaxed);
        if cur == 0 {
            return;
        }
        if ACTIVE_STREAM_SESSIONS
            .compare_exchange(cur, cur - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
    }
}

fn apply_stream_fair_share(cfg: &mut agent_rust::CaptureConfig, active_sessions: u32) {
    let enable = std::env::var("AGENT_STREAM_FAIR_SHARE_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if !enable || active_sessions <= 1 {
        return;
    }
    let total_fps_budget = std::env::var("AGENT_STREAM_TOTAL_FPS_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(120)
        .clamp(1, 1000);
    let total_bitrate_budget = std::env::var("AGENT_STREAM_TOTAL_BITRATE_BUDGET_KBPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(24_000)
        .clamp(100, 1_000_000);
    let min_fps = std::env::var("AGENT_STREAM_MIN_FPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(8)
        .clamp(1, 240);
    let min_bitrate = std::env::var("AGENT_STREAM_MIN_BITRATE_KBPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1200)
        .clamp(100, 300_000);

    let target_fps = (total_fps_budget / active_sessions.max(1))
        .clamp(min_fps, cfg.max_fps.max(1))
        .clamp(cfg.min_fps.max(1), cfg.max_fps.max(1));
    let target_br = (total_bitrate_budget / active_sessions.max(1))
        .max(min_bitrate)
        .min(cfg.max_bitrate_kbps.max(min_bitrate));

    cfg.fps = target_fps;
    cfg.max_fps = cfg.max_fps.min(target_fps).max(1);
    cfg.min_fps = cfg.min_fps.min(target_fps).max(1);
    cfg.tier_fps_l1 = cfg.tier_fps_l1.min(target_fps).max(1);
    cfg.tier_fps_l2 = cfg.tier_fps_l2.min(target_fps).max(1);
    cfg.tier_fps_l3 = cfg.tier_fps_l3.min(target_fps).max(1);
    cfg.tier_fps_l4 = cfg.tier_fps_l4.min(target_fps).max(1);
    cfg.tier_fps_l5 = cfg.tier_fps_l5.min(target_fps).max(1);
    cfg.bitrate_kbps = target_br;
    info!(
        active_sessions,
        fair_fps = cfg.fps,
        fair_bitrate_kbps = cfg.bitrate_kbps,
        "applied stream fair-share"
    );
}

fn shared_pipeline_enabled() -> bool {
    std::env::var("AGENT_SHARED_CAPTURE_ENCODE_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

fn roi_map_requested() -> bool {
    std::env::var("AGENT_ROI_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        && std::env::var("AGENT_ROI_RECT")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
}

fn roi_require_native() -> bool {
    std::env::var("AGENT_ROI_REQUIRE_NATIVE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

fn nvenc_native_roi_enabled() -> bool {
    std::env::var("AGENT_NVENC_NATIVE_ROI_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn effective_roi_request(
    roi_requested: bool,
    native_roi_enabled: bool,
    _require_native: bool,
) -> bool {
    if !roi_requested {
        return false;
    }
    // Default safety policy: do not allow ffmpeg ROI fallback when native ROI is unavailable.
    if !native_roi_enabled {
        return false;
    }
    true
}

fn get_or_start_shared_encoded_hub(
    effective_cfg: &agent_rust::CaptureConfig,
    backend: CaptureBackend,
    encoder_backend: VideoEncoderBackend,
    with_capture_ts_header: bool,
) -> Result<Arc<SharedEncodedHub>> {
    if let Some(hub) = SHARED_ENCODED_HUB.get() {
        return Ok(hub.clone());
    }
    let cfg = effective_cfg.clone();
    let capacity = cfg.queue_depth.clamp(8, 256) as usize;
    let initial_fps = cfg.fps.clamp(cfg.min_fps.max(1), cfg.max_fps.max(1)).max(1);
    let fps_ref = Arc::new(AtomicU32::new(initial_fps));
    let (tx, _) = tokio::sync::broadcast::channel::<Arc<[u8]>>(capacity);
    let hub = Arc::new(SharedEncodedHub {
        tx: tx.clone(),
        fps_ref: fps_ref.clone(),
    });
    if SHARED_ENCODED_HUB.set(hub.clone()).is_err() {
        if let Some(existing) = SHARED_ENCODED_HUB.get() {
            return Ok(existing.clone());
        }
    }

    std::thread::spawn(move || {
        let probe_enable = std::env::var("AGENT_SHARED_PIPELINE_PROBE_ENABLE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let probe_interval_ms = std::env::var("AGENT_SHARED_PIPELINE_PROBE_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1000)
            .clamp(200, 10_000);
        let probe_interval = Duration::from_millis(probe_interval_ms);
        info!(
            shared_fps = fps_ref.load(Ordering::Relaxed),
            probe_enable, probe_interval_ms, "shared pipeline loop started"
        );
        let roi_requested_raw = roi_map_requested();
        let native_roi = nvenc_native_roi_enabled();
        let roi_require_native = roi_require_native();
        let roi_requested =
            effective_roi_request(roi_requested_raw, native_roi, roi_require_native);
        if roi_requested_raw && roi_require_native && !native_roi {
            info!(
                "shared pipeline: ROI requested with requireNative=true but native ROI unsupported; disabling ROI fallback"
            );
        } else if roi_requested_raw && !roi_require_native && !native_roi {
            info!(
                "shared pipeline: ROI quality mode requested but native ROI unsupported; blocking ffmpeg ROI fallback by policy"
            );
        } else if roi_requested && !native_roi {
            info!(
                "shared pipeline: ROI map requested, using ffmpeg nvenc path (native nvenc ROI map disabled)"
            );
        }
        if encoder_backend == VideoEncoderBackend::Nvenc
            && backend == CaptureBackend::Dxgi
            && (!roi_requested || native_roi)
        {
            let (input_w, input_h) = match detect_input_resolution() {
                Ok(v) => v,
                Err(e) => {
                    warn!(
                        error = %e,
                        "shared pipeline detect input resolution failed; falling back to CPU path"
                    );
                    (0, 0)
                }
            };
            let target_w = if cfg.target_width > 0 {
                cfg.target_width
            } else {
                input_w.max(2)
            };
            let target_h = if cfg.target_height > 0 {
                cfg.target_height
            } else {
                input_h.max(2)
            };
            if target_w > 0 && target_h > 0 {
                match NativeNvencPipeline::new(target_w, target_h, &cfg) {
                    Ok(mut native) => {
                        info!(
                            target_w,
                            target_h,
                            adapter = %native.adapter_summary(),
                            "shared pipeline native NVENC dxgi path enabled"
                        );
                        let mut next_tick = Instant::now();
                        let mut probe_last = Instant::now();
                        let mut probe_frames: u64 = 0;
                        let mut probe_loop_count: u64 = 0;
                        let mut probe_wait_us: u128 = 0;
                        let mut probe_capture_us: u128 = 0;
                        let mut probe_resize_us: u128 = 0;
                        let mut probe_encode_us: u128 = 0;
                        let mut probe_capture_err: u64 = 0;
                        let mut probe_encode_err: u64 = 0;
                        let mut probe_encode_empty: u64 = 0;
                        let mut probe_sent: u64 = 0;
                        let mut probe_dropped: u64 = 0;
                        loop {
                            let loop_start = Instant::now();
                            wait_encode_tick(
                                &mut next_tick,
                                fps_ref.load(Ordering::Relaxed).max(1),
                            );
                            probe_wait_us =
                                probe_wait_us.saturating_add(loop_start.elapsed().as_micros());
                            let work_start = Instant::now();
                            match native.encode_next(false) {
                                Ok(Some(v)) if !v.bytes.is_empty() => {
                                    probe_capture_us = probe_capture_us
                                        .saturating_add(work_start.elapsed().as_micros());
                                    let encoded = pack_capture_ts_au(
                                        v.bytes,
                                        v.capture_start_us,
                                        with_capture_ts_header,
                                    );
                                    if tx.send(encoded).is_ok() {
                                        probe_sent = probe_sent.saturating_add(1);
                                    } else {
                                        probe_dropped = probe_dropped.saturating_add(1);
                                    }
                                    probe_frames = probe_frames.saturating_add(1);
                                }
                                Ok(_) => {
                                    probe_capture_us = probe_capture_us
                                        .saturating_add(work_start.elapsed().as_micros());
                                    probe_encode_empty = probe_encode_empty.saturating_add(1);
                                }
                                Err(e) => {
                                    probe_capture_us = probe_capture_us
                                        .saturating_add(work_start.elapsed().as_micros());
                                    probe_encode_err = probe_encode_err.saturating_add(1);
                                    warn!(error = %e, "shared native nvenc encode_next failed");
                                    std::thread::sleep(Duration::from_millis(2));
                                }
                            }
                            probe_loop_count = probe_loop_count.saturating_add(1);
                            if probe_enable && probe_last.elapsed() >= probe_interval {
                                let elapsed_s = probe_last.elapsed().as_secs_f64().max(0.001);
                                let fps = probe_frames as f64 / elapsed_s;
                                let wait_ms = (probe_wait_us as f64 / 1000.0) / elapsed_s;
                                let cap_ms = (probe_capture_us as f64 / 1000.0) / elapsed_s;
                                let resize_ms = (probe_resize_us as f64 / 1000.0) / elapsed_s;
                                let enc_ms = (probe_encode_us as f64 / 1000.0) / elapsed_s;
                                info!(
                                    shared_target_fps = fps_ref.load(Ordering::Relaxed),
                                    shared_fps = format!("{fps:.2}"),
                                    loops = probe_loop_count,
                                    sent = probe_sent,
                                    dropped = probe_dropped,
                                    capture_err = probe_capture_err,
                                    encode_err = probe_encode_err,
                                    encode_empty = probe_encode_empty,
                                    wait_ms_per_s = format!("{wait_ms:.2}"),
                                    capture_ms_per_s = format!("{cap_ms:.2}"),
                                    resize_ms_per_s = format!("{resize_ms:.2}"),
                                    encode_ms_per_s = format!("{enc_ms:.2}"),
                                    "shared pipeline probe"
                                );
                                probe_last = Instant::now();
                                probe_frames = 0;
                                probe_loop_count = 0;
                                probe_wait_us = 0;
                                probe_capture_us = 0;
                                probe_resize_us = 0;
                                probe_encode_us = 0;
                                probe_capture_err = 0;
                                probe_encode_err = 0;
                                probe_encode_empty = 0;
                                probe_sent = 0;
                                probe_dropped = 0;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            "shared native NVENC dxgi init failed; falling back to CPU path"
                        );
                    }
                }
            }
        }

        if encoder_backend == VideoEncoderBackend::Nvenc
            && backend == CaptureBackend::Wgc
            && (!roi_requested || native_roi)
        {
            #[cfg(windows)]
            {
                if let Ok(mut wgc) = WgcWindowCapturer::new() {
                    if let Ok(first) = wgc.capture_gpu_frame(Duration::from_millis(250)) {
                        let target_w = if cfg.target_width > 0 {
                            cfg.target_width
                        } else {
                            first.width
                        };
                        let target_h = if cfg.target_height > 0 {
                            cfg.target_height
                        } else {
                            first.height
                        };
                        match NativeNvencTexturePipeline::new(
                            wgc.device(),
                            wgc.context(),
                            target_w,
                            target_h,
                            &cfg,
                        ) {
                            Ok(mut native) => {
                                info!(
                                    target_w,
                                    target_h,
                                    "shared pipeline native NVENC wgc-texture path enabled"
                                );
                                let mut next_tick = Instant::now();
                                let mut probe_last = Instant::now();
                                let mut probe_frames: u64 = 0;
                                let mut probe_loop_count: u64 = 0;
                                let mut probe_wait_us: u128 = 0;
                                let mut probe_capture_us: u128 = 0;
                                let mut probe_resize_us: u128 = 0;
                                let mut probe_encode_us: u128 = 0;
                                let mut probe_capture_err: u64 = 0;
                                let mut probe_encode_err: u64 = 0;
                                let mut probe_encode_empty: u64 = 0;
                                let mut probe_sent: u64 = 0;
                                let mut probe_dropped: u64 = 0;
                                loop {
                                    let loop_start = Instant::now();
                                    wait_encode_tick(
                                        &mut next_tick,
                                        fps_ref.load(Ordering::Relaxed).max(1),
                                    );
                                    probe_wait_us = probe_wait_us
                                        .saturating_add(loop_start.elapsed().as_micros());
                                    let capture_start = Instant::now();
                                    let captured = match wgc
                                        .capture_gpu_frame(Duration::from_millis(120))
                                    {
                                        Ok(v) => v,
                                        Err(e) => {
                                            probe_capture_us = probe_capture_us.saturating_add(
                                                capture_start.elapsed().as_micros(),
                                            );
                                            probe_capture_err = probe_capture_err.saturating_add(1);
                                            warn!(error = %e, "shared wgc capture failed");
                                            continue;
                                        }
                                    };
                                    probe_capture_us = probe_capture_us
                                        .saturating_add(capture_start.elapsed().as_micros());
                                    let encode_start = Instant::now();
                                    match native.encode_texture(&captured.texture, false) {
                                        Ok(Some(v)) if !v.bytes.is_empty() => {
                                            probe_encode_us = probe_encode_us
                                                .saturating_add(encode_start.elapsed().as_micros());
                                            let encoded = pack_capture_ts_au(
                                                v.bytes,
                                                if captured.capture_start_us == 0 {
                                                    v.capture_start_us
                                                } else {
                                                    captured.capture_start_us
                                                },
                                                with_capture_ts_header,
                                            );
                                            if tx.send(encoded).is_ok() {
                                                probe_sent = probe_sent.saturating_add(1);
                                            } else {
                                                probe_dropped = probe_dropped.saturating_add(1);
                                            }
                                            probe_frames = probe_frames.saturating_add(1);
                                        }
                                        Ok(_) => {
                                            probe_encode_us = probe_encode_us
                                                .saturating_add(encode_start.elapsed().as_micros());
                                            probe_encode_empty =
                                                probe_encode_empty.saturating_add(1);
                                        }
                                        Err(e) => {
                                            probe_encode_us = probe_encode_us
                                                .saturating_add(encode_start.elapsed().as_micros());
                                            probe_encode_err = probe_encode_err.saturating_add(1);
                                            warn!(error = %e, "shared wgc native encode failed");
                                        }
                                    }
                                    probe_loop_count = probe_loop_count.saturating_add(1);
                                    if probe_enable && probe_last.elapsed() >= probe_interval {
                                        let elapsed_s =
                                            probe_last.elapsed().as_secs_f64().max(0.001);
                                        let fps = probe_frames as f64 / elapsed_s;
                                        let wait_ms = (probe_wait_us as f64 / 1000.0) / elapsed_s;
                                        let cap_ms = (probe_capture_us as f64 / 1000.0) / elapsed_s;
                                        let resize_ms =
                                            (probe_resize_us as f64 / 1000.0) / elapsed_s;
                                        let enc_ms = (probe_encode_us as f64 / 1000.0) / elapsed_s;
                                        info!(
                                            shared_target_fps = fps_ref.load(Ordering::Relaxed),
                                            shared_fps = format!("{fps:.2}"),
                                            loops = probe_loop_count,
                                            sent = probe_sent,
                                            dropped = probe_dropped,
                                            capture_err = probe_capture_err,
                                            encode_err = probe_encode_err,
                                            encode_empty = probe_encode_empty,
                                            wait_ms_per_s = format!("{wait_ms:.2}"),
                                            capture_ms_per_s = format!("{cap_ms:.2}"),
                                            resize_ms_per_s = format!("{resize_ms:.2}"),
                                            encode_ms_per_s = format!("{enc_ms:.2}"),
                                            "shared pipeline probe"
                                        );
                                        probe_last = Instant::now();
                                        probe_frames = 0;
                                        probe_loop_count = 0;
                                        probe_wait_us = 0;
                                        probe_capture_us = 0;
                                        probe_resize_us = 0;
                                        probe_encode_us = 0;
                                        probe_capture_err = 0;
                                        probe_encode_err = 0;
                                        probe_encode_empty = 0;
                                        probe_sent = 0;
                                        probe_dropped = 0;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    error = %e,
                                    "shared native NVENC wgc-texture init failed; falling back to CPU path"
                                );
                            }
                        }
                    } else {
                        warn!("shared WGC warmup capture failed; falling back to CPU path");
                    }
                } else {
                    warn!("shared WGC capturer init failed; falling back to CPU path");
                }
            }
            #[cfg(not(windows))]
            {
                warn!("shared WGC native path requires Windows; falling back to CPU path");
            }
        }

        let mut capturer = match build_frame_capturer(backend) {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "shared pipeline capture initialization failed");
                return;
            }
        };
        let mut encoder =
            match build_video_encoder(initial_fps, &cfg, encoder_backend, true, "quic") {
                Ok(v) => v,
                Err(e) => {
                    error!(error = %e, "shared pipeline encoder initialization failed");
                    return;
                }
            };
        let mut next_tick = Instant::now();
        let mut probe_last = Instant::now();
        let mut probe_frames: u64 = 0;
        let mut probe_loop_count: u64 = 0;
        let mut probe_wait_us: u128 = 0;
        let mut probe_capture_us: u128 = 0;
        let mut probe_resize_us: u128 = 0;
        let mut probe_encode_us: u128 = 0;
        let mut probe_capture_err: u64 = 0;
        let mut probe_encode_err: u64 = 0;
        let mut probe_encode_empty: u64 = 0;
        let mut probe_sent: u64 = 0;
        let mut probe_dropped: u64 = 0;
        loop {
            let loop_start = Instant::now();
            wait_encode_tick(&mut next_tick, fps_ref.load(Ordering::Relaxed).max(1));
            probe_wait_us = probe_wait_us.saturating_add(loop_start.elapsed().as_micros());
            let capture_start = Instant::now();
            match capturer.capture() {
                Ok((mut rgba, mut width, mut height)) => {
                    probe_capture_us =
                        probe_capture_us.saturating_add(capture_start.elapsed().as_micros());
                    if cfg.target_width > 0
                        && cfg.target_height > 0
                        && (cfg.target_width != width || cfg.target_height != height)
                    {
                        let resize_start = Instant::now();
                        if let Some((resized, rw, rh)) = resize_rgba_fast(
                            &rgba,
                            width,
                            height,
                            cfg.target_width,
                            cfg.target_height,
                        ) {
                            rgba = resized;
                            width = rw;
                            height = rh;
                        }
                        probe_resize_us =
                            probe_resize_us.saturating_add(resize_start.elapsed().as_micros());
                    }
                    let capture_start_us = unix_time_us();
                    let encode_start = Instant::now();
                    match encode_rgba_frame(
                        &mut encoder,
                        &rgba,
                        width,
                        height,
                        Some(cfg.bitrate_kbps.max(100)),
                        false,
                    ) {
                        Ok(encoded) if !encoded.is_empty() => {
                            probe_encode_us =
                                probe_encode_us.saturating_add(encode_start.elapsed().as_micros());
                            let encoded = pack_capture_ts_au(
                                encoded,
                                capture_start_us,
                                with_capture_ts_header,
                            );
                            if tx.send(encoded).is_ok() {
                                probe_sent = probe_sent.saturating_add(1);
                            } else {
                                probe_dropped = probe_dropped.saturating_add(1);
                            }
                            probe_frames = probe_frames.saturating_add(1);
                        }
                        Ok(_) => {
                            probe_encode_us =
                                probe_encode_us.saturating_add(encode_start.elapsed().as_micros());
                            probe_encode_empty = probe_encode_empty.saturating_add(1);
                        }
                        Err(e) => {
                            probe_encode_us =
                                probe_encode_us.saturating_add(encode_start.elapsed().as_micros());
                            probe_encode_err = probe_encode_err.saturating_add(1);
                            warn!(error = %e, "shared pipeline encode failed");
                        }
                    }
                }
                Err(e) => {
                    probe_capture_us =
                        probe_capture_us.saturating_add(capture_start.elapsed().as_micros());
                    probe_capture_err = probe_capture_err.saturating_add(1);
                    warn!(error = %e, "shared pipeline capture failed");
                }
            }
            probe_loop_count = probe_loop_count.saturating_add(1);
            if probe_enable && probe_last.elapsed() >= probe_interval {
                let elapsed_s = probe_last.elapsed().as_secs_f64().max(0.001);
                let fps = probe_frames as f64 / elapsed_s;
                let wait_ms = (probe_wait_us as f64 / 1000.0) / elapsed_s;
                let cap_ms = (probe_capture_us as f64 / 1000.0) / elapsed_s;
                let resize_ms = (probe_resize_us as f64 / 1000.0) / elapsed_s;
                let enc_ms = (probe_encode_us as f64 / 1000.0) / elapsed_s;
                info!(
                    shared_target_fps = fps_ref.load(Ordering::Relaxed),
                    shared_fps = format!("{fps:.2}"),
                    loops = probe_loop_count,
                    sent = probe_sent,
                    dropped = probe_dropped,
                    capture_err = probe_capture_err,
                    encode_err = probe_encode_err,
                    encode_empty = probe_encode_empty,
                    wait_ms_per_s = format!("{wait_ms:.2}"),
                    capture_ms_per_s = format!("{cap_ms:.2}"),
                    resize_ms_per_s = format!("{resize_ms:.2}"),
                    encode_ms_per_s = format!("{enc_ms:.2}"),
                    "shared pipeline probe"
                );
                probe_last = Instant::now();
                probe_frames = 0;
                probe_loop_count = 0;
                probe_wait_us = 0;
                probe_capture_us = 0;
                probe_resize_us = 0;
                probe_encode_us = 0;
                probe_capture_err = 0;
                probe_encode_err = 0;
                probe_encode_empty = 0;
                probe_sent = 0;
                probe_dropped = 0;
            }
        }
    });

    Ok(hub)
}

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
        _ => false,
    }
}

fn nvenc_recreate_on_force_idr_enabled() -> bool {
    nvenc_recreate_on_force_idr_enabled_from(
        std::env::var("AGENT_NVENC_RECREATE_ON_FORCE_IDR")
            .ok()
            .as_deref(),
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

fn should_recreate_nvenc_on_missing_idr(
    selected_transport: SessionTransport,
    encoder_backend: VideoEncoderBackend,
    missing_idr_streak: u32,
) -> bool {
    let threshold = std::env::var("AGENT_NVENC_MISSING_IDR_RECREATE_STREAK")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(12)
        .clamp(4, 240);
    selected_transport == SessionTransport::WebRtc
        && encoder_backend == VideoEncoderBackend::Nvenc
        && missing_idr_streak >= threshold
}

fn nvenc_missing_idr_recreate_cooldown() -> Duration {
    let cooldown_ms = std::env::var("AGENT_NVENC_MISSING_IDR_RECREATE_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300)
        .clamp(100, 5000);
    Duration::from_millis(cooldown_ms)
}

fn nvenc_missing_idr_recreate_budget_per_window() -> u32 {
    std::env::var("AGENT_NVENC_MISSING_IDR_RECREATE_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(3)
        .clamp(1, 32)
}

fn nvenc_missing_idr_recreate_window() -> Duration {
    let window_ms = std::env::var("AGENT_NVENC_MISSING_IDR_RECREATE_WINDOW_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(15_000)
        .clamp(1000, 120_000);
    Duration::from_millis(window_ms)
}

fn keyframe_burst_len() -> u8 {
    std::env::var("AGENT_KEYFRAME_BURST")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(6)
        .clamp(1, 30)
}

fn keyframe_burst_len_for(
    selected_transport: SessionTransport,
    encoder_backend: VideoEncoderBackend,
) -> u8 {
    if selected_transport == SessionTransport::WebRtc
        && encoder_backend == VideoEncoderBackend::Nvenc
    {
        return std::env::var("AGENT_WEBRTC_KEYFRAME_BURST")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(12)
            .clamp(1, 30);
    }
    keyframe_burst_len()
}

fn idr_interval_sec_for(
    base_idr_interval_sec: u32,
    selected_transport: SessionTransport,
    encoder_backend: VideoEncoderBackend,
) -> u32 {
    let base = base_idr_interval_sec.max(1);
    if selected_transport == SessionTransport::WebRtc
        && encoder_backend == VideoEncoderBackend::Nvenc
    {
        let tuned = std::env::var("AGENT_WEBRTC_IDR_INTERVAL_SEC")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 10);
        return base.min(tuned);
    }
    base
}

fn next_missing_idr_streak(prev: u32, force_idr: bool, has_idr: bool) -> u32 {
    if has_idr {
        0
    } else if force_idr {
        prev.saturating_add(1)
    } else {
        prev
    }
}

fn unix_time_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn maybe_build_webtransport_endpoint(advert: &WebTransportAdvert) -> serde_json::Value {
    json!({
        "url": advert.url,
        "alpn": advert.alpn,
        "certFingerprintSha256": advert.cert_fingerprint_sha256,
    })
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
                    "transports": ["webrtc", "quic", "webtransport"],
                    "capabilities": {
                        "protocols": ["webrtc", "quic", "webtransport"],
                        "platforms": ["windows"],
                        "codecs": ["h264", "hevc", "av1"],
                        "features": ["multi-end-compat", "capability-negotiation", "transport-failover"]
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
            let concurrent_clients = {
                let s = session.lock().await;
                s.sessions.len().saturating_add(1)
            };
            let selected_transport = select_transport_by_strategy(
                requested_transport,
                &controller_caps,
                concurrent_clients,
            );
            let selected_codec = select_codec_by_strategy(selected_transport, &controller_caps);
            unsafe {
                std::env::set_var("AGENT_VIDEO_CODEC_EFFECTIVE", selected_codec.as_str());
            }
            info!(
                requested_transport = requested_transport.as_str(),
                selected_transport = selected_transport.as_str(),
                selected_codec = selected_codec.as_str(),
                concurrent_clients,
                controller_caps = %controller_caps,
                "session transport negotiated"
            );

            let mut quic_advert: Option<QuicServerAdvert> = None;
            let mut webtransport_advert: Option<WebTransportAdvert> = None;
            let mut quic_tx: Option<tokio::sync::mpsc::Sender<QuicAu>> = None;
            if matches!(
                selected_transport,
                SessionTransport::Quic | SessionTransport::WebTransport
            ) {
                let bind_addr: std::net::SocketAddr = "0.0.0.0:0"
                    .parse()
                    .context("parse transport bind addr failed")?;
                match selected_transport {
                    SessionTransport::Quic => {
                        let (advert, tx) = start_quic_sender(bind_addr)?;
                        quic_advert = Some(advert);
                        quic_tx = Some(tx);
                    }
                    SessionTransport::WebTransport => {
                        let (advert, tx) = start_webtransport_sender(bind_addr)?;
                        webtransport_advert = Some(advert);
                        quic_tx = Some(tx);
                    }
                    SessionTransport::WebRtc => {}
                }
            }

            let injector = Arc::new(InputInjector::new());
            let media_ready = Arc::new(AtomicBool::new(false));
            let control_dc = Arc::new(Mutex::new(None));
            let pc = create_peer_connection(
                write.clone(),
                controller_id.clone(),
                injector.clone(),
                media_ready.clone(),
                control_dc.clone(),
            )
            .await?;
            let mut effective_capture_cfg = { capture_cfg.lock().await.clone() };
            apply_multi_client_adaptation(&mut effective_capture_cfg, concurrent_clients);
            attach_video_track_with_policy(
                pc.clone(),
                &effective_capture_cfg,
                session_running.clone(),
                media_ready,
                selected_transport,
                quic_tx,
                control_dc,
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
                    "selectedCodec": selected_codec.as_str(),
                    "quic": quic_advert.as_ref().map(|q| json!({
                        "addr": q.addr,
                        "serverName": q.server_name,
                        "certDerBase64": q.cert_der_base64,
                    })),
                    "webtransport": webtransport_advert.as_ref().map(maybe_build_webtransport_endpoint),
                    "agentCapabilities": {
                        "protocols": ["webrtc", "quic", "webtransport"],
                        "platforms": ["windows"],
                        "codecs": ["h264", "hevc", "av1"],
                        "features": ["multi-end-compat", "capability-negotiation", "transport-failover"]
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
            let controller_id = v["payload"]["controllerId"]
                .as_str()
                .unwrap_or("")
                .to_string();
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
            let controller_id = v["payload"]["controllerId"]
                .as_str()
                .unwrap_or("")
                .to_string();
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
    let get_u32 = |camel: &str, snake: &str| -> Option<u32> {
        patch
            .get(camel)
            .or_else(|| patch.get(snake))
            .and_then(|v| v.as_u64())
            .map(|v| (v as u32).clamp(1, 240))
    };
    let get_u32_raw = |camel: &str, snake: &str| -> Option<u32> {
        patch
            .get(camel)
            .or_else(|| patch.get(snake))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
    };

    if let Some(fps) = get_u32("targetFps", "fps") {
        cfg.fps = fps;
        cfg.max_fps = cfg.max_fps.max(fps);
        cfg.min_fps = cfg.min_fps.min(fps);
        cfg.tier_fps_l1 = cfg.tier_fps_l1.min(fps);
        cfg.tier_fps_l2 = cfg.tier_fps_l2.min(fps);
        cfg.tier_fps_l3 = cfg.tier_fps_l3.min(fps);
        cfg.tier_fps_l4 = cfg.tier_fps_l4.min(fps);
        cfg.tier_fps_l5 = cfg.tier_fps_l5.min(fps);
    }
    if let Some(min_fps) = get_u32("minFps", "min_fps") {
        cfg.min_fps = min_fps;
    }
    if let Some(max_fps) = get_u32("maxFps", "max_fps") {
        cfg.max_fps = max_fps;
    }
    cfg.max_fps = cfg.max_fps.max(cfg.min_fps);
    cfg.fps = cfg.fps.clamp(cfg.min_fps, cfg.max_fps);

    if let Some(v) = get_u32_raw("targetWidth", "target_width") {
        cfg.target_width = v as u32;
    }
    if let Some(v) = get_u32_raw("targetHeight", "target_height") {
        cfg.target_height = v as u32;
    }
    if let Some(v) = patch
        .get("bitrateKbps")
        .or_else(|| patch.get("bitrate_kbps"))
        .and_then(|v| v.as_u64())
    {
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
            "auto" => unsafe {
                std::env::remove_var("AGENT_WGC_WINDOW_HWND");
            },
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
    if let Some(pacer) = patch.get("quicPacer").and_then(|v| v.as_object()) {
        if let Some(v) = pacer.get("enable").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_ENABLE", if v { "1" } else { "0" });
            }
        }
        if let Some(v) = pacer.get("mode").and_then(|v| v.as_str()) {
            let mode = match v.to_ascii_lowercase().as_str() {
                "auto" => "auto",
                _ => "manual",
            };
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_MODE", mode);
            }
        }
        if let Some(v) = pacer.get("intervalMs").and_then(|v| v.as_u64()) {
            let v = v.clamp(1, 100);
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_INTERVAL_MS", v.to_string());
            }
        }
        if let Some(v) = pacer.get("burst").and_then(|v| v.as_u64()) {
            let v = v.clamp(1, 16);
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_BURST", v.to_string());
            }
        }
        if let Some(v) = pacer.get("autoOnFull").and_then(|v| v.as_u64()) {
            let v = v.clamp(1, 1000);
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_AUTO_ON_FULL", v.to_string());
            }
        }
        if let Some(v) = pacer.get("autoOffOk").and_then(|v| v.as_u64()) {
            let v = v.clamp(1, 5000);
            unsafe {
                std::env::set_var("AGENT_QUIC_PACE_AUTO_OFF_OK", v.to_string());
            }
        }
    }
    if let Some(link) = patch.get("quicQueueRateLink").and_then(|v| v.as_object()) {
        if let Some(v) = link.get("enable").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_ENABLE",
                    if v { "1" } else { "0" },
                );
            }
        }
        if let Some(v) = link.get("minFps").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_MIN_FPS",
                    v.clamp(1, 240).to_string(),
                );
            }
        }
        if let Some(v) = link.get("maxFps").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_MAX_FPS",
                    v.clamp(1, 240).to_string(),
                );
            }
        }
        if let Some(v) = link.get("downStep").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_DOWN_STEP",
                    v.clamp(1, 60).to_string(),
                );
            }
        }
        if let Some(v) = link.get("upStep").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_UP_STEP",
                    v.clamp(1, 30).to_string(),
                );
            }
        }
        if let Some(v) = link.get("fullThreshold").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_FULL_THRESHOLD",
                    v.clamp(1, 2000).to_string(),
                );
            }
        }
        if let Some(v) = link.get("okThreshold").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_OK_THRESHOLD",
                    v.clamp(1, 20_000).to_string(),
                );
            }
        }
        if let Some(v) = link.get("cooldownMs").and_then(|v| v.as_u64()) {
            unsafe {
                std::env::set_var(
                    "AGENT_QUIC_QUEUE_RATE_LINK_COOLDOWN_MS",
                    v.clamp(0, 10_000).to_string(),
                );
            }
        }
    }
    if let Some(strategy) = patch.get("transportStrategy").and_then(|v| v.as_object()) {
        if let Some(v) = strategy.get("autoEnable").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var("AGENT_TRANSPORT_AUTO_ENABLE", if v { "1" } else { "0" });
            }
        }
        if let Some(v) = strategy.get("priority").and_then(|v| v.as_str()) {
            let normalized = parse_transport_priority(v)
                .into_iter()
                .map(SessionTransport::as_str)
                .collect::<Vec<_>>()
                .join(",");
            if !normalized.is_empty() {
                unsafe {
                    std::env::set_var("AGENT_TRANSPORT_AUTO_PRIORITY", normalized);
                }
            }
        }
        if let Some(v) = strategy
            .get("multiClientUpgradeAt")
            .and_then(|v| v.as_u64())
        {
            unsafe {
                std::env::set_var(
                    "AGENT_TRANSPORT_MULTI_CLIENT_UPGRADE_AT",
                    v.clamp(1, 64).to_string(),
                );
            }
        }
        if let Some(v) = strategy.get("enableWebRtc").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var("AGENT_TRANSPORT_ENABLE_WEBRTC", if v { "1" } else { "0" });
            }
        }
        if let Some(v) = strategy.get("enableQuic").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var("AGENT_TRANSPORT_ENABLE_QUIC", if v { "1" } else { "0" });
            }
        }
        if let Some(v) = strategy.get("enableWebTransport").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var(
                    "AGENT_TRANSPORT_ENABLE_WEBTRANSPORT",
                    if v { "1" } else { "0" },
                );
            }
        }
        if let Some(v) = strategy.get("sharedEncodeEnable").and_then(|v| v.as_bool()) {
            unsafe {
                std::env::set_var(
                    "AGENT_SHARED_CAPTURE_ENCODE_ENABLE",
                    if v { "1" } else { "0" },
                );
            }
        }
    }
    if let Some(codec) = patch.get("codecPolicy").and_then(|v| v.as_object()) {
        if let Some(v) = codec.get("force").and_then(|v| v.as_str()) {
            let c = VideoCodec::parse(Some(v));
            unsafe {
                std::env::set_var("AGENT_CODEC_FORCE", c.as_str());
            }
        }
        if let Some(v) = codec.get("priority").and_then(|v| v.as_str()) {
            let normalized = parse_codec_priority(v)
                .into_iter()
                .map(VideoCodec::as_str)
                .collect::<Vec<_>>()
                .join(",");
            if !normalized.is_empty() {
                unsafe {
                    std::env::set_var("AGENT_CODEC_PRIORITY", normalized);
                }
            }
        }
    }
    if let Some(qp) = patch.get("qualityPolicy").and_then(|v| v.as_object()) {
        if let Some(vbv) = qp.get("dynamicVbv").and_then(|v| v.as_object()) {
            if let Some(v) = vbv.get("enable").and_then(|v| v.as_bool()) {
                unsafe {
                    std::env::set_var("AGENT_DYNAMIC_VBV_ENABLE", if v { "1" } else { "0" });
                }
            }
            if let Some(v) = vbv.get("minKbps").and_then(|v| v.as_u64()) {
                unsafe {
                    std::env::set_var(
                        "AGENT_DYNAMIC_VBV_MIN_KBPS",
                        v.clamp(100, 300_000).to_string(),
                    );
                }
            }
            if let Some(v) = vbv.get("maxKbps").and_then(|v| v.as_u64()) {
                unsafe {
                    std::env::set_var(
                        "AGENT_DYNAMIC_VBV_MAX_KBPS",
                        v.clamp(100, 500_000).to_string(),
                    );
                }
            }
        }
        if let Some(roi) = qp.get("roi").and_then(|v| v.as_object()) {
            let mut require_native_set = false;
            if let Some(v) = roi.get("enable").and_then(|v| v.as_bool()) {
                unsafe {
                    std::env::set_var("AGENT_ROI_ENABLE", if v { "1" } else { "0" });
                }
            }
            if let Some(v) = roi.get("boostPct").and_then(|v| v.as_u64()) {
                unsafe {
                    std::env::set_var("AGENT_ROI_BOOST_PCT", v.clamp(0, 200).to_string());
                }
            }
            if let Some(rect) = roi.get("rect").and_then(|v| v.as_object()) {
                let num = |k: &str| -> Option<f64> {
                    let v = rect.get(k)?;
                    v.as_f64()
                        .or_else(|| v.as_i64().map(|x| x as f64))
                        .or_else(|| v.as_u64().map(|x| x as f64))
                };
                let x = num("x").unwrap_or(0.0).max(0.0);
                let y = num("y").unwrap_or(0.0).max(0.0);
                let w = num("w").unwrap_or(0.0);
                let h = num("h").unwrap_or(0.0);
                if w > 0.0 && h > 0.0 {
                    unsafe {
                        std::env::set_var("AGENT_ROI_RECT", format!("{x:.6},{y:.6},{w:.6},{h:.6}"));
                    }
                }
            }
            if let Some(v) = roi.get("qoffset").and_then(|v| v.as_f64()) {
                let q = v.clamp(-1.0, 1.0);
                unsafe {
                    std::env::set_var("AGENT_ROI_QOFFSET", format!("{q:.3}"));
                }
            }
            if let Some(v) = roi.get("frameInterval").and_then(|v| v.as_u64()) {
                unsafe {
                    std::env::set_var("AGENT_ROI_FRAME_INTERVAL", v.clamp(1, 120).to_string());
                }
            }
            if let Some(v) = roi.get("minAreaPct").and_then(|v| v.as_f64()) {
                unsafe {
                    std::env::set_var(
                        "AGENT_ROI_MIN_AREA_PCT",
                        format!("{:.4}", v.clamp(0.0, 1.0)),
                    );
                }
            }
            if let Some(v) = roi.get("requireNative").and_then(|v| v.as_bool()) {
                unsafe {
                    std::env::set_var("AGENT_ROI_REQUIRE_NATIVE", if v { "1" } else { "0" });
                }
                require_native_set = true;
            }
            if !require_native_set {
                unsafe {
                    std::env::set_var("AGENT_ROI_REQUIRE_NATIVE", "1");
                }
            }
        }
        if let Some(content) = qp.get("content").and_then(|v| v.as_object()) {
            if let Some(mode) = content.get("mode").and_then(|v| v.as_str()) {
                let m = mode.trim().to_ascii_lowercase();
                if matches!(m.as_str(), "auto" | "text" | "video") {
                    unsafe {
                        std::env::set_var("AGENT_CONTENT_MODE", m);
                    }
                }
            }
        }
        if let Some(vbv) = qp.get("dynamicVbv").and_then(|v| v.as_object()) {
            if let Some(v) = vbv.get("strict").and_then(|v| v.as_bool()) {
                unsafe {
                    std::env::set_var("AGENT_DYNAMIC_VBV_STRICT_ENABLE", if v { "1" } else { "0" });
                }
            }
            if let Some(v) = vbv.get("headroomPct").and_then(|v| v.as_u64()) {
                unsafe {
                    std::env::set_var(
                        "AGENT_DYNAMIC_VBV_STRICT_HEADROOM_PCT",
                        v.clamp(0, 50).to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn apply_multi_client_adaptation(cfg: &mut agent_rust::CaptureConfig, concurrent_clients: usize) {
    let enabled = std::env::var("AGENT_MULTI_CLIENT_ADAPT_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if !enabled || concurrent_clients <= 1 {
        return;
    }
    let clients = concurrent_clients.clamp(1, 32) as u32;
    let fair_mode = std::env::var("AGENT_MULTI_CLIENT_FAIR_MODE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let fair_target_clients = std::env::var("AGENT_MULTI_CLIENT_FAIR_TARGET_CLIENTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v.clamp(1, 32));
    let max_clients_env = std::env::var("AGENT_MAX_CLIENTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v.clamp(1, 32));
    let budget_clients = if fair_mode {
        fair_target_clients
            .or(max_clients_env)
            .unwrap_or(clients)
            .max(clients)
    } else {
        clients
    };
    let fps_min = std::env::var("AGENT_MULTI_CLIENT_MIN_FPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(24)
        .clamp(1, 240);
    let br_min = std::env::var("AGENT_MULTI_CLIENT_MIN_BITRATE_KBPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(6000)
        .clamp(100, 300_000);
    let fps_ratio_num = std::env::var("AGENT_MULTI_CLIENT_FPS_RATIO_NUM")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(82)
        .clamp(1, 100);
    let fps_ratio_den = std::env::var("AGENT_MULTI_CLIENT_FPS_RATIO_DEN")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(100)
        .max(1);
    let br_ratio_num = std::env::var("AGENT_MULTI_CLIENT_BR_RATIO_NUM")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(78)
        .clamp(1, 100);
    let br_ratio_den = std::env::var("AGENT_MULTI_CLIENT_BR_RATIO_DEN")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(100)
        .max(1);
    let force_openh264_at = std::env::var("AGENT_MULTI_CLIENT_FORCE_OPENH264_AT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(3)
        .clamp(2, 32);
    let total_fps_budget = std::env::var("AGENT_MULTI_CLIENT_TOTAL_FPS_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(144)
        .clamp(1, 1000);
    let total_bitrate_budget = std::env::var("AGENT_MULTI_CLIENT_TOTAL_BITRATE_BUDGET_KBPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(32_000)
        .clamp(100, 1_000_000);

    let base_fps = cfg.fps.max(1);
    let base_br = cfg.bitrate_kbps.max(100);
    let mut target_fps = base_fps;
    let mut target_br = base_br;
    for _ in 1..budget_clients {
        target_fps = target_fps.saturating_mul(fps_ratio_num) / fps_ratio_den;
        target_br = target_br.saturating_mul(br_ratio_num) / br_ratio_den;
    }
    let fair_fps_cap = (total_fps_budget / budget_clients.max(1)).max(1);
    let fair_br_cap = (total_bitrate_budget / budget_clients.max(1)).max(100);
    target_fps = target_fps.min(fair_fps_cap);
    target_br = target_br.min(fair_br_cap);
    target_fps = target_fps
        .clamp(fps_min, cfg.max_fps.max(1))
        .clamp(cfg.min_fps.max(1), cfg.max_fps.max(1));
    target_br = target_br.clamp(br_min, cfg.max_bitrate_kbps.max(br_min));

    cfg.fps = target_fps;
    cfg.max_fps = cfg.max_fps.min(target_fps).max(1);
    cfg.min_fps = cfg.min_fps.min(target_fps).max(1);
    cfg.tier_fps_l1 = cfg.tier_fps_l1.min(target_fps).max(1);
    cfg.tier_fps_l2 = cfg.tier_fps_l2.min(target_fps).max(1);
    cfg.tier_fps_l3 = cfg.tier_fps_l3.min(target_fps).max(1);
    cfg.tier_fps_l4 = cfg.tier_fps_l4.min(target_fps).max(1);
    cfg.tier_fps_l5 = cfg.tier_fps_l5.min(target_fps).max(1);
    cfg.bitrate_kbps = target_br;
    cfg.max_fps_mode = false;
    cfg.allow_encoder_fallback = true;
    cfg.strict_gpu_direct = false;
    if budget_clients >= force_openh264_at {
        cfg.encoder = "openh264".to_string();
    }
    info!(
        concurrent_clients,
        budget_clients,
        target_fps = cfg.fps,
        target_bitrate_kbps = cfg.bitrate_kbps,
        encoder = %cfg.encoder,
        fair_mode,
        strict_gpu_direct = cfg.strict_gpu_direct,
        "applied multi-client adaptation profile"
    );
}

fn apply_dynamic_vbv_and_roi_policy(cfg: &mut agent_rust::CaptureConfig) {
    let vbv_enable = std::env::var("AGENT_DYNAMIC_VBV_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if vbv_enable {
        let vbv_min = std::env::var("AGENT_DYNAMIC_VBV_MIN_KBPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(cfg.network_adapt_floor_bitrate_kbps.max(100))
            .clamp(100, 300_000);
        let vbv_max = std::env::var("AGENT_DYNAMIC_VBV_MAX_KBPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(cfg.network_adapt_ceiling_bitrate_kbps.max(vbv_min))
            .clamp(vbv_min, 500_000);
        cfg.network_adapt_floor_bitrate_kbps = vbv_min;
        cfg.network_adapt_ceiling_bitrate_kbps = vbv_max;
        cfg.bitrate_kbps = cfg.bitrate_kbps.clamp(vbv_min, vbv_max);
        cfg.max_bitrate_kbps = cfg
            .max_bitrate_kbps
            .clamp(vbv_min, vbv_max)
            .max(cfg.bitrate_kbps);
        let strict = std::env::var("AGENT_DYNAMIC_VBV_STRICT_ENABLE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if strict {
            let headroom = std::env::var("AGENT_DYNAMIC_VBV_STRICT_HEADROOM_PCT")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(10)
                .clamp(0, 50);
            let strict_max = cfg
                .bitrate_kbps
                .saturating_mul(100 + headroom)
                .saturating_div(100)
                .max(cfg.bitrate_kbps);
            cfg.network_adapt_ceiling_bitrate_kbps = strict_max;
            cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.min(strict_max).max(cfg.bitrate_kbps);
        }
    }

    let roi_enable = std::env::var("AGENT_ROI_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if roi_enable {
        let boost_pct = std::env::var("AGENT_ROI_BOOST_PCT")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(15)
            .clamp(0, 200);
        if boost_pct > 0 {
            let boosted = cfg
                .bitrate_kbps
                .saturating_mul(100 + boost_pct)
                .saturating_div(100)
                .clamp(100, cfg.max_bitrate_kbps.max(100));
            cfg.bitrate_kbps = boosted;
            cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(boosted);
        }
    }
    let content_mode = std::env::var("AGENT_CONTENT_MODE")
        .ok()
        .unwrap_or_else(|| "auto".to_string())
        .to_ascii_lowercase();
    match content_mode.as_str() {
        "text" => {
            let new_fps = cfg
                .fps
                .saturating_mul(85)
                .saturating_div(100)
                .max(cfg.min_fps.max(12));
            cfg.fps = new_fps.min(cfg.max_fps.max(1));
            cfg.bitrate_kbps = cfg
                .bitrate_kbps
                .saturating_mul(110)
                .saturating_div(100)
                .min(cfg.max_bitrate_kbps.max(100));
            cfg.encoder_tune = "hq".to_string();
        }
        "video" => {
            let new_fps = cfg
                .fps
                .saturating_mul(115)
                .saturating_div(100)
                .max(cfg.min_fps.max(24));
            cfg.fps = new_fps.min(cfg.max_fps.max(1));
            cfg.bitrate_kbps = cfg
                .bitrate_kbps
                .saturating_mul(95)
                .saturating_div(100)
                .max(cfg.network_adapt_floor_bitrate_kbps.max(100));
            if cfg.encoder_tune == "balanced" {
                cfg.encoder_tune = "ll".to_string();
            }
        }
        _ => {}
    }
}

fn env_flag_enabled(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let s = v.trim().to_ascii_lowercase();
            !(s == "0" || s == "false" || s == "off" || s == "no")
        })
        .unwrap_or(default)
}

fn parse_fps_cap_tier(v: &str) -> Option<u32> {
    let s = v.trim().to_ascii_lowercase();
    match s.as_str() {
        "72" | "l1" | "tier1" | "safe" => Some(72),
        "120" | "l2" | "tier2" | "balanced" => Some(120),
        "144" | "l3" | "tier3" | "high" => Some(144),
        "240" | "l4" | "tier4" | "max" | "unlocked" => Some(240),
        _ => s
            .parse::<u32>()
            .ok()
            .map(|n| n.clamp(12, 240))
            .filter(|n| *n > 0),
    }
}

fn apply_fps_mode_policy(cfg: &mut agent_rust::CaptureConfig) {
    let mode = resolve_fps_mode(cfg);
    apply_fps_mode_policy_with_mode(cfg, &mode);
}

fn resolve_fps_mode(cfg: &agent_rust::CaptureConfig) -> String {
    std::env::var("AGENT_FPS_MODE")
        .ok()
        .unwrap_or_else(|| cfg.fps_mode.clone())
        .trim()
        .to_ascii_lowercase()
}

fn apply_fps_mode_policy_with_mode(cfg: &mut agent_rust::CaptureConfig, mode: &str) {
    match mode {
        "throughput" | "max" | "throughput_first" => {
            // Throughput mode should not inherit max_fps_mode's tiny-queue clamp.
            cfg.max_fps_mode = false;
            cfg.frame_pacing_enable = false;
            cfg.queue_strategy = "drop".to_string();
            cfg.queue_depth = cfg.queue_depth.clamp(8, 32);
            cfg.rtp_use_manual_packetizer = true;
            // Throughput mode prefers highest stable cadence and lets transport pacing handle bursts.
            cfg.tier_limit_enable = env_flag_enabled("AGENT_FPS_MODE_KEEP_TIER_LIMIT", false);
            if let Some(tier) = std::env::var("AGENT_CODEC_FPS_CAP_TIER")
                .ok()
                .and_then(|v| parse_fps_cap_tier(&v))
            {
                cfg.max_fps = cfg.max_fps.max(tier);
                if env_flag_enabled("AGENT_FPS_MODE_FORCE_TARGET", true) {
                    cfg.fps = cfg.fps.max(tier);
                }
            }
            cfg.max_fps = cfg.max_fps.max(cfg.fps.max(1));
            cfg.min_fps = cfg.min_fps.min(cfg.max_fps).max(1);
            cfg.idle_repeat_fps = cfg
                .idle_repeat_fps
                .max(cfg.fps)
                .clamp(1, cfg.max_fps.max(1));
            info!(
                fps_mode = mode,
                fps = cfg.fps,
                min_fps = cfg.min_fps,
                max_fps = cfg.max_fps,
                queue_depth = cfg.queue_depth,
                tier_limit_enable = cfg.tier_limit_enable,
                "applied fps mode policy"
            );
        }
        "latency" | "latency_first" => {
            cfg.max_fps_mode = false;
            cfg.frame_pacing_enable = false;
            cfg.queue_strategy = "drop".to_string();
            cfg.queue_depth = cfg.queue_depth.clamp(2, 8);
            info!(fps_mode = mode, "applied fps mode policy");
        }
        "balanced" | "balanced_first" => {
            cfg.max_fps_mode = false;
            cfg.queue_strategy = "drop".to_string();
            cfg.frame_pacing_enable = true;
            cfg.queue_depth = cfg.queue_depth.clamp(4, 16);
            cfg.tier_limit_enable = env_flag_enabled("AGENT_FPS_MODE_KEEP_TIER_LIMIT", true);
            info!(fps_mode = mode, "applied fps mode policy");
        }
        _ => {
            info!(fps_mode = mode, "unknown fps mode; keeping defaults");
        }
    }
}

fn codec_transport_fps_cap(selected_transport: SessionTransport) -> Option<u32> {
    if !env_flag_enabled("AGENT_CODEC_FPS_CAP_ENABLE", true) {
        return None;
    }
    let codec = std::env::var("AGENT_VIDEO_CODEC_EFFECTIVE")
        .ok()
        .unwrap_or_else(|| "h264".to_string())
        .to_ascii_lowercase();
    let key = match (selected_transport, codec.as_str()) {
        (SessionTransport::WebTransport, "av1") => "AGENT_CODEC_FPS_CAP_WEBTRANSPORT_AV1",
        (SessionTransport::WebTransport, "hevc") | (SessionTransport::WebTransport, "h265") => {
            "AGENT_CODEC_FPS_CAP_WEBTRANSPORT_HEVC"
        }
        (SessionTransport::Quic, "av1") => "AGENT_CODEC_FPS_CAP_QUIC_AV1",
        (SessionTransport::Quic, "hevc") | (SessionTransport::Quic, "h265") => {
            "AGENT_CODEC_FPS_CAP_QUIC_HEVC"
        }
        _ => return None,
    };
    let default = match key {
        "AGENT_CODEC_FPS_CAP_WEBTRANSPORT_AV1" | "AGENT_CODEC_FPS_CAP_QUIC_AV1" => 72,
        _ => 90,
    };
    let tier_default = std::env::var("AGENT_CODEC_FPS_CAP_TIER")
        .ok()
        .and_then(|v| parse_fps_cap_tier(&v))
        .unwrap_or(default);
    let cap = std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(tier_default)
        .clamp(12, 240);
    Some(cap)
}

fn apply_codec_transport_limits(
    selected_transport: SessionTransport,
    cfg: &mut agent_rust::CaptureConfig,
) {
    let Some(cap) = codec_transport_fps_cap(selected_transport) else {
        return;
    };
    let uplift_enable = env_flag_enabled("AGENT_CODEC_FPS_CAP_UPLIFT_ENABLE", true);
    let force_target = env_flag_enabled("AGENT_CODEC_FPS_CAP_FORCE_TARGET", true);
    let old_fps = cfg.fps;
    let old_max = cfg.max_fps;
    let old_min = cfg.min_fps;
    if uplift_enable {
        cfg.max_fps = cfg.max_fps.max(cap);
        if force_target && cfg.fps < cap {
            cfg.fps = cap;
        }
        cfg.tier_fps_l5 = cfg.tier_fps_l5.max(cap);
        cfg.tier_fps_l4 = cfg.tier_fps_l4.max(cfg.tier_fps_l3);
        cfg.tier_fps_l5 = cfg.tier_fps_l5.max(cfg.tier_fps_l4);
    }
    cfg.fps = cfg.fps.min(cap);
    cfg.max_fps = cfg.max_fps.min(cap).max(1);
    cfg.min_fps = cfg.min_fps.min(cfg.max_fps).max(1);
    cfg.idle_repeat_fps = cfg.idle_repeat_fps.min(cfg.max_fps).max(1);
    cfg.tier_fps_l1 = cfg.tier_fps_l1.min(cfg.max_fps).max(1);
    cfg.tier_fps_l2 = cfg.tier_fps_l2.min(cfg.max_fps).max(1);
    cfg.tier_fps_l3 = cfg.tier_fps_l3.min(cfg.max_fps).max(1);
    cfg.tier_fps_l4 = cfg.tier_fps_l4.min(cfg.max_fps).max(1);
    cfg.tier_fps_l5 = cfg.tier_fps_l5.min(cfg.max_fps).max(1);
    info!(
        selected_transport = selected_transport.as_str(),
        codec = %std::env::var("AGENT_VIDEO_CODEC_EFFECTIVE").unwrap_or_else(|_| "h264".to_string()),
        fps_cap = cap,
        old_fps,
        old_min_fps = old_min,
        old_max_fps = old_max,
        new_fps = cfg.fps,
        new_min_fps = cfg.min_fps,
        new_max_fps = cfg.max_fps,
        "applied codec transport fps cap"
    );
}

fn spawn_control_stats_publisher(
    control_dc: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    stats: Arc<RuntimeStats>,
    running: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(1000));
        let mut last_nack = 0_u64;
        let mut last_pli = 0_u64;
        let mut last_quic_drop = 0_u64;
        let mut last_encoded = 0_u64;
        let mut last_sent = 0_u64;
        let mut last_unique_sent = 0_u64;
        let mut last_repeated_sent = 0_u64;
        let mut last_enqueue_wait_us_total = 0_u64;
        let mut last_capture_to_send_us_total = 0_u64;
        let mut last_encode_approx_us_total = 0_u64;
        let mut last_capture_encode_samples = 0_u64;
        let mut last_transport_send_us_total = 0_u64;
        let mut last_transport_send_samples = 0_u64;
        while running.load(Ordering::SeqCst) {
            ticker.tick().await;

            let nack = stats.nack_count.load(Ordering::Relaxed);
            let pli = stats.pli_count.load(Ordering::Relaxed);
            let quic_drop = stats.quic_au_dropped.load(Ordering::Relaxed);
            let encoded_total = stats.encoded_au_total.load(Ordering::Relaxed);
            let sent_total = stats.sent_au_total.load(Ordering::Relaxed);
            let unique_sent_total = stats.unique_sent_au_total.load(Ordering::Relaxed);
            let repeated_sent_total = stats.repeated_sent_au_total.load(Ordering::Relaxed);
            let enqueue_wait_us_total = stats
                .transport_enqueue_wait_us_total
                .load(Ordering::Relaxed);
            let capture_to_send_us_total = stats
                .transport_capture_to_send_us_total
                .load(Ordering::Relaxed);
            let encode_approx_us_total = stats
                .transport_encode_approx_us_total
                .load(Ordering::Relaxed);
            let capture_encode_samples = stats
                .transport_capture_encode_samples
                .load(Ordering::Relaxed);
            let transport_send_us_total = stats.transport_send_us_total.load(Ordering::Relaxed);
            let transport_send_samples = stats.transport_send_samples.load(Ordering::Relaxed);

            let nack_per_sec = nack.saturating_sub(last_nack);
            let pli_per_sec = pli.saturating_sub(last_pli);
            let quic_drop_per_sec = quic_drop.saturating_sub(last_quic_drop);
            let encode_fps = encoded_total.saturating_sub(last_encoded);
            let send_fps = sent_total.saturating_sub(last_sent);
            let unique_send_fps = unique_sent_total.saturating_sub(last_unique_sent);
            let repeat_send_fps = repeated_sent_total.saturating_sub(last_repeated_sent);
            let queue_depth = encoded_total.saturating_sub(sent_total);
            let enqueue_wait_us_delta =
                enqueue_wait_us_total.saturating_sub(last_enqueue_wait_us_total);
            let capture_to_send_us_delta =
                capture_to_send_us_total.saturating_sub(last_capture_to_send_us_total);
            let encode_approx_us_delta =
                encode_approx_us_total.saturating_sub(last_encode_approx_us_total);
            let capture_encode_samples_delta =
                capture_encode_samples.saturating_sub(last_capture_encode_samples);
            let transport_send_us_delta =
                transport_send_us_total.saturating_sub(last_transport_send_us_total);
            let transport_send_samples_delta =
                transport_send_samples.saturating_sub(last_transport_send_samples);
            let enqueue_wait_avg_us = if encode_fps > 0 {
                enqueue_wait_us_delta as f64 / encode_fps as f64
            } else {
                0.0
            };
            let transport_send_avg_us = if transport_send_samples_delta > 0 {
                transport_send_us_delta as f64 / transport_send_samples_delta as f64
            } else {
                0.0
            };
            let p = stats.transport_latency_percentiles_ms();
            let capture_to_send_avg_us = if capture_encode_samples_delta > 0 {
                capture_to_send_us_delta as f64 / capture_encode_samples_delta as f64
            } else {
                0.0
            };
            let encode_approx_avg_us = if capture_encode_samples_delta > 0 {
                encode_approx_us_delta as f64 / capture_encode_samples_delta as f64
            } else {
                0.0
            };

            last_nack = nack;
            last_pli = pli;
            last_quic_drop = quic_drop;
            last_encoded = encoded_total;
            last_sent = sent_total;
            last_unique_sent = unique_sent_total;
            last_repeated_sent = repeated_sent_total;
            last_enqueue_wait_us_total = enqueue_wait_us_total;
            last_capture_to_send_us_total = capture_to_send_us_total;
            last_encode_approx_us_total = encode_approx_us_total;
            last_capture_encode_samples = capture_encode_samples;
            last_transport_send_us_total = transport_send_us_total;
            last_transport_send_samples = transport_send_samples;

            let payload = json!({
                "type": "agentStats",
                "payload": {
                    "tsMs": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    "targetFps": stats.target_fps.load(Ordering::Relaxed),
                    "targetBitrateKbps": stats.target_bitrate_kbps.load(Ordering::Relaxed),
                    "encodeFps": encode_fps,
                    "sendFps": send_fps,
                    "uniqueSendFps": unique_send_fps,
                    "repeatSendFps": repeat_send_fps,
                    "nackPerSec": nack_per_sec,
                    "pliPerSec": pli_per_sec,
                    "quicDropPerSec": quic_drop_per_sec,
                    "queueDepth": queue_depth,
                    "enqueueWaitAvgUs": (enqueue_wait_avg_us * 10.0).round() / 10.0,
                    "transportSendAvgUs": (transport_send_avg_us * 10.0).round() / 10.0,
                    "captureMs": (capture_to_send_avg_us / 1000.0 * 1000.0).round() / 1000.0,
                    "captureP50Ms": (p.capture.p50 * 1000.0).round() / 1000.0,
                    "captureP95Ms": (p.capture.p95 * 1000.0).round() / 1000.0,
                    "encodeMs": (encode_approx_avg_us / 1000.0 * 1000.0).round() / 1000.0,
                    "encodeP50Ms": (p.encode.p50 * 1000.0).round() / 1000.0,
                    "encodeP95Ms": (p.encode.p95 * 1000.0).round() / 1000.0,
                    "queueWaitMs": (enqueue_wait_avg_us / 1000.0 * 1000.0).round() / 1000.0,
                    "queueWaitP50Ms": (p.queue_wait.p50 * 1000.0).round() / 1000.0,
                    "queueWaitP95Ms": (p.queue_wait.p95 * 1000.0).round() / 1000.0,
                    "sendMs": (transport_send_avg_us / 1000.0 * 1000.0).round() / 1000.0,
                    "sendP50Ms": (p.send.p50 * 1000.0).round() / 1000.0,
                    "sendP95Ms": (p.send.p95 * 1000.0).round() / 1000.0
                }
            });

            let dc_opt = { control_dc.lock().await.clone() };
            let Some(dc) = dc_opt else {
                continue;
            };
            if dc.ready_state() != RTCDataChannelState::Open {
                continue;
            }
            if let Err(e) = dc.send(&Bytes::from(payload.to_string())).await {
                warn!(error = %e, "failed to send agent stats over control data channel");
            }
        }
    });
}

async fn create_peer_connection(
    ws_write: Arc<Mutex<WsWrite>>,
    controller_id: String,
    injector: Arc<InputInjector>,
    media_ready: Arc<AtomicBool>,
    control_dc: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
) -> Result<Arc<RTCPeerConnection>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    m.register_header_extension(
        RTCRtpHeaderExtensionCapability {
            uri: TX_UNIX_US_EXT_URI.to_string(),
        },
        RTPCodecType::Video,
        None,
    )?;
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

    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        if s == RTCPeerConnectionState::Connected {
            media_ready.store(true, Ordering::SeqCst);
        } else if matches!(
            s,
            RTCPeerConnectionState::Disconnected
                | RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Closed
        ) {
            media_ready.store(false, Ordering::SeqCst);
        }
        info!(state = %s, media_ready = media_ready.load(Ordering::SeqCst), "peer connection state changed");
        Box::pin(async {})
    }));
    pc.on_ice_connection_state_change(Box::new(move |s: RTCIceConnectionState| {
        info!(state = %s, "ice connection state changed");
        Box::pin(async {})
    }));
    {
        let injector = injector.clone();
        let control_dc_slot = control_dc.clone();
        pc.on_data_channel(Box::new(move |dc| {
            let injector = injector.clone();
            let control_dc_slot = control_dc_slot.clone();
            Box::pin(async move {
                let label = dc.label().to_string();
                let class = match label.as_str() {
                    "ctrl_rt" => Some(ChannelClass::Realtime),
                    "ctrl_rel" => Some(ChannelClass::Reliable),
                    "control" => Some(ChannelClass::Reliable),
                    _ => None,
                };
                if class.is_none() {
                    info!(label = %label, "received non-control data channel");
                    return;
                }
                let class = class.unwrap_or(ChannelClass::Reliable);
                info!(label = %label, class = ?class, "control data channel bound");
                {
                    let mut slot = control_dc_slot.lock().await;
                    *slot = Some(dc.clone());
                }
                let injector = injector.clone();
                dc.on_message(Box::new(move |msg| {
                    let injector = injector.clone();
                    Box::pin(async move {
                        if let Err(e) = injector.push_raw(class, &msg.data).await {
                            warn!(error = %e, "failed to decode/queue control frame");
                        }
                    })
                }));
                let control_dc_slot = control_dc_slot.clone();
                dc.on_close(Box::new(move || {
                    let control_dc_slot = control_dc_slot.clone();
                    Box::pin(async move {
                        let mut slot = control_dc_slot.lock().await;
                        *slot = None;
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
    media_ready: Arc<AtomicBool>,
    selected_transport: SessionTransport,
    quic_tx: Option<tokio::sync::mpsc::Sender<QuicAu>>,
    control_dc: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
) -> Result<()> {
    let mut effective_cfg = capture_cfg.clone();
    let with_capture_ts_header = matches!(
        selected_transport,
        SessionTransport::Quic | SessionTransport::WebTransport
    );
    let active_sessions = if with_capture_ts_header {
        let v = ACTIVE_STREAM_SESSIONS.fetch_add(1, Ordering::Relaxed) + 1;
        let running = session_running.clone();
        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            decrement_active_stream_sessions();
        });
        Some(v)
    } else {
        None
    };
    apply_fps_mode_policy(&mut effective_cfg);
    apply_capture_profile(&mut effective_cfg);
    // Re-apply fps mode after profile/template overlay to keep queue/pacing/tier
    // controls consistent with the requested runtime mode.
    apply_fps_mode_policy(&mut effective_cfg);
    apply_transport_send_policy(selected_transport, &mut effective_cfg);
    apply_codec_transport_limits(selected_transport, &mut effective_cfg);
    if let Some(v) = active_sessions {
        apply_stream_fair_share(&mut effective_cfg, v);
    }
    apply_dynamic_vbv_and_roi_policy(&mut effective_cfg);
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
    spawn_control_stats_publisher(control_dc, stats.clone(), session_running.clone());
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

    if matches!(
        selected_transport,
        SessionTransport::Quic | SessionTransport::WebTransport
    ) && shared_pipeline_enabled()
    {
        let hub = get_or_start_shared_encoded_hub(
            &effective_cfg,
            backend,
            encoder_backend,
            with_capture_ts_header,
        )?;
        let quic_sender = quic_tx
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("quic transport selected but quic sender missing"))?;
        let queue_depth = effective_cfg.queue_depth.clamp(4, 256) as usize;
        let (encoded_tx, encoded_rx) = tokio::sync::mpsc::channel::<Arc<[u8]>>(queue_depth);
        let mut encoded_sub = hub.tx.subscribe();
        let session_running_fanout = session_running.clone();
        let stats_fanout = stats.clone();
        tokio::spawn(async move {
            while session_running_fanout.load(Ordering::SeqCst) {
                match encoded_sub.recv().await {
                    Ok(encoded) => {
                        if encoded_tx.try_send(encoded).is_err() {
                            stats_fanout.quic_au_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        stats_fanout
                            .quic_au_dropped
                            .fetch_add(skipped as u64, Ordering::Relaxed);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        let stats_send = stats.clone();
        let session_running_send = session_running.clone();
        tokio::spawn(spawn_send_loop_quic(
            quic_sender,
            encoded_rx,
            stats_send,
            session_running_send,
            Some(hub.fps_ref.clone()),
        ));
        info!(
            selected_transport = selected_transport.as_str(),
            queue_depth, "attached shared capture+encode fanout for transport session"
        );
        return Ok(());
    }

    let roi_requested_raw = roi_map_requested();
    let native_roi = nvenc_native_roi_enabled();
    let roi_require_native = roi_require_native();
    let roi_requested = effective_roi_request(roi_requested_raw, native_roi, roi_require_native);
    if roi_requested_raw && roi_require_native && !native_roi {
        info!(
            "ROI requested with requireNative=true but native ROI unsupported; forcing native non-ROI path"
        );
    } else if roi_requested_raw && !roi_require_native && !native_roi {
        info!("ROI quality mode requested but native ROI unsupported; forcing native non-ROI path");
    }
    if roi_requested && !native_roi {
        info!(
            "ROI map requested: skipping native NVENC path; fallback pipeline will use ffmpeg addroi"
        );
    }
    if encoder_backend == VideoEncoderBackend::Nvenc
        && backend == CaptureBackend::Dxgi
        && (!roi_requested || native_roi)
    {
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
                let queue_link_fps = Arc::new(AtomicU32::new(effective_cfg.fps.max(1)));
                let keyframe_request2 = keyframe_request.clone();
                let stats_encode = stats.clone();
                let session_running_encode = session_running.clone();
                let effective_cfg_encode = effective_cfg.clone();
                let selected_transport_encode = selected_transport;
                let queue_link_fps_encode = queue_link_fps.clone();
                let idr_interval = Duration::from_secs(idr_interval_sec_for(
                    effective_cfg.idr_interval_sec,
                    selected_transport_encode,
                    encoder_backend,
                ) as u64);
                std::thread::spawn(move || {
                    let mut encoded_frames: u32 = 0;
                    let mut next_encode_due = Instant::now();
                    let strict_gpu_direct = effective_cfg.strict_gpu_direct;
                    let mut missing_idr_streak: u32 = 0;
                    let mut keyframe_burst_remain: u8 = 0;
                    let mut last_missing_idr_recreate =
                        std::time::Instant::now() - Duration::from_secs(10);
                    let mut missing_idr_recreate_window_start = std::time::Instant::now();
                    let mut missing_idr_recreate_count: u32 = 0;
                    let mut last_interval_force = std::time::Instant::now();
                    while session_running_encode.load(Ordering::SeqCst) {
                        wait_encode_tick(
                            &mut next_encode_due,
                            queue_link_fps_encode.load(Ordering::Relaxed).max(1),
                        );
                        let keyframe_requested = keyframe_request2.swap(false, Ordering::Relaxed);
                        if keyframe_requested {
                            keyframe_burst_remain =
                                keyframe_burst_len_for(selected_transport_encode, encoder_backend);
                        }
                        let interval_force = last_interval_force.elapsed() >= idr_interval;
                        if interval_force {
                            last_interval_force = std::time::Instant::now();
                        }
                        let in_keyframe_burst = keyframe_burst_remain > 0;
                        let force_idr = in_keyframe_burst || interval_force;
                        if in_keyframe_burst {
                            keyframe_burst_remain = keyframe_burst_remain.saturating_sub(1);
                        }
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
                                let has_idr = parse_annexb_nals_view(v.bytes.as_ref())
                                    .iter()
                                    .any(|n| n.nal_type == 5);
                                missing_idr_streak =
                                    next_missing_idr_streak(missing_idr_streak, force_idr, has_idr);
                                if has_idr {
                                    keyframe_burst_remain = 0;
                                    missing_idr_recreate_count = 0;
                                    missing_idr_recreate_window_start = std::time::Instant::now();
                                }
                                if force_idr
                                    && !has_idr
                                    && missing_idr_streak > 0
                                    && missing_idr_streak % 8 == 0
                                {
                                    warn!(
                                        missing_idr_streak,
                                        seq_like = encoded_frames,
                                        "force_idr requested but encoded AU still has no IDR"
                                    );
                                }
                                if missing_idr_recreate_window_start.elapsed()
                                    >= nvenc_missing_idr_recreate_window()
                                {
                                    missing_idr_recreate_window_start = std::time::Instant::now();
                                    missing_idr_recreate_count = 0;
                                }
                                if should_recreate_nvenc_on_missing_idr(
                                    selected_transport_encode,
                                    encoder_backend,
                                    missing_idr_streak,
                                ) && last_missing_idr_recreate.elapsed()
                                    >= nvenc_missing_idr_recreate_cooldown()
                                {
                                    if missing_idr_recreate_count
                                        >= nvenc_missing_idr_recreate_budget_per_window()
                                    {
                                        if missing_idr_recreate_count
                                            == nvenc_missing_idr_recreate_budget_per_window()
                                        {
                                            warn!(
                                                missing_idr_streak,
                                                recreate_budget =
                                                    nvenc_missing_idr_recreate_budget_per_window(),
                                                recreate_window_ms =
                                                    nvenc_missing_idr_recreate_window().as_millis()
                                                        as u64,
                                                "skip NVENC recreate on missing IDR due to budget limit"
                                            );
                                        }
                                    } else {
                                        last_missing_idr_recreate = std::time::Instant::now();
                                        match NativeNvencPipeline::new(
                                            target_w,
                                            target_h,
                                            &effective_cfg_encode,
                                        ) {
                                            Ok(v2) => {
                                                native = v2;
                                                missing_idr_streak = 0;
                                                missing_idr_recreate_count =
                                                    missing_idr_recreate_count.saturating_add(1);
                                                warn!(
                                                    "recreated native NVENC pipeline due to prolonged missing IDR after keyframe requests"
                                                );
                                            }
                                            Err(e) => warn!(
                                                error = %e,
                                                "failed to recreate native NVENC pipeline on missing IDR recovery"
                                            ),
                                        }
                                    }
                                }
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

                if matches!(
                    selected_transport,
                    SessionTransport::Quic | SessionTransport::WebTransport
                ) {
                    let quic_sender = quic_tx.as_ref().cloned().ok_or_else(|| {
                        anyhow!("quic transport selected but quic sender missing")
                    })?;
                    let stats_send = stats.clone();
                    let session_running_send = session_running.clone();
                    tokio::spawn(spawn_send_loop_quic(
                        quic_sender,
                        encoded_rx,
                        stats_send,
                        session_running_send,
                        Some(queue_link_fps.clone()),
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
                        keyframe_request.clone(),
                        media_ready.clone(),
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
                        keyframe_request.clone(),
                        media_ready.clone(),
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

    if encoder_backend == VideoEncoderBackend::Nvenc
        && backend == CaptureBackend::Wgc
        && (!roi_requested || native_roi)
    {
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
                    let queue_link_fps = Arc::new(AtomicU32::new(effective_cfg.fps.max(1)));
                    let keyframe_request2 = keyframe_request.clone();
                    let stats_encode = stats.clone();
                    let session_running_encode = session_running.clone();
                    let effective_cfg_encode = effective_cfg.clone();
                    let selected_transport_encode = selected_transport;
                    let queue_link_fps_encode = queue_link_fps.clone();
                    let idr_interval = Duration::from_secs(idr_interval_sec_for(
                        effective_cfg.idr_interval_sec,
                        selected_transport_encode,
                        encoder_backend,
                    ) as u64);
                    std::thread::spawn(move || {
                        let mut encoded_frames: u32 = 0;
                        let mut next_encode_due = Instant::now();
                        let strict_gpu_direct = effective_cfg.strict_gpu_direct;
                        let mut missing_idr_streak: u32 = 0;
                        let mut keyframe_burst_remain: u8 = 0;
                        let mut last_missing_idr_recreate =
                            std::time::Instant::now() - Duration::from_secs(10);
                        let mut missing_idr_recreate_window_start = std::time::Instant::now();
                        let mut missing_idr_recreate_count: u32 = 0;
                        let mut last_interval_force = std::time::Instant::now();
                        while session_running_encode.load(Ordering::SeqCst) {
                            wait_encode_tick(
                                &mut next_encode_due,
                                queue_link_fps_encode.load(Ordering::Relaxed).max(1),
                            );
                            let keyframe_requested =
                                keyframe_request2.swap(false, Ordering::Relaxed);
                            if keyframe_requested {
                                keyframe_burst_remain = keyframe_burst_len_for(
                                    selected_transport_encode,
                                    encoder_backend,
                                );
                            }
                            let interval_force = last_interval_force.elapsed() >= idr_interval;
                            if interval_force {
                                last_interval_force = std::time::Instant::now();
                            }
                            let in_keyframe_burst = keyframe_burst_remain > 0;
                            let force_idr = in_keyframe_burst || interval_force;
                            if in_keyframe_burst {
                                keyframe_burst_remain = keyframe_burst_remain.saturating_sub(1);
                            }
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
                            let capture_start_us =
                                capture.as_ref().map(|f| f.capture_start_us).unwrap_or(0);
                            let encoded_res = capture
                                .and_then(|frame| native.encode_texture(&frame.texture, force_idr));
                            match encoded_res {
                                Ok(Some(v)) if !v.bytes.is_empty() => {
                                    let has_idr = parse_annexb_nals_view(v.bytes.as_ref())
                                        .iter()
                                        .any(|n| n.nal_type == 5);
                                    missing_idr_streak = next_missing_idr_streak(
                                        missing_idr_streak,
                                        force_idr,
                                        has_idr,
                                    );
                                    if has_idr {
                                        keyframe_burst_remain = 0;
                                        missing_idr_recreate_count = 0;
                                        missing_idr_recreate_window_start =
                                            std::time::Instant::now();
                                    }
                                    if force_idr
                                        && !has_idr
                                        && missing_idr_streak > 0
                                        && missing_idr_streak % 8 == 0
                                    {
                                        warn!(
                                            missing_idr_streak,
                                            seq_like = encoded_frames,
                                            "WGC force_idr requested but encoded AU still has no IDR"
                                        );
                                    }
                                    if missing_idr_recreate_window_start.elapsed()
                                        >= nvenc_missing_idr_recreate_window()
                                    {
                                        missing_idr_recreate_window_start =
                                            std::time::Instant::now();
                                        missing_idr_recreate_count = 0;
                                    }
                                    if should_recreate_nvenc_on_missing_idr(
                                        selected_transport_encode,
                                        encoder_backend,
                                        missing_idr_streak,
                                    ) && last_missing_idr_recreate.elapsed()
                                        >= nvenc_missing_idr_recreate_cooldown()
                                    {
                                        if missing_idr_recreate_count
                                            >= nvenc_missing_idr_recreate_budget_per_window()
                                        {
                                            if missing_idr_recreate_count
                                                == nvenc_missing_idr_recreate_budget_per_window()
                                            {
                                                warn!(
                                                    missing_idr_streak,
                                                    recreate_budget = nvenc_missing_idr_recreate_budget_per_window(),
                                                    recreate_window_ms = nvenc_missing_idr_recreate_window().as_millis() as u64,
                                                    "skip WGC NVENC recreate on missing IDR due to budget limit"
                                                );
                                            }
                                        } else {
                                            last_missing_idr_recreate = std::time::Instant::now();
                                            match NativeNvencTexturePipeline::new(
                                                wgc.device(),
                                                wgc.context(),
                                                target_w,
                                                target_h,
                                                &effective_cfg_encode,
                                            ) {
                                                Ok(v2) => {
                                                    native = v2;
                                                    missing_idr_streak = 0;
                                                    missing_idr_recreate_count =
                                                        missing_idr_recreate_count
                                                            .saturating_add(1);
                                                    warn!(
                                                        "recreated WGC native NVENC texture pipeline due to prolonged missing IDR after keyframe requests"
                                                    );
                                                }
                                                Err(e) => warn!(
                                                    error = %e,
                                                    "failed to recreate WGC native NVENC texture pipeline on missing IDR recovery"
                                                ),
                                            }
                                        }
                                    }
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

                    if matches!(
                        selected_transport,
                        SessionTransport::Quic | SessionTransport::WebTransport
                    ) {
                        let quic_sender = quic_tx.as_ref().cloned().ok_or_else(|| {
                            anyhow!("quic transport selected but quic sender missing")
                        })?;
                        let stats_send = stats.clone();
                        let session_running_send = session_running.clone();
                        tokio::spawn(spawn_send_loop_quic(
                            quic_sender,
                            encoded_rx,
                            stats_send,
                            session_running_send,
                            Some(queue_link_fps.clone()),
                        ));
                    } else if let Some(track) = rtp_track.clone() {
                        let sender = RtpH264Sender::new(
                            track,
                            &RtpH264SenderConfig {
                                fps: effective_cfg.fps.max(1),
                                mtu: effective_cfg.rtp_mtu,
                                frame_pacing_enable: effective_cfg.frame_pacing_enable,
                                frame_pacing_batch_packets: effective_cfg
                                    .frame_pacing_batch_packets,
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
                            keyframe_request.clone(),
                            media_ready.clone(),
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
                            keyframe_request.clone(),
                            media_ready.clone(),
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
    let queue_link_fps = Arc::new(AtomicU32::new(fps.max(1)));

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
        let queue_link_fps_encode = queue_link_fps.clone();
        let idr_interval_frames = fps.max(1) * effective_cfg.idr_interval_sec.max(1);
        std::thread::spawn(move || {
            let mut encoder = match build_video_encoder(
                fps,
                &encode_cfg,
                encoder_backend,
                allow_encoder_fallback,
                selected_transport.as_str(),
            ) {
                Ok(e) => e,
                Err(e) => {
                    error!(error = %e, "H264 encoder initialization failed");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let mut encoded_frames: u32 = 0;
            let mut next_encode_due = Instant::now();

            while running.load(Ordering::Relaxed) && session_running_encode.load(Ordering::SeqCst) {
                wait_encode_tick(
                    &mut next_encode_due,
                    queue_link_fps_encode.load(Ordering::Relaxed).max(1),
                );
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

    if matches!(
        selected_transport,
        SessionTransport::Quic | SessionTransport::WebTransport
    ) {
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
            Some(queue_link_fps.clone()),
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
            keyframe_request.clone(),
            media_ready,
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
                let send_gap = frame_gap_from_fps(send_fps);
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

fn apply_transport_send_policy(
    selected_transport: SessionTransport,
    cfg: &mut agent_rust::CaptureConfig,
) {
    if selected_transport == SessionTransport::WebRtc {
        // Default to manual packetizer for low-latency RTP path, but allow
        // sample-track fallback when debugging decode interoperability.
        cfg.rtp_use_manual_packetizer = should_force_webrtc_manual_packetizer(
            std::env::var("AGENT_WEBRTC_MANUAL_RTP").ok().as_deref(),
        );
        cfg.max_fps_mode = false;
        info!(
            rtp_use_manual_packetizer = cfg.rtp_use_manual_packetizer,
            max_fps_mode = cfg.max_fps_mode,
            "applied WebRTC-safe media send policy"
        );
    }
}

fn should_force_webrtc_manual_packetizer(raw: Option<&str>) -> bool {
    match raw.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if matches!(v.as_str(), "0" | "false" | "off" | "no") => false,
        _ => true,
    }
}

async fn spawn_send_loop_sample(
    track: Arc<TrackLocalStaticSample>,
    mut encoded_rx: tokio::sync::mpsc::Receiver<Arc<[u8]>>,
    fps: u32,
    stats_send: Arc<RuntimeStats>,
    repeat_last: bool,
    idle_repeat_fps: u32,
    keyframe_request: Arc<AtomicBool>,
    media_ready: Arc<AtomicBool>,
    session_running_send: Arc<AtomicBool>,
) {
    let mut last_encoded: Option<Arc<[u8]>> = None;
    let mut bootstrap_idr: Option<Arc<[u8]>> = None;
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let h264_debug = std::env::var("AGENT_H264_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut h264_debug_left = h264_debug_budget();
    let mut next_due = Instant::now();
    let mut first_idr_sent = false;
    let mut last_gate_force = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut last_media_ready = false;
    while session_running_send.load(Ordering::SeqCst) {
        let now_ready = media_ready.load(Ordering::SeqCst);
        if now_ready != last_media_ready {
            if now_ready {
                first_idr_sent = false;
                last_encoded = bootstrap_idr.take();
                keyframe_request.store(true, Ordering::Relaxed);
                info!("sample send unblocked on connected state; forcing keyframe bootstrap");
            } else {
                first_idr_sent = false;
                warn!("sample send gated: peer connection not ready");
            }
            last_media_ready = now_ready;
        }
        if !now_ready {
            while let Ok(encoded) = encoded_rx.try_recv() {
                update_h264_param_cache(encoded.as_ref(), &mut last_sps, &mut last_pps);
                if parse_annexb_nals_view(encoded.as_ref())
                    .iter()
                    .any(|n| n.nal_type == 5)
                {
                    bootstrap_idr = Some(encoded.clone());
                }
                last_encoded = Some(encoded);
            }
            keyframe_request.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(8)).await;
            continue;
        }

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
        let send_gap = frame_gap_from_fps(send_fps);
        next_due = advance_send_deadline(next_due, send_gap, Instant::now());
        let au_for_send = if let Some(patched) =
            patch_h264_au_with_cached_params(encoded.as_ref(), &last_sps, &last_pps)
        {
            Arc::<[u8]>::from(patched)
        } else {
            encoded.clone()
        };
        let has_idr = parse_annexb_nals_view(au_for_send.as_ref())
            .iter()
            .any(|n| n.nal_type == 5);
        if should_gate_rtp_until_first_idr(true, first_idr_sent, has_idr) {
            if last_gate_force.elapsed() >= Duration::from_millis(120) {
                keyframe_request.store(true, Ordering::Relaxed);
                last_gate_force = Instant::now();
            }
            continue;
        }
        if has_idr && !first_idr_sent {
            first_idr_sent = true;
            info!("sample bootstrap: first IDR observed and sent");
        }
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
    keyframe_request: Arc<AtomicBool>,
    media_ready: Arc<AtomicBool>,
    session_running: Arc<AtomicBool>,
) {
    let mut next_due = Instant::now();
    let mut next_recover_tick = Instant::now();
    let mut last_encoded: Option<Arc<[u8]>> = None;
    let mut bootstrap_idr: Option<Arc<[u8]>> = None;
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let h264_debug = std::env::var("AGENT_H264_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut h264_debug_left = h264_debug_budget();
    let mut consecutive_send_errors: u32 = 0;
    let gate_until_first_idr = std::env::var("AGENT_RTP_WAIT_FIRST_IDR")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let mut first_idr_sent = false;
    let mut last_gate_force = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut last_media_ready = false;
    while session_running.load(Ordering::SeqCst) {
        let now_ready = media_ready.load(Ordering::SeqCst);
        if now_ready != last_media_ready {
            if now_ready {
                first_idr_sent = false;
                last_encoded = bootstrap_idr.take();
                keyframe_request.store(true, Ordering::Relaxed);
                info!("media send unblocked on connected state; forcing keyframe bootstrap");
            } else {
                first_idr_sent = false;
                warn!("media send gated: peer connection not ready");
            }
            last_media_ready = now_ready;
        }
        if !now_ready {
            while let Ok(encoded) = encoded_rx.try_recv() {
                update_h264_param_cache(encoded.as_ref(), &mut last_sps, &mut last_pps);
                if parse_annexb_nals_view(encoded.as_ref())
                    .iter()
                    .any(|n| n.nal_type == 5)
                {
                    bootstrap_idr = Some(encoded.clone());
                }
                last_encoded = Some(encoded);
            }
            keyframe_request.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(8)).await;
            continue;
        }

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
        let frame_gap = frame_gap_from_fps(send_fps);
        next_due = advance_send_deadline(next_due, frame_gap, Instant::now());
        let au_for_send = if let Some(patched) =
            patch_h264_au_with_cached_params(encoded.as_ref(), &last_sps, &last_pps)
        {
            Arc::<[u8]>::from(patched)
        } else {
            encoded.clone()
        };
        let has_idr = parse_annexb_nals_view(au_for_send.as_ref())
            .iter()
            .any(|n| n.nal_type == 5);
        if should_gate_rtp_until_first_idr(gate_until_first_idr, first_idr_sent, has_idr) {
            if last_gate_force.elapsed() >= Duration::from_millis(120) {
                keyframe_request.store(true, Ordering::Relaxed);
                last_gate_force = Instant::now();
            }
            continue;
        }
        if has_idr && !first_idr_sent {
            first_idr_sent = true;
            info!("RTP bootstrap: first IDR observed and sent");
        }
        if h264_debug && h264_debug_left > 0 {
            let nals = parse_annexb_nals_view(au_for_send.as_ref());
            let nal_types: Vec<u8> = nals.iter().map(|n| n.nal_type).collect();
            let has_sps = nal_types.contains(&7);
            let has_pps = nal_types.contains(&8);
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
    queue_link_fps: Option<Arc<AtomicU32>>,
) {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PaceMode {
        Manual,
        Auto,
    }

    struct QuicPacer {
        enabled: bool,
        mode: PaceMode,
        interval: Duration,
        burst: f64,
        tokens: f64,
        last_refill: tokio::time::Instant,
        auto_active: bool,
        pressure: i32,
        auto_on_full: i32,
        auto_off_ok: i32,
    }

    impl QuicPacer {
        fn from_env() -> Self {
            let enabled = std::env::var("AGENT_QUIC_PACE_ENABLE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let mode = match std::env::var("AGENT_QUIC_PACE_MODE")
                .ok()
                .unwrap_or_else(|| "manual".to_string())
                .to_ascii_lowercase()
                .as_str()
            {
                "auto" => PaceMode::Auto,
                _ => PaceMode::Manual,
            };
            let interval_ms = std::env::var("AGENT_QUIC_PACE_INTERVAL_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(2)
                .clamp(1, 100);
            let burst = std::env::var("AGENT_QUIC_PACE_BURST")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(4)
                .clamp(1, 16) as f64;
            let auto_on_full = std::env::var("AGENT_QUIC_PACE_AUTO_ON_FULL")
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(8)
                .clamp(1, 1000);
            let auto_off_ok = std::env::var("AGENT_QUIC_PACE_AUTO_OFF_OK")
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(64)
                .clamp(1, 5000);
            Self {
                enabled,
                mode,
                interval: Duration::from_millis(interval_ms),
                burst,
                tokens: burst,
                last_refill: tokio::time::Instant::now(),
                auto_active: false,
                pressure: 0,
                auto_on_full,
                auto_off_ok,
            }
        }

        fn should_pace(&self) -> bool {
            self.enabled
                && match self.mode {
                    PaceMode::Manual => true,
                    PaceMode::Auto => self.auto_active,
                }
        }

        fn on_send_ok(&mut self) {
            if !self.enabled || self.mode != PaceMode::Auto {
                return;
            }
            self.pressure = self.pressure.saturating_sub(1);
            if self.auto_active && self.pressure <= -self.auto_off_ok {
                self.auto_active = false;
                self.pressure = 0;
            }
        }

        fn on_send_full(&mut self) {
            if !self.enabled || self.mode != PaceMode::Auto {
                return;
            }
            self.pressure = self.pressure.saturating_add(4).clamp(-10_000, 10_000);
            if !self.auto_active && self.pressure >= self.auto_on_full {
                self.auto_active = true;
                self.tokens = self.burst;
                self.last_refill = tokio::time::Instant::now();
            }
        }

        async fn wait_turn(&mut self) {
            if !self.should_pace() {
                return;
            }
            loop {
                let now = tokio::time::Instant::now();
                let elapsed = now.saturating_duration_since(self.last_refill);
                let step = self.interval.as_secs_f64();
                if step > 0.0 {
                    self.tokens =
                        (self.tokens + elapsed.as_secs_f64() / step).clamp(0.0, self.burst);
                }
                self.last_refill = now;
                if self.tokens >= 1.0 {
                    self.tokens -= 1.0;
                    return;
                }
                let need = ((1.0 - self.tokens).max(0.0) * step).clamp(0.0, 0.050);
                tokio::time::sleep(Duration::from_secs_f64(need)).await;
            }
        }
    }

    struct QueueRateLink {
        enabled: bool,
        min_fps: u32,
        max_fps: u32,
        down_step: u32,
        up_step: u32,
        full_threshold: u32,
        ok_threshold: u32,
        cooldown: Duration,
        full_streak: u32,
        ok_streak: u32,
        last_change: Instant,
        fps_ref: Option<Arc<AtomicU32>>,
    }

    impl QueueRateLink {
        fn from_env(fps_ref: Option<Arc<AtomicU32>>) -> Self {
            let enabled = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_ENABLE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(true);
            let current_fps = fps_ref
                .as_ref()
                .map(|v| v.load(Ordering::Relaxed))
                .unwrap_or(60)
                .max(1);
            let min_fps = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_MIN_FPS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(24)
                .clamp(1, 240);
            let max_fps = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_MAX_FPS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(current_fps.max(min_fps))
                .clamp(min_fps, 240);
            let down_step = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_DOWN_STEP")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(8)
                .clamp(1, 60);
            let up_step = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_UP_STEP")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(2)
                .clamp(1, 30);
            let full_threshold = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_FULL_THRESHOLD")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(8)
                .clamp(1, 2000);
            let ok_threshold = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_OK_THRESHOLD")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(120)
                .clamp(1, 20_000);
            let cooldown_ms = std::env::var("AGENT_QUIC_QUEUE_RATE_LINK_COOLDOWN_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(200)
                .clamp(0, 10_000);
            Self {
                enabled: enabled && fps_ref.is_some(),
                min_fps,
                max_fps,
                down_step,
                up_step,
                full_threshold,
                ok_threshold,
                cooldown: Duration::from_millis(cooldown_ms),
                full_streak: 0,
                ok_streak: 0,
                last_change: Instant::now(),
                fps_ref,
            }
        }

        fn reduce_on_full(&mut self) {
            if !self.enabled {
                return;
            }
            self.full_streak = self.full_streak.saturating_add(1);
            self.ok_streak = 0;
            if self.full_streak < self.full_threshold || self.last_change.elapsed() < self.cooldown
            {
                return;
            }
            if let Some(fps_ref) = &self.fps_ref {
                let cur = fps_ref.load(Ordering::Relaxed).max(1);
                let next = cur
                    .saturating_sub(self.down_step)
                    .clamp(self.min_fps, self.max_fps);
                if next < cur {
                    fps_ref.store(next, Ordering::Relaxed);
                    self.last_change = Instant::now();
                    self.full_streak = 0;
                    warn!(
                        current_fps = cur,
                        next_fps = next,
                        "queue-pressure rate link downshift"
                    );
                }
            }
        }

        fn recover_on_ok(&mut self) {
            if !self.enabled {
                return;
            }
            self.ok_streak = self.ok_streak.saturating_add(1);
            self.full_streak = 0;
            if self.ok_streak < self.ok_threshold || self.last_change.elapsed() < self.cooldown {
                return;
            }
            if let Some(fps_ref) = &self.fps_ref {
                let cur = fps_ref.load(Ordering::Relaxed).max(1);
                let next = cur
                    .saturating_add(self.up_step)
                    .clamp(self.min_fps, self.max_fps);
                if next > cur {
                    fps_ref.store(next, Ordering::Relaxed);
                    self.last_change = Instant::now();
                    self.ok_streak = 0;
                    info!(
                        current_fps = cur,
                        next_fps = next,
                        "queue-pressure rate link recover"
                    );
                }
            }
        }
    }

    let quic_debug = std::env::var("AGENT_QUIC_DEBUG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let codec_effective = std::env::var("AGENT_VIDEO_CODEC_EFFECTIVE")
        .ok()
        .unwrap_or_else(|| "h264".to_string())
        .to_ascii_lowercase();
    let should_patch_h264 = codec_effective == "h264";
    let mut debug_left = 8usize;
    let max_au_bytes = std::env::var("AGENT_QUIC_MAX_AU_BYTES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1_500_000)
        .clamp(64 * 1024, 8 * 1024 * 1024);
    let min_gap_us = std::env::var("AGENT_QUIC_MIN_GAP_US")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .min(50_000);
    let min_gap = if min_gap_us > 0 {
        Some(Duration::from_micros(min_gap_us))
    } else {
        None
    };
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;
    let mut dropped = 0_u64;
    let mut pacer = QuicPacer::from_env();
    let mut rate_link = QueueRateLink::from_env(queue_link_fps.clone());
    let mut last_send_at: Option<Instant> = None;
    let mut last_send_attempt_at: Option<Instant> = None;
    let mut last_capture_start_us: Option<u64> = None;
    let mut last_encoded_recv_at: Option<Instant> = None;
    let mut congestion_backoff: u32 = 0;
    while session_running.load(Ordering::SeqCst) {
        let recv_wait_start = Instant::now();
        let encoded = match encoded_rx.recv().await {
            Some(v) => v,
            None => break,
        };
        if let Some(prev) = last_encoded_recv_at {
            let interval_us = recv_wait_start
                .duration_since(prev)
                .as_micros()
                .min(u64::MAX as u128) as u64;
            stats.record_transport_encode_output_interval_us(interval_us);
        }
        last_encoded_recv_at = Some(recv_wait_start);
        let enqueue_wait_us = recv_wait_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        stats.encoded_au_total.fetch_add(1, Ordering::Relaxed);
        stats
            .transport_enqueue_wait_us_total
            .fetch_add(enqueue_wait_us, Ordering::Relaxed);
        stats.record_transport_queue_wait_us(enqueue_wait_us);
        let (capture_start_us, payload) = unpack_capture_ts_au(encoded.as_ref());
        if capture_start_us > 0 {
            if let Some(prev) = last_capture_start_us {
                if capture_start_us >= prev {
                    stats.record_transport_capture_interval_us(capture_start_us - prev);
                }
            }
            last_capture_start_us = Some(capture_start_us);
        }
        let mut out = payload.to_vec();
        if should_patch_h264 {
            update_h264_param_cache(&out, &mut last_sps, &mut last_pps);
            if let Some(patched) = patch_h264_au_with_cached_params(&out, &last_sps, &last_pps) {
                out = patched;
            }
        }
        if quic_debug && debug_left > 0 {
            let take = out.len().min(12);
            let mut head = String::new();
            for b in &out[..take] {
                use std::fmt::Write as _;
                let _ = write!(&mut head, "{:02X} ", b);
            }
            let nal_types: Vec<u8> = if should_patch_h264 {
                parse_annexb_nals_view(&out)
                    .iter()
                    .map(|n| n.nal_type)
                    .collect()
            } else {
                Vec::new()
            };
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
        let out_len = out.len() as u64;
        let quic_au = QuicAu {
            payload: out,
            tx_unix_us: if capture_start_us == 0 {
                unix_time_us()
            } else {
                capture_start_us
            },
        };
        // Avoid artificial fixed-rate throttling. Only apply transport pacing when
        // pacing is active or we recently observed queue congestion.
        if should_throttle_quic_send(pacer.should_pace(), congestion_backoff) {
            pacer.wait_turn().await;
            if let (Some(gap), Some(last)) = (min_gap, last_send_at) {
                let elapsed = last.elapsed();
                if elapsed < gap {
                    tokio::time::sleep(gap - elapsed).await;
                }
            }
        }
        let capture_to_send_start_us = if capture_start_us > 0 {
            unix_time_us().saturating_sub(capture_start_us)
        } else {
            0
        };
        let send_attempt_start = Instant::now();
        if let Some(prev) = last_send_attempt_at {
            let interval_us = send_attempt_start
                .duration_since(prev)
                .as_micros()
                .min(u64::MAX as u128) as u64;
            stats.record_transport_send_interval_us(interval_us);
        }
        last_send_attempt_at = Some(send_attempt_start);
        match quic_sender.try_send(quic_au) {
            Ok(()) => {
                last_send_at = Some(Instant::now());
                pacer.on_send_ok();
                rate_link.recover_on_ok();
                if congestion_backoff > 0 {
                    congestion_backoff = congestion_backoff.saturating_sub(1);
                }
                stats.sent_au_total.fetch_add(1, Ordering::Relaxed);
                stats.unique_sent_au_total.fetch_add(1, Ordering::Relaxed);
                stats.quic_au_sent.fetch_add(1, Ordering::Relaxed);
                stats.quic_bytes_sent.fetch_add(out_len, Ordering::Relaxed);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                last_send_at = Some(Instant::now());
                pacer.on_send_full();
                rate_link.reduce_on_full();
                congestion_backoff = congestion_backoff.saturating_add(4).min(64);
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
        let send_elapsed_us = send_attempt_start
            .elapsed()
            .as_micros()
            .min(u64::MAX as u128) as u64;
        stats
            .transport_send_us_total
            .fetch_add(send_elapsed_us, Ordering::Relaxed);
        stats.transport_send_samples.fetch_add(1, Ordering::Relaxed);
        stats.record_transport_send_us(send_elapsed_us);
        if capture_to_send_start_us > 0 {
            let capture_us = capture_to_send_start_us.saturating_add(send_elapsed_us);
            let encode_us = capture_to_send_start_us.saturating_sub(send_elapsed_us);
            stats
                .transport_capture_to_send_us_total
                .fetch_add(capture_us, Ordering::Relaxed);
            stats
                .transport_encode_approx_us_total
                .fetch_add(encode_us, Ordering::Relaxed);
            stats
                .transport_capture_encode_samples
                .fetch_add(1, Ordering::Relaxed);
            stats.record_transport_capture_encode_us(capture_us, encode_us);
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
    let mut out = Vec::with_capacity(
        au.len() + sps.as_ref().map_or(0, |v| v.len()) + pps.as_ref().map_or(0, |v| v.len()),
    );
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
        if i + 4 < buf.len() && buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 0 && buf[i + 3] == 1
        {
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
        let end = if idx + 1 < starts.len() {
            starts[idx + 1].0
        } else {
            buf.len()
        };
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

fn should_gate_rtp_until_first_idr(
    wait_first_idr: bool,
    first_idr_sent: bool,
    has_idr: bool,
) -> bool {
    wait_first_idr && !first_idr_sent && !has_idr
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

fn frame_gap_from_fps(fps: u32) -> Duration {
    let fps = fps.max(1) as f64;
    Duration::from_secs_f64((1.0 / fps).max(0.000_5))
}

fn wait_encode_tick(next_due: &mut Instant, fps: u32) {
    let gap = frame_gap_from_fps(fps);
    let now = Instant::now();
    if *next_due > now {
        std::thread::sleep(*next_due - now);
    }
    let wake = Instant::now();
    *next_due = if *next_due <= wake {
        wake + gap
    } else {
        *next_due + gap
    };
}

fn should_throttle_quic_send(pacer_active: bool, congestion_backoff: u32) -> bool {
    pacer_active || congestion_backoff > 0
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
    fn nvenc_recreate_env_parse_defaults_to_disabled() {
        assert!(!nvenc_recreate_on_force_idr_enabled_from(None));
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
        assert!(!should_recreate_nvenc_on_force_idr(
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

    #[test]
    fn missing_idr_recovery_policy_only_for_webrtc_nvenc() {
        assert!(should_recreate_nvenc_on_missing_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::Nvenc,
            24,
        ));
        assert!(!should_recreate_nvenc_on_missing_idr(
            SessionTransport::Quic,
            VideoEncoderBackend::Nvenc,
            24,
        ));
        assert!(!should_recreate_nvenc_on_missing_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::OpenH264,
            24,
        ));
        assert!(!should_recreate_nvenc_on_missing_idr(
            SessionTransport::WebRtc,
            VideoEncoderBackend::Nvenc,
            8,
        ));
    }

    #[test]
    fn missing_idr_streak_updates_with_force_and_idr() {
        assert_eq!(next_missing_idr_streak(0, true, false), 1);
        assert_eq!(next_missing_idr_streak(5, true, false), 6);
        assert_eq!(next_missing_idr_streak(6, false, false), 6);
        assert_eq!(next_missing_idr_streak(6, true, true), 0);
        assert_eq!(next_missing_idr_streak(6, false, true), 0);
    }

    #[test]
    fn transport_policy_forces_manual_packetizer_for_webrtc() {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.rtp_use_manual_packetizer = false;
        cfg.max_fps_mode = true;
        apply_transport_send_policy(SessionTransport::WebRtc, &mut cfg);
        assert!(cfg.rtp_use_manual_packetizer);
        assert!(!cfg.max_fps_mode);
    }

    #[test]
    fn rtp_bootstrap_gate_waits_until_first_idr() {
        assert!(should_gate_rtp_until_first_idr(true, false, false));
        assert!(!should_gate_rtp_until_first_idr(true, false, true));
        assert!(!should_gate_rtp_until_first_idr(true, true, false));
        assert!(!should_gate_rtp_until_first_idr(false, false, false));
    }

    #[test]
    fn manual_rtp_override_parser() {
        assert!(should_force_webrtc_manual_packetizer(None));
        assert!(should_force_webrtc_manual_packetizer(Some("1")));
        assert!(!should_force_webrtc_manual_packetizer(Some("0")));
        assert!(!should_force_webrtc_manual_packetizer(Some("false")));
        assert!(!should_force_webrtc_manual_packetizer(Some("off")));
    }

    #[test]
    fn parse_transport_priority_filters_invalid_items() {
        let parsed = parse_transport_priority("webtransport,invalid,quic,webrtc");
        assert_eq!(
            parsed,
            vec![
                SessionTransport::WebTransport,
                SessionTransport::Quic,
                SessionTransport::WebRtc
            ]
        );
    }

    #[test]
    fn controller_supports_transport_respects_protocol_caps() {
        let caps = json!({ "protocols": ["webrtc", "quic"] });
        assert!(controller_supports_transport(
            &caps,
            SessionTransport::WebRtc
        ));
        assert!(controller_supports_transport(&caps, SessionTransport::Quic));
        assert!(!controller_supports_transport(
            &caps,
            SessionTransport::WebTransport
        ));
    }

    #[test]
    fn parse_codec_priority_filters_and_orders() {
        let parsed = parse_codec_priority("av1,hevc,h264");
        assert_eq!(
            parsed,
            vec![VideoCodec::Av1, VideoCodec::Hevc, VideoCodec::H264]
        );
    }

    #[test]
    fn effective_roi_request_respects_native_requirement() {
        assert!(effective_roi_request(true, true, true));
        assert!(!effective_roi_request(true, false, true));
        assert!(!effective_roi_request(true, false, false));
        assert!(!effective_roi_request(false, false, true));
    }

    #[test]
    fn codec_strategy_keeps_webrtc_on_h264() {
        let caps = json!({ "codecs": ["h264", "hevc", "av1"] });
        let selected = select_codec_by_strategy(SessionTransport::WebRtc, &caps);
        assert_eq!(selected, VideoCodec::H264);
    }

    #[test]
    fn quic_send_throttle_only_when_needed() {
        assert!(!should_throttle_quic_send(false, 0));
        assert!(should_throttle_quic_send(true, 0));
        assert!(should_throttle_quic_send(false, 1));
    }

    #[test]
    fn fps_mode_throughput_relaxes_tier_and_pacing() {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.fps = 200;
        cfg.max_fps = 200;
        cfg.queue_depth = 2;
        cfg.tier_limit_enable = true;
        cfg.frame_pacing_enable = true;
        apply_fps_mode_policy_with_mode(&mut cfg, "throughput");
        assert!(!cfg.tier_limit_enable);
        assert!(!cfg.frame_pacing_enable);
        assert!(cfg.queue_depth >= 8);
    }

    #[test]
    fn fps_mode_latency_prefers_small_queue_without_pacing() {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.queue_depth = 32;
        cfg.frame_pacing_enable = true;
        apply_fps_mode_policy_with_mode(&mut cfg, "latency");
        assert!(!cfg.frame_pacing_enable);
        assert!(cfg.queue_depth <= 8);
    }

    #[test]
    fn fps_mode_balanced_restores_pacing_window() {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.queue_depth = 1;
        cfg.frame_pacing_enable = false;
        apply_fps_mode_policy_with_mode(&mut cfg, "balanced");
        assert!(cfg.frame_pacing_enable);
        assert!(cfg.queue_depth >= 4);
    }

    #[test]
    fn fps_mode_prefers_config_when_env_missing() {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.fps_mode = "throughput".to_string();
        let mode = resolve_fps_mode(&cfg);
        assert_eq!(mode, "throughput");
    }
}
