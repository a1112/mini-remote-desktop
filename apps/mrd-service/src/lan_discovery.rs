use crate::app_state::AppState;
#[cfg(all(test, any(windows, target_os = "macos")))]
use crate::app_state::{MediaRenderFrame, MediaRenderQueueEnqueue};
use anyhow::{Context, Result};
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, DisplayMode, DisplayModeChange, LanDiscoverySnapshot,
    MediaProfile, MediaProfileNegotiation,
};
#[cfg(test)]
use mrd_ipc::{MediaSenderTransportSnapshot, MediaStageMetrics};
#[cfg(test)]
use mrd_pipeline_core::DecodedFrame;
#[cfg(test)]
use mrd_pipeline_core::DecodedFrameData;
#[cfg(test)]
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};
use mrd_proto::{DeviceId, SessionId};
#[cfg(test)]
use mrd_render::RenderFrame;
#[cfg(test)]
use mrd_transport_quic_quinn::QuicAuFrame;
#[cfg(test)]
use mrd_transport_quic_quinn::QuicAuReassemblerConfig;
use mrd_transport_quic_quinn::{
    fragment_access_unit, fragment_media_payload_v3, is_quic_media_v3_datagram, QuicAuReassembler,
    QuicMediaPayloadType, QuicMediaReassembler, QuinnDatagramEndpoint, QuinnServerBootstrap,
    QuinnServerListener, QUIC_AU_FRAGMENT_HEADER_LEN, QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};
#[cfg(any(test, target_os = "macos"))]
use mrd_transport_quic_quinn::{QuicAuReassemblerStats, QuicMediaCodec, QuicMediaFrame};
use std::collections::HashMap;
#[cfg(all(test, target_os = "macos"))]
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(all(test, target_os = "macos"))]
use std::sync::Condvar as StdCondvar;
#[cfg(test)]
use std::sync::Mutex as StdMutex;
#[cfg(any(windows, target_os = "macos"))]
use std::sync::OnceLock;
use std::time::Duration;
#[cfg(all(test, target_os = "macos"))]
use std::time::Instant as StdInstant;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, Notify};
use tokio::time::{interval, timeout, Instant};

mod capture_activity;
mod capture_sources;
mod discovery_config;
mod discovery_identity;
mod dynamic_window_fps;
mod lan_control_input;
mod local_network_identity;
mod media_access_unit;
mod media_capabilities;
mod media_capture_config;
mod media_envelope;
mod media_error_policy;
mod media_frame_capture;
mod media_frame_preparation;
mod media_keyframe_request;
mod media_ordering;
mod media_probe;
mod media_profile;
mod media_receiver;
mod media_receiver_decoder;
mod media_receiver_decoder_candidates;
mod media_receiver_runtime;
mod media_render_policy;
mod media_render_worker;
mod media_sender;
mod media_sender_telemetry;
mod media_timing;
mod media_transport;
mod peer_format;
mod peer_lookup;
mod peer_registry;
mod protocol;
mod remote_power;
mod runtime_flags;
mod service_identity;
mod session_runtime;
mod time_utils;
use capture_activity::active_window_capture_count;
pub use discovery_config::LanDiscoveryConfig;
use discovery_identity::{
    is_valid_discovery_packet, new_instance_id, now_ms, DISCOVERY_APP_ID, DISCOVERY_MAGIC,
};
use dynamic_window_fps::{
    is_winrt_window_capture_no_frame_timeout, update_dynamic_window_fps_decision,
    window_dynamic_fps_input_for_capture_error, window_dynamic_fps_input_for_captured_frame,
    DynamicWindowFpsDecision, DynamicWindowFpsPolicy,
};
#[cfg(test)]
use dynamic_window_fps::{DynamicWindowFpsInput, DynamicWindowFpsTier};
pub use lan_control_input::request_lan_control_input;
use lan_control_input::{
    accept_or_replay_lan_control_input, LanControlInputAckState, LanControlInputDedupeKey,
};
use local_network_identity::local_lan_announcement_mac_address;
use media_access_unit::{h264_access_unit_is_keyframe, LanAccessUnitCodec};
use media_capabilities::{
    lan_media_capabilities, lan_media_capabilities_with_input_control,
    LAN_MEDIA_AV1_MAIN_420_8BIT_CAPABILITY, LAN_MEDIA_COLOR_MODE_CAPABILITY,
    LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY, LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY,
};
#[cfg(all(test, target_os = "macos"))]
use media_capabilities::{
    macos_lan_media_capabilities_from_probe, probe_macos_lan_media_capabilities,
    MacosLanMediaCapabilityProbe, LAN_CAPTURE_MACOS_CAPABILITY, LAN_DECODE_VIDEOTOOLBOX_CAPABILITY,
    LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY, LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY,
    LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY, LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY,
    LAN_RENDER_MACOS_NATIVE_CAPABILITY,
};
#[cfg(all(test, windows))]
use media_capabilities::{
    LAN_CAPTURE_DXGI_CAPABILITY, LAN_DECODE_NVDEC_AV1_CAPABILITY, LAN_DECODE_NVDEC_CAPABILITY,
    LAN_DECODE_NVDEC_HEVC_CAPABILITY, LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY,
    LAN_ENCODE_NVENC_AV1_CAPABILITY, LAN_ENCODE_NVENC_H264_CAPABILITY,
    LAN_ENCODE_NVENC_HEVC_CAPABILITY, LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY,
    LAN_RENDER_D3D11_NATIVE_CAPABILITY, LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY,
};
#[cfg(test)]
use media_capture_config::window_capture_source_error;
#[cfg(all(test, windows))]
use media_capture_config::windows_lan_window_capture_uses_shared_texture;
use media_capture_config::{
    dynamic_window_fps_config_key, format_capture_source_failure, is_windows_window_source_id,
    lan_capture_config_key, lan_capture_config_matches, DynamicWindowFpsConfigKey,
    LanCaptureConfigKey,
};
#[cfg(windows)]
use media_capture_config::{
    parse_windows_window_source_id, windows_lan_capture_backend,
    windows_lan_capture_backend_for_profile, windows_lan_nvenc_h264_available,
    WindowsLanCaptureBackend,
};
#[cfg(test)]
use media_envelope::LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT;
use media_envelope::{
    decode_lan_media_envelope, encode_lan_media_envelope, lan_media_profile_id, LanMediaEnvelope,
    LAN_MEDIA_CODEC_AV1, LAN_MEDIA_CODEC_H264, LAN_MEDIA_CODEC_HEVC, LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
    LAN_MEDIA_PAYLOAD_PROBE_FRAME,
};
use media_error_policy::{
    should_log_media_receiver_decode_error, should_log_media_sender_frame_error,
    LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS,
    LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS,
};
#[cfg(target_os = "macos")]
use media_frame_capture::macos_lan_capture_stream_fps;
use media_frame_capture::{
    capture_source_kind_from_id, create_lan_frame_capture, selected_capture_source_id,
    LanSenderFrameCapture,
};
#[cfg(test)]
use media_frame_capture::{synthetic_capture_source, TEST_SYNTHETIC_CAPTURE_SOURCE_ID};
#[cfg(all(test, target_os = "macos"))]
use media_frame_capture::{MacosPumpedLanFrameCapture, MacosPumpedLanFrameState};
#[cfg(test)]
use media_frame_preparation::decoded_frame_to_rgb24;
#[cfg(test)]
use media_frame_preparation::window_h264_capture_dimensions;
use media_frame_preparation::{
    captured_frame_memory_path, h264_target_dimensions, prepare_frame_for_h264,
};
use media_keyframe_request::{
    decode_lan_keyframe_request_datagram, encode_lan_keyframe_request_datagram,
};
use media_ordering::LanMediaFrameOrderer;
#[cfg(test)]
use media_probe::decoded_video_probe_format;
#[cfg(test)]
use media_probe::{build_media_probe_frame, media_payload_bytes};
use media_probe::{decode_media_probe_frame, fnv1a64, fnv1a64_media_metadata};
#[cfg(test)]
use media_profile::default_media_profile;
#[cfg(test)]
use media_profile::format_media_profile;
#[cfg(target_os = "macos")]
use media_profile::normalize_lan_media_profile;
use media_profile::{
    apply_lan_media_profile_defaults, default_media_profile_negotiation,
    ensure_peer_can_receive_selected_media, ensure_peer_supports_requested_media,
    lan_runtime_media_profile, normalize_lan_codec_name, validate_media_profile,
};
use media_receiver::decode_lan_desktop_frame;
#[cfg(all(test, target_os = "macos"))]
use media_receiver_decoder::create_lan_video_decoder;
use media_receiver_decoder::{
    create_lan_receiver_decoder, create_lan_receiver_decoder_with_preference,
    try_decode_h264_keyframe_with_fallback,
};
#[cfg(all(test, target_os = "macos"))]
use media_receiver_decoder_candidates::preferred_lan_receiver_decoder_candidates_from_preference;
#[cfg(test)]
use media_receiver_decoder_candidates::{
    default_lan_receiver_decoder_candidates, prioritize_lan_receiver_decoder_candidates,
};
use media_receiver_runtime::{quic_media_v3_frame_to_legacy_frame, record_lan_decoded_frames};
#[cfg(test)]
use media_render_policy::{
    lan_media_payload_hash_for_mode, lan_media_payload_hash_mode_for_profile_with_override,
    lan_media_payload_hash_mode_from_env_value, lan_render_pacing_from_env_value,
    LanMediaPayloadHashMode,
};
#[cfg(all(test, any(windows, target_os = "macos")))]
use media_render_policy::{
    lan_render_cap_target_fps_for_profile, lan_render_policy_allows_service_pacing,
    lan_render_queue_capacity_for_policy, lan_render_queue_capacity_for_profile,
    LanRenderQueuePolicy,
};
#[cfg(all(test, any(windows, target_os = "macos")))]
use media_render_policy::{
    lan_render_pacing_enabled_for_profile, lan_render_pacing_render_start_delay,
    lan_render_pacing_target_fps, lan_render_pacing_target_fps_from_values,
    lan_render_queue_capacity_from_env_value, lan_render_queue_policy_for_profile_with_override,
    lan_render_queue_policy_from_env_value, render_pacing_frame_interval,
    render_pacing_precise_sleep_guard, render_profile_requests_high_resolution_timer,
    should_interrupt_render_pacing_sleep,
};
#[cfg(all(test, target_os = "macos"))]
use media_render_worker::upload_lan_render_frame;
#[cfg(all(test, any(windows, target_os = "macos")))]
use media_render_worker::{
    render_lan_frame_once, take_next_lan_render_frame_for_policy, wait_for_mutex_guard,
    LanRenderTaskOutcome,
};
#[cfg(target_os = "macos")]
use media_render_worker::{render_lan_h264_access_unit_frame, render_lan_hevc_access_unit_frame};
#[cfg(test)]
use media_sender::preferred_lan_h264_encoder_backends;
use media_sender::{create_lan_encoder, lan_sender_allows_h264_encoder_fallback, LanSenderEncoder};
use media_sender_telemetry::{
    decode_lan_sender_stats_datagram, send_lan_sender_stats_datagram, LanMediaTestImpairment,
    LanSenderDatagramFrameReport, LanSenderStatsTracker,
};
#[cfg(test)]
use media_sender_telemetry::{encode_lan_sender_stats_datagram, LanSenderStatsPayload};
#[cfg(target_os = "macos")]
use media_timing::media_frame_interval_for_fps;
use media_timing::{
    media_frame_interval, media_frame_interval_for_dynamic_decision, schedule_next_media_frame,
    sleep_until_media_frame, MediaTimerResolution,
};
#[cfg(test)]
use media_timing::{
    media_frame_precise_sleep_chunk, media_frame_precise_sleep_guard,
    media_profile_requests_high_resolution_timer,
};
use media_transport::{
    lan_datagram_frame_send_budget, lan_media_datagram_size, lan_media_reassembler_config,
    reliable_whole_frame_media_override, select_reliable_media_send_mode_for_profile,
    send_lan_media_datagram, send_lan_reliable_media_fragment,
    should_send_access_unit_as_reliable_frame, should_send_access_unit_reliably,
    use_best_effort_media_datagrams, LanDatagramSendOutcome, LanReliableMediaSendMode,
};
#[cfg(test)]
use media_transport::{
    reliable_whole_frame_media_override_from_env_value, select_reliable_media_send_mode,
};
use peer_format::{format_peer_capabilities, format_peer_transports, normalize_transport_kind};
use peer_lookup::{
    local_device_id, peer_control_addr_with_capture_source_capability,
    peer_control_addr_with_display_mode_capability,
    peer_control_addr_with_input_control_capability,
    peer_control_addr_with_remote_power_capability, session_remote_peer,
};
use peer_registry::{LanPeerRecord, LanPeerRegistry};
use protocol::{
    LanAnnouncement, LanDiscoveryPacket, LanMediaBootstrap, LanQuicBootstrap,
    DISCOVERY_PACKET_BUFFER_BYTES, LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT,
    LAN_DISPLAY_MODE_CONTROL_TRANSPORT, LAN_INPUT_CONTROL_TRANSPORT,
    LAN_MEDIA_PROFILE_CONTROL_TRANSPORT, LAN_MEDIA_PROTOCOL_VERSION,
    LAN_QUIC_MEDIA_PROFILE_TRANSPORT, LAN_QUIC_MEDIA_TRANSPORT, LAN_QUIC_MEDIA_V2_TRANSPORT,
    LAN_QUIC_MEDIA_V3_TRANSPORT, LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT,
    LAN_QUIC_RELIABLE_MEDIA_TRANSPORT, LAN_REMOTE_POWER_CONTROL_TRANSPORT, PROTOCOL_VERSION,
};
#[cfg(test)]
use protocol::{DISCOVERY_SAFE_UDP_PAYLOAD_BYTES, LAN_INPUT_CONTROL_CAPABILITY};
use remote_power::accept_lan_remote_device_power_action;
use runtime_flags::env_bool_override;
use service_identity::service_build_id;
#[cfg(test)]
use service_identity::{service_build_id_from_lookup, SERVICE_BUILD_ID_ENV};
use session_runtime::{
    mark_session_failed, negotiate_media_profile, selected_media_profile, session_allows_media,
};
#[cfg(target_os = "macos")]
use time_utils::duration_as_millis;
use time_utils::now_us;

const LAN_RELIABLE_WHOLE_FRAME_ENV: &str = "MRD_LAN_RELIABLE_WHOLE_FRAME";
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_ENV: &str = "MRD_LAN_CAPTURE_PUMP";
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_DRIVES_SENDER_ENV: &str = "MRD_LAN_CAPTURE_PUMP_DRIVES_SENDER";
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_REPEAT_LATEST_ENV: &str = "MRD_LAN_CAPTURE_PUMP_REPEAT_LATEST";
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_REPEAT_PACING_FPS_ENV: &str = "MRD_LAN_CAPTURE_PUMP_REPEAT_PACING_FPS";
const LAN_RENDER_PACING_ENV: &str = "MRD_LAN_RENDER_PACING";
const LAN_RENDER_MAX_FPS_ENV: &str = "MRD_LAN_RENDER_MAX_FPS";
const LAN_RENDER_QUEUE_CAPACITY_ENV: &str = "MRD_LAN_RENDER_QUEUE_CAPACITY";
const LAN_RENDER_QUEUE_POLICY_ENV: &str = "MRD_LAN_RENDER_QUEUE_POLICY";
const LAN_MEDIA_PAYLOAD_HASH_ENV: &str = "MRD_LAN_MEDIA_PAYLOAD_HASH";
#[cfg(windows)]
const D3D11_RENDER_PRESENT_BLOCKING_ENV: &str = "MRD_D3D11_RENDER_PRESENT_BLOCKING";
#[cfg(windows)]
const D3D11_RENDER_WAITABLE_OBJECT_ENV: &str = "MRD_D3D11_RENDER_WAITABLE_OBJECT";
const LAN_MEDIA_TARGET_WIDTH: u32 = 2560;
const LAN_MEDIA_TARGET_HEIGHT: u32 = 1600;
const LAN_MEDIA_TARGET_FPS: u32 = 165;
const LAN_MEDIA_MAX_FPS: u32 = 249;
const LAN_MEDIA_TARGET_BITRATE_MBPS: u32 = 120;
const LAN_QUIC_BEST_EFFORT_DATAGRAM_MAX_BITRATE_MBPS: u32 = 40;
const LAN_QUIC_FALLBACK_DATAGRAM_BYTES: usize = 1_200;
// Keep the default media fragment below common LAN/QUIC path MTU headroom.
// Larger datagrams reduce sender P95 but raised cross-device frame drop ratio.
const LAN_QUIC_LAN_HIGH_QUALITY_DATAGRAM_BYTES: usize = LAN_QUIC_FALLBACK_DATAGRAM_BYTES;
const LAN_QUIC_RELIABLE_WHOLE_FRAME_MIN_BITRATE_MBPS: u32 = 80;
const LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_BITRATE_MBPS: u32 = 100;
const LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_FPS: u32 = 120;
const LAN_QUIC_RELIABLE_MEDIA_MAX_BYTES: usize = 4 * 1024 * 1024;
const LAN_QUIC_RELIABLE_MEDIA_RETRY_DELAY: Duration = Duration::from_millis(10);
const LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_BITRATE_MBPS: u32 = 80;
const LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_FPS: u32 = 120;
const LAN_QUIC_DATAGRAM_SEND_BUDGET: Duration = Duration::from_millis(4);
const LAN_RENDER_PACING_PRECISE_SLEEP_MIN_FPS: u32 = 90;
const LAN_RENDER_PACING_PRECISE_SLEEP_GUARD: Duration = Duration::from_millis(2);
const LAN_RENDER_PACING_POLL_INTERVAL: Duration = Duration::from_millis(1);
const LAN_RENDER_PACING_PRESENT_LEAD: Duration = Duration::from_micros(250);
const LAN_RENDER_PACING_DEFAULT_MIN_FPS: u32 = 120;
const LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES: usize = 3;
const LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT: usize = 8;
const LAN_MEDIA_KEYFRAME_REQUEST_MIN_INTERVAL: Duration = Duration::from_millis(20);
const LAN_REMOTE_SESSION_ACK_TIMEOUT: Duration = Duration::from_secs(10);
const LAN_CONTROL_INPUT_ACK_TIMEOUT: Duration = Duration::from_millis(250);
const LAN_CONTROL_INPUT_REALTIME_ATTEMPTS: usize = 1;
const LAN_CONTROL_INPUT_RELIABLE_ATTEMPTS: usize = 3;
const LAN_CONTROL_INPUT_DEDUPE_WINDOW_MS: u64 = 10_000;
const LAN_CONTROL_INPUT_DEDUPE_CACHE_LIMIT: usize = 4096;
#[cfg(any(windows, target_os = "macos"))]
static LOCAL_RENDER_REFRESH_HZ: OnceLock<Option<u32>> = OnceLock::new();
static LAN_CONTROL_INPUT_EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_QUEUE_CAPACITY: usize = 2;
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_REPEAT_GRACE_MAX: Duration = Duration::from_millis(4);
#[cfg(target_os = "macos")]
const LAN_CAPTURE_PUMP_ERROR_BACKOFF: Duration = Duration::from_millis(5);
const LAN_MEDIA_REASSEMBLER_FRAME_TIMEOUT_MS: u64 = 1_500;
const LAN_MEDIA_REASSEMBLER_MAX_PENDING_FRAMES: usize = 256;
// Small bounded reorder window: absorbs normal QUIC stream/datagram jitter at 144-180 Hz
// without letting a genuinely missing frame add visible input latency.
const LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES: usize = 4;

#[derive(Debug)]
pub struct LanDiscoveryState {
    config: LanDiscoveryConfig,
    instance_id: String,
    running: AtomicBool,
    last_probe_ms: AtomicU64,
    peers: Mutex<LanPeerRegistry>,
    recent_control_inputs: Mutex<HashMap<LanControlInputDedupeKey, LanControlInputAckState>>,
    probe_requested: Notify,
    peer_changed: Notify,
}

impl LanDiscoveryState {
    pub fn new(config: LanDiscoveryConfig) -> Self {
        Self {
            config,
            instance_id: new_instance_id(),
            running: AtomicBool::new(false),
            last_probe_ms: AtomicU64::new(0),
            peers: Mutex::new(LanPeerRegistry::default()),
            recent_control_inputs: Mutex::new(HashMap::new()),
            probe_requested: Notify::new(),
            peer_changed: Notify::new(),
        }
    }

    pub fn discovery_port(&self) -> u16 {
        self.config.discovery_port
    }

    fn probe_targets(&self, discovery_port: u16) -> Vec<SocketAddr> {
        let mut targets = Vec::with_capacity(self.config.probe_endpoints.len() + 1);
        if self.config.broadcast_enabled {
            targets.push(SocketAddr::from(([255, 255, 255, 255], discovery_port)));
        }
        for endpoint in &self.config.probe_endpoints {
            if !targets.iter().any(|target| target == endpoint) {
                targets.push(*endpoint);
            }
        }
        targets
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn request_probe(&self) {
        self.probe_requested.notify_one();
    }

    pub async fn request_probe_and_wait(&self, wait: Duration) -> LanDiscoverySnapshot {
        let notified = self.peer_changed.notified();
        self.request_probe();
        let _ = timeout(wait, notified).await;
        self.snapshot().await
    }

    async fn upsert_peer(&self, announcement: LanAnnouncement, addr: SocketAddr) {
        if announcement.instance_id == self.instance_id {
            return;
        }

        let peer = LanPeerRecord {
            device_id: announcement.device_id,
            device_name: announcement.device_name,
            device_type: announcement.device_type,
            ip: addr.ip(),
            discovery_port: announcement.discovery_port,
            transports: announcement.transports,
            protocol_version: announcement.protocol_version,
            service_build_id: announcement.service_build_id,
            media_protocol_version: announcement.media_protocol_version,
            media_capabilities: announcement.media_capabilities,
            mac_address: announcement.mac_address,
            last_seen_ms: now_ms(),
        };

        self.peers.lock().await.upsert(peer);
        self.peer_changed.notify_one();
    }

    async fn prune_stale_peers(&self) {
        let ttl_ms = self.config.peer_ttl.as_millis() as u64;
        let now = now_ms();
        self.peers.lock().await.prune_stale(now, ttl_ms);
    }

    pub async fn snapshot(&self) -> LanDiscoverySnapshot {
        self.prune_stale_peers().await;
        let now = now_ms();
        let peers = self.peers.lock().await.snapshot(now);

        let last_probe = self.last_probe_ms.load(Ordering::Relaxed);
        LanDiscoverySnapshot {
            enabled: self.config.enabled,
            running: self.running.load(Ordering::Relaxed),
            discovery_port: self.config.discovery_port,
            instance_id: self.instance_id.clone(),
            last_probe_ms: if last_probe == 0 {
                None
            } else {
                Some(last_probe)
            },
            peers,
        }
    }

    pub async fn peer_control_addr(&self, device_id: &DeviceId) -> Option<SocketAddr> {
        self.prune_stale_peers().await;
        self.peers.lock().await.control_addr(device_id)
    }

    pub async fn peer_transports(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.prune_stale_peers().await;
        self.peers.lock().await.transports(device_id)
    }

    pub async fn peer_media_capabilities(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.prune_stale_peers().await;
        self.peers.lock().await.media_capabilities(device_id)
    }
}

impl Default for LanDiscoveryState {
    fn default() -> Self {
        Self::new(LanDiscoveryConfig::default())
    }
}

struct LanRemoteAcceptResult {
    accepted: bool,
    message: Option<String>,
    media: Option<LanMediaBootstrap>,
    media_profile: Option<MediaProfileNegotiation>,
}

type LanEncoderConfigKey = (
    usize,
    usize,
    u32,
    u32,
    LanAccessUnitCodec,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<u8>,
    Option<String>,
);

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl LanRemoteAcceptResult {
    fn rejected(message: impl Into<String>) -> Self {
        Self {
            accepted: false,
            message: Some(message.into()),
            media: None,
            media_profile: None,
        }
    }
}

pub async fn send_probe(
    socket: &UdpSocket,
    discovery_port: u16,
    state: &LanDiscoveryState,
) -> Result<()> {
    let packet = LanDiscoveryPacket::Probe {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: state.instance_id.clone(),
        device_id: None,
        timestamp_ms: now_ms(),
    };
    for target in state.probe_targets(discovery_port) {
        send_packet(socket, &packet, target).await?;
    }
    state.last_probe_ms.store(now_ms(), Ordering::Relaxed);
    Ok(())
}

pub async fn start_lan_discovery(app_state: Arc<AppState>) -> Result<()> {
    if !app_state.lan_discovery.config.enabled {
        return Ok(());
    }

    let port = app_state.lan_discovery.discovery_port();
    let socket = Arc::new(
        UdpSocket::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("failed to bind LAN discovery UDP port {port}"))?,
    );
    if app_state.lan_discovery.config.broadcast_enabled {
        socket
            .set_broadcast(true)
            .context("failed to enable LAN discovery UDP broadcast")?;
    }

    app_state
        .lan_discovery
        .running
        .store(true, Ordering::Relaxed);

    let receive_socket = socket.clone();
    let receive_state = app_state.clone();
    tokio::spawn(async move {
        receive_loop(receive_socket, receive_state).await;
    });

    let announce_socket = socket.clone();
    let announce_state = app_state.clone();
    tokio::spawn(async move {
        announce_loop(announce_socket, announce_state).await;
    });

    send_probe(&socket, port, &app_state.lan_discovery).await?;
    Ok(())
}

pub async fn request_lan_remote_session(
    app_state: &Arc<AppState>,
    target_device_id: &DeviceId,
    session_id: &SessionId,
    transport_kind: &str,
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(target_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", target_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(target_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", target_device_id.0))?;
    let peer_media_capabilities = app_state
        .lan_discovery
        .peer_media_capabilities(target_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", target_device_id.0))?;
    ensure_peer_supports_requested_media(
        target_device_id,
        transport_kind,
        &peer_transports,
        requested_profile.as_ref(),
        &peer_media_capabilities,
    )?;

    let (source_device_id, source_device_name) = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, name)| (id.0.clone(), name.clone()))
            .context("local device is not registered")?
    };

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN remote request UDP socket")?;
    let packet = LanDiscoveryPacket::RemoteSessionRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        source_device_name,
        transport_kind: transport_kind.to_string(),
        source_discovery_port: Some(app_state.lan_discovery.discovery_port()),
        source_media_capabilities: lan_media_capabilities(),
        requested_media_profile: requested_profile,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, ack_addr) = timeout(LAN_REMOTE_SESSION_ACK_TIMEOUT, socket.recv_from(&mut buffer))
        .await
        .context("LAN remote request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::RemoteSessionAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            media,
            media_profile,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let negotiation = media_profile.unwrap_or_else(default_media_profile_negotiation);
                app_state
                    .media_profiles
                    .lock()
                    .await
                    .set(session_id.clone(), negotiation.clone());
                app_state
                    .peer_media_capabilities
                    .lock()
                    .await
                    .set(session_id.clone(), peer_media_capabilities);
                {
                    let mut sessions = app_state.sessions.lock().await;
                    if sessions.get(session_id).is_none() {
                        sessions.insert(
                            session_id.clone(),
                            SessionSnapshot {
                                session_id: session_id.clone(),
                                transport: normalize_transport_kind(transport_kind),
                                source_device_id: None,
                                target_device_id: Some(target_device_id.clone()),
                                local_listen_addr: None,
                                local_server_name: None,
                                local_cert_der_b64: None,
                                remote_listen_addr: None,
                                remote_server_name: None,
                                remote_cert_der_b64: None,
                                lifecycle_state: SessionLifecycleState::Connecting,
                                last_error: None,
                                sender_active: false,
                                receiver_active: false,
                            },
                        );
                    }
                }
                start_lan_media_receiver(
                    app_state.clone(),
                    session_id.clone(),
                    transport_kind,
                    media,
                    ack_addr.ip(),
                )
                .await?;
                Ok(negotiation)
            } else {
                anyhow::bail!(
                    "LAN peer rejected remote session: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN remote session response"),
    }
}

pub async fn request_lan_media_profile_update(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    requested_profile: MediaProfile,
) -> Result<MediaProfileNegotiation> {
    validate_media_profile(&requested_profile)?;
    let peer_device_id = {
        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id.0))?;
        snapshot
            .target_device_id
            .clone()
            .or_else(|| snapshot.source_device_id.clone())
            .with_context(|| format!("session has no remote peer: {}", session_id.0))?
    };
    let target = app_state
        .lan_discovery
        .peer_control_addr(&peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(&peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_media_capabilities = app_state
        .lan_discovery
        .peer_media_capabilities(&peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    ensure_peer_supports_requested_media(
        &peer_device_id,
        "quic",
        &peer_transports,
        Some(&requested_profile),
        &peer_media_capabilities,
    )?;

    let source_device_id = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, _)| id.0.clone())
            .context("local device is not registered")?
    };

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN media profile update UDP socket")?;
    let packet = LanDiscoveryPacket::MediaProfileUpdate {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        requested_media_profile: requested_profile,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN media profile update timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::MediaProfileUpdateAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            media_profile,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let negotiation =
                    media_profile.context("LAN peer accepted profile update without result")?;
                app_state
                    .media_profiles
                    .lock()
                    .await
                    .set(session_id.clone(), negotiation.clone());
                Ok(negotiation)
            } else {
                anyhow::bail!(
                    "LAN peer rejected media profile update: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN media profile update response"),
    }
}

pub async fn request_lan_capture_sources(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target =
        peer_control_addr_with_capture_source_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN capture sources request UDP socket")?;
    let packet = LanDiscoveryPacket::CaptureSourcesRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        include_previews,
        limit,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
        .await
        .context("LAN capture sources request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::CaptureSourcesAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            sources,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                Ok(sources)
            } else {
                anyhow::bail!(
                    "LAN peer rejected capture source listing: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN capture sources response"),
    }
}

pub async fn request_lan_remote_device_power_action(
    app_state: &Arc<AppState>,
    target_device_id: &DeviceId,
    action: mrd_ipc::RemoteDevicePowerAction,
) -> Result<()> {
    let target =
        peer_control_addr_with_remote_power_capability(app_state, target_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN remote power UDP socket")?;
    let packet = LanDiscoveryPacket::RemoteDevicePowerAction {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        source_device_id,
        action: action.clone(),
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN remote power request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::RemoteDevicePowerActionAck {
            magic,
            app_id,
            device_id,
            action: ack_action,
            accepted,
            message,
            ..
        } if is_valid_discovery_packet(&magic, &app_id)
            && device_id == target_device_id.0
            && ack_action == action =>
        {
            if accepted {
                Ok(())
            } else {
                anyhow::bail!(
                    "LAN peer rejected remote power action: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN remote power response"),
    }
}

pub async fn request_lan_capture_source_select(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: String,
) -> Result<CaptureSourceSelection> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target =
        peer_control_addr_with_capture_source_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN capture source select UDP socket")?;
    let packet = LanDiscoveryPacket::CaptureSourceSelect {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        source_id,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN capture source select timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::CaptureSourceSelectAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            selection,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let selection =
                    selection.context("LAN peer accepted capture source without selection")?;
                close_existing_display_lan_receiver_sessions_for_target(
                    app_state,
                    session_id,
                    &selection.source,
                )
                .await;
                store_capture_source_selection(app_state, session_id, selection.clone()).await;
                Ok(selection)
            } else {
                anyhow::bail!(
                    "LAN peer rejected capture source select: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN capture source select response"),
    }
}

pub async fn request_lan_display_modes(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: Option<String>,
) -> Result<Vec<DisplayMode>> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target = peer_control_addr_with_display_mode_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN display modes request UDP socket")?;
    let packet = LanDiscoveryPacket::DisplayModesRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        source_id,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
        .await
        .context("LAN display modes request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::DisplayModesAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            modes,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                Ok(modes)
            } else {
                anyhow::bail!(
                    "LAN peer rejected display mode listing: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN display modes response"),
    }
}

pub async fn request_lan_display_mode_set(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    mode: DisplayMode,
    restore_after_session: bool,
) -> Result<DisplayModeChange> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target = peer_control_addr_with_display_mode_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN display mode set UDP socket")?;
    let packet = LanDiscoveryPacket::DisplayModeSet {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        mode,
        restore_after_session,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(4), socket.recv_from(&mut buffer))
        .await
        .context("LAN display mode set timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::DisplayModeSetAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            change,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let change = change.context("LAN peer accepted display mode set without change")?;
                record_remote_display_mode_change(app_state, session_id, &change).await;
                Ok(change)
            } else {
                anyhow::bail!(
                    "LAN peer rejected display mode set: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN display mode set response"),
    }
}

pub async fn request_lan_display_mode_restore(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<DisplayModeChange> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target = peer_control_addr_with_display_mode_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN display mode restore UDP socket")?;
    let packet = LanDiscoveryPacket::DisplayModeRestore {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(4), socket.recv_from(&mut buffer))
        .await
        .context("LAN display mode restore timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::DisplayModeRestoreAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            change,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let change =
                    change.context("LAN peer accepted display mode restore without change")?;
                clear_remote_display_mode_change(app_state, session_id).await;
                Ok(change)
            } else {
                anyhow::bail!(
                    "LAN peer rejected display mode restore: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN display mode restore response"),
    }
}

async fn record_remote_display_mode_change(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    change: &DisplayModeChange,
) {
    let Some(active) = change.active.as_ref() else {
        return;
    };
    let requested = change.requested.clone().unwrap_or_else(|| active.clone());
    app_state.display_modes.lock().await.record_change(
        session_id.clone(),
        requested,
        change.previous.clone(),
        active.clone(),
        change.restore_required,
    );
    reconcile_media_profile_to_display_mode(app_state, session_id, active).await;
}

async fn clear_remote_display_mode_change(app_state: &Arc<AppState>, session_id: &SessionId) {
    app_state.display_modes.lock().await.remove(session_id);
    let selection = app_state.capture_sources.lock().await.get(session_id);
    if let Some(selection) = selection {
        reconcile_media_profile_to_capture_source(app_state, session_id, &selection.source).await;
    }
}

async fn announce_loop(socket: Arc<UdpSocket>, app_state: Arc<AppState>) {
    let mut ticker = interval(app_state.lan_discovery.config.announce_interval);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Some(announcement) = build_announcement(&app_state).await {
                    let packet = LanDiscoveryPacket::Announce(announcement);
                    let targets = app_state
                        .lan_discovery
                        .probe_targets(app_state.lan_discovery.discovery_port());
                    for target in targets {
                        if let Err(error) = send_packet(&socket, &packet, target).await {
                            tracing::warn!(%error, %target, "failed to send LAN discovery announce");
                        } else {
                            app_state
                                .lan_discovery
                                .last_probe_ms
                                .store(now_ms(), Ordering::Relaxed);
                        }
                    }
                }
                app_state.lan_discovery.prune_stale_peers().await;
            }
            _ = app_state.lan_discovery.probe_requested.notified() => {
                if let Err(error) = send_probe(&socket, app_state.lan_discovery.discovery_port(), &app_state.lan_discovery).await {
                    tracing::warn!(%error, "failed to send LAN discovery probe");
                }
            }
        }
    }
}

async fn receive_loop(socket: Arc<UdpSocket>, app_state: Arc<AppState>) {
    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((len, addr)) => {
                if let Err(error) = handle_packet(&socket, &app_state, &buffer[..len], addr).await {
                    tracing::debug!(%error, %addr, "ignored LAN discovery packet");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "LAN discovery UDP receive failed");
            }
        }
    }
}

async fn handle_packet(
    socket: &UdpSocket,
    app_state: &Arc<AppState>,
    bytes: &[u8],
    addr: SocketAddr,
) -> Result<()> {
    let packet: LanDiscoveryPacket = serde_json::from_slice(bytes)?;
    match packet {
        LanDiscoveryPacket::Probe {
            magic,
            app_id,
            instance_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }
            if let Some(announcement) = build_announcement(app_state).await {
                send_packet(socket, &LanDiscoveryPacket::Announce(announcement), addr).await?;
            }
        }
        LanDiscoveryPacket::Announce(announcement) => {
            if is_valid_discovery_packet(&announcement.magic, &announcement.app_id) {
                app_state
                    .lan_discovery
                    .upsert_peer(announcement, addr)
                    .await;
            }
        }
        LanDiscoveryPacket::RemoteSessionRequest {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            transport_kind,
            source_media_capabilities,
            requested_media_profile,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let accept_result = accept_lan_remote_session(
                app_state,
                SessionId(session_id.clone()),
                DeviceId(source_device_id),
                transport_kind,
                source_media_capabilities,
                requested_media_profile,
            )
            .await;

            let ack = LanDiscoveryPacket::RemoteSessionAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.clone(),
                accepted: accept_result.accepted,
                message: accept_result.message,
                media: accept_result.media,
                media_profile: accept_result.media_profile,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::RemoteSessionAck { .. } => {}
        LanDiscoveryPacket::MediaProfileUpdate {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            requested_media_profile,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id = SessionId(session_id);
            let update_result =
                accept_lan_media_profile_update(app_state, &session_id, requested_media_profile)
                    .await;
            let (accepted, message, media_profile) = match update_result {
                Ok(negotiation) => (true, Some("updated".to_string()), Some(negotiation)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id.0,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN media profile update"
            );
            let ack = LanDiscoveryPacket::MediaProfileUpdateAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.0,
                accepted,
                message,
                media_profile,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::MediaProfileUpdateAck { .. } => {}
        LanDiscoveryPacket::CaptureSourcesRequest {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            include_previews,
            limit,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let sources_result = accept_lan_capture_sources_request(
                app_state,
                &SessionId(session_id.clone()),
                include_previews,
                limit,
            )
            .await;
            let (accepted, message, sources) = match sources_result {
                Ok(sources) => (true, Some("listed".to_string()), sources),
                Err(error) => (false, Some(error.to_string()), Vec::new()),
            };
            tracing::info!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN capture sources request"
            );
            let ack = capture_sources::fit_capture_sources_ack_packet(
                app_state.lan_discovery.instance_id.clone(),
                session_id,
                accepted,
                message,
                sources,
            );
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::CaptureSourcesAck { .. } => {}
        LanDiscoveryPacket::CaptureSourceSelect {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            source_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id = SessionId(session_id);
            let select_result =
                accept_lan_capture_source_select(app_state, &session_id, &source_id).await;
            let (accepted, message, selection) = match select_result {
                Ok(selection) => (true, Some("selected".to_string()), Some(selection)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id.0,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN capture source select"
            );
            let ack = LanDiscoveryPacket::CaptureSourceSelectAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.0,
                accepted,
                message,
                selection,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::CaptureSourceSelectAck { .. } => {}
        LanDiscoveryPacket::DisplayModesRequest {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            source_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id_value = SessionId(session_id.clone());
            let modes_result =
                accept_lan_display_modes_request(app_state, &session_id_value, source_id).await;
            let (accepted, message, modes) = match modes_result {
                Ok(modes) => (true, Some("listed".to_string()), modes),
                Err(error) => (false, Some(error.to_string()), Vec::new()),
            };
            tracing::info!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN display modes request"
            );
            let ack = LanDiscoveryPacket::DisplayModesAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id,
                accepted,
                message,
                modes,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::DisplayModesAck { .. } => {}
        LanDiscoveryPacket::DisplayModeSet {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            mode,
            restore_after_session,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id_value = SessionId(session_id.clone());
            let set_result = accept_lan_display_mode_set(
                app_state,
                &session_id_value,
                mode,
                restore_after_session,
            )
            .await;
            let (accepted, message, change) = match set_result {
                Ok(change) => (true, Some("changed".to_string()), Some(change)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN display mode set"
            );
            let ack = LanDiscoveryPacket::DisplayModeSetAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id,
                accepted,
                message,
                change,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::DisplayModeSetAck { .. } => {}
        LanDiscoveryPacket::DisplayModeRestore {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id_value = SessionId(session_id.clone());
            let restore_result =
                accept_lan_display_mode_restore(app_state, &session_id_value).await;
            let (accepted, message, change) = match restore_result {
                Ok(change) => (true, Some("restored".to_string()), Some(change)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN display mode restore"
            );
            let ack = LanDiscoveryPacket::DisplayModeRestoreAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id,
                accepted,
                message,
                change,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::DisplayModeRestoreAck { .. } => {}
        LanDiscoveryPacket::ControlInput {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            event_id,
            event,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id_value = SessionId(session_id.clone());
            let ack_state = accept_or_replay_lan_control_input(
                app_state,
                &session_id_value,
                &source_device_id,
                event_id,
                &event,
            )
            .await;
            tracing::debug!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted = ack_state.accepted,
                "handled LAN control input"
            );
            let ack = LanDiscoveryPacket::ControlInputAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id,
                event_id,
                accepted: ack_state.accepted,
                message: ack_state.message,
                lane: ack_state.lane,
                event_count: ack_state.event_count,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::ControlInputAck { .. } => {}
        LanDiscoveryPacket::RemoteDevicePowerAction {
            magic,
            app_id,
            instance_id,
            source_device_id,
            action,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let local_device_id = local_device_id(app_state).await?;
            let action_result = accept_lan_remote_device_power_action(&action);
            let (accepted, message) = match action_result {
                Ok(()) => (true, Some("accepted".to_string())),
                Err(error) => (false, Some(error.to_string())),
            };
            tracing::info!(
                source_device_id = %source_device_id,
                local_device_id = %local_device_id,
                accepted,
                "handled LAN remote device power action"
            );
            let ack = LanDiscoveryPacket::RemoteDevicePowerActionAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                device_id: local_device_id,
                action,
                accepted,
                message,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::RemoteDevicePowerActionAck { .. } => {}
    }

    Ok(())
}

async fn accept_lan_remote_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    source_device_id: DeviceId,
    transport_kind: String,
    source_media_capabilities: Vec<String>,
    requested_profile: Option<MediaProfile>,
) -> LanRemoteAcceptResult {
    let is_registered = {
        let devices = app_state.devices.lock().await;
        devices.is_registered()
    };
    if !is_registered {
        return LanRemoteAcceptResult::rejected("local device is not registered");
    }

    let transport = normalize_transport_kind(&transport_kind);
    if transport == "webrtc" {
        return LanRemoteAcceptResult::rejected(
            "LAN WebRTC media path is not implemented in mrd-service yet",
        );
    }
    if transport != "quic" {
        return LanRemoteAcceptResult::rejected(format!(
            "unsupported LAN media transport: {transport}"
        ));
    }
    let negotiation = match negotiate_media_profile(requested_profile) {
        Ok(value) => value,
        Err(error) => return LanRemoteAcceptResult::rejected(error.to_string()),
    };
    if let Err(error) = ensure_peer_can_receive_selected_media(
        source_device_id.0.as_str(),
        &negotiation.selected,
        &source_media_capabilities,
    ) {
        return LanRemoteAcceptResult::rejected(error.to_string());
    }
    let (listener, bootstrap) = match QuinnServerListener::bind("0.0.0.0:0").await {
        Ok(value) => value,
        Err(error) => {
            return LanRemoteAcceptResult::rejected(format!(
                "failed to start LAN QUIC media listener: {error}"
            ));
        }
    };

    let local_media = LanMediaBootstrap {
        transport_kind: "quic".to_string(),
        quic: Some(LanQuicBootstrap {
            listen_addr: bootstrap.listen_addr.to_string(),
            server_name: bootstrap.server_name.clone(),
            cert_der: bootstrap.cert_der.clone(),
        }),
    };
    app_state
        .media_profiles
        .lock()
        .await
        .set(session_id.clone(), negotiation.clone());
    app_state
        .peer_media_capabilities
        .lock()
        .await
        .set(session_id.clone(), source_media_capabilities);

    let local_listen_addr = bootstrap.listen_addr.to_string();
    let local_server_name = bootstrap.server_name.clone();
    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport,
                source_device_id: Some(source_device_id),
                target_device_id: None,
                local_listen_addr: Some(local_listen_addr),
                local_server_name: Some(local_server_name),
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Listening,
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );
    }
    #[cfg(test)]
    {
        app_state.capture_sources.lock().await.set(
            session_id.clone(),
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: synthetic_capture_source(),
                status: "selected".to_string(),
                reason: Some("test synthetic capture source".to_string()),
            },
        );
    }
    #[cfg(not(test))]
    {
        if let Ok(source) = crate::capture_source::default_capture_source(false) {
            app_state.capture_sources.lock().await.set(
                session_id.clone(),
                CaptureSourceSelection {
                    session_id: session_id.clone(),
                    source,
                    status: "selected".to_string(),
                    reason: Some("default fullscreen capture source".to_string()),
                },
            );
        }
    }
    spawn_quic_media_sender(app_state.clone(), session_id.clone(), listener).await;

    LanRemoteAcceptResult {
        accepted: true,
        message: Some("accepted".to_string()),
        media: Some(local_media),
        media_profile: Some(negotiation),
    }
}

async fn accept_lan_media_profile_update(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    requested_profile: MediaProfile,
) -> Result<MediaProfileNegotiation> {
    validate_media_profile(&requested_profile)?;
    {
        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id.0))?;
        if normalize_transport_kind(&snapshot.transport) != "quic" {
            anyhow::bail!(
                "media profile update is only supported for LAN QUIC sessions, got {}",
                snapshot.transport
            );
        }
        if snapshot.lifecycle_state.is_terminal() {
            anyhow::bail!(
                "media profile update rejected for {} session",
                snapshot.lifecycle_state
            );
        }
    }

    let mut negotiation = negotiate_media_profile(Some(requested_profile))?;
    let selected_source = app_state.capture_sources.lock().await.get(session_id);
    if let Some(selection) = selected_source.as_ref() {
        reconcile_negotiation_to_capture_source(&mut negotiation, &selection.source);
    }
    let active_display_mode = app_state.display_modes.lock().await.active_mode(session_id);
    if let Some(mode) = active_display_mode.as_ref() {
        reconcile_negotiation_to_display_mode(&mut negotiation, mode);
    }
    let peer_media_capabilities = app_state
        .peer_media_capabilities
        .lock()
        .await
        .get(session_id)
        .unwrap_or_default();
    ensure_peer_can_receive_selected_media(
        session_id.0.as_str(),
        &negotiation.selected,
        &peer_media_capabilities,
    )?;
    app_state
        .media_profiles
        .lock()
        .await
        .set(session_id.clone(), negotiation.clone());
    Ok(negotiation)
}

async fn accept_lan_capture_sources_request(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    ensure_active_sender_session(app_state, session_id, "capture source listing").await?;
    crate::capture_source::list_capture_sources(include_previews, limit)
}

async fn accept_lan_capture_source_select(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: &str,
) -> Result<CaptureSourceSelection> {
    let source = crate::capture_source::find_capture_source(source_id)?;
    accept_lan_capture_source_select_from_sources(app_state, session_id, source_id, vec![source])
        .await
}

async fn accept_lan_capture_source_select_from_sources(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: &str,
    sources: Vec<CaptureSource>,
) -> Result<CaptureSourceSelection> {
    ensure_active_sender_session(app_state, session_id, "capture source selection").await?;
    let source = sources
        .into_iter()
        .find(|source| source.id.eq_ignore_ascii_case(source_id))
        .with_context(|| format!("capture source not found: {source_id}"))?;
    let selection = CaptureSourceSelection {
        session_id: session_id.clone(),
        source,
        status: "selected".to_string(),
        reason: None,
    };
    close_existing_display_lan_sender_sessions_for_source(app_state, session_id, &selection.source)
        .await;
    store_capture_source_selection(app_state, session_id, selection.clone()).await;
    Ok(selection)
}

async fn accept_lan_display_modes_request(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: Option<String>,
) -> Result<Vec<DisplayMode>> {
    ensure_active_sender_session(app_state, session_id, "display mode listing").await?;
    crate::display_mode::list_display_modes(source_id.as_deref())
}

async fn accept_lan_display_mode_set(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    mode: DisplayMode,
    restore_after_session: bool,
) -> Result<DisplayModeChange> {
    ensure_active_sender_session(app_state, session_id, "display mode set").await?;
    let (previous, active) = crate::display_mode::set_display_mode(&mode)?;
    let change = app_state.display_modes.lock().await.record_change(
        session_id.clone(),
        mode,
        previous,
        active.clone(),
        restore_after_session,
    );
    reconcile_media_profile_to_display_mode(app_state, session_id, &active).await;
    Ok(change)
}

#[cfg(test)]
async fn accept_lan_display_mode_set_from_modes(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    requested: DisplayMode,
    restore_after_session: bool,
    modes: Vec<DisplayMode>,
) -> Result<DisplayModeChange> {
    ensure_active_sender_session(app_state, session_id, "display mode set").await?;
    let previous = modes.iter().find(|mode| mode.is_current).cloned();
    let active = crate::display_mode::choose_display_mode(
        &modes,
        requested.width,
        requested.height,
        requested.refresh_hz,
    )
    .with_context(|| {
        format!(
            "no display mode matches {}x{}@{}",
            requested.width, requested.height, requested.refresh_hz
        )
    })?;
    let change = app_state.display_modes.lock().await.record_change(
        session_id.clone(),
        requested,
        previous,
        active.clone(),
        restore_after_session,
    );
    reconcile_media_profile_to_display_mode(app_state, session_id, &active).await;
    Ok(change)
}

async fn accept_lan_display_mode_restore(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<DisplayModeChange> {
    ensure_active_sender_session(app_state, session_id, "display mode restore").await?;
    let restore_mode = app_state
        .display_modes
        .lock()
        .await
        .restore_mode(session_id)
        .with_context(|| format!("no temporary display mode recorded for {}", session_id.0))?;
    let (previous, active) = crate::display_mode::set_display_mode(&restore_mode)
        .unwrap_or_else(|_| (None, restore_mode.clone()));
    Ok(app_state.display_modes.lock().await.record_restore(
        session_id.clone(),
        previous.unwrap_or_else(|| restore_mode.clone()),
        active,
    ))
}

#[cfg(test)]
async fn accept_lan_display_mode_restore_with_mode(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    restored_mode: DisplayMode,
) -> Result<DisplayModeChange> {
    ensure_active_sender_session(app_state, session_id, "display mode restore").await?;
    let previous = app_state
        .display_modes
        .lock()
        .await
        .active_mode(session_id)
        .with_context(|| format!("no temporary display mode recorded for {}", session_id.0))?;
    Ok(app_state.display_modes.lock().await.record_restore(
        session_id.clone(),
        previous,
        restored_mode,
    ))
}

async fn store_capture_source_selection(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    selection: CaptureSourceSelection,
) {
    reconcile_media_profile_to_capture_source(app_state, session_id, &selection.source).await;
    app_state
        .capture_sources
        .lock()
        .await
        .set(session_id.clone(), selection);
}

async fn reconcile_media_profile_to_capture_source(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source: &CaptureSource,
) {
    let active_display_mode = app_state.display_modes.lock().await.active_mode(session_id);
    let mut profiles = app_state.media_profiles.lock().await;
    let mut negotiation = profiles
        .get(session_id)
        .unwrap_or_else(default_media_profile_negotiation);
    reconcile_negotiation_to_capture_source(&mut negotiation, source);
    if let Some(mode) = active_display_mode.as_ref() {
        reconcile_negotiation_to_display_mode(&mut negotiation, mode);
    }

    profiles.set(session_id.clone(), negotiation);
}

async fn reconcile_media_profile_to_display_mode(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    mode: &DisplayMode,
) {
    let mut profiles = app_state.media_profiles.lock().await;
    let mut negotiation = profiles
        .get(session_id)
        .unwrap_or_else(default_media_profile_negotiation);
    reconcile_negotiation_to_display_mode(&mut negotiation, mode);
    profiles.set(session_id.clone(), negotiation);
}

fn reconcile_negotiation_to_capture_source(
    negotiation: &mut MediaProfileNegotiation,
    source: &CaptureSource,
) {
    let capability_limited = negotiate_media_profile(Some(negotiation.requested.clone()))
        .unwrap_or_else(|_| MediaProfileNegotiation {
            requested: negotiation.requested.clone(),
            selected: negotiation.selected.clone(),
            status: negotiation.status.clone(),
            reason: negotiation.reason.clone(),
            selected_source_id: None,
            selected_width: None,
            selected_height: None,
            downgrade_reason: negotiation.downgrade_reason.clone(),
        });

    let mut selected = capability_limited.selected.clone();
    let mut downgrade_reason = capability_limited.downgrade_reason.clone();

    if source.width > 0 && source.height > 0 {
        let (selected_width, selected_height) = h264_target_dimensions(
            source.width as usize,
            source.height as usize,
            &capability_limited.selected,
        );
        if selected_width as u32 != selected.width || selected_height as u32 != selected.height {
            downgrade_reason =
                Some("matched selected capture source dimensions and aspect ratio".to_string());
        }
        selected.width = selected_width as u32;
        selected.height = selected_height as u32;
    }
    negotiation.selected = selected.clone();
    negotiation.selected_source_id = Some(source.id.clone());
    negotiation.selected_width = Some(negotiation.selected.width);
    negotiation.selected_height = Some(negotiation.selected.height);

    if negotiation.selected != negotiation.requested {
        negotiation.status = "downgraded".to_string();
        negotiation.reason = downgrade_reason
            .clone()
            .or(capability_limited.reason.clone());
        negotiation.downgrade_reason = downgrade_reason.or(capability_limited.downgrade_reason);
    } else {
        negotiation.status = "accepted".to_string();
        negotiation.reason = None;
        negotiation.downgrade_reason = None;
    }
}

fn reconcile_negotiation_to_display_mode(
    negotiation: &mut MediaProfileNegotiation,
    mode: &DisplayMode,
) {
    let mut selected = negotiation.selected.clone();
    let mut changed_for_display = false;

    if mode.width > 0 && mode.height > 0 {
        let (selected_width, selected_height) =
            h264_target_dimensions(mode.width as usize, mode.height as usize, &selected);
        if selected_width as u32 != selected.width || selected_height as u32 != selected.height {
            changed_for_display = true;
        }
        selected.width = selected_width as u32;
        selected.height = selected_height as u32;
    }

    if mode.refresh_hz > 0 && selected.fps > mode.refresh_hz {
        selected.fps = mode.refresh_hz;
        changed_for_display = true;
    }

    negotiation.selected = selected;
    negotiation.selected_width = Some(negotiation.selected.width);
    negotiation.selected_height = Some(negotiation.selected.height);

    if negotiation.selected != negotiation.requested {
        negotiation.status = "downgraded".to_string();
        if changed_for_display {
            let reason = "matched active display mode dimensions and refresh rate".to_string();
            negotiation.reason = Some(reason.clone());
            negotiation.downgrade_reason = Some(reason);
        }
    } else {
        negotiation.status = "accepted".to_string();
        negotiation.reason = None;
        negotiation.downgrade_reason = None;
    }
}

async fn ensure_active_sender_session(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    operation: &str,
) -> Result<()> {
    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    if normalize_transport_kind(&snapshot.transport) != "quic" {
        anyhow::bail!("{operation} is only supported for LAN QUIC sessions");
    }
    if !snapshot.sender_active {
        anyhow::bail!("{operation} requires an active target sender session");
    }
    if snapshot.lifecycle_state.is_terminal() {
        anyhow::bail!(
            "{operation} rejected for {} session",
            snapshot.lifecycle_state
        );
    }
    Ok(())
}

async fn build_announcement(app_state: &Arc<AppState>) -> Option<LanAnnouncement> {
    let (device_id, device_name) = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, name)| (id.0.clone(), name.clone()))
    }?;

    let mut transports = vec![
        "quic".to_string(),
        LAN_QUIC_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_V3_TRANSPORT.to_string(),
        LAN_QUIC_RELIABLE_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT.to_string(),
        LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
        LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT.to_string(),
        LAN_DISPLAY_MODE_CONTROL_TRANSPORT.to_string(),
        LAN_REMOTE_POWER_CONTROL_TRANSPORT.to_string(),
    ];
    let input_control_available = app_state.control_input().lock().await.is_available();
    if input_control_available {
        transports.push(LAN_INPUT_CONTROL_TRANSPORT.to_string());
    }

    Some(LanAnnouncement {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        device_id,
        device_name,
        device_type: "rdesk".to_string(),
        protocol_version: PROTOCOL_VERSION,
        discovery_port: app_state.lan_discovery.discovery_port(),
        transports,
        service_build_id: Some(service_build_id()),
        media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
        media_capabilities: lan_media_capabilities_with_input_control(input_control_available),
        mac_address: local_lan_announcement_mac_address(),
        timestamp_ms: now_ms(),
    })
}

async fn send_packet(
    socket: &UdpSocket,
    packet: &LanDiscoveryPacket,
    target: SocketAddr,
) -> Result<()> {
    let bytes = serde_json::to_vec(packet)?;
    socket.send_to(&bytes, target).await?;
    Ok(())
}

async fn start_lan_media_receiver(
    app_state: Arc<AppState>,
    session_id: SessionId,
    requested_transport: &str,
    media: Option<LanMediaBootstrap>,
    peer_ip: IpAddr,
) -> Result<()> {
    let requested_transport = normalize_transport_kind(requested_transport);
    if requested_transport == "webrtc" {
        anyhow::bail!("LAN WebRTC media path is not implemented in mrd-service yet");
    }
    if requested_transport != "quic" {
        anyhow::bail!("unsupported LAN media transport: {requested_transport}");
    }

    let media = media.context("LAN peer accepted session without media bootstrap")?;
    if normalize_transport_kind(&media.transport_kind) != "quic" {
        anyhow::bail!(
            "LAN peer returned unexpected media transport: {}",
            media.transport_kind
        );
    }
    let quic = media
        .quic
        .context("LAN peer accepted QUIC session without QUIC bootstrap")?;
    let bootstrap = quic_bootstrap_for_peer(quic.clone(), peer_ip)?;
    let endpoint = QuinnDatagramEndpoint::connect_client("0.0.0.0:0", &bootstrap)
        .await
        .context("failed to connect LAN QUIC media receiver")?;

    {
        let mut sessions = app_state.sessions.lock().await;
        if let Some(snapshot) = sessions.get(&session_id).cloned() {
            sessions.insert(
                session_id.clone(),
                SessionSnapshot {
                    remote_listen_addr: Some(bootstrap.listen_addr.to_string()),
                    remote_server_name: Some(bootstrap.server_name.clone()),
                    remote_cert_der_b64: None,
                    lifecycle_state: SessionLifecycleState::Streaming,
                    last_error: None,
                    receiver_active: true,
                    ..snapshot
                },
            );
        }
    }

    spawn_quic_media_receiver(app_state, session_id, endpoint).await;
    Ok(())
}

fn quic_bootstrap_for_peer(
    quic: LanQuicBootstrap,
    peer_ip: IpAddr,
) -> Result<QuinnServerBootstrap> {
    let listen_addr = quic
        .listen_addr
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid LAN QUIC listen addr: {}", quic.listen_addr))?;
    Ok(QuinnServerBootstrap {
        transport: "quic_quinn",
        listen_addr: SocketAddr::new(peer_ip, listen_addr.port()),
        server_name: quic.server_name,
        cert_der: quic.cert_der,
    })
}

async fn close_existing_display_lan_receiver_sessions_for_target(
    app_state: &Arc<AppState>,
    next_session_id: &SessionId,
    next_source: &CaptureSource,
) {
    if is_window_capture_source(next_source) {
        return;
    }
    let target_device_id = {
        let sessions = app_state.sessions.lock().await;
        sessions
            .get(next_session_id)
            .and_then(|snapshot| snapshot.target_device_id.clone())
    };
    let Some(target_device_id) = target_device_id else {
        return;
    };
    let stale_sessions = {
        let sessions = app_state.sessions.lock().await;
        let capture_sources = app_state.capture_sources.lock().await;
        sessions
            .list_all()
            .into_iter()
            .filter(|snapshot| {
                snapshot.session_id != *next_session_id
                    && snapshot.target_device_id.as_ref() == Some(&target_device_id)
                    && snapshot.receiver_active
                    && !capture_sources
                        .get(&snapshot.session_id)
                        .is_some_and(|selection| is_window_capture_source(&selection.source))
                    && !snapshot.lifecycle_state.is_terminal()
            })
            .map(|snapshot| snapshot.session_id)
            .collect::<Vec<_>>()
    };
    close_lan_media_sessions(
        app_state,
        stale_sessions,
        "replaced by newer display receiver session",
    )
    .await;
}

async fn close_existing_display_lan_sender_sessions_for_source(
    app_state: &Arc<AppState>,
    next_session_id: &SessionId,
    next_source: &CaptureSource,
) {
    if is_window_capture_source(next_source) {
        return;
    }
    let source_device_id = {
        let sessions = app_state.sessions.lock().await;
        sessions
            .get(next_session_id)
            .and_then(|snapshot| snapshot.source_device_id.clone())
    };
    let Some(source_device_id) = source_device_id else {
        return;
    };
    let stale_sessions = {
        let sessions = app_state.sessions.lock().await;
        let capture_sources = app_state.capture_sources.lock().await;
        sessions
            .list_all()
            .into_iter()
            .filter(|snapshot| {
                let selected_source = capture_sources.get(&snapshot.session_id);
                let selected_source_is_window = selected_source
                    .as_ref()
                    .is_some_and(|selection| is_window_capture_source(&selection.source));
                let same_controller = snapshot.source_device_id.as_ref() == Some(&source_device_id);
                let same_capture_source = selected_source.as_ref().is_some_and(|selection| {
                    selection.source.id.eq_ignore_ascii_case(&next_source.id)
                });
                snapshot.session_id != *next_session_id
                    && (same_controller || same_capture_source)
                    && snapshot.sender_active
                    && normalize_transport_kind(&snapshot.transport) == "quic"
                    && !selected_source_is_window
                    && !snapshot.lifecycle_state.is_terminal()
            })
            .map(|snapshot| snapshot.session_id)
            .collect::<Vec<_>>()
    };
    close_lan_media_sessions(
        app_state,
        stale_sessions,
        "replaced by newer display sender session from same source device",
    )
    .await;
}

fn is_window_capture_source(source: &CaptureSource) -> bool {
    source.source_kind.eq_ignore_ascii_case("window")
}

async fn close_lan_media_sessions(
    app_state: &Arc<AppState>,
    session_ids: Vec<SessionId>,
    reason: &'static str,
) {
    for session_id in session_ids {
        tracing::info!(session_id = %session_id.0, reason, "closing stale LAN media session");
        {
            let mut sessions = app_state.sessions.lock().await;
            if let Some(snapshot) = sessions.get(&session_id).cloned() {
                sessions.insert(
                    session_id.clone(),
                    SessionSnapshot {
                        lifecycle_state: SessionLifecycleState::Closed,
                        last_error: None,
                        sender_active: false,
                        receiver_active: false,
                        ..snapshot
                    },
                );
            }
        }
        app_state
            .media_tasks
            .lock()
            .await
            .abort_session(&session_id);
        app_state.media_profiles.lock().await.remove(&session_id);
        app_state.capture_sources.lock().await.remove(&session_id);
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .remove(&session_id);
        #[cfg(any(windows, target_os = "macos"))]
        app_state
            .media_surface_renderers
            .lock()
            .await
            .detach_session(&session_id);
        #[cfg(any(windows, target_os = "macos"))]
        app_state
            .media_render_queues
            .lock()
            .await
            .remove(&session_id);
        app_state.media_pipelines.lock().await.remove(&session_id);
    }
}

async fn spawn_quic_media_sender(
    app_state: Arc<AppState>,
    session_id: SessionId,
    listener: QuinnServerListener,
) {
    let registry = app_state.media_tasks.clone();
    let task_app_state = app_state;
    let failure_app_state = task_app_state.clone();
    let task_session_id = session_id.clone();
    let failure_session_id = task_session_id.clone();
    let handle = tokio::spawn(async move {
        let local_addr = listener.local_addr();
        let result = async move {
            let endpoint = listener
                .accept()
                .await
                .context("LAN QUIC media listener failed to accept receiver")?;
            send_quic_media_loop(task_app_state, endpoint, task_session_id).await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, %local_addr, "LAN QUIC media sender stopped");
            mark_session_failed(
                &failure_app_state,
                &failure_session_id,
                format!("LAN QUIC media sender failed: {error}"),
            )
            .await;
        }
    });
    let abort_handle = handle.abort_handle();
    drop(handle);
    registry.lock().await.register(session_id, abort_handle);
}

async fn send_quic_media_loop(
    app_state: Arc<AppState>,
    endpoint: QuinnDatagramEndpoint,
    session_id: SessionId,
) -> Result<()> {
    let negotiated_max_datagram_size = endpoint
        .max_datagram_size()
        .unwrap_or(LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
        .max(QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN.max(QUIC_AU_FRAGMENT_HEADER_LEN) + 1);
    let keyframe_requests = Arc::new(AtomicU64::new(0));
    let _control_reader = spawn_lan_media_control_reader(
        endpoint.clone(),
        session_id.clone(),
        keyframe_requests.clone(),
    );

    let mut frame_id = 1_u64;
    let mut active_capture_config: Option<LanCaptureConfigKey> = None;
    let mut capture: Option<LanSenderFrameCapture> = None;
    let mut encoder: Option<LanSenderEncoder> = None;
    let mut encoder_config: Option<LanEncoderConfigKey> = None;
    let mut pending_keyframe_request = false;
    let mut consecutive_frame_errors = 0_u32;
    let mut next_frame_at = Instant::now();
    let mut active_frame_interval = Duration::ZERO;
    let mut media_timer_resolution = MediaTimerResolution::default();
    let mut sender_stats = LanSenderStatsTracker::new(Instant::now());
    let mut test_impairment = LanMediaTestImpairment::from_env()?;
    let mut dynamic_window_fps_config: Option<DynamicWindowFpsConfigKey> = None;
    let mut dynamic_window_fps_policy: Option<DynamicWindowFpsPolicy> = None;
    let mut dynamic_window_fps_decision: Option<DynamicWindowFpsDecision> = None;
    let reliable_media_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_RELIABLE_MEDIA_TRANSPORT);
    let persistent_media_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT);
    let media_v3_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_MEDIA_V3_TRANSPORT);
    let high_quality_datagram_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_MEDIA_PROFILE_TRANSPORT);
    loop {
        if !session_allows_media(&app_state, &session_id).await {
            return Ok(());
        }
        let new_keyframe_requests = keyframe_requests.swap(0, Ordering::Relaxed);
        if new_keyframe_requests > 0 {
            pending_keyframe_request = true;
            sender_stats.record_ms("sender.keyframe_request", new_keyframe_requests as f64);
        }
        let profile = selected_media_profile(&app_state, &session_id).await;
        media_timer_resolution.update_for_profile(&profile);
        let reliable_media_send_mode = select_reliable_media_send_mode_for_profile(
            reliable_media_supported,
            persistent_media_supported,
            &profile,
        );
        let max_datagram_size = lan_media_datagram_size(
            negotiated_max_datagram_size,
            &profile,
            high_quality_datagram_supported,
        );
        let requested_codec = LanAccessUnitCodec::from_profile(&profile);
        let source_id = selected_capture_source_id(&app_state, &session_id).await?;
        let selected_config_key = lan_capture_config_key(&source_id, &profile);
        let selected_dynamic_window_fps_config_key =
            dynamic_window_fps_config_key(&source_id, &profile);
        let selected_source_is_window = is_windows_window_source_id(&source_id);
        let selected_window_capture_count = if selected_source_is_window {
            active_window_capture_count(&app_state).await
        } else {
            0
        };
        if selected_source_is_window {
            if dynamic_window_fps_config.as_ref() != Some(&selected_dynamic_window_fps_config_key) {
                let policy = DynamicWindowFpsPolicy::new(profile.fps);
                dynamic_window_fps_decision = Some(policy.current());
                dynamic_window_fps_policy = Some(policy);
                dynamic_window_fps_config = Some(selected_dynamic_window_fps_config_key);
            }
        } else {
            dynamic_window_fps_config = None;
            dynamic_window_fps_policy = None;
            dynamic_window_fps_decision = None;
        }
        let capture_repeats_latest_frame = capture
            .as_ref()
            .is_some_and(LanSenderFrameCapture::repeats_latest_frame);
        let frame_interval = if capture_repeats_latest_frame {
            macos_capture_pump_repeat_frame_interval(&profile)
        } else if selected_source_is_window {
            media_frame_interval_for_dynamic_decision(&profile, dynamic_window_fps_decision)
        } else {
            media_frame_interval(&profile)
        };
        if active_frame_interval != frame_interval {
            active_frame_interval = frame_interval;
            next_frame_at = Instant::now() + frame_interval;
        }
        let capture_drives_sender_pacing = capture
            .as_ref()
            .is_some_and(LanSenderFrameCapture::drives_sender_pacing);
        if capture_drives_sender_pacing {
            next_frame_at = Instant::now() + frame_interval;
        } else if let Some(delay_until) =
            schedule_next_media_frame(Instant::now(), &mut next_frame_at, frame_interval)
        {
            let pacing_started = Instant::now();
            sleep_until_media_frame(delay_until, &profile).await;
            sender_stats.record_elapsed("sender.pacing_wait", pacing_started);
        }
        let loop_started = Instant::now();

        if !lan_capture_config_matches(active_capture_config.as_ref(), &source_id, &profile) {
            match create_lan_frame_capture(&source_id, &profile).await {
                Ok(next_capture) => match LanSenderFrameCapture::new(next_capture, &profile) {
                    Ok(next_sender_capture) => {
                        capture = Some(next_sender_capture);
                        encoder = None;
                        encoder_config = None;
                        active_capture_config = Some(selected_config_key.clone());
                        consecutive_frame_errors = 0;
                        set_session_last_error(&app_state, &session_id, None).await;
                    }
                    Err(error) => {
                        capture = None;
                        encoder = None;
                        encoder_config = None;
                        active_capture_config = None;
                        update_dynamic_window_fps_decision(
                            &mut dynamic_window_fps_policy,
                            &mut dynamic_window_fps_decision,
                            false,
                            false,
                            selected_window_capture_count,
                        );
                        handle_media_sender_frame_error(
                            &app_state,
                            &session_id,
                            &source_id,
                            &mut consecutive_frame_errors,
                            format_capture_source_failure(
                                &source_id,
                                format!("failed to initialize LAN capture sender: {error:#}"),
                                is_windows_window_source_id,
                            ),
                            selected_source_is_window,
                        )
                        .await?;
                        continue;
                    }
                },
                Err(error) => {
                    capture = None;
                    encoder = None;
                    encoder_config = None;
                    active_capture_config = None;
                    update_dynamic_window_fps_decision(
                        &mut dynamic_window_fps_policy,
                        &mut dynamic_window_fps_decision,
                        false,
                        false,
                        selected_window_capture_count,
                    );
                    handle_media_sender_frame_error(
                        &app_state,
                        &session_id,
                        &source_id,
                        &mut consecutive_frame_errors,
                        format_capture_source_failure(
                            &source_id,
                            format!("failed to create LAN capture source: {error:#}"),
                            is_windows_window_source_id,
                        ),
                        selected_source_is_window,
                    )
                    .await?;
                    continue;
                }
            }
        }

        let capture_started = Instant::now();
        let raw_frame_result = capture
            .as_mut()
            .context("LAN media capture was not initialized")
            .and_then(|capture| {
                capture
                    .capture_frame()
                    .context("failed to capture LAN desktop frame")
            });
        sender_stats.record_elapsed("sender.capture", capture_started);
        let raw_capture = match raw_frame_result {
            Ok(capture) => capture,
            Err(error) => {
                let error_source_id = active_capture_config
                    .as_ref()
                    .map(|config| config.source_id.as_str())
                    .unwrap_or("<unknown>")
                    .to_string();
                if selected_source_is_window && is_winrt_window_capture_no_frame_timeout(&error) {
                    if let Some(policy) = dynamic_window_fps_policy.as_mut() {
                        dynamic_window_fps_decision =
                            Some(policy.update(window_dynamic_fps_input_for_capture_error(
                                &error,
                                selected_window_capture_count,
                            )));
                    }
                    continue;
                }
                capture = None;
                encoder = None;
                encoder_config = None;
                active_capture_config = None;
                update_dynamic_window_fps_decision(
                    &mut dynamic_window_fps_policy,
                    &mut dynamic_window_fps_decision,
                    false,
                    false,
                    selected_window_capture_count,
                );
                handle_media_sender_frame_error(
                    &app_state,
                    &session_id,
                    &error_source_id,
                    &mut consecutive_frame_errors,
                    format_capture_source_failure(
                        &error_source_id,
                        format!("{error:#}"),
                        is_windows_window_source_id,
                    ),
                    is_windows_window_source_id(&error_source_id),
                )
                .await?;
                continue;
            }
        };
        if raw_capture.repeated_latest_frame {
            sender_stats.record_repeated_latest_frame();
        }
        let raw_frame = raw_capture.frame;
        sender_stats.record_captured_frame(&raw_frame);
        let capture_memory_path = captured_frame_memory_path(&raw_frame).to_string();
        if let Some(policy) = dynamic_window_fps_policy.as_mut() {
            dynamic_window_fps_decision = Some(policy.update(
                window_dynamic_fps_input_for_captured_frame(selected_window_capture_count),
            ));
        }
        let prepare_started = Instant::now();
        let frame_result = prepare_frame_for_h264(raw_frame, &profile);
        sender_stats.record_elapsed("sender.prepare", prepare_started);
        let frame = match frame_result {
            Ok(frame) => frame,
            Err(error) => {
                handle_media_sender_frame_error(
                    &app_state,
                    &session_id,
                    active_capture_config
                        .as_ref()
                        .map(|config| config.source_id.as_str())
                        .unwrap_or("<unknown>"),
                    &mut consecutive_frame_errors,
                    format!("failed to prepare captured frame for H.264: {error:#}"),
                    false,
                )
                .await?;
                continue;
            }
        };
        let expected_encoder_config = (
            frame.width,
            frame.height,
            profile.fps,
            profile.bitrate_mbps,
            requested_codec,
            profile.color_mode.clone(),
            profile.color_pipeline.clone(),
            profile.codec_profile.clone(),
            profile.bit_depth,
            profile.pixel_format.clone(),
        );
        if encoder_config.as_ref() != Some(&expected_encoder_config) {
            let peer_media_capabilities = app_state
                .peer_media_capabilities
                .lock()
                .await
                .get(&session_id)
                .unwrap_or_default();
            let allow_h264_fallback =
                lan_sender_allows_h264_encoder_fallback(requested_codec, &peer_media_capabilities);
            let encoder_create_started = Instant::now();
            match create_lan_encoder(
                requested_codec,
                frame.width,
                frame.height,
                profile.fps,
                profile.bitrate_mbps.saturating_mul(1_000_000).max(1),
                &profile,
                allow_h264_fallback,
            )
            .context("failed to create LAN media encoder")
            {
                Ok(next_encoder) => {
                    sender_stats.record_elapsed("sender.encoder_create", encoder_create_started);
                    let runtime_profile = lan_runtime_media_profile(&profile, next_encoder.codec);
                    let fallback_reason = (next_encoder.codec != requested_codec).then(|| {
                        format!(
                            "{} unavailable; fell back to {} via {}",
                            requested_codec.display_name(),
                            next_encoder.codec.display_name(),
                            next_encoder.backend
                        )
                    });
                    {
                        let mut pipelines = app_state.media_pipelines.lock().await;
                        pipelines.set_active_encoder(session_id.clone(), next_encoder.backend);
                        pipelines.set_active_media_profile(session_id.clone(), &runtime_profile);
                        pipelines.set_codec_fallback_reason(session_id.clone(), fallback_reason);
                    }
                    encoder = Some(next_encoder);
                    encoder_config = Some(expected_encoder_config);
                }
                Err(error) => {
                    sender_stats.record_elapsed("sender.encoder_create", encoder_create_started);
                    encoder = None;
                    encoder_config = None;
                    handle_media_sender_frame_error(
                        &app_state,
                        &session_id,
                        active_capture_config
                            .as_ref()
                            .map(|config| config.source_id.as_str())
                            .unwrap_or("<unknown>"),
                        &mut consecutive_frame_errors,
                        format!("{error:#}"),
                        false,
                    )
                    .await?;
                    continue;
                }
            }
        }

        if pending_keyframe_request {
            if let Some(encoder) = encoder.as_mut() {
                encoder.encoder.request_keyframe();
                pending_keyframe_request = false;
            }
        }

        let encode_started = Instant::now();
        let encode_result = encoder
            .as_mut()
            .context("LAN media encoder was not initialized")
            .and_then(|encoder| {
                encoder
                    .encoder
                    .encode(&frame)
                    .context("failed to encode LAN desktop frame")
            });
        sender_stats.record_elapsed("sender.encode", encode_started);
        let access_units = match encode_result {
            Ok(access_units) => access_units,
            Err(error) => {
                encoder = None;
                encoder_config = None;
                handle_media_sender_frame_error(
                    &app_state,
                    &session_id,
                    active_capture_config
                        .as_ref()
                        .map(|config| config.source_id.as_str())
                        .unwrap_or("<unknown>"),
                    &mut consecutive_frame_errors,
                    format!("{error:#}"),
                    false,
                )
                .await?;
                continue;
            }
        };

        for access_unit in access_units {
            let runtime_codec = encoder
                .as_ref()
                .map(|encoder| encoder.codec)
                .unwrap_or(requested_codec);
            let runtime_profile = lan_runtime_media_profile(&profile, runtime_codec);
            let is_keyframe = match runtime_codec {
                LanAccessUnitCodec::H264 => {
                    h264_access_unit_is_keyframe(access_unit.is_keyframe, &access_unit.bytes)
                }
                LanAccessUnitCodec::Hevc | LanAccessUnitCodec::Av1 => access_unit.is_keyframe,
            };
            sender_stats.record_encoded_access_unit(access_unit.bytes.len(), is_keyframe);
            let fragment_started = Instant::now();
            let fragments = if media_v3_supported {
                match fragment_media_payload_v3(
                    QuicMediaPayloadType::AccessUnit,
                    runtime_codec.quic_codec(),
                    lan_media_profile_id(&profile),
                    frame_id as u32,
                    access_unit.timestamp_us,
                    is_keyframe,
                    &access_unit.bytes,
                    test_impairment.effective_datagram_size(max_datagram_size),
                )
                .context("failed to fragment LAN QUIC media v3 frame")
                {
                    Ok(fragments) => fragments,
                    Err(error) => {
                        handle_media_sender_frame_error(
                            &app_state,
                            &session_id,
                            active_capture_config
                                .as_ref()
                                .map(|config| config.source_id.as_str())
                                .unwrap_or("<unknown>"),
                            &mut consecutive_frame_errors,
                            format!("{error:#}"),
                            true,
                        )
                        .await?;
                        frame_id = frame_id.wrapping_add(1).max(1);
                        continue;
                    }
                }
            } else {
                let media_payload = match encode_lan_media_envelope(LanMediaEnvelope {
                    payload_type: LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
                    codec: runtime_codec.envelope_codec(),
                    sequence: frame_id,
                    timestamp_us: access_unit.timestamp_us,
                    profile: runtime_profile.clone(),
                    payload: access_unit.bytes.clone(),
                }) {
                    Ok(media_payload) => media_payload,
                    Err(error) => {
                        handle_media_sender_frame_error(
                            &app_state,
                            &session_id,
                            active_capture_config
                                .as_ref()
                                .map(|config| config.source_id.as_str())
                                .unwrap_or("<unknown>"),
                            &mut consecutive_frame_errors,
                            format!("{error:#}"),
                            true,
                        )
                        .await?;
                        frame_id = frame_id.wrapping_add(1).max(1);
                        continue;
                    }
                };
                match fragment_access_unit(
                    frame_id as u32,
                    access_unit.timestamp_us,
                    is_keyframe,
                    &media_payload,
                    test_impairment.effective_datagram_size(max_datagram_size),
                )
                .context("failed to fragment LAN QUIC media v2 frame")
                {
                    Ok(fragments) => fragments,
                    Err(error) => {
                        handle_media_sender_frame_error(
                            &app_state,
                            &session_id,
                            active_capture_config
                                .as_ref()
                                .map(|config| config.source_id.as_str())
                                .unwrap_or("<unknown>"),
                            &mut consecutive_frame_errors,
                            format!("{error:#}"),
                            true,
                        )
                        .await?;
                        frame_id = frame_id.wrapping_add(1).max(1);
                        continue;
                    }
                }
            };
            sender_stats.record_elapsed("sender.fragment", fragment_started);
            test_impairment.record_mtu_fragmentation(max_datagram_size);

            let reliable_media_enabled =
                reliable_media_send_mode != LanReliableMediaSendMode::Disabled;
            let send_as_reliable_frame = should_send_access_unit_as_reliable_frame(
                reliable_media_enabled,
                media_v3_supported,
                fragments.len(),
                &profile,
                reliable_whole_frame_media_override(),
            );
            let reliable_fragments = if send_as_reliable_frame {
                let reliable_fragment_started = Instant::now();
                let result = fragment_media_payload_v3(
                    QuicMediaPayloadType::AccessUnit,
                    runtime_codec.quic_codec(),
                    lan_media_profile_id(&profile),
                    frame_id as u32,
                    access_unit.timestamp_us,
                    is_keyframe,
                    &access_unit.bytes,
                    LAN_QUIC_RELIABLE_MEDIA_MAX_BYTES,
                )
                .context("failed to fragment LAN QUIC reliable media v3 frame");
                sender_stats.record_elapsed("sender.reliable_fragment", reliable_fragment_started);
                match result {
                    Ok(reliable_fragments) => Some(reliable_fragments),
                    Err(error) => {
                        handle_media_sender_frame_error(
                            &app_state,
                            &session_id,
                            active_capture_config
                                .as_ref()
                                .map(|config| config.source_id.as_str())
                                .unwrap_or("<unknown>"),
                            &mut consecutive_frame_errors,
                            format!("{error:#}"),
                            true,
                        )
                        .await?;
                        frame_id = frame_id.wrapping_add(1).max(1);
                        continue;
                    }
                }
            } else if should_send_access_unit_reliably(
                reliable_media_enabled,
                is_keyframe,
                access_unit.bytes.len(),
                max_datagram_size,
            ) {
                Some(fragments.clone())
            } else {
                None
            };

            let mut send_result = Ok(());
            if send_as_reliable_frame {
                let reliable_send_started = Instant::now();
                let mut reliable_fragments_sent = 0_u64;
                for reliable_fragment in reliable_fragments.unwrap_or_default() {
                    let delay = test_impairment.next_delay();
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let send = send_lan_reliable_media_fragment(
                        &endpoint,
                        reliable_media_send_mode,
                        reliable_fragment,
                    )
                    .await;
                    if let Err(error) = send {
                        send_result = Err(error).with_context(|| {
                            format!("failed to send LAN QUIC reliable media frame {}", frame_id)
                        });
                        break;
                    }
                    reliable_fragments_sent = reliable_fragments_sent.saturating_add(1);
                }
                sender_stats.record_elapsed("sender.send_reliable", reliable_send_started);
                sender_stats.record_reliable_frame(
                    reliable_fragments_sent,
                    reliable_fragments_sent > 0 && send_result.is_ok(),
                );
            } else {
                let best_effort_datagrams = use_best_effort_media_datagrams(&profile);
                let datagram_send_started = Instant::now();
                let datagram_send_deadline =
                    lan_datagram_frame_send_budget(&profile, reliable_media_enabled)
                        .and_then(|budget| datagram_send_started.checked_add(budget));
                let mut datagram_report = LanSenderDatagramFrameReport {
                    fragments_attempted: fragments.len() as u64,
                    ..LanSenderDatagramFrameReport::default()
                };
                let mut skip_unsent_datagram_frame = false;
                for (fragment_index, fragment) in fragments.iter().enumerate() {
                    let frame_send_started =
                        datagram_report.fragments_sent > 0 || datagram_report.fragments_delayed > 0;
                    let remaining_send_budget = if frame_send_started {
                        None
                    } else {
                        datagram_send_deadline
                            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                    };
                    if !frame_send_started
                        && remaining_send_budget.is_some_and(|remaining| remaining.is_zero())
                    {
                        datagram_report.fragments_dropped_for_budget = datagram_report
                            .fragments_dropped_for_budget
                            .saturating_add((fragments.len() - fragment_index) as u64);
                        datagram_report.cut_short_for_budget = true;
                        skip_unsent_datagram_frame = true;
                        break;
                    }
                    let decision = test_impairment.next_datagram_decision();
                    if decision.drop_datagram {
                        datagram_report.fragments_dropped_by_impairment = datagram_report
                            .fragments_dropped_by_impairment
                            .saturating_add(1);
                        continue;
                    }
                    if !decision.delay.is_zero() {
                        datagram_report.fragments_delayed =
                            datagram_report.fragments_delayed.saturating_add(1);
                        let delayed_endpoint = endpoint.clone();
                        let delayed_session_id = session_id.clone();
                        let delayed_frame_id = frame_id;
                        let delayed_fragment = fragment.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(decision.delay).await;
                            let send_result = send_lan_media_datagram(
                                &delayed_endpoint,
                                delayed_fragment,
                                !best_effort_datagrams,
                                None,
                            )
                            .await;
                            if let Err(error) = send_result {
                                tracing::debug!(
                                    %error,
                                    session_id = %delayed_session_id.0,
                                    frame_id = delayed_frame_id,
                                    "delayed LAN QUIC media datagram send failed"
                                );
                            }
                        });
                        continue;
                    }
                    let send_fragment_result = send_lan_media_datagram(
                        &endpoint,
                        fragment.clone(),
                        !best_effort_datagrams,
                        remaining_send_budget,
                    )
                    .await;
                    match send_fragment_result {
                        Ok(LanDatagramSendOutcome::Sent) => {
                            datagram_report.fragments_sent =
                                datagram_report.fragments_sent.saturating_add(1);
                        }
                        Ok(LanDatagramSendOutcome::DroppedForCapacity) => {
                            datagram_report.fragments_dropped_for_capacity = datagram_report
                                .fragments_dropped_for_capacity
                                .saturating_add((fragments.len() - fragment_index) as u64);
                            datagram_report.cut_short_for_capacity = true;
                            if !frame_send_started {
                                skip_unsent_datagram_frame = true;
                            }
                            break;
                        }
                        Err(error) => {
                            send_result = Err(error).with_context(|| {
                                format!("failed to send LAN QUIC media frame {}", frame_id)
                            });
                            break;
                        }
                    }
                }
                sender_stats.record_datagram_frame(datagram_report);
                sender_stats.record_elapsed("sender.send_datagram", datagram_send_started);

                if skip_unsent_datagram_frame {
                    continue;
                }

                if send_result.is_ok() {
                    if let Some(reliable_fragments) = reliable_fragments {
                        let reliable_endpoint = endpoint.clone();
                        let reliable_session_id = session_id.clone();
                        let reliable_frame_id = frame_id;
                        let reliable_send_mode = reliable_media_send_mode;
                        tokio::spawn(async move {
                            for reliable_fragment in reliable_fragments {
                                if let Err(error) = send_lan_reliable_media_fragment(
                                    &reliable_endpoint,
                                    reliable_send_mode,
                                    reliable_fragment,
                                )
                                .await
                                {
                                    tracing::warn!(
                                        %error,
                                        session_id = %reliable_session_id.0,
                                        frame_id = reliable_frame_id,
                                        "LAN QUIC reliable keyframe fragment send failed"
                                    );
                                    break;
                                }
                            }
                        });
                    }
                }
            }

            if let Err(error) = send_result {
                handle_media_sender_frame_error(
                    &app_state,
                    &session_id,
                    active_capture_config
                        .as_ref()
                        .map(|config| config.source_id.as_str())
                        .unwrap_or("<unknown>"),
                    &mut consecutive_frame_errors,
                    format!("{error:#}"),
                    true,
                )
                .await?;
                frame_id = frame_id.wrapping_add(1).max(1);
                continue;
            }
            sender_stats.frame_completed();
            if let Some(stats_payload) = sender_stats.take_payload(
                Instant::now(),
                frame_id,
                active_capture_config
                    .as_ref()
                    .map(|config| config.source_id.clone()),
                active_capture_config
                    .as_ref()
                    .and_then(|config| capture_source_kind_from_id(&config.source_id)),
                Some(capture_memory_path.clone()),
                &profile,
                dynamic_window_fps_decision,
                test_impairment.snapshot(),
            ) {
                {
                    let mut pipelines = app_state.media_pipelines.lock().await;
                    pipelines.set_stage_metrics(session_id.clone(), stats_payload.metrics.clone());
                    pipelines.set_test_impairment(
                        session_id.clone(),
                        stats_payload.test_impairment.clone(),
                    );
                    pipelines.set_sender_transport(
                        session_id.clone(),
                        stats_payload.sender_transport.clone(),
                    );
                }
                let stats_send_started = Instant::now();
                if let Err(error) =
                    send_lan_sender_stats_datagram(&endpoint, max_datagram_size, &stats_payload)
                {
                    tracing::debug!(
                        %error,
                        session_id = %session_id.0,
                        frame_id,
                        "LAN sender stats datagram was dropped"
                    );
                }
                sender_stats.record_elapsed("sender.stats_send", stats_send_started);
            }
            frame_id = frame_id.wrapping_add(1).max(1);
        }
        sender_stats.record_elapsed("sender.loop", loop_started);

        if consecutive_frame_errors > 0 {
            consecutive_frame_errors = 0;
            set_session_last_error(&app_state, &session_id, None).await;
        }
    }
}

async fn handle_media_sender_frame_error(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: &str,
    consecutive_frame_errors: &mut u32,
    message: String,
    fail_after_limit: bool,
) -> Result<()> {
    *consecutive_frame_errors = consecutive_frame_errors.saturating_add(1);
    let decorated_message = if fail_after_limit {
        format!(
            "LAN media sender transient frame error {}/{} for source '{}': {}",
            *consecutive_frame_errors,
            LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS,
            source_id,
            message
        )
    } else {
        format!(
            "LAN media sender recoverable frame error {} for source '{}': {}",
            *consecutive_frame_errors, source_id, message
        )
    };

    if should_log_media_sender_frame_error(*consecutive_frame_errors) {
        tracing::warn!(
            session_id = %session_id.0,
            source_id,
            consecutive_frame_errors = *consecutive_frame_errors,
            error = %message,
            "LAN media sender skipped a frame"
        );
    }
    set_session_last_error(app_state, session_id, Some(decorated_message.clone())).await;

    if fail_after_limit
        && *consecutive_frame_errors >= LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS
    {
        anyhow::bail!("{decorated_message}");
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn lan_capture_pump_enabled() -> bool {
    env_bool_override(std::env::var(LAN_CAPTURE_PUMP_ENV).ok().as_deref()).unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn lan_capture_pump_drives_sender() -> bool {
    env_bool_override(
        std::env::var(LAN_CAPTURE_PUMP_DRIVES_SENDER_ENV)
            .ok()
            .as_deref(),
    )
    .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn lan_capture_pump_repeat_latest() -> bool {
    lan_capture_pump_repeat_latest_from_env_value(
        std::env::var(LAN_CAPTURE_PUMP_REPEAT_LATEST_ENV)
            .ok()
            .as_deref(),
    )
}

#[cfg(target_os = "macos")]
fn lan_capture_pump_repeat_latest_from_env_value(value: Option<&str>) -> bool {
    // ScreenCaptureKit can deliver below the requested cadence when the source display
    // refreshes slowly or its contents are mostly idle. Prefer a fresh frame during the
    // short grace window, then repeat the latest retained CVPixelBuffer so the transport
    // and renderer still keep the negotiated frame cadence.
    env_bool_override(value).unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn macos_capture_pump_repeat_pacing_fps(profile: &MediaProfile) -> u32 {
    std::env::var(LAN_CAPTURE_PUMP_REPEAT_PACING_FPS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|fps| *fps > 0)
        .unwrap_or_else(|| profile.fps.max(1))
        .clamp(profile.fps.max(1), 240)
}

#[cfg(target_os = "macos")]
fn macos_capture_pump_repeat_frame_interval(profile: &MediaProfile) -> Duration {
    media_frame_interval_for_fps(macos_capture_pump_repeat_pacing_fps(profile))
}

#[cfg(target_os = "macos")]
fn macos_capture_pump_repeat_grace_timeout(profile: &MediaProfile) -> Duration {
    (media_frame_interval_for_fps(macos_lan_capture_stream_fps(profile)) / 2)
        .min(LAN_CAPTURE_PUMP_REPEAT_GRACE_MAX)
}

#[cfg(not(target_os = "macos"))]
fn macos_capture_pump_repeat_frame_interval(profile: &MediaProfile) -> Duration {
    media_frame_interval(profile)
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_enabled() -> bool {
    media_receiver::compressed_proxy_enabled()
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_enabled_for_profile(profile: &MediaProfile) -> bool {
    media_receiver::compressed_proxy_enabled_for_profile(profile)
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_local_render_fps_cap() -> Option<u32> {
    lan_local_render_refresh_hz()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn lan_local_render_fps_cap() -> Option<u32> {
    None
}

#[cfg(windows)]
fn lan_local_render_refresh_hz() -> Option<u32> {
    if let Some(refresh_hz) = std::env::var(LAN_RENDER_MAX_FPS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
    {
        return Some(refresh_hz);
    }

    *LOCAL_RENDER_REFRESH_HZ.get_or_init(crate::display_mode::highest_current_refresh_hz)
}

#[cfg(target_os = "macos")]
fn lan_local_render_refresh_hz() -> Option<u32> {
    if let Some(refresh_hz) = std::env::var(LAN_RENDER_MAX_FPS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
    {
        return Some(refresh_hz);
    }

    *LOCAL_RENDER_REFRESH_HZ.get_or_init(mrd_capture_macos::highest_current_display_refresh_hz)
}

async fn maybe_send_lan_keyframe_request(
    endpoint: &QuinnDatagramEndpoint,
    session_id: &SessionId,
    profile: &MediaProfile,
    sequence: &mut u32,
    last_sent_at: &mut Option<Instant>,
    stats: &mut LanSenderStatsTracker,
) {
    let now = Instant::now();
    if last_sent_at.is_some_and(|last| {
        now.checked_duration_since(last)
            .is_some_and(|elapsed| elapsed < LAN_MEDIA_KEYFRAME_REQUEST_MIN_INTERVAL)
    }) {
        return;
    }
    *last_sent_at = Some(now);
    *sequence = sequence.wrapping_add(1).max(1);
    let max_datagram_size = endpoint
        .max_datagram_size()
        .unwrap_or(LAN_QUIC_FALLBACK_DATAGRAM_BYTES);
    match encode_lan_keyframe_request_datagram(profile, *sequence, max_datagram_size) {
        Ok(datagram) => match endpoint.send_datagram(datagram) {
            Ok(()) => {
                stats.record_ms("receiver.request_keyframe", 1.0);
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    session_id = %session_id.0,
                    "LAN media receiver failed to send keyframe request"
                );
            }
        },
        Err(error) => {
            tracing::debug!(
                %error,
                session_id = %session_id.0,
                "LAN media receiver failed to encode keyframe request"
            );
        }
    }
}

fn spawn_lan_media_control_reader(
    endpoint: QuinnDatagramEndpoint,
    session_id: SessionId,
    keyframe_requests: Arc<AtomicU64>,
) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
        loop {
            let datagram = match endpoint.read_datagram().await {
                Ok(datagram) => datagram,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id = %session_id.0,
                        "LAN media sender control reader stopped"
                    );
                    break;
                }
            };
            match decode_lan_keyframe_request_datagram(&datagram) {
                Ok(true) => {
                    keyframe_requests.fetch_add(1, Ordering::Relaxed);
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id = %session_id.0,
                        bytes = datagram.len(),
                        "LAN media sender ignored invalid control datagram"
                    );
                }
            }
        }
    }))
}

async fn set_session_last_error(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    last_error: Option<String>,
) {
    let mut sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id).cloned() else {
        return;
    };
    if snapshot.lifecycle_state.is_terminal() {
        return;
    }
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
            last_error,
            ..snapshot
        },
    );
}

async fn spawn_quic_media_receiver(
    app_state: Arc<AppState>,
    session_id: SessionId,
    endpoint: QuinnDatagramEndpoint,
) {
    let registry = app_state.media_tasks.clone();
    let failure_app_state = app_state.clone();
    let task_session_id = session_id.clone();
    let failure_session_id = task_session_id.clone();
    let handle = tokio::spawn(async move {
        if let Err(error) =
            receive_quic_media_loop(app_state, task_session_id.clone(), endpoint).await
        {
            tracing::warn!(%error, session_id = %task_session_id.0, "LAN QUIC media receiver stopped");
            mark_session_failed(
                &failure_app_state,
                &failure_session_id,
                format!("LAN QUIC media receiver failed: {error}"),
            )
            .await;
        }
    });
    let abort_handle = handle.abort_handle();
    drop(handle);
    registry.lock().await.register(session_id, abort_handle);
}

async fn receive_quic_media_loop(
    app_state: Arc<AppState>,
    session_id: SessionId,
    endpoint: QuinnDatagramEndpoint,
) -> Result<()> {
    let mut reassembler = QuicAuReassembler::new(lan_media_reassembler_config());
    let mut media_v3_reassembler = QuicMediaReassembler::new(lan_media_reassembler_config());
    let mut frame_orderer =
        LanMediaFrameOrderer::new(LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES);
    #[cfg(target_os = "macos")]
    let mut media_v3_frame_orderer =
        LanMediaFrameOrderer::<QuicMediaFrame>::new(LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES);
    let mut decoder = create_lan_receiver_decoder(&app_state, &session_id)
        .await
        .context("failed to create LAN media receiver decoder")?;
    let mut consecutive_decode_errors = 0_u32;
    let mut decoder_waits_for_keyframe = true;
    let persistent_reliable_media_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT);
    let per_message_reliable_media_supported = app_state
        .peer_media_capabilities
        .lock()
        .await
        .supports(&session_id, LAN_QUIC_RELIABLE_MEDIA_TRANSPORT);
    let initial_media_profile = selected_media_profile(&app_state, &session_id).await;
    let reliable_media_read_mode = select_reliable_media_send_mode_for_profile(
        per_message_reliable_media_supported,
        persistent_reliable_media_supported,
        &initial_media_profile,
    );
    let mut reliable_media_rx = if reliable_media_read_mode != LanReliableMediaSendMode::Disabled {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let reliable_endpoint = endpoint.clone();
        tokio::spawn(async move {
            loop {
                let result = match reliable_media_read_mode {
                    LanReliableMediaSendMode::Disabled => break,
                    LanReliableMediaSendMode::PerMessage => {
                        reliable_endpoint
                            .read_reliable_message(LAN_QUIC_RELIABLE_MEDIA_MAX_BYTES)
                            .await
                    }
                    LanReliableMediaSendMode::Persistent => {
                        reliable_endpoint
                            .read_reliable_message_persistent(LAN_QUIC_RELIABLE_MEDIA_MAX_BYTES)
                            .await
                    }
                };
                let should_retry = result.is_err();
                if tx
                    .send(result.map_err(|error| error.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
                if should_retry {
                    tokio::time::sleep(LAN_QUIC_RELIABLE_MEDIA_RETRY_DELAY).await;
                }
            }
        });
        Some(rx)
    } else {
        None
    };
    let mut datagram_media_enabled = true;
    let mut receiver_stats = LanSenderStatsTracker::new(Instant::now());
    let mut keyframe_request_sequence = 0_u32;
    let mut last_keyframe_request_at = None;
    maybe_send_lan_keyframe_request(
        &endpoint,
        &session_id,
        &initial_media_profile,
        &mut keyframe_request_sequence,
        &mut last_keyframe_request_at,
        &mut receiver_stats,
    )
    .await;
    loop {
        if !session_allows_media(&app_state, &session_id).await {
            return Ok(());
        }
        let read_started = Instant::now();
        let media_message = if let Some(rx) = reliable_media_rx.as_mut() {
            if datagram_media_enabled {
                let datagram_endpoint = endpoint.clone();
                tokio::select! {
                    result = datagram_endpoint.read_datagram() => {
                        match result {
                            Ok(message) => message,
                            Err(error) => {
                                datagram_media_enabled = false;
                                tracing::warn!(
                                    %error,
                                    session_id = %session_id.0,
                                    "LAN QUIC datagram media reader disabled while reliable media remains active"
                                );
                                continue;
                            }
                        }
                    }
                    message = rx.recv() => {
                        match message {
                            Some(Ok(message)) => message,
                            Some(Err(error)) => {
                                tracing::warn!(
                                    %error,
                                    session_id = %session_id.0,
                                    "LAN QUIC reliable media reader retrying"
                                );
                                continue;
                            }
                            None => {
                                reliable_media_rx = None;
                                tracing::warn!(
                                    session_id = %session_id.0,
                                    "LAN QUIC reliable media reader stopped"
                                );
                                continue;
                            }
                        }
                    }
                }
            } else {
                match rx.recv().await {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        tracing::warn!(
                            %error,
                            session_id = %session_id.0,
                            "LAN QUIC reliable media reader retrying"
                        );
                        continue;
                    }
                    None => {
                        reliable_media_rx = None;
                        tracing::warn!(
                            session_id = %session_id.0,
                            "LAN QUIC reliable media reader stopped"
                        );
                        continue;
                    }
                }
            }
        } else {
            endpoint
                .read_datagram()
                .await
                .context("failed to read LAN QUIC media datagram")?
        };
        receiver_stats.record_elapsed("receiver.read", read_started);
        receiver_stats.record_elapsed("receiver.message_wait", read_started);
        if !session_allows_media(&app_state, &session_id).await {
            return Ok(());
        }
        match decode_lan_sender_stats_datagram(&media_message) {
            Ok(Some(stats)) => {
                let mut pipelines = app_state.media_pipelines.lock().await;
                pipelines.set_stage_metrics(session_id.clone(), stats.metrics);
                pipelines.set_test_impairment(session_id.clone(), stats.test_impairment);
                pipelines.set_sender_transport(session_id.clone(), stats.sender_transport);
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                app_state.probes.lock().await.record_probe_drop(
                    &session_id,
                    media_message.len() as u64,
                    now_ms(),
                    format!("failed to decode LAN sender stats datagram: {error}"),
                );
                continue;
            }
        }
        let reassemble_started = Instant::now();
        let reassembled_frame = if is_quic_media_v3_datagram(&media_message) {
            let reassembled_v3_frame = media_v3_reassembler
                .push_datagram(&media_message)
                .context("failed to reassemble LAN QUIC media v3 frame")?;
            receiver_stats.record_elapsed("receiver.reassemble", reassemble_started);
            let Some(frame) = reassembled_v3_frame else {
                continue;
            };

            #[cfg(target_os = "macos")]
            if quic_media_v3_compressed_direct_render_candidate(&frame)
                && macos_render_proxy_compressed_media_surface_available(&app_state, &session_id)
                    .await
            {
                let proxy_forward_started = Instant::now();
                if render_lan_quic_media_v3_compressed_access_unit_frame(
                    &app_state,
                    &session_id,
                    &mut media_v3_frame_orderer,
                    frame.clone(),
                    media_v3_reassembler.stats(),
                    &mut receiver_stats,
                    &mut consecutive_decode_errors,
                    &mut decoder_waits_for_keyframe,
                    &endpoint,
                    &mut keyframe_request_sequence,
                    &mut last_keyframe_request_at,
                )
                .await
                {
                    let proxy_forward_ms = duration_as_millis(proxy_forward_started.elapsed());
                    receiver_stats.record_ms("receiver.proxy_forward", proxy_forward_ms);
                    app_state
                        .media_pipelines
                        .lock()
                        .await
                        .record_stage_duration_ms(
                            session_id.clone(),
                            "receiver.proxy_forward_direct_v3",
                            proxy_forward_ms,
                        );
                    flush_lan_receiver_stage_metrics(&app_state, &session_id, &mut receiver_stats)
                        .await;
                    continue;
                }
            }

            quic_media_v3_frame_to_legacy_frame(
                &app_state,
                &session_id,
                frame,
                media_v3_reassembler.stats(),
            )
            .await?
        } else {
            let reassembled_frame = reassembler
                .push_datagram(&media_message)
                .context("failed to reassemble LAN QUIC media v2 frame")?;
            receiver_stats.record_elapsed("receiver.reassemble", reassemble_started);
            reassembled_frame
        };

        if let Some(frame) = reassembled_frame {
            let ready_frames = frame_orderer.push(frame);
            receiver_stats.record_ms("receiver.ready_frames", ready_frames.len() as f64);
            for frame in ready_frames {
                let envelope = match decode_lan_media_envelope(&frame.payload) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        app_state.probes.lock().await.record_probe_drop(
                            &session_id,
                            frame.payload.len() as u64,
                            now_ms(),
                            format!("invalid LAN media v2 envelope: {error}"),
                        );
                        continue;
                    }
                };

                match envelope.payload_type {
                    LAN_MEDIA_PAYLOAD_ACCESS_UNIT => {
                        let frame_codec =
                            match LanAccessUnitCodec::from_envelope_codec(envelope.codec) {
                                Ok(codec) => codec,
                                Err(error) => {
                                    app_state.probes.lock().await.record_probe_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                        format!("{error:#}"),
                                    );
                                    continue;
                                }
                            };
                        if decoder.codec != frame_codec {
                            let next_decoder = create_lan_receiver_decoder_with_preference(
                                &app_state,
                                &session_id,
                                frame_codec,
                                None,
                            )
                            .await
                            .with_context(|| {
                                format!(
                                    "failed to switch LAN media receiver decoder to {}",
                                    frame_codec.display_name()
                                )
                            });
                            match next_decoder {
                                Ok(next_decoder) => {
                                    decoder = next_decoder;
                                    decoder_waits_for_keyframe = true;
                                    consecutive_decode_errors = 0;
                                }
                                Err(error) => {
                                    app_state.probes.lock().await.record_probe_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                        format!("{error:#}"),
                                    );
                                    decoder_waits_for_keyframe = true;
                                    continue;
                                }
                            }
                        }
                        if decoder_waits_for_keyframe && !frame.is_keyframe {
                            app_state.probes.lock().await.record_transient_frame_drop(
                                &session_id,
                                frame.payload.len() as u64,
                                now_ms(),
                            );
                            maybe_send_lan_keyframe_request(
                                &endpoint,
                                &session_id,
                                &envelope.profile,
                                &mut keyframe_request_sequence,
                                &mut last_keyframe_request_at,
                                &mut receiver_stats,
                            )
                            .await;
                            continue;
                        }

                        #[cfg(target_os = "macos")]
                        if matches!(
                            frame_codec,
                            LanAccessUnitCodec::H264 | LanAccessUnitCodec::Hevc
                        ) && match frame_codec {
                            LanAccessUnitCodec::H264 => {
                                macos_render_proxy_compressed_media_enabled()
                            }
                            LanAccessUnitCodec::Hevc => {
                                macos_render_proxy_compressed_media_enabled_for_profile(
                                    &envelope.profile,
                                )
                            }
                            LanAccessUnitCodec::Av1 => false,
                        } {
                            let proxy_forward_started = Instant::now();
                            let proxy_result = match frame_codec {
                                LanAccessUnitCodec::H264 => {
                                    render_lan_h264_access_unit_frame(
                                        &app_state,
                                        &session_id,
                                        bytes::Bytes::from(envelope.payload.clone()),
                                        envelope.sequence,
                                        envelope.timestamp_us,
                                        &envelope.profile,
                                    )
                                    .await
                                }
                                LanAccessUnitCodec::Hevc => {
                                    render_lan_hevc_access_unit_frame(
                                        &app_state,
                                        &session_id,
                                        bytes::Bytes::from(envelope.payload.clone()),
                                        envelope.sequence,
                                        envelope.timestamp_us,
                                        &envelope.profile,
                                    )
                                    .await
                                }
                                LanAccessUnitCodec::Av1 => Ok(false),
                            };
                            match proxy_result {
                                Ok(true) => {
                                    receiver_stats.record_elapsed(
                                        "receiver.proxy_forward",
                                        proxy_forward_started,
                                    );
                                    consecutive_decode_errors = 0;
                                    decoder_waits_for_keyframe = false;
                                    continue;
                                }
                                Ok(false)
                                    if macos_render_proxy_compressed_media_surface_available(
                                        &app_state,
                                        &session_id,
                                    )
                                    .await =>
                                {
                                    receiver_stats.record_elapsed(
                                        "receiver.proxy_forward",
                                        proxy_forward_started,
                                    );
                                    app_state.probes.lock().await.record_transient_frame_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                    );
                                    maybe_send_lan_keyframe_request(
                                        &endpoint,
                                        &session_id,
                                        &envelope.profile,
                                        &mut keyframe_request_sequence,
                                        &mut last_keyframe_request_at,
                                        &mut receiver_stats,
                                    )
                                    .await;
                                    decoder_waits_for_keyframe = true;
                                    continue;
                                }
                                Ok(false) => {}
                                Err(error) => {
                                    receiver_stats.record_elapsed(
                                        "receiver.proxy_forward",
                                        proxy_forward_started,
                                    );
                                    tracing::warn!(
                                        %error,
                                        session_id = %session_id.0,
                                        sequence = envelope.sequence,
                                        codec = frame_codec.display_name(),
                                        "LAN media receiver failed to forward access unit to macOS render proxy"
                                    );
                                    app_state.probes.lock().await.record_probe_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                        format!(
                                            "failed to forward LAN {} access unit to macOS render proxy: {error:#}",
                                            frame_codec.display_name()
                                        ),
                                    );
                                    decoder_waits_for_keyframe = true;
                                    continue;
                                }
                            }
                        }

                        let decode_started = Instant::now();
                        match decode_lan_desktop_frame(
                            frame_codec,
                            decoder.decoder.as_mut(),
                            &envelope.payload,
                        ) {
                            Ok(decoded_frames) if !decoded_frames.is_empty() => {
                                receiver_stats.record_elapsed("receiver.decode", decode_started);
                                consecutive_decode_errors = 0;
                                decoder_waits_for_keyframe = false;
                                let record_started = Instant::now();
                                record_lan_decoded_frames(
                                    &app_state,
                                    &session_id,
                                    decoded_frames,
                                    frame.payload.len() as u64,
                                    envelope.sequence,
                                    envelope.timestamp_us,
                                    &envelope.profile,
                                    &envelope.payload,
                                )
                                .await;
                                receiver_stats.record_elapsed("receiver.record", record_started);
                            }
                            Ok(_) => {
                                receiver_stats.record_elapsed("receiver.decode", decode_started);
                            }
                            Err(error) => {
                                receiver_stats.record_elapsed("receiver.decode", decode_started);
                                let error = if frame.is_keyframe
                                    && frame_codec == LanAccessUnitCodec::H264
                                {
                                    match try_decode_h264_keyframe_with_fallback(
                                        &app_state,
                                        &session_id,
                                        decoder.backend,
                                        &envelope.payload,
                                        &error,
                                    )
                                    .await
                                    {
                                        Ok((next_decoder, decoded_frames)) => {
                                            decoder = next_decoder;
                                            consecutive_decode_errors = 0;
                                            decoder_waits_for_keyframe = false;
                                            let record_started = Instant::now();
                                            record_lan_decoded_frames(
                                                &app_state,
                                                &session_id,
                                                decoded_frames,
                                                frame.payload.len() as u64,
                                                envelope.sequence,
                                                envelope.timestamp_us,
                                                &envelope.profile,
                                                &envelope.payload,
                                            )
                                            .await;
                                            receiver_stats
                                                .record_elapsed("receiver.record", record_started);
                                            continue;
                                        }
                                        Err(fallback_error) => fallback_error,
                                    }
                                } else {
                                    error
                                };
                                consecutive_decode_errors =
                                    consecutive_decode_errors.saturating_add(1);
                                let reassembler_stats = reassembler.stats();
                                let payload_hash =
                                    format!("fnv1a64:{:016x}", fnv1a64(&envelope.payload));
                                let message = format!(
                                "failed to decode LAN {} media v2 access unit: sequence={}, keyframe={}, bytes={}, hash={}, reassembler={{completed:{}, expired:{}, evicted:{}, duplicate:{}, rejected:{}, pending:{}}}: {error}",
                                frame_codec.display_name(),
                                envelope.sequence,
                                frame.is_keyframe,
                                envelope.payload.len(),
                                payload_hash,
                                reassembler_stats.completed_frames,
                                reassembler_stats.expired_frames,
                                reassembler_stats.evicted_frames,
                                reassembler_stats.duplicate_fragments,
                                reassembler_stats.rejected_fragments,
                                reassembler_stats.pending_frames
                            );
                                if should_log_media_receiver_decode_error(consecutive_decode_errors)
                                {
                                    tracing::warn!(
                                        session_id = %session_id.0,
                                        sequence = envelope.sequence,
                                        is_keyframe = frame.is_keyframe,
                                        consecutive_decode_errors,
                                        error = %error,
                                        "LAN media receiver dropped a decoded frame"
                                    );
                                }

                                if frame.is_keyframe {
                                    app_state.probes.lock().await.record_probe_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                        message,
                                    );
                                    decoder_waits_for_keyframe = true;
                                    decoder = create_lan_receiver_decoder_with_preference(
                                        &app_state,
                                        &session_id,
                                        frame_codec,
                                        Some(decoder.backend),
                                    )
                                    .await
                                    .context(
                                        "failed to reset LAN media receiver decoder after decode error",
                                    )?;
                                    consecutive_decode_errors = 0;
                                } else {
                                    app_state.probes.lock().await.record_transient_frame_drop(
                                        &session_id,
                                        frame.payload.len() as u64,
                                        now_ms(),
                                    );
                                    if consecutive_decode_errors
                                        >= LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS
                                    {
                                        tracing::warn!(
                                            session_id = %session_id.0,
                                            consecutive_decode_errors,
                                            backend = decoder.backend,
                                            "LAN media receiver reset decoder after non-keyframe decode loss"
                                        );
                                        decoder_waits_for_keyframe = true;
                                        decoder = create_lan_receiver_decoder_with_preference(
                                            &app_state,
                                            &session_id,
                                            frame_codec,
                                            Some(decoder.backend),
                                        )
                                        .await
                                        .context(
                                            "failed to reset LAN media receiver decoder after decode loss",
                                        )?;
                                        consecutive_decode_errors = 0;
                                    }
                                }
                            }
                        }
                    }
                    LAN_MEDIA_PAYLOAD_PROBE_FRAME => {
                        match decode_media_probe_frame(&envelope.payload) {
                            Ok(stats) => {
                                app_state.probes.lock().await.record_media_probe_frame(
                                    &session_id,
                                    stats,
                                    now_ms(),
                                );
                            }
                            Err(error) => {
                                app_state.probes.lock().await.record_probe_drop(
                                    &session_id,
                                    frame.payload.len() as u64,
                                    now_ms(),
                                    format!("failed to decode LAN media v2 probe frame: {error}"),
                                );
                            }
                        }
                    }
                    payload_type => app_state.probes.lock().await.record_probe_drop(
                        &session_id,
                        frame.payload.len() as u64,
                        now_ms(),
                        format!("unsupported LAN media v2 payload type: {payload_type}"),
                    ),
                }
            }
        }
        flush_lan_receiver_stage_metrics(&app_state, &session_id, &mut receiver_stats).await;
    }
}

async fn flush_lan_receiver_stage_metrics(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    receiver_stats: &mut LanSenderStatsTracker,
) {
    if let Some(metrics) = receiver_stats.take_stage_metrics(Instant::now()) {
        app_state
            .media_pipelines
            .lock()
            .await
            .set_stage_metrics(session_id.clone(), metrics);
    }
}

#[cfg(target_os = "macos")]
fn quic_media_v3_compressed_direct_render_candidate(frame: &QuicMediaFrame) -> bool {
    media_receiver::compressed_direct_render_candidate(
        macos_render_proxy_compressed_media_enabled(),
        frame.payload_type,
        frame.codec,
    )
}

#[cfg(target_os = "macos")]
async fn macos_render_proxy_compressed_media_surface_available(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> bool {
    if !macos_render_proxy_compressed_media_enabled() {
        return false;
    }
    app_state
        .media_surface_renderers
        .lock()
        .await
        .session_surface_count(session_id)
        > 0
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
async fn render_lan_quic_media_v3_compressed_access_unit_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    frame_orderer: &mut LanMediaFrameOrderer<QuicMediaFrame>,
    frame: QuicMediaFrame,
    reassembler_stats: QuicAuReassemblerStats,
    receiver_stats: &mut LanSenderStatsTracker,
    consecutive_decode_errors: &mut u32,
    decoder_waits_for_keyframe: &mut bool,
    endpoint: &QuinnDatagramEndpoint,
    keyframe_request_sequence: &mut u32,
    last_keyframe_request_at: &mut Option<Instant>,
) -> bool {
    if !quic_media_v3_compressed_direct_render_candidate(&frame) {
        return false;
    }

    let frame_codec = frame.codec;
    let mut profile = selected_media_profile(app_state, session_id).await;
    let expected_profile_id = lan_media_profile_id(&profile);
    if frame.profile_id != expected_profile_id {
        tracing::debug!(
            session_id = %session_id.0,
            frame_id = frame.frame_id,
            expected_profile_id,
            received_profile_id = frame.profile_id,
            completed = reassembler_stats.completed_frames,
            expired = reassembler_stats.expired_frames,
            evicted = reassembler_stats.evicted_frames,
            duplicate = reassembler_stats.duplicate_fragments,
            rejected = reassembler_stats.rejected_fragments,
            pending = reassembler_stats.pending_frames,
            codec = ?frame_codec,
            "LAN media receiver dropped stale v3 compressed profile frame before legacy envelope conversion"
        );
        app_state.probes.lock().await.record_transient_frame_drop(
            session_id,
            frame.payload.len() as u64,
            now_ms(),
        );
        return true;
    }

    profile.codec = match frame_codec {
        QuicMediaCodec::H264 => "h264".to_string(),
        QuicMediaCodec::Hevc => "hevc".to_string(),
        _ => return false,
    };
    normalize_lan_media_profile(&mut profile);
    if !macos_render_proxy_compressed_media_enabled_for_profile(&profile) {
        return false;
    }

    let ready_frames = frame_orderer.push(frame);
    receiver_stats.record_ms("receiver.ready_frames", ready_frames.len() as f64);
    for ready_frame in ready_frames {
        if *decoder_waits_for_keyframe && !ready_frame.is_keyframe() {
            app_state.probes.lock().await.record_transient_frame_drop(
                session_id,
                ready_frame.payload.len() as u64,
                now_ms(),
            );
            maybe_send_lan_keyframe_request(
                endpoint,
                session_id,
                &profile,
                keyframe_request_sequence,
                last_keyframe_request_at,
                receiver_stats,
            )
            .await;
            continue;
        }

        let render_result = match frame_codec {
            QuicMediaCodec::H264 => {
                render_lan_h264_access_unit_frame(
                    app_state,
                    session_id,
                    ready_frame.payload.clone(),
                    u64::from(ready_frame.frame_id),
                    ready_frame.timestamp_us,
                    &profile,
                )
                .await
            }
            QuicMediaCodec::Hevc => {
                render_lan_hevc_access_unit_frame(
                    app_state,
                    session_id,
                    ready_frame.payload.clone(),
                    u64::from(ready_frame.frame_id),
                    ready_frame.timestamp_us,
                    &profile,
                )
                .await
            }
            _ => return false,
        };
        match render_result {
            Ok(true) => {
                *consecutive_decode_errors = 0;
                *decoder_waits_for_keyframe = false;
            }
            Ok(false) => {
                app_state.probes.lock().await.record_transient_frame_drop(
                    session_id,
                    ready_frame.payload.len() as u64,
                    now_ms(),
                );
                maybe_send_lan_keyframe_request(
                    endpoint,
                    session_id,
                    &profile,
                    keyframe_request_sequence,
                    last_keyframe_request_at,
                    receiver_stats,
                )
                .await;
                *decoder_waits_for_keyframe = true;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %session_id.0,
                    sequence = ready_frame.frame_id,
                    codec = ?frame_codec,
                    "LAN media receiver failed to forward v3 compressed access unit to macOS render proxy"
                );
                app_state.probes.lock().await.record_probe_drop(
                    session_id,
                    ready_frame.payload.len() as u64,
                    now_ms(),
                    format!(
                        "failed to forward LAN v3 {:?} access unit to macOS render proxy: {error:#}",
                        frame_codec
                    ),
                );
                maybe_send_lan_keyframe_request(
                    endpoint,
                    session_id,
                    &profile,
                    keyframe_request_sequence,
                    last_keyframe_request_at,
                    receiver_stats,
                )
                .await;
                *decoder_waits_for_keyframe = true;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests;
