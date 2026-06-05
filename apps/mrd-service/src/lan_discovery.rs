use crate::app_state::{AppState, DecodedVideoFrameStats};
#[cfg(any(windows, target_os = "macos"))]
use crate::app_state::{MediaRenderFrame, MediaRenderQueueEnqueue, MediaRenderQueueRegistry};
use anyhow::{Context, Result};
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, ControlInputEvent, ControlInputLane, DisplayMode,
    DisplayModeChange, LanDiscoverySnapshot, LanPeerInfo, MediaProfile, MediaProfileNegotiation,
};
#[cfg(test)]
use mrd_ipc::{MediaSenderTransportSnapshot, MediaStageMetrics};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use mrd_pipeline_core::FrameCapture;
use mrd_pipeline_core::{
    CapturedFrame, ColorMode, DecodedFrame, DecodedFrameData, FramePixelFormat, VideoDecoder,
    VideoEncoder,
};
use mrd_proto::{DeviceId, SessionId};
#[cfg(any(windows, target_os = "macos"))]
use mrd_render::{RenderFrame, RendererSnapshot};
#[cfg(test)]
use mrd_transport_quic_quinn::QuicAuReassemblerConfig;
use mrd_transport_quic_quinn::{
    fragment_access_unit, fragment_media_payload_v3, is_quic_media_v3_datagram, QuicAuFrame,
    QuicAuReassembler, QuicAuReassemblerStats, QuicMediaCodec, QuicMediaFrame,
    QuicMediaPayloadType, QuicMediaReassembler, QuinnDatagramEndpoint, QuinnServerBootstrap,
    QuinnServerListener, QUIC_AU_FRAGMENT_HEADER_LEN, QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Condvar as StdCondvar;
#[cfg(any(windows, target_os = "macos"))]
use std::sync::{Mutex as StdMutex, OnceLock};
#[cfg(any(windows, target_os = "macos"))]
use std::sync::{MutexGuard as StdMutexGuard, TryLockError};
#[cfg(any(windows, target_os = "macos"))]
use std::thread;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant as StdInstant;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, Notify};
use tokio::time::{interval, timeout, Instant};

mod capture_activity;
mod discovery_config;
mod discovery_identity;
mod dynamic_window_fps;
mod lan_control_input;
mod media_access_unit;
mod media_capture_config;
mod media_envelope;
mod media_error_policy;
mod media_keyframe_request;
mod media_ordering;
mod media_probe;
mod media_profile;
mod media_receiver_decoder_candidates;
mod media_render_policy;
mod media_sender_telemetry;
mod media_timing;
mod media_transport;
mod peer_format;
mod service_identity;
mod time_utils;
use capture_activity::active_window_capture_count;
pub use discovery_config::LanDiscoveryConfig;
use discovery_identity::{
    default_app_id, is_valid_discovery_packet, new_instance_id, now_ms, DISCOVERY_APP_ID,
    DISCOVERY_MAGIC,
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
use media_access_unit::{describe_lan_access_unit, h264_access_unit_is_keyframe};
#[cfg(test)]
use media_capture_config::window_capture_source_error;
use media_capture_config::{
    dynamic_window_fps_config_key, format_capture_source_failure, lan_capture_config_key,
    lan_capture_config_matches, DynamicWindowFpsConfigKey, LanCaptureConfigKey,
};
#[cfg(test)]
use media_envelope::LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT;
use media_envelope::{
    decode_lan_media_envelope, encode_lan_media_envelope, lan_media_codec_name,
    lan_media_profile_id, LanMediaEnvelope, LAN_MEDIA_CODEC_H264, LAN_MEDIA_CODEC_HEVC,
    LAN_MEDIA_PAYLOAD_ACCESS_UNIT, LAN_MEDIA_PAYLOAD_PROBE_FRAME,
};
use media_error_policy::{
    should_log_media_receiver_decode_error, should_log_media_sender_frame_error,
    LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS,
    LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS,
};
use media_keyframe_request::{
    decode_lan_keyframe_request_datagram, encode_lan_keyframe_request_datagram,
};
use media_ordering::LanMediaFrameOrderer;
#[cfg(test)]
use media_probe::{build_media_probe_frame, media_payload_bytes};
use media_probe::{
    decode_media_probe_frame, decoded_video_probe_format, fnv1a64, fnv1a64_media_metadata,
};
#[cfg(test)]
use media_profile::format_media_profile;
use media_profile::{
    apply_lan_media_profile_defaults, clamp_media_profile_to_lan_capability, default_media_profile,
    default_media_profile_negotiation, ensure_peer_can_receive_selected_media,
    ensure_peer_supports_requested_media, lan_color_mode_for_profile,
    lan_profile_requests_hevc_main10, lan_runtime_media_profile,
    missing_profile_receiver_media_capabilities, normalize_lan_codec_name,
    normalize_lan_media_profile, validate_media_profile,
};
#[cfg(all(test, target_os = "macos"))]
use media_receiver_decoder_candidates::preferred_lan_receiver_decoder_candidates_from_preference;
#[cfg(test)]
use media_receiver_decoder_candidates::{
    default_lan_receiver_decoder_candidates, prioritize_lan_receiver_decoder_candidates,
};
use media_receiver_decoder_candidates::{
    lan_receiver_decoder_candidates, preferred_lan_receiver_decoder_candidates,
};
use media_render_policy::lan_media_payload_hash_for_profile;
#[cfg(test)]
use media_render_policy::{
    lan_media_payload_hash_for_mode, lan_media_payload_hash_mode_for_profile_with_override,
    lan_media_payload_hash_mode_from_env_value, lan_render_pacing_from_env_value,
    LanMediaPayloadHashMode,
};
#[cfg(any(windows, target_os = "macos"))]
use media_render_policy::{
    lan_render_cap_target_fps_for_profile, lan_render_pacing_render_start_delay,
    lan_render_pacing_target_fps, lan_render_policy_allows_service_pacing,
    lan_render_queue_capacity_for_policy, lan_render_queue_capacity_for_profile,
    lan_render_queue_policy_for_profile, native_render_waitable_swapchain_pacing_enabled,
    render_pacing_precise_sleep_guard, render_profile_requests_high_resolution_timer,
    should_interrupt_render_pacing_sleep, LanRenderQueuePolicy,
};
#[cfg(all(test, any(windows, target_os = "macos")))]
use media_render_policy::{
    lan_render_pacing_enabled_for_profile, lan_render_pacing_target_fps_from_values,
    lan_render_queue_capacity_from_env_value, lan_render_queue_policy_for_profile_with_override,
    lan_render_queue_policy_from_env_value, render_pacing_frame_interval,
};
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
use service_identity::service_build_id;
#[cfg(test)]
use service_identity::{service_build_id_from_lookup, SERVICE_BUILD_ID_ENV};
use time_utils::{duration_as_millis, now_us};

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
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_COMPRESSED_MEDIA_ENV: &str = "MRD_MACOS_RENDER_PROXY_COMPRESSED_MEDIA";
#[cfg(windows)]
const D3D11_RENDER_PRESENT_BLOCKING_ENV: &str = "MRD_D3D11_RENDER_PRESENT_BLOCKING";
#[cfg(windows)]
const D3D11_RENDER_WAITABLE_OBJECT_ENV: &str = "MRD_D3D11_RENDER_WAITABLE_OBJECT";
const PROTOCOL_VERSION: u32 = 1;
const DISCOVERY_PACKET_BUFFER_BYTES: usize = 65_535;
const DISCOVERY_SAFE_UDP_PAYLOAD_BYTES: usize = 60_000;
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
const LAN_RENDER_SURFACE_RENDERER_LOCK_TIMEOUT: Duration = Duration::from_millis(2);
const LAN_RENDER_SURFACE_RENDERER_LOCK_POLL_INTERVAL: Duration = Duration::from_micros(100);
const LAN_RENDER_PACING_DEFAULT_MIN_FPS: u32 = 120;
const LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES: usize = 3;
const LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT: usize = 8;
const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";
const LAN_QUIC_MEDIA_PROFILE_TRANSPORT: &str = "quic_datagram_2k144";
const LAN_QUIC_MEDIA_V2_TRANSPORT: &str = "quic_datagram_media_v2";
const LAN_QUIC_MEDIA_V3_TRANSPORT: &str = "quic_datagram_media_v3";
const LAN_QUIC_RELIABLE_MEDIA_TRANSPORT: &str = "quic_stream_media_v2";
const LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT: &str = "quic_stream_media_v3";
const LAN_MEDIA_PROFILE_CONTROL_TRANSPORT: &str = "media_profile_control_v1";
const LAN_MEDIA_KEYFRAME_REQUEST_MIN_INTERVAL: Duration = Duration::from_millis(20);
const LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT: &str = "capture_source_control_v1";
const LAN_DISPLAY_MODE_CONTROL_TRANSPORT: &str = "display_mode_control_v1";
const LAN_INPUT_CONTROL_TRANSPORT: &str = "input_control_v1";
const LAN_CONTROL_INPUT_ACK_TIMEOUT: Duration = Duration::from_millis(250);
const LAN_CONTROL_INPUT_REALTIME_ATTEMPTS: usize = 1;
const LAN_CONTROL_INPUT_RELIABLE_ATTEMPTS: usize = 3;
const LAN_CONTROL_INPUT_DEDUPE_WINDOW_MS: u64 = 10_000;
const LAN_CONTROL_INPUT_DEDUPE_CACHE_LIMIT: usize = 4096;
const LAN_MEDIA_PROTOCOL_VERSION: u32 = 3;
#[cfg(windows)]
const LAN_CAPTURE_DXGI_CAPABILITY: &str = "dxgi_capture";
#[cfg(windows)]
const LAN_ENCODE_NVENC_H264_CAPABILITY: &str = "nvenc_h264";
#[cfg(windows)]
const LAN_ENCODE_NVENC_HEVC_CAPABILITY: &str = "encode.nvenc_hevc";
#[cfg(windows)]
const LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY: &str = "encode.nvenc_hevc_main10";
#[cfg(windows)]
const LAN_DECODE_NVDEC_CAPABILITY: &str = "nvdec";
#[cfg(windows)]
const LAN_DECODE_NVDEC_HEVC_CAPABILITY: &str = "decode.nvdec_hevc";
#[cfg(windows)]
const LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY: &str = "decode.nvdec_hevc_main10";
const LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY: &str = "media.hevc_main_420_8bit";
const LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY: &str = "media.hevc_main10_420_10bit";
const LAN_MEDIA_COLOR_MODE_CAPABILITY: &str = "media.color_mode_v1";
#[cfg(windows)]
const LAN_RENDER_D3D11_NATIVE_CAPABILITY: &str = "d3d11_native_render";
#[cfg(windows)]
const LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY: &str = "render.d3d11_shared_nv12";
const LAN_INPUT_CONTROL_CAPABILITY: &str = "control.keyboard_mouse";
#[cfg(target_os = "macos")]
const LAN_CAPTURE_MACOS_CAPABILITY: &str = "macos_capture";
#[cfg(target_os = "macos")]
const LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY: &str = "videotoolbox_h264";
#[cfg(target_os = "macos")]
const LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY: &str = "videotoolbox_hevc";
#[cfg(target_os = "macos")]
const LAN_DECODE_VIDEOTOOLBOX_CAPABILITY: &str = "videotoolbox";
#[cfg(target_os = "macos")]
const LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY: &str = "decode.videotoolbox_h264";
#[cfg(target_os = "macos")]
const LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY: &str = "decode.videotoolbox_hevc";
#[cfg(target_os = "macos")]
const LAN_RENDER_MACOS_NATIVE_CAPABILITY: &str = "macos_native_render";
#[cfg(any(windows, target_os = "macos"))]
static LOCAL_RENDER_REFRESH_HZ: OnceLock<Option<u32>> = OnceLock::new();
#[cfg(any(windows, target_os = "macos"))]
static LAN_RENDER_NO_SURFACE_LOG_COUNT: AtomicU64 = AtomicU64::new(0);
#[cfg(any(windows, target_os = "macos"))]
static LAN_RENDER_PRESENT_LOG_COUNT: AtomicU64 = AtomicU64::new(0);
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
    peers: Mutex<HashMap<String, StoredLanPeer>>,
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
            peers: Mutex::new(HashMap::new()),
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
        targets.push(SocketAddr::from(([255, 255, 255, 255], discovery_port)));
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

        let peer = StoredLanPeer {
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
            last_seen_ms: now_ms(),
        };

        self.peers.lock().await.insert(peer.device_id.clone(), peer);
        self.peer_changed.notify_one();
    }

    async fn prune_stale_peers(&self) {
        let ttl_ms = self.config.peer_ttl.as_millis() as u64;
        let now = now_ms();
        self.peers
            .lock()
            .await
            .retain(|_, peer| now.saturating_sub(peer.last_seen_ms) <= ttl_ms);
    }

    pub async fn snapshot(&self) -> LanDiscoverySnapshot {
        self.prune_stale_peers().await;
        let now = now_ms();
        let peers = self
            .peers
            .lock()
            .await
            .values()
            .map(|peer| {
                let p2p_control_addr = SocketAddr::new(peer.ip, peer.discovery_port).to_string();
                LanPeerInfo {
                    device_id: DeviceId(peer.device_id.clone()),
                    device_name: peer.device_name.clone(),
                    device_type: peer.device_type.clone(),
                    ip: peer.ip.to_string(),
                    discovery_port: peer.discovery_port,
                    p2p_control_addr,
                    transports: peer.transports.clone(),
                    protocol_version: peer.protocol_version,
                    service_build_id: peer.service_build_id.clone(),
                    media_protocol_version: peer.media_protocol_version,
                    media_capabilities: peer.media_capabilities.clone(),
                    age_ms: now.saturating_sub(peer.last_seen_ms),
                    p2p_available: true,
                }
            })
            .collect();

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
        self.peers
            .lock()
            .await
            .get(&device_id.0)
            .map(|peer| SocketAddr::new(peer.ip, peer.discovery_port))
    }

    pub async fn peer_transports(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.prune_stale_peers().await;
        self.peers
            .lock()
            .await
            .get(&device_id.0)
            .map(|peer| peer.transports.clone())
    }

    pub async fn peer_media_capabilities(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.prune_stale_peers().await;
        self.peers.lock().await.get(&device_id.0).map(|peer| {
            let mut capabilities = peer.media_capabilities.clone();
            for transport in &peer.transports {
                if !capabilities
                    .iter()
                    .any(|capability| capability == transport)
                {
                    capabilities.push(transport.clone());
                }
            }
            capabilities
        })
    }
}

impl Default for LanDiscoveryState {
    fn default() -> Self {
        Self::new(LanDiscoveryConfig::default())
    }
}

#[derive(Debug, Clone)]
struct StoredLanPeer {
    device_id: String,
    device_name: String,
    device_type: String,
    ip: IpAddr,
    discovery_port: u16,
    transports: Vec<String>,
    protocol_version: u32,
    service_build_id: Option<String>,
    media_protocol_version: Option<u32>,
    media_capabilities: Vec<String>,
    last_seen_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LanDiscoveryPacket {
    Probe {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        device_id: Option<String>,
        timestamp_ms: u64,
    },
    Announce(LanAnnouncement),
    RemoteSessionRequest {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        source_device_name: String,
        transport_kind: String,
        #[serde(default)]
        source_discovery_port: Option<u16>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        source_media_capabilities: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_media_profile: Option<MediaProfile>,
        timestamp_ms: u64,
    },
    RemoteSessionAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<LanMediaBootstrap>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_profile: Option<MediaProfileNegotiation>,
        timestamp_ms: u64,
    },
    MediaProfileUpdate {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        requested_media_profile: MediaProfile,
        timestamp_ms: u64,
    },
    MediaProfileUpdateAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_profile: Option<MediaProfileNegotiation>,
        timestamp_ms: u64,
    },
    CaptureSourcesRequest {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        include_previews: bool,
        limit: Option<u32>,
        timestamp_ms: u64,
    },
    CaptureSourcesAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        sources: Vec<CaptureSource>,
        timestamp_ms: u64,
    },
    CaptureSourceSelect {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        source_id: String,
        timestamp_ms: u64,
    },
    CaptureSourceSelectAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<CaptureSourceSelection>,
        timestamp_ms: u64,
    },
    DisplayModesRequest {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        source_id: Option<String>,
        timestamp_ms: u64,
    },
    DisplayModesAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        modes: Vec<DisplayMode>,
        timestamp_ms: u64,
    },
    DisplayModeSet {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        mode: DisplayMode,
        restore_after_session: bool,
        timestamp_ms: u64,
    },
    DisplayModeSetAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        change: Option<DisplayModeChange>,
        timestamp_ms: u64,
    },
    DisplayModeRestore {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        timestamp_ms: u64,
    },
    DisplayModeRestoreAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        change: Option<DisplayModeChange>,
        timestamp_ms: u64,
    },
    ControlInput {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        #[serde(default)]
        event_id: u64,
        event: ControlInputEvent,
        timestamp_ms: u64,
    },
    ControlInputAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        #[serde(default)]
        event_id: u64,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lane: Option<ControlInputLane>,
        event_count: u32,
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanAnnouncement {
    magic: String,
    #[serde(default = "default_app_id")]
    app_id: String,
    instance_id: String,
    device_id: String,
    device_name: String,
    device_type: String,
    protocol_version: u32,
    discovery_port: u16,
    transports: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    media_capabilities: Vec<String>,
    timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanMediaBootstrap {
    transport_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quic: Option<LanQuicBootstrap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanQuicBootstrap {
    listen_addr: String,
    server_name: String,
    cert_der: Vec<u8>,
}

struct LanRemoteAcceptResult {
    accepted: bool,
    message: Option<String>,
    media: Option<LanMediaBootstrap>,
    media_profile: Option<MediaProfileNegotiation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanAccessUnitCodec {
    H264,
    Hevc,
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

impl LanAccessUnitCodec {
    fn from_profile(profile: &MediaProfile) -> Self {
        if normalize_lan_codec_name(&profile.codec) == Some("hevc") {
            Self::Hevc
        } else {
            Self::H264
        }
    }

    fn from_envelope_codec(codec: u8) -> Result<Self> {
        match codec {
            LAN_MEDIA_CODEC_H264 => Ok(Self::H264),
            LAN_MEDIA_CODEC_HEVC => Ok(Self::Hevc),
            _ => anyhow::bail!("unsupported LAN media access unit codec: {codec}"),
        }
    }

    fn quic_codec(self) -> QuicMediaCodec {
        match self {
            Self::H264 => QuicMediaCodec::H264,
            Self::Hevc => QuicMediaCodec::Hevc,
        }
    }

    fn envelope_codec(self) -> u8 {
        match self {
            Self::H264 => LAN_MEDIA_CODEC_H264,
            Self::Hevc => LAN_MEDIA_CODEC_HEVC,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
        }
    }
}

struct LanSenderEncoder {
    codec: LanAccessUnitCodec,
    backend: &'static str,
    encoder: Box<dyn VideoEncoder + Send>,
}

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
    socket
        .set_broadcast(true)
        .context("failed to enable LAN discovery UDP broadcast")?;

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
    let (len, ack_addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
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
            let ack = fit_capture_sources_ack_packet(
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
        timestamp_ms: now_ms(),
    })
}

fn lan_media_capabilities() -> Vec<String> {
    lan_media_capabilities_with_input_control(cfg!(windows))
}

fn lan_media_capabilities_with_input_control(input_control_available: bool) -> Vec<String> {
    let mut capabilities = vec![
        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
        LAN_QUIC_MEDIA_V3_TRANSPORT.to_string(),
        LAN_QUIC_RELIABLE_MEDIA_TRANSPORT.to_string(),
        LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT.to_string(),
    ];
    #[cfg(windows)]
    {
        capabilities.extend([
            LAN_CAPTURE_DXGI_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_H264_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_HEVC_CAPABILITY.to_string(),
            LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_HEVC_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY.to_string(),
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
            LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY.to_string(),
            LAN_MEDIA_COLOR_MODE_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_NATIVE_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY.to_string(),
            crate::display_mode::capability_name().to_string(),
        ]);
    }
    #[cfg(target_os = "macos")]
    {
        capabilities.extend(macos_lan_media_capabilities());
    }
    #[cfg(target_os = "linux")]
    {
        capabilities.extend([
            "pipewire_capture".to_string(),
            "openh264_fallback".to_string(),
            "software_decode".to_string(),
        ]);
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        capabilities.extend([
            "openh264_fallback".to_string(),
            "software_decode".to_string(),
        ]);
    }
    if input_control_available {
        capabilities.push(LAN_INPUT_CONTROL_CAPABILITY.to_string());
    }
    capabilities
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct MacosLanMediaCapabilityProbe {
    videotoolbox_h264_encoder: bool,
    videotoolbox_hevc_encoder: bool,
    videotoolbox_h264_decoder: bool,
    videotoolbox_hevc_decoder: bool,
}

#[cfg(target_os = "macos")]
fn macos_lan_media_capabilities() -> Vec<String> {
    static MACOS_LAN_MEDIA_CAPABILITIES: OnceLock<Vec<String>> = OnceLock::new();
    MACOS_LAN_MEDIA_CAPABILITIES
        .get_or_init(|| {
            macos_lan_media_capabilities_from_probe(probe_macos_lan_media_capabilities())
        })
        .clone()
}

#[cfg(target_os = "macos")]
fn probe_macos_lan_media_capabilities() -> MacosLanMediaCapabilityProbe {
    MacosLanMediaCapabilityProbe {
        videotoolbox_h264_encoder: mrd_codec_videotoolbox::VideoToolboxH264Encoder::new(
            640, 480, 30,
        )
        .is_ok(),
        videotoolbox_hevc_encoder: mrd_codec_videotoolbox::VideoToolboxHevcEncoder::new(
            640, 480, 30,
        )
        .is_ok(),
        videotoolbox_h264_decoder: videotoolbox_decoder_enabled()
            && mrd_codec_videotoolbox::VideoToolboxH264Decoder::new().is_ok(),
        videotoolbox_hevc_decoder: videotoolbox_decoder_enabled()
            && mrd_codec_videotoolbox::VideoToolboxHevcDecoder::new().is_ok(),
    }
}

#[cfg(target_os = "macos")]
fn macos_lan_media_capabilities_from_probe(probe: MacosLanMediaCapabilityProbe) -> Vec<String> {
    let mut capabilities = vec![
        LAN_CAPTURE_MACOS_CAPABILITY.to_string(),
        LAN_RENDER_MACOS_NATIVE_CAPABILITY.to_string(),
        "openh264_fallback".to_string(),
        "software_decode".to_string(),
    ];
    if probe.videotoolbox_h264_encoder {
        capabilities.push(LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string());
    }
    if probe.videotoolbox_hevc_encoder {
        capabilities.push(LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string());
        capabilities.push(LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string());
    }
    if probe.videotoolbox_h264_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string());
    }
    if probe.videotoolbox_hevc_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string());
    }
    if probe.videotoolbox_h264_decoder && probe.videotoolbox_hevc_decoder {
        capabilities.push(LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string());
    }
    capabilities
}

#[cfg(target_os = "macos")]
fn videotoolbox_decoder_enabled() -> bool {
    !matches!(
        std::env::var("MRD_DISABLE_VIDEOTOOLBOX_DECODER").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
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

async fn peer_control_addr_with_capture_source_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    if !peer_transports
        .iter()
        .any(|transport| transport.eq_ignore_ascii_case(LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT))
    {
        anyhow::bail!(
            "LAN peer does not advertise required capture source control [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT,
            peer_device_id.0,
            format_peer_transports(&peer_transports)
        );
    }
    Ok(target)
}

async fn peer_control_addr_with_display_mode_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    if !peer_transports
        .iter()
        .any(|transport| transport.eq_ignore_ascii_case(LAN_DISPLAY_MODE_CONTROL_TRANSPORT))
    {
        anyhow::bail!(
            "LAN peer does not advertise required display mode control [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            LAN_DISPLAY_MODE_CONTROL_TRANSPORT,
            peer_device_id.0,
            format_peer_transports(&peer_transports)
        );
    }
    Ok(target)
}

async fn peer_control_addr_with_input_control_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    if !peer_transports
        .iter()
        .any(|transport| transport.eq_ignore_ascii_case(LAN_INPUT_CONTROL_TRANSPORT))
    {
        anyhow::bail!(
            "LAN peer does not advertise required input control [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            LAN_INPUT_CONTROL_TRANSPORT,
            peer_device_id.0,
            format_peer_transports(&peer_transports)
        );
    }
    Ok(target)
}

async fn session_remote_peer(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<DeviceId> {
    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    snapshot
        .target_device_id
        .clone()
        .or_else(|| snapshot.source_device_id.clone())
        .with_context(|| format!("session has no remote peer: {}", session_id.0))
}

async fn local_device_id(app_state: &Arc<AppState>) -> Result<String> {
    let devices = app_state.devices.lock().await;
    devices
        .get_local_device()
        .map(|(id, _)| id.0.clone())
        .context("local device is not registered")
}

fn fit_capture_sources_ack_packet(
    instance_id: String,
    session_id: String,
    accepted: bool,
    message: Option<String>,
    sources: Vec<CaptureSource>,
) -> LanDiscoveryPacket {
    let sources = sources
        .into_iter()
        .map(strip_capture_source_preview)
        .collect();
    let mut packet = LanDiscoveryPacket::CaptureSourcesAck {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id,
        session_id,
        accepted,
        message,
        sources,
        timestamp_ms: now_ms(),
    };

    while serialized_packet_len(&packet) > DISCOVERY_SAFE_UDP_PAYLOAD_BYTES {
        let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = &mut packet else {
            break;
        };

        if sources.len() > 1 {
            sources.pop();
            continue;
        }

        break;
    }

    packet
}

fn strip_capture_source_preview(mut source: CaptureSource) -> CaptureSource {
    source.preview_data_url = None;
    source.preview_width = None;
    source.preview_height = None;
    source
}

fn serialized_packet_len(packet: &LanDiscoveryPacket) -> usize {
    serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
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
                LanAccessUnitCodec::Hevc => access_unit.is_keyframe,
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

fn create_lan_encoder(
    requested_codec: LanAccessUnitCodec,
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
    allow_h264_fallback: bool,
) -> Result<LanSenderEncoder> {
    match requested_codec {
        LanAccessUnitCodec::Hevc => {
            match create_lan_hevc_encoder(width, height, fps, bitrate, profile) {
                Ok((backend, encoder)) => Ok(LanSenderEncoder {
                    codec: LanAccessUnitCodec::Hevc,
                    backend,
                    encoder,
                }),
                Err(hevc_error) => {
                    if !allow_h264_fallback {
                        anyhow::bail!(
                        "HEVC unavailable ({hevc_error}); H.264 fallback blocked because the peer does not advertise H.264 receiver capability"
                    );
                    }
                    let (backend, encoder) =
                        create_lan_h264_encoder(width, height, fps, bitrate, profile)
                            .with_context(|| {
                                format!(
                                    "HEVC unavailable ({hevc_error}); H.264 fallback also failed"
                                )
                            })?;
                    Ok(LanSenderEncoder {
                        codec: LanAccessUnitCodec::H264,
                        backend,
                        encoder,
                    })
                }
            }
        }
        LanAccessUnitCodec::H264 => {
            let (backend, encoder) = create_lan_h264_encoder(width, height, fps, bitrate, profile)?;
            Ok(LanSenderEncoder {
                codec: LanAccessUnitCodec::H264,
                backend,
                encoder,
            })
        }
    }
}

fn lan_sender_allows_h264_encoder_fallback(
    requested_codec: LanAccessUnitCodec,
    peer_media_capabilities: &[String],
) -> bool {
    requested_codec == LanAccessUnitCodec::Hevc
        && peer_can_receive_codec(peer_media_capabilities, LanAccessUnitCodec::H264)
}

fn peer_can_receive_codec(peer_media_capabilities: &[String], codec: LanAccessUnitCodec) -> bool {
    let mut profile = default_media_profile();
    profile.codec = codec.name().to_string();
    missing_profile_receiver_media_capabilities(&profile, peer_media_capabilities).is_empty()
}

#[cfg(windows)]
fn create_lan_hevc_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let color_mode = lan_color_mode_for_profile(profile)?;
    if lan_profile_requests_hevc_main10(profile) {
        return mrd_encode_nvenc::NvencHevcEncoder::new_main10_with_bitrate(
            width, height, fps, bitrate,
        )
        .map(|encoder| {
            (
                "nvenc_hevc_main10",
                Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>,
            )
        })
        .map_err(|error| anyhow::anyhow!(error.to_string()));
    }
    match mrd_encode_nvenc::NvencHevcEncoder::new_max_speed_with_bitrate(
        width, height, fps, bitrate,
    ) {
        Ok(encoder) => Ok((
            "nvenc_hevc_p1_ultra_low_latency",
            Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>,
        )),
        Err(max_speed_error) => {
            mrd_encode_nvenc::NvencHevcEncoder::new_main_with_bitrate(width, height, fps, bitrate)
                .map(|encoder| {
                    (
                        "nvenc_hevc",
                        Box::new(encoder.with_color_mode(color_mode))
                            as Box<dyn VideoEncoder + Send>,
                    )
                })
                .map_err(|error| {
                    anyhow::anyhow!(
                        "nvenc_hevc_p1_ultra_low_latency: {max_speed_error}; nvenc_hevc: {error}"
                    )
                })
        }
    }
}

#[cfg(target_os = "macos")]
fn create_lan_hevc_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    if lan_profile_requests_hevc_main10(profile) {
        anyhow::bail!("VideoToolbox HEVC Main10 LAN encoding is unavailable");
    }
    let color_mode = lan_color_mode_for_profile(profile)?;
    if color_mode != ColorMode::Full {
        anyhow::bail!(
            "VideoToolbox HEVC LAN encoding does not support color_mode={}",
            color_mode.as_str()
        );
    }
    mrd_codec_videotoolbox::VideoToolboxHevcEncoder::new_with_bitrate(width, height, fps, bitrate)
        .map(|encoder| {
            (
                "videotoolbox_hevc",
                Box::new(encoder) as Box<dyn VideoEncoder + Send>,
            )
        })
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn create_lan_hevc_encoder(
    _width: usize,
    _height: usize,
    _fps: u32,
    _bitrate: u32,
    _profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    anyhow::bail!("NVENC HEVC is unavailable on this platform")
}

fn create_lan_h264_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let color_mode = lan_color_mode_for_profile(profile)?;
    let mut last_error = None;
    for backend in preferred_lan_h264_encoder_backends() {
        let encoder: Result<Box<dyn VideoEncoder + Send>> = match *backend {
            #[cfg(windows)]
            "nvenc_h264" => mrd_encode_nvenc::NvencH264Encoder::new_max_speed_with_bitrate(
                width, height, fps, bitrate,
            )
            .map(|encoder| {
                Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>
            })
            .map_err(|error| anyhow::anyhow!(error.to_string())),
            #[cfg(target_os = "macos")]
            "videotoolbox_h264" => {
                if color_mode != ColorMode::Full {
                    Err(anyhow::anyhow!(
                        "VideoToolbox H.264 LAN encoding does not support color_mode={}",
                        color_mode.as_str()
                    ))
                } else {
                    mrd_codec_videotoolbox::VideoToolboxH264Encoder::new_with_bitrate(
                        width, height, fps, bitrate,
                    )
                    .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                }
            }
            "openh264" => {
                if color_mode != ColorMode::Full {
                    Err(anyhow::anyhow!(
                        "OpenH264 LAN encoding does not support color_mode={}",
                        color_mode.as_str()
                    ))
                } else {
                    OpenH264Encoder::new_with_bitrate(width, height, fps, bitrate)
                        .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
                        .map_err(|error| anyhow::anyhow!(error.to_string()))
                }
            }
            _ => Err(anyhow::anyhow!(
                "unknown LAN H.264 encoder backend: {backend}"
            )),
        };
        match encoder {
            Ok(encoder) => return Ok((backend, encoder)),
            Err(error) => last_error = Some(format!("{backend}: {error}")),
        }
    }

    anyhow::bail!(
        "no LAN H.264 encoder available{}",
        last_error
            .map(|error| format!("; last error: {error}"))
            .unwrap_or_default()
    )
}

#[cfg(windows)]
fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["nvenc_h264", "openh264"]
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["openh264"]
}

#[cfg(target_os = "macos")]
fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["videotoolbox_h264", "openh264"]
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
    env_bool_override(
        std::env::var(LAN_CAPTURE_PUMP_REPEAT_LATEST_ENV)
            .ok()
            .as_deref(),
    )
    .unwrap_or(false)
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

fn env_bool_override(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        "" => None,
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_enabled() -> bool {
    macos_render_proxy_compressed_media_override().unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_override() -> Option<bool> {
    env_bool_override(
        std::env::var(MACOS_RENDER_PROXY_COMPRESSED_MEDIA_ENV)
            .ok()
            .as_deref(),
    )
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_enabled_for_profile(profile: &MediaProfile) -> bool {
    macos_render_proxy_compressed_media_enabled_for_values(
        profile.codec.as_str(),
        profile.width,
        profile.height,
        profile.fps,
        macos_render_proxy_compressed_media_override(),
    )
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_compressed_media_enabled_for_values(
    codec: &str,
    width: u32,
    height: u32,
    fps: u32,
    override_value: Option<bool>,
) -> bool {
    if let Some(enabled) = override_value {
        return enabled;
    }
    !(codec.trim().eq_ignore_ascii_case("hevc")
        && high_throughput_media_profile(width, height, fps))
}

#[cfg(target_os = "macos")]
fn high_throughput_media_profile(width: u32, height: u32, fps: u32) -> bool {
    fps >= 120 && u64::from(width).saturating_mul(u64::from(height)) >= 2_560_u64 * 1_440
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
    macos_render_proxy_compressed_media_enabled()
        && frame.payload_type == QuicMediaPayloadType::AccessUnit
        && matches!(frame.codec, QuicMediaCodec::H264 | QuicMediaCodec::Hevc)
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

async fn quic_media_v3_frame_to_legacy_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    frame: QuicMediaFrame,
    reassembler_stats: QuicAuReassemblerStats,
) -> Result<Option<QuicAuFrame>> {
    let profile = selected_media_profile(app_state, session_id).await;
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
            "LAN media receiver dropped stale v3 profile frame"
        );
        app_state.probes.lock().await.record_transient_frame_drop(
            session_id,
            frame.payload.len() as u64,
            now_ms(),
        );
        return Ok(None);
    }

    let payload_type = match frame.payload_type {
        QuicMediaPayloadType::AccessUnit => LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
        QuicMediaPayloadType::Probe => LAN_MEDIA_PAYLOAD_PROBE_FRAME,
        QuicMediaPayloadType::Control => 3,
    };
    let codec = match frame.codec {
        QuicMediaCodec::None => 0,
        QuicMediaCodec::H264 => LAN_MEDIA_CODEC_H264,
        QuicMediaCodec::Hevc => LAN_MEDIA_CODEC_HEVC,
        unsupported => {
            anyhow::bail!("unsupported LAN media v3 codec: {unsupported:?}");
        }
    };
    if frame.payload_type == QuicMediaPayloadType::AccessUnit
        && !matches!(codec, LAN_MEDIA_CODEC_H264 | LAN_MEDIA_CODEC_HEVC)
    {
        anyhow::bail!("LAN media v3 access unit has unsupported codec: {codec}");
    }

    let mut envelope_profile = profile;
    if frame.payload_type == QuicMediaPayloadType::AccessUnit && codec != 0 {
        envelope_profile.codec = lan_media_codec_name(codec).to_string();
        normalize_lan_media_profile(&mut envelope_profile);
    }

    let envelope_payload = encode_lan_media_envelope(LanMediaEnvelope {
        payload_type,
        codec,
        sequence: u64::from(frame.frame_id),
        timestamp_us: frame.timestamp_us,
        profile: envelope_profile,
        payload: frame.payload.to_vec(),
    })?;

    Ok(Some(QuicAuFrame {
        frame_id: frame.frame_id,
        timestamp_us: frame.timestamp_us,
        is_keyframe: frame.is_keyframe(),
        payload: envelope_payload.into(),
    }))
}

#[allow(clippy::too_many_arguments)]
async fn record_lan_decoded_frames(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    decoded_frames: Vec<DecodedFrame>,
    bytes_received: u64,
    sequence: u64,
    timestamp_us: u64,
    profile: &MediaProfile,
    encoded_payload: &[u8],
) {
    for decoded_frame in decoded_frames {
        let width = decoded_frame.width as u32;
        let height = decoded_frame.height as u32;
        let decoded_pixel_format = decoded_frame_pixel_format(&decoded_frame);

        #[cfg(any(windows, target_os = "macos"))]
        if let Err(error) = render_lan_decoded_frame(app_state, session_id, decoded_frame).await {
            tracing::warn!(
                %error,
                session_id = %session_id.0,
                sequence,
                "LAN media receiver failed to present decoded frame"
            );
        }

        app_state
            .media_pipelines
            .lock()
            .await
            .record_active_media_sample(
                session_id.clone(),
                profile,
                width,
                height,
                decoded_pixel_format.clone(),
            );
        app_state
            .media_pipelines
            .lock()
            .await
            .record_stage_duration_ms(
                session_id.clone(),
                format!("receiver.format.{decoded_pixel_format}"),
                1.0,
            );
        let payload_hash =
            lan_media_payload_hash_for_profile(profile, sequence, timestamp_us, encoded_payload);

        app_state.probes.lock().await.record_decoded_video_frame(
            session_id,
            DecodedVideoFrameStats {
                bytes_received,
                sequence,
                timestamp_us,
                width,
                height,
                target_fps: profile.fps,
                target_bitrate_mbps: profile.bitrate_mbps,
                encoded_bytes: encoded_payload.len() as u32,
                format: decoded_video_probe_format(&profile.codec),
                pixel_format: decoded_pixel_format,
                payload_hash,
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            now_ms(),
        );
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Debug)]
enum LanRenderTaskOutcome {
    Rendered {
        upload_duration_ms: f64,
        render_proxy_upload_ms: Option<f64>,
        render_proxy_transport_ms: Option<f64>,
        render_proxy_decode_ms: Option<f64>,
        render_proxy_draw_present_ms: Option<f64>,
        render_proxy_next_drawable_ms: Option<f64>,
        render_proxy_encode_commit_ms: Option<f64>,
        lock_wait_ms: f64,
        presented_frames: u64,
        present_skips: u64,
        waitable_wait_ms: f64,
        waitable_waits: u64,
        waitable_timeouts: u64,
    },
    Dropped,
    Idle,
}

#[cfg(any(windows, target_os = "macos"))]
async fn render_lan_decoded_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    decoded_frame: DecodedFrame,
) -> Result<()> {
    if app_state
        .media_surface_renderers
        .lock()
        .await
        .session_surface_count(session_id)
        == 0
    {
        return Ok(());
    }

    let render_frame = MediaRenderFrame::Decoded(decoded_frame_to_render_frame(decoded_frame)?);
    let render_profile = selected_media_profile(app_state, session_id).await;
    let render_queue_policy = lan_render_queue_policy_for_profile(&render_profile);
    let max_pending_frames =
        lan_render_queue_capacity_for_policy(&render_profile, render_queue_policy);
    let render_pacing_target_fps = lan_render_cap_target_fps_for_profile(&render_profile);
    let (enqueue, enqueue_gap_ms) = {
        let mut render_queues = app_state.media_render_queues.lock().await;
        let now = Instant::now();
        let enqueue_gap_ms = render_queues
            .record_enqueued(session_id, now)
            .map(duration_as_millis);
        let enqueue =
            render_queues.enqueue_bounded(session_id.clone(), render_frame, max_pending_frames);
        (enqueue, enqueue_gap_ms)
    };
    if let Some(enqueue_gap_ms) = enqueue_gap_ms {
        let mut pipelines = app_state.media_pipelines.lock().await;
        pipelines.set_render_pacing_target_fps(session_id.clone(), render_pacing_target_fps);
        pipelines.set_render_queue_policy(session_id.clone(), Some(render_queue_policy.as_str()));
        pipelines.record_stage_duration_ms(
            session_id.clone(),
            "render_enqueue_gap",
            enqueue_gap_ms,
        );
    } else {
        app_state
            .media_pipelines
            .lock()
            .await
            .set_render_pacing_target_fps(session_id.clone(), render_pacing_target_fps);
        app_state
            .media_pipelines
            .lock()
            .await
            .set_render_queue_policy(session_id.clone(), Some(render_queue_policy.as_str()));
    }
    match enqueue {
        MediaRenderQueueEnqueue::Start(frame) => {
            spawn_lan_render_worker(app_state.clone(), session_id.clone(), frame);
        }
        MediaRenderQueueEnqueue::Queued { replaced, depth } => {
            let mut pipelines = app_state.media_pipelines.lock().await;
            pipelines.record_queue_depth(session_id.clone(), depth as u32);
            if replaced {
                if render_queue_policy == LanRenderQueuePolicy::Latest {
                    pipelines.record_render_queue_replacements(session_id.clone(), 1);
                    pipelines.increment_render_stale_frame_drops(session_id.clone(), 1);
                } else {
                    pipelines.increment_render_queue_replacements(session_id.clone(), 1);
                }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn render_lan_h264_access_unit_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    payload: bytes::Bytes,
    sequence: u64,
    timestamp_us: u64,
    profile: &MediaProfile,
) -> Result<bool> {
    if !macos_render_proxy_compressed_media_enabled_for_profile(profile) {
        return Ok(false);
    }
    if app_state
        .media_surface_renderers
        .lock()
        .await
        .session_surface_count(session_id)
        == 0
    {
        return Ok(false);
    }

    let payload_len = payload.len();
    let payload_hash =
        lan_media_payload_hash_for_profile(profile, sequence, timestamp_us, &payload);
    let render_queue_policy = lan_render_queue_policy_for_profile(profile);
    let max_pending_frames = lan_render_queue_capacity_for_policy(profile, render_queue_policy);
    let render_pacing_target_fps = lan_render_cap_target_fps_for_profile(profile);
    let render_frame = MediaRenderFrame::H264AccessUnit {
        width: profile.width as usize,
        height: profile.height as usize,
        timestamp_us,
        payload,
    };
    let (enqueue, enqueue_gap_ms) = {
        let mut render_queues = app_state.media_render_queues.lock().await;
        let now = Instant::now();
        let enqueue_gap_ms = render_queues
            .record_enqueued(session_id, now)
            .map(duration_as_millis);
        let enqueue =
            render_queues.enqueue_bounded(session_id.clone(), render_frame, max_pending_frames);
        (enqueue, enqueue_gap_ms)
    };

    {
        let mut pipelines = app_state.media_pipelines.lock().await;
        pipelines.set_active_decoder(session_id.clone(), "rdesk_videotoolbox");
        pipelines.record_active_media_sample(
            session_id.clone(),
            profile,
            profile.width,
            profile.height,
            "proxy_h264",
        );
        pipelines.record_stage_duration_ms(session_id.clone(), "receiver.format.proxy_h264", 1.0);
        pipelines.set_render_pacing_target_fps(session_id.clone(), render_pacing_target_fps);
        pipelines.set_render_queue_policy(session_id.clone(), Some(render_queue_policy.as_str()));
        if let Some(enqueue_gap_ms) = enqueue_gap_ms {
            pipelines.record_stage_duration_ms(
                session_id.clone(),
                "render_enqueue_gap",
                enqueue_gap_ms,
            );
        }
    }

    match enqueue {
        MediaRenderQueueEnqueue::Start(frame) => {
            spawn_lan_render_worker(app_state.clone(), session_id.clone(), frame);
        }
        MediaRenderQueueEnqueue::Queued { replaced, depth } => {
            let mut pipelines = app_state.media_pipelines.lock().await;
            pipelines.record_queue_depth(session_id.clone(), depth as u32);
            if replaced {
                if render_queue_policy == LanRenderQueuePolicy::Latest {
                    pipelines.record_render_queue_replacements(session_id.clone(), 1);
                    pipelines.increment_render_stale_frame_drops(session_id.clone(), 1);
                } else {
                    pipelines.increment_render_queue_replacements(session_id.clone(), 1);
                }
            }
        }
    }

    app_state.probes.lock().await.record_decoded_video_frame(
        session_id,
        DecodedVideoFrameStats {
            bytes_received: payload_len as u64,
            sequence,
            timestamp_us,
            width: profile.width,
            height: profile.height,
            target_fps: profile.fps,
            target_bitrate_mbps: profile.bitrate_mbps,
            encoded_bytes: payload_len as u32,
            format: decoded_video_probe_format(&profile.codec),
            pixel_format: "proxy_h264".to_string(),
            payload_hash,
            preview_width: None,
            preview_height: None,
            rgb24: None,
        },
        now_ms(),
    );
    Ok(true)
}

#[cfg(target_os = "macos")]
async fn render_lan_hevc_access_unit_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    payload: bytes::Bytes,
    sequence: u64,
    timestamp_us: u64,
    profile: &MediaProfile,
) -> Result<bool> {
    if !macos_render_proxy_compressed_media_enabled_for_profile(profile) {
        return Ok(false);
    }
    if app_state
        .media_surface_renderers
        .lock()
        .await
        .session_surface_count(session_id)
        == 0
    {
        return Ok(false);
    }

    let payload_len = payload.len();
    let payload_hash =
        lan_media_payload_hash_for_profile(profile, sequence, timestamp_us, &payload);
    let render_queue_policy = lan_render_queue_policy_for_profile(profile);
    let max_pending_frames = lan_render_queue_capacity_for_policy(profile, render_queue_policy);
    let render_pacing_target_fps = lan_render_cap_target_fps_for_profile(profile);
    let render_frame = MediaRenderFrame::HevcAccessUnit {
        width: profile.width as usize,
        height: profile.height as usize,
        timestamp_us,
        payload,
    };
    let (enqueue, enqueue_gap_ms) = {
        let mut render_queues = app_state.media_render_queues.lock().await;
        let now = Instant::now();
        let enqueue_gap_ms = render_queues
            .record_enqueued(session_id, now)
            .map(duration_as_millis);
        let enqueue =
            render_queues.enqueue_bounded(session_id.clone(), render_frame, max_pending_frames);
        (enqueue, enqueue_gap_ms)
    };

    {
        let mut pipelines = app_state.media_pipelines.lock().await;
        pipelines.set_active_decoder(session_id.clone(), "rdesk_videotoolbox_hevc");
        pipelines.record_active_media_sample(
            session_id.clone(),
            profile,
            profile.width,
            profile.height,
            "proxy_hevc",
        );
        pipelines.record_stage_duration_ms(session_id.clone(), "receiver.format.proxy_hevc", 1.0);
        pipelines.set_render_pacing_target_fps(session_id.clone(), render_pacing_target_fps);
        pipelines.set_render_queue_policy(session_id.clone(), Some(render_queue_policy.as_str()));
        if let Some(enqueue_gap_ms) = enqueue_gap_ms {
            pipelines.record_stage_duration_ms(
                session_id.clone(),
                "render_enqueue_gap",
                enqueue_gap_ms,
            );
        }
    }

    match enqueue {
        MediaRenderQueueEnqueue::Start(frame) => {
            spawn_lan_render_worker(app_state.clone(), session_id.clone(), frame);
        }
        MediaRenderQueueEnqueue::Queued { replaced, depth } => {
            let mut pipelines = app_state.media_pipelines.lock().await;
            pipelines.record_queue_depth(session_id.clone(), depth as u32);
            if replaced {
                if render_queue_policy == LanRenderQueuePolicy::Latest {
                    pipelines.record_render_queue_replacements(session_id.clone(), 1);
                    pipelines.increment_render_stale_frame_drops(session_id.clone(), 1);
                } else {
                    pipelines.increment_render_queue_replacements(session_id.clone(), 1);
                }
            }
        }
    }

    app_state.probes.lock().await.record_decoded_video_frame(
        session_id,
        DecodedVideoFrameStats {
            bytes_received: payload_len as u64,
            sequence,
            timestamp_us,
            width: profile.width,
            height: profile.height,
            target_fps: profile.fps,
            target_bitrate_mbps: profile.bitrate_mbps,
            encoded_bytes: payload_len as u32,
            format: decoded_video_probe_format(&profile.codec),
            pixel_format: "proxy_hevc".to_string(),
            payload_hash,
            preview_width: None,
            preview_height: None,
            rgb24: None,
        },
        now_ms(),
    );
    Ok(true)
}

#[cfg(any(windows, target_os = "macos"))]
fn spawn_lan_render_worker(
    app_state: Arc<AppState>,
    session_id: SessionId,
    first_frame: MediaRenderFrame,
) {
    let fallback_app_state = app_state.clone();
    let fallback_session_id = session_id.clone();
    let fallback_first_frame = first_frame.clone();
    let handle = tokio::runtime::Handle::current();
    let spawn_result = thread::Builder::new()
        .name("mrd-lan-render".to_string())
        .spawn(move || {
            #[cfg(windows)]
            configure_lan_render_thread_priority();
            handle.block_on(run_lan_render_worker(app_state, session_id, first_frame));
        });

    if let Err(error) = spawn_result {
        tracing::warn!(
            %error,
            session_id = %fallback_session_id.0,
            "failed to spawn dedicated LAN render thread; falling back to Tokio task"
        );
        tokio::spawn(run_lan_render_worker(
            fallback_app_state,
            fallback_session_id,
            fallback_first_frame,
        ));
    }
}

#[cfg(windows)]
fn configure_lan_render_thread_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
    };

    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.is_err() {
        tracing::debug!("failed to raise LAN render thread priority");
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn run_lan_render_worker(
    app_state: Arc<AppState>,
    session_id: SessionId,
    first_frame: MediaRenderFrame,
) {
    let mut frame = first_frame;
    let mut timer_resolution = MediaTimerResolution::default();
    loop {
        let render_profile = selected_media_profile(&app_state, &session_id).await;
        let render_queue_policy = lan_render_queue_policy_for_profile(&render_profile);
        if render_profile_requests_high_resolution_timer(&render_profile) {
            timer_resolution.request();
        } else {
            timer_resolution.release();
        }
        pace_lan_render_frame(
            &app_state,
            &session_id,
            &render_profile,
            render_queue_policy,
        )
        .await;
        match render_lan_frame_once(app_state.clone(), session_id.clone(), frame).await {
            Ok(LanRenderTaskOutcome::Rendered {
                upload_duration_ms,
                render_proxy_upload_ms,
                render_proxy_transport_ms,
                render_proxy_decode_ms,
                render_proxy_draw_present_ms,
                render_proxy_next_drawable_ms,
                render_proxy_encode_commit_ms,
                lock_wait_ms,
                presented_frames,
                present_skips,
                waitable_wait_ms,
                waitable_waits,
                waitable_timeouts,
            }) => {
                {
                    let mut pipelines = app_state.media_pipelines.lock().await;
                    pipelines.record_stage_duration_ms(
                        session_id.clone(),
                        "render_upload",
                        upload_duration_ms,
                    );
                    if let Some(render_proxy_upload_ms) = render_proxy_upload_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_upload",
                            render_proxy_upload_ms,
                        );
                    }
                    if let Some(render_proxy_transport_ms) = render_proxy_transport_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_transport",
                            render_proxy_transport_ms,
                        );
                    }
                    if let Some(render_proxy_decode_ms) = render_proxy_decode_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_decode",
                            render_proxy_decode_ms,
                        );
                    }
                    if let Some(render_proxy_draw_present_ms) = render_proxy_draw_present_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_draw_present",
                            render_proxy_draw_present_ms,
                        );
                    }
                    if let Some(render_proxy_next_drawable_ms) = render_proxy_next_drawable_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_next_drawable",
                            render_proxy_next_drawable_ms,
                        );
                    }
                    if let Some(render_proxy_encode_commit_ms) = render_proxy_encode_commit_ms {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_proxy_encode_commit",
                            render_proxy_encode_commit_ms,
                        );
                    }
                    if lock_wait_ms > 0.0 {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_lock_wait",
                            lock_wait_ms,
                        );
                    }
                    if presented_frames > 0 {
                        pipelines.increment_render_presented_frames(
                            session_id.clone(),
                            presented_frames,
                        );
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_present",
                            upload_duration_ms,
                        );
                    }
                    if present_skips > 0 {
                        pipelines.increment_render_present_skips(session_id.clone(), present_skips);
                    }
                    if waitable_waits > 0 {
                        pipelines.record_stage_duration_ms(
                            session_id.clone(),
                            "render_waitable_wait",
                            waitable_wait_ms / waitable_waits as f64,
                        );
                    }
                    if waitable_timeouts > 0 {
                        pipelines.increment_render_waitable_timeouts(
                            session_id.clone(),
                            waitable_timeouts,
                        );
                    }
                }
                if presented_frames > 0 {
                    let present_gap_ms = app_state
                        .media_render_queues
                        .lock()
                        .await
                        .record_presented(&session_id, Instant::now())
                        .map(duration_as_millis);
                    if let Some(present_gap_ms) = present_gap_ms {
                        app_state
                            .media_pipelines
                            .lock()
                            .await
                            .record_stage_duration_ms(
                                session_id.clone(),
                                "render_present_gap",
                                present_gap_ms,
                            );
                    }
                }
            }
            Ok(LanRenderTaskOutcome::Dropped) => {
                app_state
                    .media_pipelines
                    .lock()
                    .await
                    .increment_render_lock_drops(session_id.clone(), 1);
            }
            Ok(LanRenderTaskOutcome::Idle) => {}
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %session_id.0,
                    "LAN media receiver failed to present decoded frame"
                );
            }
        }

        let (next_frame, stale_drops) = {
            let mut render_queues = app_state.media_render_queues.lock().await;
            take_next_lan_render_frame_for_policy(
                &mut render_queues,
                &session_id,
                render_queue_policy,
            )
        };
        if stale_drops > 0 {
            app_state
                .media_pipelines
                .lock()
                .await
                .increment_render_stale_frame_drops(session_id.clone(), stale_drops as u64);
        }
        match next_frame {
            Some(next_frame) => {
                let mut pipelines = app_state.media_pipelines.lock().await;
                pipelines.record_queue_depth(session_id.clone(), 0);
                frame = next_frame;
            }
            None => {
                app_state
                    .media_pipelines
                    .lock()
                    .await
                    .record_queue_depth(session_id.clone(), 0);
                break;
            }
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn pace_lan_render_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    profile: &MediaProfile,
    policy: LanRenderQueuePolicy,
) {
    if !lan_render_policy_allows_service_pacing(
        policy,
        profile,
        native_render_waitable_swapchain_pacing_enabled(),
    ) {
        return;
    }

    let target_fps = lan_render_pacing_target_fps(profile);
    let max_pending_frames = lan_render_queue_capacity_for_profile(profile);
    let delay = app_state.media_render_queues.lock().await.pacing_delay(
        session_id,
        target_fps,
        Instant::now(),
    );
    let delay = lan_render_pacing_render_start_delay(delay, target_fps);
    if delay < Duration::from_micros(500) {
        return;
    }

    let started = Instant::now();
    let interrupted = sleep_until_lan_render_frame(
        app_state,
        session_id,
        target_fps,
        max_pending_frames,
        started + delay,
    )
    .await;
    app_state
        .media_pipelines
        .lock()
        .await
        .record_stage_duration_ms(
            session_id.clone(),
            "render_pacing_wait",
            started.elapsed().as_secs_f64() * 1000.0,
        );
    if interrupted {
        app_state
            .media_pipelines
            .lock()
            .await
            .record_stage_duration_ms(session_id.clone(), "render_pacing_interrupt", 1.0);
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn sleep_until_lan_render_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    target_fps: u32,
    max_pending_frames: usize,
    deadline: Instant,
) -> bool {
    let guard = render_pacing_precise_sleep_guard(target_fps);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }

        let pending_depth = app_state
            .media_render_queues
            .lock()
            .await
            .pending_depth(session_id);
        if should_interrupt_render_pacing_sleep(pending_depth, max_pending_frames) {
            return true;
        }

        let remaining = deadline - now;
        if remaining > guard {
            let sleep_for = (remaining - guard).min(LAN_RENDER_PACING_POLL_INTERVAL);
            std::thread::sleep(sleep_for);
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn take_next_lan_render_frame_for_policy(
    render_queues: &mut MediaRenderQueueRegistry,
    session_id: &SessionId,
    policy: LanRenderQueuePolicy,
) -> (Option<MediaRenderFrame>, usize) {
    match policy {
        LanRenderQueuePolicy::Latest => render_queues.take_latest_or_finish(session_id),
        LanRenderQueuePolicy::PacedFifo => (render_queues.take_next_or_finish(session_id), 0),
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn render_lan_frame_once(
    app_state: Arc<AppState>,
    session_id: SessionId,
    frame: MediaRenderFrame,
) -> Result<LanRenderTaskOutcome> {
    let renderers = {
        let render_registry = app_state.media_surface_renderers.lock().await;
        render_registry.renderers_for_session(&session_id)
    };
    if renderers.is_empty() {
        let no_surface_count = LAN_RENDER_NO_SURFACE_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if no_surface_count <= 5 || no_surface_count % 120 == 0 {
            tracing::warn!(
                session_id = %session_id.0,
                no_surface_count,
                "lan-render no surface renderer for session"
            );
        }
        return Ok(LanRenderTaskOutcome::Idle);
    }

    let mut rendered = 0;
    let mut upload_duration_ms = 0.0_f64;
    let mut lock_wait_ms = 0.0_f64;
    let mut presented_frames = 0_u64;
    let mut present_skips = 0_u64;
    let mut render_queue_replacements = 0_u64;
    let mut waitable_wait_ms = 0.0_f64;
    let mut waitable_waits = 0_u64;
    let mut waitable_timeouts = 0_u64;
    let mut render_proxy_upload_ms = 0.0_f64;
    let mut render_proxy_transport_ms = 0.0_f64;
    let mut render_proxy_decode_ms = 0.0_f64;
    let mut render_proxy_draw_present_ms = 0.0_f64;
    let mut render_proxy_next_drawable_ms = 0.0_f64;
    let mut render_proxy_encode_commit_ms = 0.0_f64;
    let mut render_proxy_samples = 0_u64;
    let mut render_proxy_decode_samples = 0_u64;
    let mut render_proxy_draw_present_samples = 0_u64;
    let mut render_proxy_next_drawable_samples = 0_u64;
    let mut render_proxy_encode_commit_samples = 0_u64;
    let mut renderer_snapshots = Vec::<RendererSnapshot>::new();
    let renderer_count = renderers.len();
    let mut frame_for_last_renderer = Some(frame);
    for (renderer_index, renderer) in renderers.iter().enumerate() {
        let lock_started = Instant::now();
        let Some(mut renderer) =
            wait_for_mutex_guard(renderer.as_ref(), LAN_RENDER_SURFACE_RENDERER_LOCK_TIMEOUT)
                .map_err(|error| anyhow::anyhow!(error))?
        else {
            lock_wait_ms += lock_started.elapsed().as_secs_f64() * 1000.0;
            if rendered == 0 {
                return Ok(LanRenderTaskOutcome::Dropped);
            }
            continue;
        };
        lock_wait_ms += lock_started.elapsed().as_secs_f64() * 1000.0;
        let before = renderer.snapshot();
        let upload_started = Instant::now();
        let frame_for_renderer = if renderer_index + 1 == renderer_count {
            frame_for_last_renderer
                .take()
                .ok_or_else(|| anyhow::anyhow!("render frame was already consumed"))?
        } else {
            frame_for_last_renderer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("render frame was already consumed"))?
                .clone()
        };
        upload_lan_render_frame(renderer.as_mut(), frame_for_renderer)
            .map_err(|error| anyhow::anyhow!("upload frame to native renderer failed: {error}"))?;
        let after = renderer.snapshot();
        let wait_delta = renderer_snapshot_waitable_delta(&before, &after);
        let upload_elapsed_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
        let upload_without_wait_ms = (upload_elapsed_ms - wait_delta.wait_ms).max(0.0);
        upload_duration_ms += upload_without_wait_ms;
        if renderer_snapshot_uses_render_proxy(&after) {
            if let Some(proxy_upload_ms) = after.last_render_draw_present_ms {
                render_proxy_upload_ms += proxy_upload_ms;
                render_proxy_transport_ms += (upload_without_wait_ms - proxy_upload_ms).max(0.0);
                if let Some(proxy_decode_ms) = after.last_render_prepare_wait_ms {
                    render_proxy_decode_ms += proxy_decode_ms;
                    render_proxy_decode_samples = render_proxy_decode_samples.saturating_add(1);
                }
                if let Some(proxy_draw_present_ms) = after.last_render_shared_resource_ms {
                    render_proxy_draw_present_ms += proxy_draw_present_ms;
                    render_proxy_draw_present_samples =
                        render_proxy_draw_present_samples.saturating_add(1);
                }
                if let Some(proxy_next_drawable_ms) = after.last_render_wait_for_drawable_ms {
                    render_proxy_next_drawable_ms += proxy_next_drawable_ms;
                    render_proxy_next_drawable_samples =
                        render_proxy_next_drawable_samples.saturating_add(1);
                }
                if let Some(proxy_encode_commit_ms) = after.last_render_encode_commit_ms {
                    render_proxy_encode_commit_ms += proxy_encode_commit_ms;
                    render_proxy_encode_commit_samples =
                        render_proxy_encode_commit_samples.saturating_add(1);
                }
                render_proxy_samples = render_proxy_samples.saturating_add(1);
            }
        }
        waitable_wait_ms += wait_delta.wait_ms;
        waitable_waits = waitable_waits.saturating_add(wait_delta.waits);
        waitable_timeouts = waitable_timeouts.saturating_add(wait_delta.timeouts);
        render_queue_replacements = render_queue_replacements.saturating_add(
            renderer_snapshot_render_queue_replacement_delta(&before, &after),
        );
        let uploaded_delta = after
            .uploaded_frame_count
            .saturating_sub(before.uploaded_frame_count);
        let mut presented_delta = after
            .presented_frame_count
            .saturating_sub(before.presented_frame_count);
        let skipped_delta = after
            .present_skipped_count
            .saturating_sub(before.present_skipped_count);
        if uploaded_delta > 0
            && presented_delta == 0
            && skipped_delta == 0
            && after.last_present_status.is_none()
        {
            presented_delta = uploaded_delta;
        }
        presented_frames = presented_frames.saturating_add(presented_delta);
        present_skips = present_skips.saturating_add(skipped_delta);
        renderer_snapshots.push(after);
        rendered += 1;
    }

    if rendered > 0 {
        {
            let mut pipelines = app_state.media_pipelines.lock().await;
            for snapshot in &renderer_snapshots {
                pipelines.record_renderer_snapshot(session_id.clone(), snapshot);
            }
            pipelines
                .record_render_queue_replacements(session_id.clone(), render_queue_replacements);
        }
        let present_log_count = LAN_RENDER_PRESENT_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if present_log_count <= 5 || present_log_count % 120 == 0 {
            tracing::info!(
                session_id = %session_id.0,
                renderer_count = renderers.len(),
                rendered,
                presented_frames,
                present_skips,
                "lan-render uploaded frame to native surface"
            );
        }
        Ok(LanRenderTaskOutcome::Rendered {
            upload_duration_ms,
            render_proxy_upload_ms: (render_proxy_samples > 0).then_some(render_proxy_upload_ms),
            render_proxy_transport_ms: (render_proxy_samples > 0)
                .then_some(render_proxy_transport_ms),
            render_proxy_decode_ms: (render_proxy_decode_samples > 0)
                .then_some(render_proxy_decode_ms),
            render_proxy_draw_present_ms: (render_proxy_draw_present_samples > 0)
                .then_some(render_proxy_draw_present_ms),
            render_proxy_next_drawable_ms: (render_proxy_next_drawable_samples > 0)
                .then_some(render_proxy_next_drawable_ms),
            render_proxy_encode_commit_ms: (render_proxy_encode_commit_samples > 0)
                .then_some(render_proxy_encode_commit_ms),
            lock_wait_ms,
            presented_frames,
            present_skips,
            waitable_wait_ms,
            waitable_waits,
            waitable_timeouts,
        })
    } else {
        Ok(LanRenderTaskOutcome::Idle)
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn renderer_snapshot_uses_render_proxy(snapshot: &RendererSnapshot) -> bool {
    snapshot
        .swap_chain_present_mode
        .as_deref()
        .is_some_and(|mode| mode.starts_with("render_proxy"))
}

#[cfg(any(windows, target_os = "macos"))]
fn renderer_snapshot_render_queue_replacement_delta(
    before: &RendererSnapshot,
    after: &RendererSnapshot,
) -> u64 {
    after
        .render_queue_replacements
        .unwrap_or_default()
        .saturating_sub(before.render_queue_replacements.unwrap_or_default())
}

#[cfg(any(windows, target_os = "macos"))]
fn upload_lan_render_frame(
    renderer: &mut dyn mrd_render::RendererInstance,
    frame: MediaRenderFrame,
) -> Result<(), mrd_render::RenderError> {
    match frame {
        MediaRenderFrame::Decoded(frame) => renderer.upload_frame(frame),
        #[cfg(target_os = "macos")]
        MediaRenderFrame::H264AccessUnit {
            width,
            height,
            timestamp_us,
            payload,
        } => renderer.upload_h264_access_unit(width, height, timestamp_us, payload),
        #[cfg(target_os = "macos")]
        MediaRenderFrame::HevcAccessUnit {
            width,
            height,
            timestamp_us,
            payload,
        } => renderer.upload_hevc_access_unit(width, height, timestamp_us, payload),
    }
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
struct RendererWaitableDelta {
    wait_ms: f64,
    waits: u64,
    timeouts: u64,
}

#[cfg(any(windows, target_os = "macos"))]
fn renderer_snapshot_waitable_delta(
    before: &RendererSnapshot,
    after: &RendererSnapshot,
) -> RendererWaitableDelta {
    let before_waits = before.waitable_wait_count.unwrap_or_default();
    let after_waits = after.waitable_wait_count.unwrap_or_default();
    let before_total = before.waitable_wait_total_ms.unwrap_or_default();
    let after_total = after.waitable_wait_total_ms.unwrap_or_default();
    let before_timeouts = before.waitable_timeout_count.unwrap_or_default();
    let after_timeouts = after.waitable_timeout_count.unwrap_or_default();
    RendererWaitableDelta {
        wait_ms: (after_total - before_total).max(0.0),
        waits: after_waits.saturating_sub(before_waits),
        timeouts: after_timeouts.saturating_sub(before_timeouts),
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn wait_for_mutex_guard<'a, T>(
    mutex: &'a StdMutex<T>,
    wait_timeout: Duration,
) -> Result<Option<StdMutexGuard<'a, T>>, String> {
    let started = std::time::Instant::now();
    let mut spins = 0;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(Some(guard)),
            Err(TryLockError::Poisoned(_)) => {
                return Err("native renderer lock was poisoned".into())
            }
            Err(TryLockError::WouldBlock) => {
                if started.elapsed() >= wait_timeout {
                    return Ok(None);
                }
                if spins < 16 {
                    spins += 1;
                    std::hint::spin_loop();
                } else {
                    std::thread::sleep(LAN_RENDER_SURFACE_RENDERER_LOCK_POLL_INTERVAL);
                }
            }
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn decoded_frame_to_render_frame(frame: DecodedFrame) -> Result<RenderFrame> {
    match frame.data {
        DecodedFrameData::CpuRgb24(data) => {
            let expected_len = frame
                .width
                .checked_mul(frame.height)
                .and_then(|pixels| pixels.checked_mul(3))
                .ok_or_else(|| anyhow::anyhow!("decoded RGB render frame byte size overflow"))?;
            if data.len() != expected_len {
                anyhow::bail!("decoded RGB render frame has invalid byte length");
            }
            Ok(RenderFrame::from_rgb24(frame.width, frame.height, data))
        }
        DecodedFrameData::CpuBgra32(data) => {
            let expected_len = frame
                .width
                .checked_mul(frame.height)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| anyhow::anyhow!("decoded BGRA render frame byte size overflow"))?;
            if data.len() != expected_len {
                anyhow::bail!("decoded BGRA render frame has invalid byte length");
            }
            Ok(RenderFrame::from_bgra32(frame.width, frame.height, data))
        }
        DecodedFrameData::CpuNv12 { data, pitch } => {
            if pitch < frame.width {
                anyhow::bail!("decoded NV12 render frame pitch is smaller than width");
            }
            let y_bytes = pitch
                .checked_mul(frame.height)
                .ok_or_else(|| anyhow::anyhow!("decoded NV12 luma byte size overflow"))?;
            let uv_bytes = pitch
                .checked_mul(frame.height.div_ceil(2))
                .ok_or_else(|| anyhow::anyhow!("decoded NV12 chroma byte size overflow"))?;
            let expected_len = y_bytes
                .checked_add(uv_bytes)
                .ok_or_else(|| anyhow::anyhow!("decoded NV12 byte size overflow"))?;
            if data.len() < expected_len {
                anyhow::bail!("decoded NV12 render frame has invalid byte length");
            }
            Ok(RenderFrame::from_nv12(
                frame.width,
                frame.height,
                data,
                pitch,
            ))
        }
        DecodedFrameData::CpuI420 { .. } => {
            let (width, height, rgb24) = decoded_frame_to_rgb24(frame)?;
            Ok(RenderFrame::from_rgb24(
                width as usize,
                height as usize,
                rgb24,
            ))
        }
        DecodedFrameData::CpuP010 { .. } => {
            anyhow::bail!("CPU P010 decoded frames are not supported by the native renderer yet")
        }
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 {
            shared_handle_y,
            shared_handle_uv,
            ..
        } => Ok(RenderFrame::from_d3d11_shared_nv12(
            frame.width,
            frame.height,
            shared_handle_y,
            shared_handle_uv,
        )),
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedP010 {
            shared_handle_y,
            shared_handle_uv,
            ..
        } => Ok(RenderFrame::from_d3d11_shared_p010(
            frame.width,
            frame.height,
            shared_handle_y,
            shared_handle_uv,
        )),
    }
}

struct LanReceiverDecoder {
    codec: LanAccessUnitCodec,
    backend: &'static str,
    decoder: Box<dyn VideoDecoder>,
}

async fn create_lan_receiver_decoder(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<LanReceiverDecoder> {
    let profile = selected_media_profile(app_state, session_id).await;
    let requested_codec = LanAccessUnitCodec::from_profile(&profile);
    match create_lan_receiver_decoder_with_preference(app_state, session_id, requested_codec, None)
        .await
    {
        Ok(decoder) => Ok(decoder),
        Err(error) if requested_codec == LanAccessUnitCodec::Hevc => {
            app_state
                .media_pipelines
                .lock()
                .await
                .set_codec_fallback_reason(
                    session_id.clone(),
                    Some(format!(
                        "{} receiver unavailable; fell back to H.264: {error:#}",
                        requested_codec.display_name()
                    )),
                );
            create_lan_receiver_decoder_with_preference(
                app_state,
                session_id,
                LanAccessUnitCodec::H264,
                None,
            )
            .await
        }
        Err(error) => Err(error),
    }
}

async fn create_lan_receiver_decoder_with_preference(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    codec: LanAccessUnitCodec,
    preferred_backend: Option<&'static str>,
) -> Result<LanReceiverDecoder> {
    let mut last_error = None;
    let selected_profile = selected_media_profile(app_state, session_id).await;
    for backend in lan_receiver_decoder_candidates(codec, preferred_backend) {
        match create_lan_video_decoder(backend) {
            Ok(decoder) => {
                let mut pipelines = app_state.media_pipelines.lock().await;
                pipelines.set_active_decoder(session_id.clone(), backend);
                let runtime_profile = lan_runtime_media_profile(&selected_profile, codec);
                pipelines.set_active_media_profile(session_id.clone(), &runtime_profile);
                return Ok(LanReceiverDecoder {
                    codec,
                    backend,
                    decoder,
                });
            }
            Err(error) => {
                last_error = Some(format!("{backend}: {error}"));
            }
        }
    }

    anyhow::bail!(
        "no LAN {} receiver decoder available{}",
        codec.display_name(),
        last_error
            .map(|error| format!("; last error: {error}"))
            .unwrap_or_default()
    )
}

async fn try_decode_h264_keyframe_with_fallback(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    failed_backend: &'static str,
    payload: &[u8],
    primary_error: &anyhow::Error,
) -> Result<(LanReceiverDecoder, Vec<DecodedFrame>)> {
    let mut errors = vec![format!("{failed_backend}: {primary_error:#}")];
    for backend in preferred_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264)
        .into_iter()
        .filter(|backend| *backend != failed_backend)
    {
        let mut decoder = match create_lan_video_decoder(backend) {
            Ok(decoder) => decoder,
            Err(error) => {
                errors.push(format!("{backend}: create failed: {error}"));
                continue;
            }
        };
        match decode_h264_desktop_frame(decoder.as_mut(), payload) {
            Ok(decoded_frames) if !decoded_frames.is_empty() => {
                app_state
                    .media_pipelines
                    .lock()
                    .await
                    .set_active_decoder(session_id.clone(), backend);
                tracing::warn!(
                    session_id = %session_id.0,
                    failed_backend,
                    fallback_backend = backend,
                    primary_error = %primary_error,
                    "LAN media receiver switched decoder after keyframe decode failure"
                );
                return Ok((
                    LanReceiverDecoder {
                        codec: LanAccessUnitCodec::H264,
                        backend,
                        decoder,
                    },
                    decoded_frames,
                ));
            }
            Ok(_) => errors.push(format!("{backend}: decoded no frames")),
            Err(error) => errors.push(format!("{backend}: {error:#}")),
        }
    }

    anyhow::bail!(
        "all LAN H.264 receiver decoders failed for keyframe: {}",
        errors.join(" | ")
    )
}

fn create_lan_video_decoder(backend: &str) -> Result<Box<dyn VideoDecoder>> {
    #[cfg(target_os = "macos")]
    if backend == "videotoolbox" {
        return mrd_codec_videotoolbox::VideoToolboxH264Decoder::new()
            .map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>)
            .map_err(|error| anyhow::anyhow!(error.to_string()));
    }
    #[cfg(target_os = "macos")]
    if backend == "videotoolbox_hevc" {
        return mrd_codec_videotoolbox::VideoToolboxHevcDecoder::new()
            .map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>)
            .map_err(|error| anyhow::anyhow!(error.to_string()));
    }

    mrd_decode::create_decoder(backend).map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn session_allows_media(app_state: &Arc<AppState>, session_id: &SessionId) -> bool {
    let sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id) else {
        return false;
    };
    !snapshot.lifecycle_state.is_terminal()
}

async fn mark_session_failed(app_state: &Arc<AppState>, session_id: &SessionId, reason: String) {
    let mut sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id).cloned() else {
        return;
    };
    if snapshot.lifecycle_state == SessionLifecycleState::Closed {
        return;
    }
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
            lifecycle_state: SessionLifecycleState::Failed {
                message: reason.clone(),
            },
            last_error: Some(reason),
            sender_active: false,
            receiver_active: false,
            ..snapshot
        },
    );
}

enum LanFrameCapture {
    #[cfg(windows)]
    DxgiShared(mrd_capture_dxgi::DxgiSharedTextureCapture),
    #[cfg(windows)]
    Winrt(mrd_capture_winrt::WinrtCapture),
    #[cfg(target_os = "macos")]
    Macos(mrd_capture_macos::MacosScreenCapture),
    #[cfg(target_os = "macos")]
    MacosSyntheticCv(MacosSyntheticCvPixelBufferCapture),
    #[cfg(target_os = "linux")]
    Pipewire(mrd_capture_pipewire::PipewireScreenCapture),
    #[cfg(test)]
    Synthetic(SyntheticFrameCapture),
}

#[cfg(windows)]
unsafe impl Send for LanFrameCapture {}

impl LanFrameCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame> {
        match self {
            #[cfg(windows)]
            LanFrameCapture::DxgiShared(capture) => {
                mrd_pipeline_core::FrameCapture::capture_frame(capture)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
            }
            #[cfg(windows)]
            LanFrameCapture::Winrt(capture) => capture
                .capture_frame()
                .map_err(|error| anyhow::anyhow!(error.to_string())),
            #[cfg(target_os = "macos")]
            LanFrameCapture::Macos(capture) => capture
                .capture_frame()
                .map_err(|error| anyhow::anyhow!(error.to_string())),
            #[cfg(target_os = "macos")]
            LanFrameCapture::MacosSyntheticCv(capture) => capture.capture_frame(),
            #[cfg(target_os = "linux")]
            LanFrameCapture::Pipewire(capture) => capture
                .capture_frame()
                .map_err(|error| anyhow::anyhow!(error.to_string())),
            #[cfg(test)]
            LanFrameCapture::Synthetic(capture) => {
                Ok(mrd_pipeline_core::FrameCapture::capture_frame(capture)?)
            }
            #[cfg(not(any(windows, target_os = "macos", target_os = "linux", test)))]
            _ => anyhow::bail!("Frame capture not supported on this platform"),
        }
    }
}

enum LanSenderFrameCapture {
    Direct(LanFrameCapture),
    #[cfg(target_os = "macos")]
    Pumped(MacosPumpedLanFrameCapture),
}

struct LanCapturedSenderFrame {
    frame: CapturedFrame,
    repeated_latest_frame: bool,
}

impl LanSenderFrameCapture {
    fn new(capture: LanFrameCapture, _profile: &MediaProfile) -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            if matches!(capture, LanFrameCapture::Macos(_)) && lan_capture_pump_enabled() {
                return Ok(Self::Pumped(MacosPumpedLanFrameCapture::new(
                    capture,
                    macos_capture_pump_repeat_grace_timeout(_profile),
                )?));
            }
        }

        Ok(Self::Direct(capture))
    }

    fn capture_frame(&mut self) -> Result<LanCapturedSenderFrame> {
        match self {
            Self::Direct(capture) => Ok(LanCapturedSenderFrame {
                frame: capture.capture_frame()?,
                repeated_latest_frame: false,
            }),
            #[cfg(target_os = "macos")]
            Self::Pumped(capture) => capture.capture_frame(),
        }
    }

    fn drives_sender_pacing(&self) -> bool {
        match self {
            Self::Direct(_) => false,
            #[cfg(target_os = "macos")]
            Self::Pumped(_) => lan_capture_pump_drives_sender(),
        }
    }

    fn repeats_latest_frame(&self) -> bool {
        match self {
            Self::Direct(_) => false,
            #[cfg(target_os = "macos")]
            Self::Pumped(_) => lan_capture_pump_repeat_latest(),
        }
    }
}

#[cfg(target_os = "macos")]
struct MacosPumpedLanFrameCapture {
    shared: Arc<(StdMutex<MacosPumpedLanFrameState>, StdCondvar)>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    repeat_grace_timeout: Duration,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacosPumpedLanFrameState {
    frames: VecDeque<CapturedFrame>,
    latest_frame: Option<CapturedFrame>,
    sequence: u64,
    error: Option<String>,
}

#[cfg(target_os = "macos")]
impl MacosPumpedLanFrameCapture {
    fn new(mut capture: LanFrameCapture, repeat_grace_timeout: Duration) -> Result<Self> {
        let shared = Arc::new((
            StdMutex::new(MacosPumpedLanFrameState {
                frames: VecDeque::new(),
                latest_frame: None,
                sequence: 0,
                error: None,
            }),
            StdCondvar::new(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_shared = shared.clone();
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("mrd-lan-capture-pump".to_string())
            .spawn(move || {
                while !worker_stop.load(Ordering::Relaxed) {
                    match capture.capture_frame() {
                        Ok(frame) => {
                            let (lock, cvar) = &*worker_shared;
                            if let Ok(mut state) = lock.lock() {
                                while state.frames.len() >= LAN_CAPTURE_PUMP_QUEUE_CAPACITY {
                                    state.frames.pop_front();
                                }
                                state.latest_frame = Some(frame.clone());
                                state.frames.push_back(frame);
                                state.sequence = state.sequence.wrapping_add(1).max(1);
                                state.error = None;
                                cvar.notify_all();
                            }
                        }
                        Err(error) => {
                            let (lock, cvar) = &*worker_shared;
                            if let Ok(mut state) = lock.lock() {
                                state.error = Some(format!("{error:#}"));
                                cvar.notify_all();
                            }
                            thread::sleep(LAN_CAPTURE_PUMP_ERROR_BACKOFF);
                        }
                    }
                }
            })
            .context("failed to start macOS LAN capture pump")?;

        Ok(Self {
            shared,
            stop,
            worker: Some(worker),
            repeat_grace_timeout,
        })
    }

    fn capture_frame(&mut self) -> Result<LanCapturedSenderFrame> {
        let deadline = StdInstant::now() + LAN_CAPTURE_PUMP_WAIT_TIMEOUT;
        let (lock, cvar) = &*self.shared;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow::anyhow!("macOS LAN capture pump state poisoned"))?;
        let mut waited_for_repeat_grace = false;

        loop {
            if let Some(frame) = state.frames.pop_back() {
                state.frames.clear();
                return Ok(LanCapturedSenderFrame {
                    frame,
                    repeated_latest_frame: false,
                });
            }

            if let Some(error) = state.error.take() {
                anyhow::bail!("macOS LAN capture pump failed: {error}");
            }

            if lan_capture_pump_repeat_latest() {
                if state.latest_frame.is_some()
                    && !waited_for_repeat_grace
                    && !self.repeat_grace_timeout.is_zero()
                {
                    let now = StdInstant::now();
                    if now < deadline {
                        let wait = self
                            .repeat_grace_timeout
                            .min(deadline.saturating_duration_since(now));
                        let (guard, _) = cvar.wait_timeout(state, wait).map_err(|_| {
                            anyhow::anyhow!("macOS LAN capture pump state poisoned")
                        })?;
                        state = guard;
                        waited_for_repeat_grace = true;
                        continue;
                    }
                }

                if let Some(frame) = state.latest_frame.as_ref() {
                    let mut repeated = frame.clone();
                    repeated.timestamp_us = now_us();
                    return Ok(LanCapturedSenderFrame {
                        frame: repeated,
                        repeated_latest_frame: true,
                    });
                }
            }

            let now = StdInstant::now();
            if now >= deadline {
                anyhow::bail!("macOS LAN capture pump timed out waiting for a captured frame");
            }

            let wait = deadline.saturating_duration_since(now);
            let (guard, _) = cvar
                .wait_timeout(state, wait)
                .map_err(|_| anyhow::anyhow!("macOS LAN capture pump state poisoned"))?;
            state = guard;
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacosPumpedLanFrameCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.shared.1.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const std::ffi::c_void,
        width: usize,
        height: usize,
        pixel_format_type: u32,
        pixel_buffer_attributes: *const std::ffi::c_void,
        pixel_buffer_out: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
    fn CVPixelBufferGetHeightOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
const MACOS_SYNTHETIC_CV_SUCCESS: i32 = 0;
#[cfg(target_os = "macos")]
const MACOS_SYNTHETIC_CV_PIXEL_FORMAT_NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
#[cfg(target_os = "macos")]
const MACOS_SYNTHETIC_CV_BUFFER_POOL_CAPACITY: usize = 16;

#[cfg(target_os = "macos")]
struct MacosSyntheticCvPixelBuffer {
    ptr: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacosSyntheticCvPixelBuffer {}

#[cfg(target_os = "macos")]
impl MacosSyntheticCvPixelBuffer {
    fn new_nv12(width: usize, height: usize) -> Result<Self> {
        let mut pixel_buffer = std::ptr::null_mut();
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null(),
                width,
                height,
                MACOS_SYNTHETIC_CV_PIXEL_FORMAT_NV12_VIDEO_RANGE,
                std::ptr::null(),
                &mut pixel_buffer,
            )
        };
        if status != MACOS_SYNTHETIC_CV_SUCCESS || pixel_buffer.is_null() {
            anyhow::bail!("CVPixelBufferCreate(NV12 synthetic capture) failed: status={status}");
        }
        Ok(Self { ptr: pixel_buffer })
    }

    fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.ptr
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacosSyntheticCvPixelBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                CFRelease(self.ptr.cast_const());
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}

#[cfg(target_os = "macos")]
struct MacosSyntheticCvPixelBufferCapture {
    width: usize,
    height: usize,
    frame_index: u64,
    buffers: Vec<MacosSyntheticCvPixelBuffer>,
}

#[cfg(target_os = "macos")]
impl MacosSyntheticCvPixelBufferCapture {
    fn new(profile: &MediaProfile) -> Result<Self> {
        let width = even_dimension(profile.width as usize).max(2);
        let height = even_dimension(profile.height as usize).max(2);
        let mut buffers = Vec::with_capacity(MACOS_SYNTHETIC_CV_BUFFER_POOL_CAPACITY);
        for _ in 0..MACOS_SYNTHETIC_CV_BUFFER_POOL_CAPACITY {
            buffers.push(MacosSyntheticCvPixelBuffer::new_nv12(width, height)?);
        }
        tracing::info!(
            source_id = crate::capture_source::TEST_SYNTHETIC_CV_CAPTURE_SOURCE_ID,
            width,
            height,
            pool_capacity = buffers.len(),
            "created macOS synthetic CVPixelBuffer LAN capture"
        );
        Ok(Self {
            width,
            height,
            frame_index: 0,
            buffers,
        })
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame> {
        let buffer_index = (self.frame_index as usize) % self.buffers.len();
        let pixel_buffer = self.buffers[buffer_index].as_ptr();
        self.fill_pixel_buffer(pixel_buffer)?;
        let timestamp_us = now_us();
        self.frame_index = self.frame_index.wrapping_add(1);
        CapturedFrame::from_macos_cv_pixel_buffer(
            self.width,
            self.height,
            FramePixelFormat::Nv12,
            timestamp_us,
            pixel_buffer,
        )
        .ok_or_else(|| anyhow::anyhow!("failed to retain synthetic macOS CVPixelBuffer frame"))
    }

    fn fill_pixel_buffer(&self, pixel_buffer: *mut std::ffi::c_void) -> Result<()> {
        let status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 0) };
        if status != MACOS_SYNTHETIC_CV_SUCCESS {
            anyhow::bail!("CVPixelBufferLockBaseAddress(synthetic) failed: status={status}");
        }

        let y_value = 16_u8.saturating_add((self.frame_index % 220) as u8);
        let fill_result = Self::fill_plane(pixel_buffer, 0, y_value)
            .and_then(|_| Self::fill_plane(pixel_buffer, 1, 128));
        let unlock_status = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, 0) };
        if let Err(error) = fill_result {
            return Err(error);
        }
        if unlock_status != MACOS_SYNTHETIC_CV_SUCCESS {
            anyhow::bail!(
                "CVPixelBufferUnlockBaseAddress(synthetic) failed: status={unlock_status}"
            );
        }
        Ok(())
    }

    fn fill_plane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
        value: u8,
    ) -> Result<()> {
        let base = unsafe { CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, plane_index) };
        if base.is_null() {
            anyhow::bail!("synthetic CVPixelBuffer plane {plane_index} base address is null");
        }
        let stride = unsafe { CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, plane_index) };
        let rows = unsafe { CVPixelBufferGetHeightOfPlane(pixel_buffer, plane_index) };
        let len = stride
            .checked_mul(rows)
            .ok_or_else(|| anyhow::anyhow!("synthetic CVPixelBuffer plane size overflow"))?;
        let plane = unsafe { std::slice::from_raw_parts_mut(base.cast::<u8>(), len) };
        plane.fill(value);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug)]
struct SyntheticFrameCapture {
    width: usize,
    height: usize,
    frame_index: u64,
}

#[cfg(test)]
impl SyntheticFrameCapture {
    fn new(profile: &MediaProfile) -> Self {
        let width = even_dimension(profile.width as usize).clamp(2, 640);
        let height = even_dimension(profile.height as usize).clamp(2, 360);
        Self {
            width,
            height,
            frame_index: 0,
        }
    }
}

#[cfg(test)]
impl mrd_pipeline_core::FrameCapture for SyntheticFrameCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, mrd_pipeline_core::PipelineError> {
        let mut rgb = vec![0_u8; self.width * self.height * 3];
        for y in 0..self.height {
            for x in 0..self.width {
                let index = (y * self.width + x) * 3;
                rgb[index] = ((x + self.frame_index as usize * 3) % 256) as u8;
                rgb[index + 1] = ((y + self.frame_index as usize * 5) % 256) as u8;
                rgb[index + 2] = (((x ^ y) + self.frame_index as usize * 7) % 256) as u8;
            }
        }
        self.frame_index = self.frame_index.wrapping_add(1);
        Ok(CapturedFrame::from_cpu(
            self.width,
            self.height,
            FramePixelFormat::Rgb24,
            now_ms().saturating_mul(1_000),
            rgb,
        ))
    }
}

#[cfg(test)]
const TEST_SYNTHETIC_CAPTURE_SOURCE_ID: &str = "test:synthetic";

#[cfg(test)]
fn synthetic_capture_source() -> CaptureSource {
    CaptureSource {
        id: TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string(),
        platform: "test".to_string(),
        source_kind: "display".to_string(),
        title: "Synthetic desktop frame source".to_string(),
        class_name: "SyntheticCapture".to_string(),
        width: 640,
        height: 360,
        process_id: 0,
        app_name: Some("mrd-service test source".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }
}

async fn selected_capture_source_id(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<String> {
    if let Some(selection) = app_state.capture_sources.lock().await.get(session_id) {
        return Ok(selection.source.id);
    }

    #[cfg(test)]
    {
        Ok(TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string())
    }

    #[cfg(not(test))]
    {
        let source = crate::capture_source::default_capture_source(false)
            .context("no default capture source is available for LAN media sender")?;
        app_state.capture_sources.lock().await.set(
            session_id.clone(),
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: source.clone(),
                status: "selected".to_string(),
                reason: Some("default fullscreen capture source".to_string()),
            },
        );
        Ok(source.id)
    }
}

async fn create_lan_frame_capture(
    source_id: &str,
    _profile: &MediaProfile,
) -> Result<LanFrameCapture> {
    #[cfg(test)]
    if source_id == TEST_SYNTHETIC_CAPTURE_SOURCE_ID {
        return Ok(LanFrameCapture::Synthetic(SyntheticFrameCapture::new(
            _profile,
        )));
    }

    #[cfg(windows)]
    {
        create_windows_lan_frame_capture(source_id, _profile)
    }

    #[cfg(target_os = "macos")]
    {
        if crate::capture_source::test_synthetic_cv_capture_enabled()
            && crate::capture_source::is_test_synthetic_cv_capture_source_id(source_id)
        {
            return Ok(LanFrameCapture::MacosSyntheticCv(
                MacosSyntheticCvPixelBufferCapture::new(_profile)?,
            ));
        }
        return create_macos_lan_frame_capture(source_id, _profile);
    }

    #[cfg(target_os = "linux")]
    {
        return Ok(LanFrameCapture::Pipewire(
            crate::capture_source::create_frame_capture_async(source_id).await?,
        ));
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!(
            "remote desktop capture is currently only available on Windows, macOS, and Linux"
        )
    }
}

#[cfg(target_os = "macos")]
fn create_macos_lan_frame_capture(
    source_id: &str,
    profile: &MediaProfile,
) -> Result<LanFrameCapture> {
    let mut capture = crate::capture_source::create_frame_capture(source_id)?;
    let (target_width, target_height) =
        h264_target_dimensions(capture.width(), capture.height(), profile);
    capture.set_target_dimensions(target_width, target_height);
    if std::env::var("MRD_MACOS_CAPTURE_FPS").is_err() {
        capture.set_target_fps(macos_lan_capture_stream_fps(profile));
    }
    Ok(LanFrameCapture::Macos(capture))
}

#[cfg(target_os = "macos")]
fn macos_lan_capture_stream_fps(profile: &MediaProfile) -> u32 {
    let requested_fps = if lan_capture_pump_enabled() && lan_capture_pump_drives_sender() {
        profile.fps.max(1)
    } else {
        profile.fps.max(1).saturating_mul(2)
    };
    requested_fps.clamp(1, 240)
}

#[cfg(windows)]
fn create_windows_lan_frame_capture(
    source_id: &str,
    profile: &MediaProfile,
) -> Result<LanFrameCapture> {
    let nvenc_h264_available = windows_lan_nvenc_h264_available();
    match windows_lan_capture_backend(source_id, nvenc_h264_available) {
        WindowsLanCaptureBackend::DxgiShared => {
            let device_name = crate::display_mode::display_device_name_for_source_id(source_id)
                .with_context(|| format!("failed to resolve Windows display for {source_id}"))?;
            let mut capture =
                mrd_capture_dxgi::DxgiSharedTextureCapture::new_for_device_name(&device_name)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .with_context(|| {
                        format!(
                            "failed to create DXGI shared capture for {source_id} ({device_name})"
                        )
                    })?;
            if windows_lan_capture_backend_for_profile(
                source_id,
                capture.width(),
                capture.height(),
                profile,
                nvenc_h264_available,
            ) != WindowsLanCaptureBackend::DxgiShared
            {
                return create_windows_lan_winrt_capture(source_id);
            }
            capture.set_target_dimensions(profile.width as usize, profile.height as usize);
            Ok(LanFrameCapture::DxgiShared(capture))
        }
        WindowsLanCaptureBackend::WinrtWindowShared => {
            let hwnd = parse_windows_window_source_id(source_id)?;
            let mut capture =
                mrd_capture_winrt::WinrtCapture::from_window_handle_shared_texture(hwnd)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .with_context(|| {
                        format!("failed to create WinRT shared window capture for {source_id}")
                    })?;
            if windows_lan_capture_backend_for_profile(
                source_id,
                capture.width(),
                capture.height(),
                profile,
                nvenc_h264_available,
            ) != WindowsLanCaptureBackend::WinrtWindowShared
            {
                return create_windows_lan_winrt_capture(source_id);
            }
            let (target_width, target_height) =
                window_h264_capture_dimensions(profile.width as usize, profile.height as usize);
            capture.set_target_dimensions(target_width, target_height);
            capture
                .start()
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .with_context(|| {
                    format!(
                        "failed to start WinRT shared window capture for {source_id} (WinrtWindowShared, hwnd=0x{hwnd:x})"
                    )
                })?;
            Ok(LanFrameCapture::Winrt(capture))
        }
        WindowsLanCaptureBackend::Winrt => create_windows_lan_winrt_capture(source_id),
    }
}

#[cfg(windows)]
fn create_windows_lan_winrt_capture(source_id: &str) -> Result<LanFrameCapture> {
    Ok(LanFrameCapture::Winrt(
        crate::capture_source::create_frame_capture(source_id)?,
    ))
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsLanCaptureBackend {
    DxgiShared,
    WinrtWindowShared,
    Winrt,
}

#[cfg(windows)]
fn windows_lan_capture_backend(
    source_id: &str,
    nvenc_h264_available: bool,
) -> WindowsLanCaptureBackend {
    let normalized = source_id.trim().to_ascii_lowercase();
    if normalized.starts_with("windows:display-shared:") {
        WindowsLanCaptureBackend::DxgiShared
    } else if normalized.starts_with("windows:window:")
        && windows_lan_window_capture_uses_shared_texture(nvenc_h264_available)
    {
        WindowsLanCaptureBackend::WinrtWindowShared
    } else {
        WindowsLanCaptureBackend::Winrt
    }
}

#[cfg(windows)]
fn windows_lan_capture_backend_for_profile(
    source_id: &str,
    source_width: usize,
    source_height: usize,
    profile: &MediaProfile,
    nvenc_h264_available: bool,
) -> WindowsLanCaptureBackend {
    let backend = windows_lan_capture_backend(source_id, nvenc_h264_available);
    if matches!(backend, WindowsLanCaptureBackend::WinrtWindowShared)
        && windows_lan_profile_requires_scaling_path(source_width, source_height, profile)
    {
        WindowsLanCaptureBackend::Winrt
    } else {
        backend
    }
}

#[cfg(windows)]
fn windows_lan_profile_requires_scaling_path(
    source_width: usize,
    source_height: usize,
    profile: &MediaProfile,
) -> bool {
    let (target_width, target_height) =
        h264_target_dimensions(source_width, source_height, profile);
    let native_width = even_dimension(source_width).max(2);
    let native_height = even_dimension(source_height).max(2);
    target_width < native_width || target_height < native_height
}

#[cfg(windows)]
fn windows_lan_window_capture_uses_shared_texture(nvenc_h264_available: bool) -> bool {
    nvenc_h264_available
}

#[cfg(windows)]
fn windows_lan_nvenc_h264_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| mrd_encode_nvenc::NvencH264Encoder::probe_h264_available().is_ok())
}

#[cfg(windows)]
fn parse_windows_window_source_id(source_id: &str) -> Result<isize> {
    crate::capture_source::parse_windows_window_hwnd_source_id(source_id)
}

fn is_windows_window_source_id(source_id: &str) -> bool {
    source_id
        .trim()
        .to_ascii_lowercase()
        .starts_with("windows:window:")
}

fn capture_source_kind_from_id(source_id: &str) -> Option<String> {
    source_id
        .trim()
        .split(':')
        .nth(1)
        .filter(|kind| !kind.is_empty())
        .map(|kind| kind.replace('-', "_"))
}

fn captured_frame_memory_path(frame: &CapturedFrame) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if frame.macos_cv_pixel_buffer().is_some() {
            return "macos_cv_pixel_buffer";
        }
    }

    #[cfg(windows)]
    {
        if frame.d3d11_shared_bgra().is_some() {
            return "d3d11_shared_bgra";
        }
    }

    "cpu"
}

fn prepare_frame_for_h264(frame: CapturedFrame, profile: &MediaProfile) -> Result<CapturedFrame> {
    if frame.width < 2 || frame.height < 2 {
        anyhow::bail!(
            "captured frame is too small: {}x{}",
            frame.width,
            frame.height
        );
    }

    let (target_width, target_height) = h264_target_dimensions(frame.width, frame.height, profile);

    #[cfg(target_os = "macos")]
    if frame.macos_cv_pixel_buffer().is_some() {
        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }
        anyhow::bail!(
            "macOS CVPixelBuffer capture requires exact selected profile dimensions: source {}x{}, selected {}x{}",
            frame.width,
            frame.height,
            target_width,
            target_height
        );
    }

    #[cfg(windows)]
    if frame.d3d11_shared_bgra().is_some() {
        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }
        anyhow::bail!(
            "D3D11 shared capture requires exact selected profile dimensions: source {}x{}, selected {}x{}",
            frame.width,
            frame.height,
            target_width,
            target_height
        );
    }

    if frame.pixel_format == FramePixelFormat::Nv12 {
        let required_len = nv12_cpu_frame_len(frame.width, frame.height)
            .ok_or_else(|| anyhow::anyhow!("captured NV12 byte size overflow"))?;
        if frame.data.len() < required_len {
            anyhow::bail!(
                "captured NV12 frame is truncated: {} < {}",
                frame.data.len(),
                required_len
            );
        }

        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }

        let source_rgb = nv12_to_rgb24(&frame.data, frame.width, frame.width, frame.height)?;
        let mut rgb = Vec::with_capacity(target_width * target_height * 3);
        for y in 0..target_height {
            let source_y = y * frame.height / target_height;
            for x in 0..target_width {
                let source_x = x * frame.width / target_width;
                let offset = (source_y * frame.width + source_x) * 3;
                rgb.extend_from_slice(&source_rgb[offset..offset + 3]);
            }
        }

        return Ok(CapturedFrame::from_cpu(
            target_width,
            target_height,
            FramePixelFormat::Rgb24,
            frame.timestamp_us,
            rgb,
        ));
    }

    let bytes_per_pixel = frame_bytes_per_pixel(frame.pixel_format);
    let source_stride = frame
        .width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| anyhow::anyhow!("captured frame stride overflow"))?;
    let required_len = source_stride
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow::anyhow!("captured frame byte size overflow"))?;
    if frame.data.len() < required_len {
        anyhow::bail!(
            "captured frame is truncated: {} < {}",
            frame.data.len(),
            required_len
        );
    }

    if target_width == frame.width && target_height == frame.height {
        return Ok(frame);
    }

    let mut rgb = Vec::with_capacity(target_width * target_height * 3);
    for y in 0..target_height {
        let source_y = y * frame.height / target_height;
        for x in 0..target_width {
            let source_x = x * frame.width / target_width;
            let (r, g, b) = read_captured_rgb(&frame, source_x, source_y, source_stride);
            rgb.extend_from_slice(&[r, g, b]);
        }
    }

    Ok(CapturedFrame::from_cpu(
        target_width,
        target_height,
        FramePixelFormat::Rgb24,
        frame.timestamp_us,
        rgb,
    ))
}

fn h264_target_dimensions(width: usize, height: usize, profile: &MediaProfile) -> (usize, usize) {
    let max_width = profile.width.max(2) as f64;
    let max_height = profile.height.max(2) as f64;
    let scale = (max_width / width as f64)
        .min(max_height / height as f64)
        .min(1.0);
    let target_width = even_dimension(((width as f64 * scale).round() as usize).max(2));
    let target_height = even_dimension(((height as f64 * scale).round() as usize).max(2));
    (target_width.max(2), target_height.max(2))
}

#[cfg(any(windows, test))]
fn window_h264_capture_dimensions(width: usize, height: usize) -> (usize, usize) {
    (even_dimension(width).max(2), even_dimension(height).max(2))
}

fn even_dimension(value: usize) -> usize {
    value & !1
}

fn frame_bytes_per_pixel(pixel_format: FramePixelFormat) -> usize {
    match pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
        FramePixelFormat::Nv12 => 1,
    }
}

fn nv12_cpu_frame_len(width: usize, height: usize) -> Option<usize> {
    width.checked_mul(height).and_then(|y_size| {
        width
            .checked_mul(height.div_ceil(2))
            .and_then(|uv_size| y_size.checked_add(uv_size))
    })
}

fn read_captured_rgb(
    frame: &CapturedFrame,
    x: usize,
    y: usize,
    source_stride: usize,
) -> (u8, u8, u8) {
    let bytes_per_pixel = frame_bytes_per_pixel(frame.pixel_format);
    let index = y * source_stride + x * bytes_per_pixel;
    match frame.pixel_format {
        FramePixelFormat::Bgra32 => (
            frame.data[index + 2],
            frame.data[index + 1],
            frame.data[index],
        ),
        FramePixelFormat::Rgba32 => (
            frame.data[index],
            frame.data[index + 1],
            frame.data[index + 2],
        ),
        FramePixelFormat::Rgb24 => (
            frame.data[index],
            frame.data[index + 1],
            frame.data[index + 2],
        ),
        FramePixelFormat::Nv12 => unreachable!("NV12 is handled before packed RGB scaling"),
    }
}

fn decode_h264_desktop_frame(
    decoder: &mut dyn VideoDecoder,
    payload: &[u8],
) -> Result<Vec<DecodedFrame>> {
    decode_lan_desktop_frame(LanAccessUnitCodec::H264, decoder, payload)
}

fn decode_lan_desktop_frame(
    codec: LanAccessUnitCodec,
    decoder: &mut dyn VideoDecoder,
    payload: &[u8],
) -> Result<Vec<DecodedFrame>> {
    if let Err(error) = decoder.push_access_unit(payload) {
        anyhow::bail!(
            "failed to decode LAN {} access unit: {error}; {}",
            codec.display_name(),
            describe_lan_access_unit(codec, payload)
        );
    }
    Ok(decoder.drain_decoded_frames())
}

fn decoded_frame_pixel_format(frame: &DecodedFrame) -> String {
    match &frame.data {
        DecodedFrameData::CpuRgb24(_) => "cpu_rgb24",
        DecodedFrameData::CpuBgra32(_) => "cpu_bgra32",
        DecodedFrameData::CpuI420 { .. } => "cpu_i420",
        DecodedFrameData::CpuNv12 { .. } => "cpu_nv12",
        DecodedFrameData::CpuP010 { .. } => "cpu_p010",
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 { .. } => "d3d11_shared_nv12",
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedP010 { .. } => "d3d11_shared_p010",
    }
    .to_string()
}

fn decoded_frame_to_rgb24(frame: DecodedFrame) -> Result<(u32, u32, Vec<u8>)> {
    let expected_pixels = frame
        .width
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow::anyhow!("decoded frame dimensions overflow"))?;
    let rgb = match frame.data {
        DecodedFrameData::CpuRgb24(data) => {
            let expected_len = expected_pixels
                .checked_mul(3)
                .ok_or_else(|| anyhow::anyhow!("decoded RGB frame byte size overflow"))?;
            if data.len() != expected_len {
                anyhow::bail!("decoded RGB frame has invalid byte length");
            }
            data
        }
        DecodedFrameData::CpuBgra32(data) => {
            let expected_len = expected_pixels
                .checked_mul(4)
                .ok_or_else(|| anyhow::anyhow!("decoded BGRA frame byte size overflow"))?;
            if data.len() != expected_len {
                anyhow::bail!("decoded BGRA frame has invalid byte length");
            }
            let mut rgb = Vec::with_capacity(expected_pixels * 3);
            for pixel in data.chunks_exact(4) {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
            rgb
        }
        DecodedFrameData::CpuNv12 { data, pitch } => {
            nv12_to_rgb24(&data, pitch, frame.width, frame.height)?
        }
        DecodedFrameData::CpuI420 {
            data,
            y_pitch,
            uv_pitch,
        } => i420_to_rgb24(&data, y_pitch, uv_pitch, frame.width, frame.height)?,
        _ => anyhow::bail!("decoded frame is not CPU RGB/BGRA/NV12/I420 backed"),
    };

    Ok((frame.width as u32, frame.height as u32, rgb))
}

fn nv12_to_rgb24(data: &[u8], pitch: usize, width: usize, height: usize) -> Result<Vec<u8>> {
    if pitch < width {
        anyhow::bail!("NV12 pitch is smaller than frame width");
    }
    let y_bytes = pitch
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("NV12 luma byte size overflow"))?;
    let uv_height = height.div_ceil(2);
    let uv_bytes = pitch
        .checked_mul(uv_height)
        .ok_or_else(|| anyhow::anyhow!("NV12 chroma byte size overflow"))?;
    let expected_len = y_bytes
        .checked_add(uv_bytes)
        .ok_or_else(|| anyhow::anyhow!("NV12 byte size overflow"))?;
    if data.len() < expected_len {
        anyhow::bail!("NV12 frame has invalid byte length");
    }

    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let y_row = y * pitch;
        let uv_row = y_bytes + (y / 2) * pitch;
        for x in 0..width {
            let luma = data[y_row + x] as i32;
            let uv_x = (x / 2) * 2;
            let u = data[uv_row + uv_x] as i32;
            let v = data[uv_row + uv_x + 1] as i32;
            let c = (luma - 16).max(0);
            let d = u - 128;
            let e = v - 128;
            rgb.push(clamp_yuv_to_u8((298 * c + 409 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c - 100 * d - 208 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c + 516 * d + 128) >> 8));
        }
    }
    Ok(rgb)
}

fn i420_to_rgb24(
    data: &[u8],
    y_pitch: usize,
    uv_pitch: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    if y_pitch < width {
        anyhow::bail!("I420 Y pitch is smaller than frame width");
    }
    let chroma_width = width.div_ceil(2);
    if uv_pitch < chroma_width {
        anyhow::bail!("I420 UV pitch is smaller than chroma width");
    }
    let chroma_height = height.div_ceil(2);
    let y_bytes = y_pitch
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("I420 luma byte size overflow"))?;
    let uv_bytes = uv_pitch
        .checked_mul(chroma_height)
        .ok_or_else(|| anyhow::anyhow!("I420 chroma byte size overflow"))?;
    let expected_len = y_bytes
        .checked_add(uv_bytes)
        .and_then(|bytes| bytes.checked_add(uv_bytes))
        .ok_or_else(|| anyhow::anyhow!("I420 byte size overflow"))?;
    if data.len() < expected_len {
        anyhow::bail!("I420 frame has invalid byte length");
    }

    let u_base = y_bytes;
    let v_base = y_bytes + uv_bytes;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let y_row = y * y_pitch;
        let uv_row = (y / 2) * uv_pitch;
        for x in 0..width {
            let luma = data[y_row + x] as i32;
            let u = data[u_base + uv_row + x / 2] as i32;
            let v = data[v_base + uv_row + x / 2] as i32;
            let c = (luma - 16).max(0);
            let d = u - 128;
            let e = v - 128;
            rgb.push(clamp_yuv_to_u8((298 * c + 409 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c - 100 * d - 208 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c + 516 * d + 128) >> 8));
        }
    }
    Ok(rgb)
}

fn clamp_yuv_to_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

async fn selected_media_profile(app_state: &Arc<AppState>, session_id: &SessionId) -> MediaProfile {
    app_state
        .media_profiles
        .lock()
        .await
        .get(session_id)
        .map(|negotiation| negotiation.selected)
        .unwrap_or_else(default_media_profile)
}

fn negotiate_media_profile(
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    clamp_media_profile_to_lan_capability(requested_profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct SharedRecordingInputInjector {
        events: std::sync::Arc<std::sync::Mutex<Vec<mrd_input::InputEvent>>>,
    }

    impl SharedRecordingInputInjector {
        fn new(events: std::sync::Arc<std::sync::Mutex<Vec<mrd_input::InputEvent>>>) -> Self {
            Self { events }
        }
    }

    impl mrd_input::InputInjector for SharedRecordingInputInjector {
        fn is_available(&self) -> bool {
            true
        }

        fn inject(&mut self, event: &mrd_input::InputEvent) -> Result<(), mrd_input::InputError> {
            self.events.lock().expect("record input event").push(*event);
            Ok(())
        }
    }

    #[cfg(windows)]
    #[derive(Debug, Clone, Copy)]
    struct WindowsTestVirtualScreen {
        left: i32,
        top: i32,
        width: i32,
        height: i32,
    }

    #[cfg(windows)]
    struct CursorRestoreGuard {
        position: (i32, i32),
    }

    #[cfg(windows)]
    impl CursorRestoreGuard {
        fn new(position: (i32, i32)) -> Self {
            Self { position }
        }
    }

    #[cfg(windows)]
    impl Drop for CursorRestoreGuard {
        fn drop(&mut self) {
            let _ = force_cursor_position(self.position);
        }
    }

    #[cfg(windows)]
    static KEYBOARD_SMOKE_EVENTS: OnceLock<StdMutex<Vec<KeyboardSmokeEvent>>> = OnceLock::new();

    #[cfg(windows)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum KeyboardSmokeEvent {
        KeyDown(u16),
        KeyUp(u16),
    }

    #[cfg(windows)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct KeyboardSmokeResult {
        key_down: bool,
        key_up: bool,
    }

    #[cfg(windows)]
    struct KeyboardSmokeWindow {
        hwnd: windows::Win32::Foundation::HWND,
    }

    #[cfg(windows)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct KeyboardSmokeFocusSnapshot {
        hwnd: isize,
        foreground: isize,
        focus: isize,
    }

    #[cfg(windows)]
    impl KeyboardSmokeWindow {
        fn create() -> windows::core::Result<Self> {
            use windows::core::PCWSTR;
            use windows::Win32::Foundation::HINSTANCE;
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, RegisterClassW, ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
                SW_SHOW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
            };

            keyboard_smoke_events()
                .lock()
                .expect("clear keyboard smoke events")
                .clear();

            let class_name = wide_null(&format!(
                "MrdServiceKeyboardSmoke{}{}",
                std::process::id(),
                now_ms()
            ));
            let title = wide_null("MRD service LAN input keyboard smoke");
            unsafe {
                let hmodule = GetModuleHandleW(None)?;
                let hinstance = HINSTANCE(hmodule.0);
                let window_class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(keyboard_smoke_wnd_proc),
                    hInstance: hinstance,
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    ..Default::default()
                };
                if RegisterClassW(&window_class) == 0 {
                    return Err(windows::core::Error::from_thread());
                }

                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR(class_name.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    360,
                    180,
                    None,
                    None,
                    Some(hinstance),
                    None,
                )?;
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                pump_keyboard_smoke_window_messages();

                Ok(Self { hwnd })
            }
        }

        fn focus(&mut self) {
            use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
            use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, SetForegroundWindow};

            unsafe {
                let _ = BringWindowToTop(self.hwnd);
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetActiveWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                pump_keyboard_smoke_window_messages();
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        async fn wait_for_key_events(
            &mut self,
            virtual_key: u16,
            timeout: Duration,
        ) -> windows::core::Result<KeyboardSmokeResult> {
            let deadline = Instant::now() + timeout;
            loop {
                pump_keyboard_smoke_window_messages();
                let result = keyboard_smoke_result(virtual_key);
                if result.key_down && result.key_up {
                    return Ok(result);
                }
                if Instant::now() >= deadline {
                    return Ok(result);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        fn focus_snapshot(&self) -> KeyboardSmokeFocusSnapshot {
            unsafe {
                KeyboardSmokeFocusSnapshot {
                    hwnd: self.hwnd.0 as isize,
                    foreground: windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0
                        as isize,
                    focus: windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 as isize,
                }
            }
        }
    }

    #[cfg(windows)]
    impl Drop for KeyboardSmokeWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
            }
            pump_keyboard_smoke_window_messages();
        }
    }

    #[cfg(windows)]
    unsafe extern "system" fn keyboard_smoke_wnd_proc(
        hwnd: windows::Win32::Foundation::HWND,
        message: u32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_KEYDOWN, WM_KEYUP};

        match message {
            WM_KEYDOWN => {
                keyboard_smoke_events()
                    .lock()
                    .expect("record keyboard smoke key down")
                    .push(KeyboardSmokeEvent::KeyDown(wparam.0 as u16));
                windows::Win32::Foundation::LRESULT(0)
            }
            WM_KEYUP => {
                keyboard_smoke_events()
                    .lock()
                    .expect("record keyboard smoke key up")
                    .push(KeyboardSmokeEvent::KeyUp(wparam.0 as u16));
                windows::Win32::Foundation::LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    #[cfg(windows)]
    fn keyboard_smoke_events() -> &'static StdMutex<Vec<KeyboardSmokeEvent>> {
        KEYBOARD_SMOKE_EVENTS.get_or_init(|| StdMutex::new(Vec::new()))
    }

    #[cfg(windows)]
    fn keyboard_smoke_result(virtual_key: u16) -> KeyboardSmokeResult {
        let ime_process_key = windows::Win32::UI::Input::KeyboardAndMouse::VK_PROCESSKEY.0;
        let events = keyboard_smoke_events()
            .lock()
            .expect("read keyboard smoke events");
        KeyboardSmokeResult {
            key_down: events.iter().any(|event| {
                matches!(
                    *event,
                    KeyboardSmokeEvent::KeyDown(key)
                        if key == virtual_key || key == ime_process_key
                )
            }),
            key_up: events.iter().any(|event| {
                matches!(
                    *event,
                    KeyboardSmokeEvent::KeyUp(key)
                        if key == virtual_key || key == ime_process_key
                )
            }),
        }
    }

    #[cfg(windows)]
    fn pump_keyboard_smoke_window_messages() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };

        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    #[cfg(windows)]
    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(windows)]
    fn current_cursor_position() -> windows::core::Result<(i32, i32)> {
        let mut point = windows::Win32::Foundation::POINT::default();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point)?;
        }
        Ok((point.x, point.y))
    }

    #[cfg(windows)]
    fn current_virtual_screen() -> WindowsTestVirtualScreen {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };

        WindowsTestVirtualScreen {
            left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
            top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
            width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) },
            height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) },
        }
    }

    #[cfg(windows)]
    fn cursor_smoke_target(
        start: (i32, i32),
        screen: WindowsTestVirtualScreen,
        delta: i32,
    ) -> (i32, i32) {
        let right = screen.left.saturating_add(screen.width.saturating_sub(1));
        let bottom = screen.top.saturating_add(screen.height.saturating_sub(1));
        (
            offset_inside_range(start.0, screen.left, right, delta),
            offset_inside_range(start.1, screen.top, bottom, delta),
        )
    }

    #[cfg(windows)]
    fn offset_inside_range(value: i32, min: i32, max: i32, delta: i32) -> i32 {
        if value.saturating_add(delta) <= max {
            value.saturating_add(delta)
        } else {
            value.saturating_sub(delta).max(min)
        }
    }

    #[cfg(windows)]
    async fn wait_for_cursor_near(
        expected: (i32, i32),
        tolerance: i32,
        timeout: Duration,
    ) -> windows::core::Result<Option<(i32, i32)>> {
        let deadline = Instant::now() + timeout;
        loop {
            let current = current_cursor_position()?;
            if cursor_distance(current, expected) <= tolerance {
                return Ok(Some(current));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(windows)]
    fn cursor_distance(left: (i32, i32), right: (i32, i32)) -> i32 {
        left.0.abs_diff(right.0).max(left.1.abs_diff(right.1)) as i32
    }

    #[cfg(windows)]
    fn force_cursor_position(position: (i32, i32)) -> windows::core::Result<()> {
        unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(position.0, position.1) }
    }

    #[test]
    fn dynamic_window_fps_enters_active_tier_on_changed_frame() {
        let mut policy = DynamicWindowFpsPolicy::new(120);
        let decision = policy.update(DynamicWindowFpsInput {
            frame_changed: true,
            input_active: false,
            source_available: true,
            active_window_capture_count: 1,
        });
        assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
        assert_eq!(decision.target_fps, 120);
    }

    #[test]
    fn successful_window_capture_frame_is_dynamic_fps_activity() {
        let input = window_dynamic_fps_input_for_captured_frame(3);

        assert!(input.frame_changed);
        assert!(input.source_available);
        assert_eq!(input.active_window_capture_count, 3);
    }

    #[test]
    fn winrt_no_frame_timeout_is_dynamic_fps_idle_not_source_loss() {
        let error = anyhow::anyhow!(
            "failed to capture LAN desktop frame: WinRT capture produced no frame within 1000 ms"
        );

        assert!(is_winrt_window_capture_no_frame_timeout(&error));
        let input = window_dynamic_fps_input_for_capture_error(&error, 2);
        assert!(!input.frame_changed);
        assert!(input.source_available);
        assert_eq!(input.active_window_capture_count, 2);
    }

    #[test]
    fn non_timeout_window_capture_error_is_dynamic_fps_source_loss() {
        let error = anyhow::anyhow!("failed to capture LAN desktop frame: access denied");

        assert!(!is_winrt_window_capture_no_frame_timeout(&error));
        let input = window_dynamic_fps_input_for_capture_error(&error, 2);
        assert!(!input.frame_changed);
        assert!(!input.source_available);
        assert_eq!(input.active_window_capture_count, 2);
    }

    #[test]
    fn invalid_window_source_error_is_source_loss_not_display_fallback() {
        let error =
            window_capture_source_error("windows:window:0x0", "window hwnd must not be zero");

        assert_eq!(error.code, "WINDOW_CAPTURE_SOURCE_NOT_FOUND");
        assert!(error.message.contains("windows:window:0x0"));
        assert!(!error.message.contains("display"));
    }

    #[test]
    fn dynamic_window_fps_caps_idle_window() {
        let mut policy = DynamicWindowFpsPolicy::new(120);
        for _ in 0..10 {
            policy.update(DynamicWindowFpsInput {
                frame_changed: false,
                input_active: false,
                source_available: true,
                active_window_capture_count: 1,
            });
        }
        let decision = policy.current();
        assert_eq!(decision.tier, DynamicWindowFpsTier::Idle);
        assert_eq!(decision.target_fps, 15);
    }

    #[test]
    fn dynamic_window_fps_reduces_background_fps_under_multi_window_pressure() {
        let mut policy = DynamicWindowFpsPolicy::new(144);
        let decision = policy.update(DynamicWindowFpsInput {
            frame_changed: true,
            input_active: false,
            source_available: true,
            active_window_capture_count: 3,
        });
        assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
        assert_eq!(decision.target_fps, 60);
    }

    #[test]
    fn dynamic_window_fps_suspended_keeps_nonzero_heartbeat_target() {
        let mut policy = DynamicWindowFpsPolicy::new(120);
        let decision = policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: false,
            source_available: false,
            active_window_capture_count: 1,
        });

        assert_eq!(decision.tier, DynamicWindowFpsTier::Suspended);
        assert_eq!(decision.target_fps, 1);
    }

    #[test]
    fn dynamic_window_fps_recovers_from_suspended_to_active_on_changed_frame() {
        let mut policy = DynamicWindowFpsPolicy::new(120);
        let suspended = policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: false,
            source_available: false,
            active_window_capture_count: 1,
        });
        assert_eq!(suspended.tier, DynamicWindowFpsTier::Suspended);

        let decision = policy.update(DynamicWindowFpsInput {
            frame_changed: true,
            input_active: false,
            source_available: true,
            active_window_capture_count: 1,
        });

        assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
        assert_eq!(decision.target_fps, 120);
    }

    #[test]
    fn dynamic_window_fps_recovers_from_idle_to_active_on_input() {
        let mut policy = DynamicWindowFpsPolicy::new(120);
        for _ in 0..10 {
            policy.update(DynamicWindowFpsInput {
                frame_changed: false,
                input_active: false,
                source_available: true,
                active_window_capture_count: 1,
            });
        }
        assert_eq!(policy.current().tier, DynamicWindowFpsTier::Idle);

        let decision = policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: true,
            source_available: true,
            active_window_capture_count: 1,
        });

        assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
        assert_eq!(decision.target_fps, 120);
    }

    #[test]
    fn dynamic_window_fps_config_changes_when_profile_fps_changes() {
        let source_id = "window:1234";
        let profile_60 = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            ..MediaProfile::default()
        };
        let profile_120 = MediaProfile {
            fps: 120,
            ..profile_60.clone()
        };

        assert_eq!(
            lan_capture_config_key(source_id, &profile_60),
            lan_capture_config_key(source_id, &profile_120)
        );
        assert_ne!(
            dynamic_window_fps_config_key(source_id, &profile_60),
            dynamic_window_fps_config_key(source_id, &profile_120)
        );
    }

    #[test]
    fn media_frame_interval_uses_dynamic_window_target_when_present() {
        let profile = MediaProfile {
            fps: 144,
            ..MediaProfile::default()
        };
        let decision = DynamicWindowFpsDecision {
            tier: DynamicWindowFpsTier::Idle,
            target_fps: 12,
        };

        assert_eq!(
            media_frame_interval_for_dynamic_decision(&profile, Some(decision)),
            Duration::from_micros(83_333)
        );
    }

    #[test]
    fn dynamic_window_fps_interval_falls_back_to_profile_target_when_decision_absent() {
        let profile = MediaProfile {
            fps: 25,
            ..MediaProfile::default()
        };

        assert_eq!(
            media_frame_interval_for_dynamic_decision(&profile, None),
            Duration::from_micros(40_000)
        );
    }

    #[test]
    fn dynamic_window_fps_interval_clamps_zero_target_to_one_fps() {
        let profile = MediaProfile {
            fps: 144,
            ..MediaProfile::default()
        };
        let decision = DynamicWindowFpsDecision {
            tier: DynamicWindowFpsTier::Suspended,
            target_fps: 0,
        };

        assert_eq!(
            media_frame_interval_for_dynamic_decision(&profile, Some(decision)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn lan_discovery_config_reads_env_port_and_probe_endpoints() {
        let config = LanDiscoveryConfig::from_env_lookup(|key| match key {
            "MRD_LAN_DISCOVERY_PORT" => Some("21216".to_string()),
            "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS" => {
                Some("127.0.0.1:21217, 127.0.0.1:21218".to_string())
            }
            _ => None,
        })
        .expect("env config");

        assert_eq!(config.discovery_port, 21216);
        assert_eq!(
            config.probe_endpoints,
            vec![
                "127.0.0.1:21217".parse::<SocketAddr>().unwrap(),
                "127.0.0.1:21218".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn lan_media_test_impairment_is_disabled_by_default() {
        let config = LanMediaTestImpairment::from_env_lookup(|_| None).expect("default config");
        assert!(!config.enabled());
        assert_eq!(config.effective_datagram_size(1200), 1200);
    }

    #[test]
    fn lan_media_test_impairment_uses_seeded_loss_decisions() {
        let mut impairment = LanMediaTestImpairment::from_env_lookup(|key| match key {
            "MRD_LAN_TEST_IMPAIRMENT_LOSS_PCT" => Some("100".to_string()),
            "MRD_LAN_TEST_IMPAIRMENT_BASE_DELAY_MS" => Some("2".to_string()),
            "MRD_LAN_TEST_IMPAIRMENT_JITTER_MS" => Some("3".to_string()),
            "MRD_LAN_TEST_IMPAIRMENT_MTU_BYTES" => Some("900".to_string()),
            "MRD_LAN_TEST_IMPAIRMENT_SEED" => Some("42".to_string()),
            _ => None,
        })
        .expect("impairment config");

        assert!(impairment.enabled());
        assert_eq!(impairment.effective_datagram_size(1200), 900);
        let decision = impairment.next_datagram_decision();
        assert!(decision.drop_datagram);
        assert!(decision.delay >= Duration::from_millis(2));
        assert!(decision.delay <= Duration::from_millis(5));
    }

    #[test]
    fn lan_instance_ids_are_unique_within_same_process_millisecond() {
        let ids = (0..8).map(|_| new_instance_id()).collect::<Vec<_>>();
        let unique_ids = ids.iter().collect::<std::collections::HashSet<_>>();

        assert_eq!(unique_ids.len(), ids.len());
    }

    #[tokio::test]
    async fn snapshot_exposes_recent_peer() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let snapshot = state.snapshot().await;
        assert_eq!(snapshot.peers.len(), 1);
        assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
        assert_eq!(snapshot.peers[0].p2p_control_addr, "192.168.1.50:21116");
        assert!(snapshot.peers[0].p2p_available);
        assert_eq!(snapshot.peers[0].media_capabilities, Vec::<String>::new());
    }

    #[tokio::test]
    async fn snapshot_exposes_lan_media_v3_peer_capabilities_with_v2_rollout_compatibility() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: 21116,
                    transports: vec![
                        "quic".to_string(),
                        LAN_QUIC_MEDIA_TRANSPORT.to_string(),
                        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
                    ],
                    service_build_id: Some("build-a".to_string()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: lan_media_capabilities(),
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let peer = state.snapshot().await.peers.pop().expect("peer");

        assert_eq!(peer.service_build_id.as_deref(), Some("build-a"));
        assert_eq!(peer.media_protocol_version, Some(3));
        #[cfg(windows)]
        for capability in [
            LAN_CAPTURE_DXGI_CAPABILITY,
            LAN_ENCODE_NVENC_H264_CAPABILITY,
            LAN_ENCODE_NVENC_HEVC_CAPABILITY,
            LAN_ENCODE_NVENC_HEVC_MAIN10_CAPABILITY,
            LAN_DECODE_NVDEC_CAPABILITY,
            LAN_DECODE_NVDEC_HEVC_CAPABILITY,
            LAN_DECODE_NVDEC_HEVC_MAIN10_CAPABILITY,
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY,
            LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY,
            LAN_MEDIA_COLOR_MODE_CAPABILITY,
            LAN_RENDER_D3D11_NATIVE_CAPABILITY,
            LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY,
        ] {
            assert!(peer.media_capabilities.contains(&capability.to_string()));
        }
        #[cfg(target_os = "macos")]
        {
            for capability in [
                LAN_CAPTURE_MACOS_CAPABILITY,
                LAN_RENDER_MACOS_NATIVE_CAPABILITY,
            ] {
                assert!(peer.media_capabilities.contains(&capability.to_string()));
            }
            let probe = probe_macos_lan_media_capabilities();
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()),
                probe.videotoolbox_h264_encoder
            );
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()),
                probe.videotoolbox_hevc_encoder
            );
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()),
                probe.videotoolbox_hevc_encoder
            );
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()),
                probe.videotoolbox_h264_decoder
            );
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()),
                probe.videotoolbox_hevc_decoder
            );
            assert_eq!(
                peer.media_capabilities
                    .contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()),
                probe.videotoolbox_h264_decoder && probe.videotoolbox_hevc_decoder
            );
        }
        assert!(peer
            .media_capabilities
            .contains(&LAN_QUIC_RELIABLE_MEDIA_TRANSPORT.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_QUIC_MEDIA_V2_TRANSPORT.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_QUIC_MEDIA_V3_TRANSPORT.to_string()));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn announcement_advertises_keyboard_mouse_input_control() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("local-device".to_string()),
            "Local Device".to_string(),
        );

        let announcement = build_announcement(&app_state)
            .await
            .expect("registered device announcement");

        assert!(announcement
            .transports
            .contains(&LAN_INPUT_CONTROL_TRANSPORT.to_string()));
        assert!(announcement
            .media_capabilities
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn announcement_omits_keyboard_mouse_input_control_when_injector_unavailable() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("local-device".to_string()),
            "Local Device".to_string(),
        );
        app_state
            .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new(
                "blocked by test",
            ))
            .await;

        let announcement = build_announcement(&app_state)
            .await
            .expect("registered device announcement");

        assert!(!announcement
            .transports
            .contains(&LAN_INPUT_CONTROL_TRANSPORT.to_string()));
        assert!(!announcement
            .media_capabilities
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
    }

    #[test]
    fn lan_media_capabilities_follow_input_control_availability() {
        assert!(lan_media_capabilities_with_input_control(true)
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
        assert!(!lan_media_capabilities_with_input_control(false)
            .contains(&LAN_INPUT_CONTROL_CAPABILITY.to_string()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lan_media_capabilities_follow_videotoolbox_probe() {
        let without_videotoolbox =
            macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
                videotoolbox_h264_encoder: false,
                videotoolbox_hevc_encoder: false,
                videotoolbox_h264_decoder: false,
                videotoolbox_hevc_decoder: false,
            });
        assert!(without_videotoolbox.contains(&LAN_CAPTURE_MACOS_CAPABILITY.to_string()));
        assert!(without_videotoolbox.contains(&LAN_RENDER_MACOS_NATIVE_CAPABILITY.to_string()));
        assert!(
            !without_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string())
        );
        assert!(
            !without_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string())
        );
        assert!(
            !without_videotoolbox.contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string())
        );
        assert!(
            !without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string())
        );
        assert!(
            !without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string())
        );
        assert!(!without_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

        let h264_decode_only =
            macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
                videotoolbox_h264_encoder: false,
                videotoolbox_hevc_encoder: false,
                videotoolbox_h264_decoder: true,
                videotoolbox_hevc_decoder: false,
            });
        assert!(h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
        assert!(!h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
        assert!(!h264_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

        let hevc_decode_only =
            macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
                videotoolbox_h264_encoder: false,
                videotoolbox_hevc_encoder: false,
                videotoolbox_h264_decoder: false,
                videotoolbox_hevc_decoder: true,
            });
        assert!(!hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
        assert!(hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
        assert!(!hevc_decode_only.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));

        let with_videotoolbox =
            macos_lan_media_capabilities_from_probe(MacosLanMediaCapabilityProbe {
                videotoolbox_h264_encoder: true,
                videotoolbox_hevc_encoder: true,
                videotoolbox_h264_decoder: true,
                videotoolbox_hevc_decoder: true,
            });
        assert!(with_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
        assert!(with_videotoolbox.contains(&LAN_ENCODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
        assert!(with_videotoolbox.contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()));
        assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_H264_CAPABILITY.to_string()));
        assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_HEVC_CAPABILITY.to_string()));
        assert!(with_videotoolbox.contains(&LAN_DECODE_VIDEOTOOLBOX_CAPABILITY.to_string()));
    }

    #[test]
    fn service_build_id_prefers_runtime_override() {
        let build_id = service_build_id_from_lookup(|key| {
            if key == SERVICE_BUILD_ID_ENV {
                Some("peer-runtime-build".to_string())
            } else {
                None
            }
        });

        assert_eq!(build_id, "peer-runtime-build");
    }

    #[tokio::test]
    async fn peer_control_addr_returns_discovered_endpoint() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21117,
                    transports: vec!["webrtc".to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let addr = state
            .peer_control_addr(&DeviceId("remote-device".to_string()))
            .await
            .expect("peer addr");

        assert_eq!(addr.to_string(), "192.168.1.50:21117");
    }

    #[tokio::test]
    async fn request_lan_control_input_forwards_to_peer_injector() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let session_id = SessionId("input-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let result = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::MouseButton {
                button: mrd_ipc::ControlInputButton::Left,
                pressed: true,
            },
        )
        .await
        .expect("control input ack");

        assert_eq!(result.lane, mrd_ipc::ControlInputLane::Reliable);
        assert_eq!(result.event_count, 1);
        handler.await.unwrap();

        let snapshot = target_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());
        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 1);
        assert_eq!(snapshot.realtime.injected_messages, 0);
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "manual smoke test: moves the local cursor through LAN control input and restores it"]
    async fn lan_control_input_sendinput_smoke_moves_cursor_through_udp_handler() {
        let start = current_cursor_position().expect("read starting cursor position");
        let _restore = CursorRestoreGuard::new(start);
        let target = cursor_smoke_target(start, current_virtual_screen(), 80);
        assert_ne!(target, start, "smoke target must move the cursor");

        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let session_id = SessionId("input-sendinput-smoke-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let result = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::MouseMove {
                x: target.0,
                y: target.1,
            },
        )
        .await
        .expect("control input ack");
        handler.await.unwrap();
        let moved = wait_for_cursor_near(target, 4, Duration::from_millis(500))
            .await
            .expect("wait for LAN SendInput cursor target");
        let snapshot = target_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());

        eprintln!(
            "lan sendinput smoke start={start:?} target={target:?} moved={moved:?} lane={:?} snapshot={:?}",
            result.lane,
            snapshot.realtime
        );
        assert_eq!(result.lane, mrd_ipc::ControlInputLane::Realtime);
        assert_eq!(result.event_count, 1);
        assert!(moved.is_some());
        assert_eq!(snapshot.realtime.accepted_messages, 1);
        assert_eq!(snapshot.realtime.injected_messages, 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "manual smoke test: sends a key through LAN control input into a focused window"]
    async fn lan_control_input_sendinput_keyboard_smoke_sends_key_through_udp_handler() {
        let mut window = KeyboardSmokeWindow::create().expect("create keyboard smoke window");
        window.focus();

        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let session_id = SessionId("input-sendinput-keyboard-smoke-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            for _ in 0..2 {
                let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
                handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                    .await
                    .unwrap();
            }
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let key_down = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            },
        )
        .await
        .expect("control input key-down ack");
        let key_up = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: false,
            },
        )
        .await
        .expect("control input key-up ack");
        handler.await.unwrap();

        let events = window
            .wait_for_key_events(0x41, Duration::from_millis(500))
            .await
            .expect("wait for LAN SendInput key events");
        let snapshot = target_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());

        eprintln!(
            "lan keyboard sendinput smoke key_down={:?} key_up={:?} focus={:?} events={:?} lane_down={:?} lane_up={:?} snapshot={:?}",
            events.key_down,
            events.key_up,
            window.focus_snapshot(),
            keyboard_smoke_events()
                .lock()
                .expect("read keyboard smoke events"),
            key_down.lane,
            key_up.lane,
            snapshot.reliable
        );
        assert_eq!(key_down.lane, mrd_ipc::ControlInputLane::Reliable);
        assert_eq!(key_up.lane, mrd_ipc::ControlInputLane::Reliable);
        assert_eq!(key_down.event_count, 1);
        assert_eq!(key_up.event_count, 1);
        assert!(events.key_down);
        assert!(events.key_up);
        assert_eq!(snapshot.reliable.accepted_messages, 2);
        assert_eq!(snapshot.reliable.injected_messages, 2);
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "manual smoke test: sends a key through service IPC, LAN control input, and SendInput"]
    async fn ipc_control_input_keyboard_smoke_routes_to_lan_sendinput_target_window() {
        let mut window = KeyboardSmokeWindow::create().expect("create keyboard smoke window");
        window.focus();

        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let controller_server = crate::ipc_server::IpcServer::new(controller_state.clone());
        let session_id = SessionId("ipc-input-sendinput-keyboard-smoke-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            for _ in 0..2 {
                let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
                handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                    .await
                    .unwrap();
            }
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let key_down = controller_server
            .handle_request(mrd_ipc::IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            })
            .await;
        let key_up = controller_server
            .handle_request(mrd_ipc::IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: false,
                },
            })
            .await;
        handler.await.unwrap();

        let events = window
            .wait_for_key_events(0x41, Duration::from_millis(500))
            .await
            .expect("wait for IPC LAN SendInput key events");
        let snapshot = target_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());

        eprintln!(
            "ipc lan keyboard sendinput smoke key_down={:?} key_up={:?} focus={:?} events={:?} response_down={:?} response_up={:?} snapshot={:?}",
            events.key_down,
            events.key_up,
            window.focus_snapshot(),
            keyboard_smoke_events()
                .lock()
                .expect("read keyboard smoke events"),
            key_down,
            key_up,
            snapshot.reliable
        );
        assert_eq!(
            key_down,
            mrd_ipc::IpcResponse::ControlInputAccepted {
                session_id: session_id.clone(),
                lane: mrd_ipc::ControlInputLane::Reliable,
                event_count: 1,
            }
        );
        assert_eq!(
            key_up,
            mrd_ipc::IpcResponse::ControlInputAccepted {
                session_id: session_id.clone(),
                lane: mrd_ipc::ControlInputLane::Reliable,
                event_count: 1,
            }
        );
        assert!(events.key_down);
        assert!(events.key_up);
        assert_eq!(snapshot.reliable.accepted_messages, 2);
        assert_eq!(snapshot.reliable.injected_messages, 2);
    }

    #[tokio::test]
    async fn accepted_lan_control_input_scales_mouse_move_to_selected_source_size() {
        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        target_state
            .replace_control_input_for_test(SharedRecordingInputInjector::new(recorded.clone()))
            .await;
        let session_id = SessionId("input-scale-session".to_string());
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        target_state.capture_sources.lock().await.set(
            session_id.clone(),
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: CaptureSource {
                    id: TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string(),
                    platform: "test".to_string(),
                    source_kind: "display".to_string(),
                    title: "Synthetic 2K desktop source".to_string(),
                    class_name: "SyntheticCapture".to_string(),
                    width: 2560,
                    height: 1440,
                    process_id: 0,
                    app_name: Some("mrd-service test source".to_string()),
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                },
                status: "selected".to_string(),
                reason: None,
            },
        );
        let mut selected = default_media_profile();
        selected.width = 1280;
        selected.height = 720;
        target_state.media_profiles.lock().await.set(
            session_id.clone(),
            MediaProfileNegotiation {
                requested: selected.clone(),
                selected,
                status: "accepted".to_string(),
                reason: None,
                selected_source_id: Some(TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string()),
                selected_width: Some(1280),
                selected_height: Some(720),
                downgrade_reason: None,
            },
        );

        let ack = accept_or_replay_lan_control_input(
            &target_state,
            &session_id,
            "controller-device",
            11,
            &mrd_ipc::ControlInputEvent::MouseMove { x: 640, y: 360 },
        )
        .await;

        assert!(ack.accepted);
        assert_eq!(ack.lane, Some(mrd_ipc::ControlInputLane::Realtime));
        assert_eq!(
            recorded.lock().expect("recorded input").as_slice(),
            &[mrd_input::InputEvent::MouseMove { x: 1280, y: 720 }]
        );
    }

    #[tokio::test]
    async fn reliable_lan_control_input_retries_after_missing_ack() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let session_id = SessionId("input-retry-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let attempts = Arc::new(AtomicU64::new(0));
        let handler_attempts = attempts.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (_len, _addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handler_attempts.fetch_add(1, Ordering::SeqCst);

            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handler_attempts.fetch_add(1, Ordering::SeqCst);
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let result = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::MouseButton {
                button: mrd_ipc::ControlInputButton::Left,
                pressed: true,
            },
        )
        .await;

        if result.is_err() {
            handler.abort();
        }
        let result = result.expect("reliable control input should retry after a missing ack");
        handler.await.unwrap();

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(result.lane, mrd_ipc::ControlInputLane::Reliable);
        assert_eq!(result.event_count, 1);
    }

    #[tokio::test]
    async fn duplicate_reliable_lan_control_input_replays_ack_without_reinjecting() {
        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );
        target_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let session_id = SessionId("input-dedupe-session".to_string());
        target_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let event = mrd_ipc::ControlInputEvent::Key {
            key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        };

        let first = accept_or_replay_lan_control_input(
            &target_state,
            &session_id,
            "controller-device",
            42,
            &event,
        )
        .await;
        let second = accept_or_replay_lan_control_input(
            &target_state,
            &session_id,
            "controller-device",
            42,
            &event,
        )
        .await;

        assert!(first.accepted);
        assert_eq!(second.accepted, first.accepted);
        assert_eq!(second.lane, first.lane);
        assert_eq!(second.event_count, first.event_count);
        let snapshot = target_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 1);
    }

    #[tokio::test]
    async fn realtime_lan_control_input_does_not_retry_without_ack() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        let session_id = SessionId("input-realtime-session".to_string());
        controller_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let attempts = Arc::new(AtomicU64::new(0));
        let handler_attempts = attempts.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (_len, _addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handler_attempts.fetch_add(1, Ordering::SeqCst);
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: vec![LAN_INPUT_CONTROL_CAPABILITY.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let result = request_lan_control_input(
            &controller_state,
            &session_id,
            mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
        )
        .await;

        handler.await.unwrap();
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn remote_session_request_auto_accepts_session() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let request = LanDiscoveryPacket::RemoteSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            instance_id: "controller-instance".to_string(),
            session_id: "session-1".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            transport_kind: "quic".to_string(),
            source_discovery_port: Some(21116),
            source_media_capabilities: lan_media_capabilities(),
            requested_media_profile: Some(MediaProfile {
                width: 3840,
                height: 2160,
                fps: 240,
                bitrate_mbps: 120,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            }),
            timestamp_ms: now_ms(),
        };
        let bytes = serde_json::to_vec(&request).unwrap();

        handle_packet(
            &service_socket,
            &app_state,
            &bytes,
            ack_socket.local_addr().unwrap(),
        )
        .await
        .unwrap();

        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
        match ack {
            LanDiscoveryPacket::RemoteSessionAck {
                session_id,
                accepted,
                media,
                media_profile,
                ..
            } => {
                assert_eq!(session_id, "session-1");
                assert!(accepted);
                let media = media.expect("QUIC media bootstrap");
                assert_eq!(media.transport_kind, "quic");
                let quic = media.quic.expect("QUIC bootstrap details");
                assert!(!quic.listen_addr.ends_with(":0"));
                assert!(!quic.server_name.is_empty());
                assert!(!quic.cert_der.is_empty());
                let negotiation = media_profile.expect("media profile negotiation");
                assert_eq!(negotiation.status, "downgraded");
                assert_eq!(negotiation.selected.width, LAN_MEDIA_TARGET_WIDTH);
                assert_eq!(negotiation.selected.height, LAN_MEDIA_TARGET_HEIGHT);
                assert_eq!(negotiation.selected.fps, 240);
                assert_eq!(
                    negotiation.selected.bitrate_mbps,
                    LAN_MEDIA_TARGET_BITRATE_MBPS
                );
                assert_eq!(negotiation.selected.codec, "hevc");
                assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
                assert_eq!(
                    negotiation.selected.chroma_subsampling.as_deref(),
                    Some("4:2:0")
                );
            }
            _ => panic!("expected remote session ack"),
        }

        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(&SessionId("session-1".to_string()))
            .expect("accepted session");
        assert_eq!(
            snapshot.source_device_id,
            Some(DeviceId("controller-device".to_string()))
        );
        assert_eq!(snapshot.transport, "quic");
        assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Listening);
        assert!(snapshot.sender_active);
        assert!(snapshot.local_listen_addr.is_some());
        assert!(app_state.peer_media_capabilities.lock().await.supports(
            &SessionId("session-1".to_string()),
            LAN_QUIC_RELIABLE_MEDIA_TRANSPORT
        ));
    }

    #[tokio::test]
    async fn remote_session_request_rejects_webrtc_until_media_path_exists() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let request = LanDiscoveryPacket::RemoteSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            instance_id: "controller-instance".to_string(),
            session_id: "session-1".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            transport_kind: "webrtc".to_string(),
            source_discovery_port: None,
            source_media_capabilities: Vec::new(),
            requested_media_profile: None,
            timestamp_ms: now_ms(),
        };
        let bytes = serde_json::to_vec(&request).unwrap();

        handle_packet(
            &service_socket,
            &app_state,
            &bytes,
            ack_socket.local_addr().unwrap(),
        )
        .await
        .unwrap();

        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
        match ack {
            LanDiscoveryPacket::RemoteSessionAck {
                accepted, message, ..
            } => {
                assert!(!accepted);
                assert!(message
                    .expect("reject message")
                    .contains("WebRTC media path is not implemented"));
            }
            _ => panic!("expected remote session ack"),
        }
    }

    #[tokio::test]
    #[ignore = "TODO: fix flaky integration test - requires full media pipeline in test environment"]
    async fn request_lan_remote_session_records_quic_datagram_frames() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        tokio::time::sleep(Duration::from_millis(1)).await;

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: service_addr.port(),
                    transports: vec![
                        "quic".to_string(),
                        LAN_QUIC_MEDIA_TRANSPORT.to_string(),
                        LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
                        LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
                        LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
                    ],
                    service_build_id: Some(service_build_id()),
                    media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
                    media_capabilities: lan_media_capabilities(),
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let session_id = SessionId("session-quic-media".to_string());
        request_lan_remote_session(
            &controller_state,
            &DeviceId("target-device".to_string()),
            &session_id,
            "quic",
            Some(MediaProfile {
                width: 640,
                height: 360,
                fps: 60,
                bitrate_mbps: 5,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }),
        )
        .await
        .unwrap();
        handler.await.unwrap();

        let mut snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        for _ in 0..40 {
            if snapshot.frames_decoded > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        }

        assert!(snapshot.frames_received > 0);
        assert!(snapshot.frames_decoded > 0);
        assert!(snapshot.media_probe_valid);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("h264_desktop_frame")
        );
        assert_eq!(snapshot.media_probe_width, Some(640));
        assert_eq!(snapshot.media_probe_height, Some(360));
        assert!(snapshot.last_media_sequence.unwrap_or_default() > 0);
        assert!(snapshot
            .last_media_payload_hash
            .as_deref()
            .unwrap_or_default()
            .starts_with("fnv1a64:"));
        assert_eq!(snapshot.media_probe_target_fps, Some(60));
        assert_eq!(snapshot.media_probe_target_bitrate_mbps, Some(5));
        assert!(snapshot.media_probe_payload_bytes.unwrap_or_default() > 0);
        assert!(snapshot.latest_frame_data_url.is_none());
        let session_snapshot = controller_state
            .sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .expect("controller session snapshot");
        assert!(
            session_snapshot.receiver_active,
            "controller should mark the LAN QUIC receiver active after connecting"
        );
        assert_eq!(
            session_snapshot.lifecycle_state,
            SessionLifecycleState::Streaming
        );
        assert!(
            controller_state
                .media_tasks
                .lock()
                .await
                .active_count(&session_id)
                > 0,
            "controller should register the LAN receiver media task"
        );

        crate::handlers::session::stop_session(&controller_state, session_id.clone()).await;
        let stopped_snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let after_stop_snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        assert_eq!(
            after_stop_snapshot.frames_decoded, stopped_snapshot.frames_decoded,
            "stopped LAN receiver must not keep recording probe frames"
        );
    }

    #[tokio::test]
    async fn request_lan_remote_session_rejects_legacy_quic_peer_without_media_capability() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );

        let peer_addr: SocketAddr = "127.0.0.1:32216".parse().unwrap();
        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "legacy-target-instance".to_string(),
                    device_id: "legacy-target-device".to_string(),
                    device_name: "Legacy Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: peer_addr.port(),
                    transports: vec!["quic".to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                peer_addr,
            )
            .await;

        let error = request_lan_remote_session(
            &controller_state,
            &DeviceId("legacy-target-device".to_string()),
            &SessionId("session-legacy-peer".to_string()),
            "quic",
            None,
        )
        .await
        .expect_err("legacy QUIC peer should fail before session request");

        assert!(error.to_string().contains("quic_datagram"));
        assert!(error.to_string().contains("Rebuild and restart"));
    }

    #[tokio::test]
    async fn request_lan_remote_session_rejects_peer_without_2k144_media_profile() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );

        let peer_addr: SocketAddr = "127.0.0.1:32217".parse().unwrap();
        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "stale-target-instance".to_string(),
                    device_id: "stale-target-device".to_string(),
                    device_name: "Stale Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: peer_addr.port(),
                    transports: vec!["quic".to_string(), LAN_QUIC_MEDIA_TRANSPORT.to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                peer_addr,
            )
            .await;

        let error = request_lan_remote_session(
            &controller_state,
            &DeviceId("stale-target-device".to_string()),
            &SessionId("session-stale-peer".to_string()),
            "quic",
            None,
        )
        .await
        .expect_err("stale QUIC datagram peer should fail before session request");

        assert!(error.to_string().contains("quic_datagram_2k144"));
        assert!(error.to_string().contains("Rebuild and restart"));
    }

    #[tokio::test]
    async fn snapshot_ignores_own_instance() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: state.instance_id().to_string(),
                    device_id: "self-device".to_string(),
                    device_name: "Self".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                "127.0.0.1:21116".parse().unwrap(),
            )
            .await;

        assert!(state.snapshot().await.peers.is_empty());
    }

    #[tokio::test]
    async fn request_probe_and_wait_returns_after_peer_update() {
        let state = Arc::new(LanDiscoveryState::default());
        let waiting_state = state.clone();
        let waiter = tokio::spawn(async move {
            waiting_state
                .request_probe_and_wait(Duration::from_secs(1))
                .await
        });

        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string(), "quic".to_string()],
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let snapshot = waiter.await.unwrap();
        assert_eq!(snapshot.peers.len(), 1);
        assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
    }

    #[test]
    fn discovery_packet_requires_rdesk_namespace() {
        assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, DISCOVERY_APP_ID));
        assert!(!is_valid_discovery_packet(DISCOVERY_MAGIC, "rsharemouse"));
    }

    #[test]
    fn media_probe_frame_uses_hevc_compressed_profile() {
        let profile = default_media_profile();
        let frame = build_media_probe_frame(42, 123_456, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.sequence, 42);
        assert_eq!(stats.width, 2560);
        assert_eq!(stats.height, 1600);
        assert_eq!(stats.target_fps, 165);
        assert_eq!(stats.target_bitrate_mbps, 120);
        assert_eq!(stats.format, "compressed_hevc_test_pattern");
        assert!(stats.bytes_received < (2560_u64 * 1600 * 4));
        assert!(stats.payload_hash.starts_with("fnv1a64:"));
    }

    #[test]
    fn media_profile_negotiation_clamps_to_lan_capability() {
        let negotiation = negotiate_media_profile(Some(MediaProfile {
            width: 3840,
            height: 2160,
            fps: 300,
            bitrate_mbps: 160,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap();

        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(negotiation.selected.width, 2560);
        assert_eq!(negotiation.selected.height, 1600);
        assert_eq!(negotiation.selected.fps, 249);
        assert_eq!(negotiation.selected.bitrate_mbps, 120);
        assert_eq!(negotiation.selected.codec, "hevc");
    }

    #[test]
    fn media_profile_negotiation_preserves_supported_hevc_main_420_profile() {
        let negotiation = negotiate_media_profile(Some(MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
            ..MediaProfile::default()
        }))
        .unwrap();

        assert_eq!(negotiation.status, "accepted");
        assert_eq!(negotiation.selected.codec, "hevc");
        assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
        assert_eq!(
            negotiation.selected.chroma_subsampling.as_deref(),
            Some("4:2:0")
        );
        assert_eq!(negotiation.selected.pixel_format.as_deref(), Some("nv12"));
        assert_eq!(negotiation.selected.hdr_enabled, Some(false));
    }

    #[test]
    fn media_profile_negotiation_normalizes_h265_aliases_to_hevc() {
        for codec in ["h265", "H.265", " HEVC "] {
            let negotiation = negotiate_media_profile(Some(MediaProfile {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate_mbps: 20,
                codec: codec.to_string(),
                ..MediaProfile::default()
            }))
            .unwrap();

            assert_eq!(negotiation.selected.codec, "hevc");
            assert_eq!(negotiation.selected.codec_profile.as_deref(), Some("main"));
            assert_eq!(
                negotiation.selected.chroma_subsampling.as_deref(),
                Some("4:2:0")
            );
            assert_eq!(negotiation.selected.pixel_format.as_deref(), Some("nv12"));
            assert_eq!(
                LanAccessUnitCodec::from_profile(&negotiation.selected),
                LanAccessUnitCodec::Hevc
            );
        }
    }

    #[test]
    fn media_profile_negotiation_allows_high_refresh_canary_profiles() {
        let negotiation = negotiate_media_profile(Some(MediaProfile {
            width: 1920,
            height: 1080,
            fps: 249,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }))
        .unwrap();

        assert_eq!(negotiation.status, "accepted");
        assert_eq!(negotiation.selected.width, 1920);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(negotiation.selected.fps, 249);
        assert_eq!(negotiation.selected.bitrate_mbps, 20);
    }

    #[test]
    fn requested_hevc_profile_requires_peer_hevc_media_capabilities() {
        let error = ensure_peer_supports_requested_media(
            &DeviceId("mac-target".to_string()),
            "quic",
            &test_required_lan_media_transports(),
            Some(&MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            }),
            &["videotoolbox_h264".to_string()],
        )
        .expect_err("HEVC request should require HEVC encoder and media profile caps");

        let message = error.to_string();
        assert!(message.contains("hevc encoder"));
        assert!(message.contains(LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY));
        assert!(message.contains("mac-target"));
    }

    #[test]
    fn requested_hevc_profile_accepts_macos_videotoolbox_capabilities() {
        ensure_peer_supports_requested_media(
            &DeviceId("mac-target".to_string()),
            "quic",
            &test_required_lan_media_transports(),
            Some(&MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "HEVC".to_string(),
                ..MediaProfile::default()
            }),
            &[
                "videotoolbox_hevc".to_string(),
                LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
            ],
        )
        .expect("macOS VideoToolbox HEVC peer should pass HEVC request preflight");
    }

    #[test]
    fn requested_non_full_color_profile_requires_peer_color_mode_capability() {
        let error = ensure_peer_supports_requested_media(
            &DeviceId("windows-target".to_string()),
            "quic",
            &test_required_lan_media_transports(),
            Some(&MediaProfile {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate_mbps: 12,
                codec: "h264".to_string(),
                color_mode: Some("grayscale".to_string()),
                color_pipeline: Some("sdr8".to_string()),
                ..MediaProfile::default()
            }),
            &["encode.nvenc_h264".to_string()],
        )
        .expect_err("non-full color modes require an explicit peer color transform capability");

        let message = error.to_string();
        assert!(message.contains("media.color_mode_v1"));
        assert!(message.contains("color=grayscale"));
    }

    #[test]
    fn requested_hdr_main10_profile_requires_peer_main10_media_capabilities() {
        let error = ensure_peer_supports_requested_media(
            &DeviceId("windows-target".to_string()),
            "quic",
            &test_required_lan_media_transports(),
            Some(&MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 80,
                codec: "hevc".to_string(),
                codec_profile: Some("main10".to_string()),
                bit_depth: Some(10),
                chroma_subsampling: Some("4:2:0".to_string()),
                pixel_format: Some("p010".to_string()),
                color_pipeline: Some("hdr_main10".to_string()),
                ..MediaProfile::default()
            }),
            &[
                "encode.nvenc_hevc".to_string(),
                LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
            ],
        )
        .expect_err("HDR/Main10 HEVC must not be accepted as 8-bit HEVC");

        let message = error.to_string();
        assert!(message.contains("encode.nvenc_hevc_main10"));
        assert!(message.contains("media.hevc_main10_420_10bit"));
        assert!(message.contains("pipeline=hdr_main10"));
    }

    #[test]
    fn selected_hevc_profile_requires_receiver_hevc_decoder_capabilities() {
        let error = ensure_peer_can_receive_selected_media(
            "mac-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            },
            &["videotoolbox_h264".to_string()],
        )
        .expect_err("HEVC stream should require a HEVC-capable receiver");

        let message = error.to_string();
        assert!(message.contains("hevc decoder"));
        assert!(message.contains("mac-controller"));
    }

    #[test]
    fn selected_hevc_profile_does_not_treat_videotoolbox_encoder_as_decoder() {
        let error = ensure_peer_can_receive_selected_media(
            "mac-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            },
            &["videotoolbox_hevc".to_string()],
        )
        .expect_err("VideoToolbox HEVC encoder capability is not a decoder capability");

        assert!(error.to_string().contains("hevc decoder"));
    }

    #[test]
    fn selected_hdr_main10_profile_requires_receiver_main10_decoder_capabilities() {
        let error = ensure_peer_can_receive_selected_media(
            "windows-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 80,
                codec: "hevc".to_string(),
                codec_profile: Some("main10".to_string()),
                bit_depth: Some(10),
                chroma_subsampling: Some("4:2:0".to_string()),
                pixel_format: Some("p010".to_string()),
                color_pipeline: Some("hdr_main10".to_string()),
                ..MediaProfile::default()
            },
            &["decode.nvdec_hevc".to_string()],
        )
        .expect_err("Main10 selected profiles require a Main10-capable receiver");

        let message = error.to_string();
        assert!(message.contains("hevc main10 decoder"));
        assert!(message.contains("decode.nvdec_hevc_main10"));
    }

    #[test]
    fn selected_hevc_profile_rejects_generic_videotoolbox_decoder_alias() {
        let error = ensure_peer_can_receive_selected_media(
            "mac-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            },
            &[
                "decode.videotoolbox".to_string(),
                "videotoolbox".to_string(),
            ],
        )
        .expect_err("HEVC stream should require the HEVC-specific VideoToolbox decoder cap");

        let message = error.to_string();
        assert!(message.contains("hevc decoder"));
        assert!(message.contains("decode.videotoolbox_hevc"));
    }

    #[test]
    fn hevc_sender_h264_fallback_requires_peer_h264_receiver_capability() {
        assert!(!lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.videotoolbox_hevc".to_string()],
        ));
        assert!(lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.videotoolbox_h264".to_string()],
        ));
        assert!(lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.nvdec".to_string()],
        ));
        assert!(!lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::H264,
            &["decode.videotoolbox_h264".to_string()],
        ));
    }

    #[test]
    fn selected_h264_profile_accepts_macos_videotoolbox_h264_receiver() {
        ensure_peer_can_receive_selected_media(
            "mac-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 80,
                codec: "H264".to_string(),
                ..MediaProfile::default()
            },
            &["decode.videotoolbox_h264".to_string()],
        )
        .expect("macOS VideoToolbox H.264 receiver should pass H.264 selected profile preflight");
    }

    #[test]
    fn selected_hevc_profile_accepts_macos_videotoolbox_receiver() {
        ensure_peer_can_receive_selected_media(
            "mac-controller",
            &MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "HEVC".to_string(),
                ..MediaProfile::default()
            },
            &["decode.videotoolbox_hevc".to_string()],
        )
        .expect("macOS VideoToolbox receiver should pass HEVC selected profile preflight");
    }

    #[test]
    fn lan_color_mode_for_profile_maps_stable_strings() {
        let mut profile = default_media_profile();
        assert_eq!(
            lan_color_mode_for_profile(&profile).unwrap(),
            mrd_pipeline_core::ColorMode::Full
        );

        profile.color_mode = Some("full".to_string());
        assert_eq!(
            lan_color_mode_for_profile(&profile).unwrap(),
            mrd_pipeline_core::ColorMode::Full
        );

        profile.color_mode = Some("grayscale".to_string());
        assert_eq!(
            lan_color_mode_for_profile(&profile).unwrap(),
            mrd_pipeline_core::ColorMode::Grayscale
        );

        profile.color_mode = Some("monochrome".to_string());
        assert_eq!(
            lan_color_mode_for_profile(&profile).unwrap(),
            mrd_pipeline_core::ColorMode::Monochrome
        );

        profile.color_mode = Some("low_chroma".to_string());
        assert_eq!(
            lan_color_mode_for_profile(&profile).unwrap(),
            mrd_pipeline_core::ColorMode::LowChroma
        );

        profile.color_mode = Some("sepia".to_string());
        let error = lan_color_mode_for_profile(&profile).expect_err("unknown color mode rejected");
        assert!(error.to_string().contains("unsupported LAN color_mode"));
    }

    #[test]
    fn lan_profile_requests_hevc_main10_from_stable_profile_fields() {
        let mut profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        assert!(!lan_profile_requests_hevc_main10(&profile));

        profile.codec_profile = Some("main10".to_string());
        assert!(lan_profile_requests_hevc_main10(&profile));

        profile.codec_profile = None;
        profile.bit_depth = Some(10);
        assert!(lan_profile_requests_hevc_main10(&profile));

        profile.bit_depth = None;
        profile.pixel_format = Some("p010".to_string());
        assert!(lan_profile_requests_hevc_main10(&profile));

        profile.pixel_format = None;
        profile.color_pipeline = Some("hdr_main10".to_string());
        assert!(lan_profile_requests_hevc_main10(&profile));
    }

    #[tokio::test]
    async fn remote_session_accept_rejects_source_without_selected_hevc_decoder() {
        let app_state = Arc::new(AppState::new());
        app_state
            .devices
            .lock()
            .await
            .register(DeviceId("mac-target".to_string()), "Mac Target".to_string());

        let result = accept_lan_remote_session(
            &app_state,
            SessionId("session-hevc-receiver-missing".to_string()),
            DeviceId("mac-controller".to_string()),
            "quic".to_string(),
            vec!["videotoolbox_h264".to_string()],
            Some(MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            }),
        )
        .await;

        assert!(!result.accepted);
        assert!(result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("hevc decoder"));
        assert!(app_state
            .sessions
            .lock()
            .await
            .get(&SessionId("session-hevc-receiver-missing".to_string()))
            .is_none());
    }

    #[tokio::test]
    async fn media_profile_update_rejects_receiver_without_selected_hevc_decoder() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("session-hevc-update-receiver-missing".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("mac-controller".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .set(session_id.clone(), vec!["videotoolbox_h264".to_string()]);

        let error = accept_lan_media_profile_update(
            &app_state,
            &session_id,
            MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 40,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            },
        )
        .await
        .expect_err("HEVC update should require receiver HEVC decoder caps");

        assert!(error.to_string().contains("hevc decoder"));
        assert!(app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .is_none());
    }

    #[test]
    fn lan_media_reassembler_config_allows_decode_backpressure() {
        let config = lan_media_reassembler_config();

        assert!(config.frame_timeout >= Duration::from_millis(1_000));
        assert!(config.max_pending_frames >= 128);
    }

    #[test]
    fn lan_media_frame_orderer_holds_late_frames_until_gap_arrives() {
        let mut orderer = LanMediaFrameOrderer::new(8);

        let first = orderer.push(test_quic_au_frame(1, false));
        let third = orderer.push(test_quic_au_frame(3, false));
        let ready = orderer.push(test_quic_au_frame(2, false));

        assert_eq!(frame_ids(&first), vec![1]);
        assert!(third.is_empty());
        assert_eq!(frame_ids(&ready), vec![2, 3]);
    }

    #[test]
    fn lan_media_frame_orderer_handles_v3_media_frames() {
        let mut orderer = LanMediaFrameOrderer::<QuicMediaFrame>::new(8);

        let first = orderer.push(test_quic_media_frame(1, true));
        let third = orderer.push(test_quic_media_frame(3, false));
        let ready = orderer.push(test_quic_media_frame(2, false));

        assert_eq!(media_frame_ids(&first), vec![1]);
        assert!(third.is_empty());
        assert_eq!(media_frame_ids(&ready), vec![2, 3]);
    }

    #[test]
    fn lan_media_frame_orderer_skips_gap_when_pending_limit_is_reached() {
        let mut orderer = LanMediaFrameOrderer::new(2);

        assert_eq!(
            frame_ids(&orderer.push(test_quic_au_frame(10, true))),
            vec![10]
        );
        assert!(orderer.push(test_quic_au_frame(12, false)).is_empty());
        let ready = orderer.push(test_quic_au_frame(13, false));

        assert_eq!(frame_ids(&ready), vec![12, 13]);
    }

    #[test]
    fn lan_media_frame_orderer_releases_first_late_frame_at_low_latency_limit() {
        let mut orderer = LanMediaFrameOrderer::new(1);

        assert_eq!(
            frame_ids(&orderer.push(test_quic_au_frame(20, true))),
            vec![20]
        );
        let ready = orderer.push(test_quic_au_frame(22, false));

        assert_eq!(frame_ids(&ready), vec![22]);
    }

    #[test]
    fn production_lan_media_frame_orderer_absorbs_short_high_refresh_reordering() {
        let mut orderer = LanMediaFrameOrderer::new(LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES);

        assert_eq!(
            frame_ids(&orderer.push(test_quic_au_frame(100, true))),
            vec![100]
        );
        assert!(orderer.push(test_quic_au_frame(102, false)).is_empty());
        assert!(orderer.push(test_quic_au_frame(103, false)).is_empty());
        let ready = orderer.push(test_quic_au_frame(101, false));

        assert_eq!(frame_ids(&ready), vec![101, 102, 103]);
    }

    #[test]
    fn decoder_candidate_preference_keeps_fallback_backend_first() {
        let candidates = prioritize_lan_receiver_decoder_candidates(
            vec!["nvdec", "h264_software"],
            Some("h264_software"),
        );

        assert_eq!(candidates, vec!["h264_software", "nvdec"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_decoder_defaults_to_hardware_then_ffmpeg_fallback() {
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264),
            &[
                "nvdec_d3d11_shared",
                "nvdec",
                "ffmpeg_h264",
                "h264_software"
            ]
        );
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::Hevc),
            &["nvdec_hevc_d3d11_shared", "nvdec_hevc", "ffmpeg_hevc"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_receiver_decoder_defaults_to_videotoolbox_then_ffmpeg_fallback() {
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264),
            &["videotoolbox", "ffmpeg_h264", "h264_software"]
        );
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::Hevc),
            &["videotoolbox_hevc", "ffmpeg_hevc"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_receiver_decoder_videotoolbox_preference_is_codec_specific() {
        assert_eq!(
            preferred_lan_receiver_decoder_candidates_from_preference(
                LanAccessUnitCodec::H264,
                "videotoolbox"
            ),
            vec!["videotoolbox", "h264_software"]
        );
        assert_eq!(
            preferred_lan_receiver_decoder_candidates_from_preference(
                LanAccessUnitCodec::Hevc,
                "videotoolbox"
            ),
            vec!["videotoolbox_hevc", "ffmpeg_hevc"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_receiver_decoder_backends_create_videotoolbox_decoders() {
        let h264 =
            create_lan_video_decoder("videotoolbox").expect("create H.264 VideoToolbox decoder");
        assert_eq!(
            h264.output_memory_kind(),
            mrd_pipeline_core::FrameMemoryKind::Cpu
        );

        let hevc = create_lan_video_decoder("videotoolbox_hevc")
            .expect("create HEVC VideoToolbox decoder");
        assert_eq!(
            hevc.output_memory_kind(),
            mrd_pipeline_core::FrameMemoryKind::Cpu
        );
    }

    fn test_quic_au_frame(frame_id: u32, is_keyframe: bool) -> QuicAuFrame {
        let payload = [frame_id as u8, u8::from(is_keyframe)];
        let datagrams =
            fragment_access_unit(frame_id, u64::from(frame_id), is_keyframe, &payload, 1200)
                .expect("fragmented frame");
        let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());
        reassembler
            .push_datagram(&datagrams[0])
            .expect("reassembled frame")
            .expect("complete frame")
    }

    fn frame_ids(frames: &[QuicAuFrame]) -> Vec<u32> {
        frames.iter().map(|frame| frame.frame_id).collect()
    }

    fn test_required_lan_media_transports() -> Vec<String> {
        vec![
            LAN_QUIC_MEDIA_TRANSPORT.to_string(),
            LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
            LAN_QUIC_MEDIA_V2_TRANSPORT.to_string(),
            LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
        ]
    }

    fn test_quic_media_frame(frame_id: u32, is_keyframe: bool) -> QuicMediaFrame {
        QuicMediaFrame {
            payload_type: QuicMediaPayloadType::AccessUnit,
            codec: QuicMediaCodec::H264,
            profile_id: 123,
            frame_id,
            timestamp_us: u64::from(frame_id),
            flags: if is_keyframe {
                mrd_transport_quic_quinn::QUIC_MEDIA_V3_FLAG_KEYFRAME
            } else {
                0
            },
            payload: bytes::Bytes::from_static(b"h264-au"),
        }
    }

    fn media_frame_ids(frames: &[QuicMediaFrame]) -> Vec<u32> {
        frames.iter().map(|frame| frame.frame_id).collect()
    }

    #[tokio::test]
    async fn capture_source_selection_changes_active_sender_session() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("capture-source-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let source = mrd_ipc::CaptureSource {
            id: "windows:window:0x1234".to_string(),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: "Target App".to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: Some("Target App".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };
        let selection = accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "windows:window:0x1234",
            vec![source],
        )
        .await
        .unwrap();

        assert_eq!(selection.status, "selected");
        assert_eq!(selection.source.id, "windows:window:0x1234");
        assert_eq!(
            app_state
                .capture_sources
                .lock()
                .await
                .get(&session_id)
                .expect("selected capture source")
                .source
                .source_kind,
            "window"
        );
    }

    #[tokio::test]
    async fn capture_source_selection_reconciles_media_profile_to_source_dimensions() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("capture-source-profile-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 1920,
                height: 1080,
                fps: 120,
                bitrate_mbps: 20,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );

        let source = mrd_ipc::CaptureSource {
            id: "linux:display:1".to_string(),
            platform: "linux".to_string(),
            source_kind: "display".to_string(),
            title: "Linux Display".to_string(),
            class_name: "PipeWirePortal".to_string(),
            width: 1728,
            height: 1080,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };

        accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "linux:display:1",
            vec![source],
        )
        .await
        .unwrap();

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("reconciled media profile");
        assert_eq!(
            negotiation.selected_source_id.as_deref(),
            Some("linux:display:1")
        );
        assert_eq!(negotiation.selected.width, 1728);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(negotiation.selected_width, Some(1728));
        assert_eq!(negotiation.selected_height, Some(1080));
        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(
            negotiation.downgrade_reason.as_deref(),
            Some("matched selected capture source dimensions and aspect ratio")
        );
    }

    #[tokio::test]
    async fn capture_source_selection_preserves_source_aspect_ratio() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("capture-source-aspect-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate_mbps: 20,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );

        let source = mrd_ipc::CaptureSource {
            id: "windows:display:0".to_string(),
            platform: "windows".to_string(),
            source_kind: "display".to_string(),
            title: "Display 1".to_string(),
            class_name: "Monitor".to_string(),
            width: 2560,
            height: 1600,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };

        accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "windows:display:0",
            vec![source],
        )
        .await
        .unwrap();

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("reconciled media profile");
        assert_eq!(
            negotiation.selected_source_id.as_deref(),
            Some("windows:display:0")
        );
        assert_eq!(negotiation.selected.width, 1728);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(negotiation.selected_width, Some(1728));
        assert_eq!(negotiation.selected_height, Some(1080));
        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(
            negotiation.downgrade_reason.as_deref(),
            Some("matched selected capture source dimensions and aspect ratio")
        );
    }

    #[tokio::test]
    async fn display_mode_set_chooses_matching_mode_and_records_restore() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("display-mode-session".to_string());
        app_state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), sender_snapshot(&session_id));
        let modes = vec![
            display_mode("current", 2560, 1600, 60, true),
            display_mode("target", 1920, 1080, 144, false),
        ];

        let change = accept_lan_display_mode_set_from_modes(
            &app_state,
            &session_id,
            display_mode("requested", 1920, 1080, 144, false),
            true,
            modes,
        )
        .await
        .unwrap();

        assert_eq!(change.status, "changed");
        assert_eq!(
            change.previous.as_ref().map(|mode| mode.id.as_str()),
            Some("current")
        );
        assert_eq!(
            change.active.as_ref().map(|mode| mode.id.as_str()),
            Some("target")
        );
        assert_eq!(
            app_state
                .display_modes
                .lock()
                .await
                .restore_mode(&session_id)
                .as_ref()
                .map(|mode| mode.id.as_str()),
            Some("current")
        );
    }

    #[tokio::test]
    async fn display_mode_set_clamps_media_profile_to_active_refresh() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("display-mode-profile-session".to_string());
        app_state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), sender_snapshot(&session_id));
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 2560,
                height: 1600,
                fps: 165,
                bitrate_mbps: 120,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );
        let modes = vec![
            display_mode("current", 2560, 1440, 144, true),
            display_mode("active", 1920, 1200, 144, false),
        ];

        accept_lan_display_mode_set_from_modes(
            &app_state,
            &session_id,
            display_mode("requested", 2560, 1600, 165, false),
            true,
            modes,
        )
        .await
        .unwrap();

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("profile after display mode set");
        assert_eq!(negotiation.selected.width, 1920);
        assert_eq!(negotiation.selected.height, 1200);
        assert_eq!(negotiation.selected.fps, 144);
        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(
            negotiation.downgrade_reason.as_deref(),
            Some("matched active display mode dimensions and refresh rate")
        );
    }

    #[tokio::test]
    async fn remote_display_mode_ack_updates_controller_expected_profile() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("controller-display-mode-profile-session".to_string());
        app_state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), sender_snapshot(&session_id));
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 2560,
                height: 1600,
                fps: 165,
                bitrate_mbps: 120,
                codec: "hevc".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );
        store_capture_source_selection(
            &app_state,
            &session_id,
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: mrd_ipc::CaptureSource {
                    id: "windows:display-shared:0".to_string(),
                    platform: "windows".to_string(),
                    source_kind: "display_shared".to_string(),
                    title: "Display 1".to_string(),
                    class_name: "WinRTMonitorShared".to_string(),
                    width: 1920,
                    height: 1200,
                    process_id: 0,
                    app_name: Some("Display".to_string()),
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                },
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        record_remote_display_mode_change(
            &app_state,
            &session_id,
            &DisplayModeChange {
                session_id: session_id.clone(),
                requested: Some(display_mode("requested", 1920, 1200, 144, false)),
                previous: Some(display_mode("previous", 2560, 1600, 165, true)),
                active: Some(display_mode("active", 1920, 1200, 144, true)),
                status: "changed".to_string(),
                reason: None,
                restore_required: true,
            },
        )
        .await;

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("controller profile after display mode ack");
        assert_eq!(negotiation.selected.width, 1920);
        assert_eq!(negotiation.selected.height, 1200);
        assert_eq!(negotiation.selected.fps, 144);
        assert_eq!(
            negotiation.downgrade_reason.as_deref(),
            Some("matched active display mode dimensions and refresh rate")
        );
    }

    #[tokio::test]
    async fn capture_source_selection_tracks_different_windows_per_session() {
        let app_state = Arc::new(AppState::default());
        let session_a = SessionId("window-a".to_string());
        let session_b = SessionId("window-b".to_string());

        store_capture_source_selection(
            &app_state,
            &session_a,
            CaptureSourceSelection {
                session_id: session_a.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        store_capture_source_selection(
            &app_state,
            &session_b,
            CaptureSourceSelection {
                session_id: session_b.clone(),
                source: test_window_capture_source("windows:window:0x2222"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        assert_eq!(
            selected_capture_source_id(&app_state, &session_a)
                .await
                .unwrap(),
            "windows:window:0x1111"
        );
        assert_eq!(
            selected_capture_source_id(&app_state, &session_b)
                .await
                .unwrap(),
            "windows:window:0x2222"
        );
    }

    #[tokio::test]
    async fn active_window_capture_count_counts_selected_window_sessions() {
        let app_state = Arc::new(AppState::default());
        let session_a = SessionId("window-a".to_string());
        let session_b = SessionId("window-b".to_string());
        let session_display = SessionId("display".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                session_a.clone(),
                sender_snapshot_for_source(&session_a, "controller-a"),
            );
            sessions.insert(
                session_b.clone(),
                sender_snapshot_for_source(&session_b, "controller-b"),
            );
            sessions.insert(
                session_display.clone(),
                sender_snapshot_for_source(&session_display, "controller-c"),
            );
        }

        store_capture_source_selection(
            &app_state,
            &session_a,
            CaptureSourceSelection {
                session_id: session_a.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &session_b,
            CaptureSourceSelection {
                session_id: session_b.clone(),
                source: test_window_capture_source("windows:window:0x2222"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &session_display,
            CaptureSourceSelection {
                session_id: session_display.clone(),
                source: test_display_capture_source("windows:display-shared:0"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        assert_eq!(active_window_capture_count(&app_state).await, 2);
    }

    #[tokio::test]
    async fn window_sender_selection_keeps_same_source_device_sessions_active() {
        let app_state = Arc::new(AppState::default());
        let next_session = SessionId("new-window-controller-a".to_string());
        let old_display = SessionId("old-display-controller-a".to_string());
        let old_window = SessionId("old-window-controller-a".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                next_session.clone(),
                sender_snapshot_for_source(&next_session, "controller-a"),
            );
            sessions.insert(
                old_display.clone(),
                sender_snapshot_for_source(&old_display, "controller-a"),
            );
            sessions.insert(
                old_window.clone(),
                sender_snapshot_for_source(&old_window, "controller-a"),
            );
        }

        store_capture_source_selection(
            &app_state,
            &old_display,
            CaptureSourceSelection {
                session_id: old_display.clone(),
                source: test_display_capture_source("windows:display-shared:0"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &old_window,
            CaptureSourceSelection {
                session_id: old_window.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        close_existing_display_lan_sender_sessions_for_source(
            &app_state,
            &next_session,
            &test_window_capture_source("windows:window:0x2222"),
        )
        .await;

        let sessions = app_state.sessions.lock().await;
        assert!(sessions.get(&old_display).unwrap().sender_active);
        assert_eq!(
            sessions.get(&old_display).unwrap().lifecycle_state,
            SessionLifecycleState::Listening
        );
        assert!(sessions.get(&old_window).unwrap().sender_active);
        assert_eq!(
            sessions.get(&old_window).unwrap().lifecycle_state,
            SessionLifecycleState::Listening
        );
    }

    #[tokio::test]
    async fn display_sender_selection_closes_existing_display_sessions_for_same_controller_or_source(
    ) {
        let app_state = Arc::new(AppState::default());
        let next_session = SessionId("new-display-controller-a".to_string());
        let old_display = SessionId("old-display-controller-a".to_string());
        let old_window = SessionId("old-window-controller-a".to_string());
        let other_controller_other_source = SessionId("display-controller-b-other".to_string());
        let other_controller_same_source = SessionId("display-controller-b-same".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                next_session.clone(),
                sender_snapshot_for_source(&next_session, "controller-a"),
            );
            sessions.insert(
                old_display.clone(),
                sender_snapshot_for_source(&old_display, "controller-a"),
            );
            sessions.insert(
                old_window.clone(),
                sender_snapshot_for_source(&old_window, "controller-a"),
            );
            sessions.insert(
                other_controller_other_source.clone(),
                sender_snapshot_for_source(&other_controller_other_source, "controller-b"),
            );
            sessions.insert(
                other_controller_same_source.clone(),
                sender_snapshot_for_source(&other_controller_same_source, "controller-b"),
            );
        }

        store_capture_source_selection(
            &app_state,
            &old_display,
            CaptureSourceSelection {
                session_id: old_display.clone(),
                source: test_display_capture_source("windows:display-shared:0"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &old_window,
            CaptureSourceSelection {
                session_id: old_window.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &other_controller_other_source,
            CaptureSourceSelection {
                session_id: other_controller_other_source.clone(),
                source: test_display_capture_source("windows:display-shared:1"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &other_controller_same_source,
            CaptureSourceSelection {
                session_id: other_controller_same_source.clone(),
                source: test_display_capture_source("windows:display-shared:2"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        close_existing_display_lan_sender_sessions_for_source(
            &app_state,
            &next_session,
            &test_display_capture_source("windows:display-shared:2"),
        )
        .await;

        let sessions = app_state.sessions.lock().await;
        assert_eq!(
            sessions.get(&old_display).unwrap().lifecycle_state,
            SessionLifecycleState::Closed
        );
        assert!(!sessions.get(&old_display).unwrap().sender_active);
        assert!(sessions.get(&old_window).unwrap().sender_active);
        assert_eq!(
            sessions.get(&old_window).unwrap().lifecycle_state,
            SessionLifecycleState::Listening
        );
        assert!(
            sessions
                .get(&other_controller_other_source)
                .unwrap()
                .sender_active
        );
        assert_eq!(
            sessions
                .get(&other_controller_other_source)
                .unwrap()
                .lifecycle_state,
            SessionLifecycleState::Listening
        );
        assert_eq!(
            sessions
                .get(&other_controller_same_source)
                .unwrap()
                .lifecycle_state,
            SessionLifecycleState::Closed
        );
        assert!(
            !sessions
                .get(&other_controller_same_source)
                .unwrap()
                .sender_active
        );
    }

    #[tokio::test]
    async fn window_receiver_selection_keeps_same_target_sessions_active() {
        let app_state = Arc::new(AppState::default());
        let next_session = SessionId("new-window-target-a".to_string());
        let old_display = SessionId("old-display-target-a".to_string());
        let old_window = SessionId("old-window-target-a".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                next_session.clone(),
                receiver_snapshot_for_target(&next_session, "target-a"),
            );
            sessions.insert(
                old_display.clone(),
                receiver_snapshot_for_target(&old_display, "target-a"),
            );
            sessions.insert(
                old_window.clone(),
                receiver_snapshot_for_target(&old_window, "target-a"),
            );
        }

        store_capture_source_selection(
            &app_state,
            &old_display,
            CaptureSourceSelection {
                session_id: old_display.clone(),
                source: test_display_capture_source("windows:display-shared:0"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &old_window,
            CaptureSourceSelection {
                session_id: old_window.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        close_existing_display_lan_receiver_sessions_for_target(
            &app_state,
            &next_session,
            &test_window_capture_source("windows:window:0x2222"),
        )
        .await;

        let sessions = app_state.sessions.lock().await;
        assert!(sessions.get(&old_display).unwrap().receiver_active);
        assert_eq!(
            sessions.get(&old_display).unwrap().lifecycle_state,
            SessionLifecycleState::Streaming
        );
        assert!(sessions.get(&old_window).unwrap().receiver_active);
        assert_eq!(
            sessions.get(&old_window).unwrap().lifecycle_state,
            SessionLifecycleState::Streaming
        );
    }

    #[tokio::test]
    async fn display_receiver_selection_closes_only_existing_display_sessions_for_same_target() {
        let app_state = Arc::new(AppState::default());
        let next_session = SessionId("new-display-target-a".to_string());
        let old_display = SessionId("old-display-target-a".to_string());
        let old_window = SessionId("old-window-target-a".to_string());
        let other_target = SessionId("display-target-b".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                next_session.clone(),
                receiver_snapshot_for_target(&next_session, "target-a"),
            );
            sessions.insert(
                old_display.clone(),
                receiver_snapshot_for_target(&old_display, "target-a"),
            );
            sessions.insert(
                old_window.clone(),
                receiver_snapshot_for_target(&old_window, "target-a"),
            );
            sessions.insert(
                other_target.clone(),
                receiver_snapshot_for_target(&other_target, "target-b"),
            );
        }

        store_capture_source_selection(
            &app_state,
            &old_display,
            CaptureSourceSelection {
                session_id: old_display.clone(),
                source: test_display_capture_source("windows:display-shared:0"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &old_window,
            CaptureSourceSelection {
                session_id: old_window.clone(),
                source: test_window_capture_source("windows:window:0x1111"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;
        store_capture_source_selection(
            &app_state,
            &other_target,
            CaptureSourceSelection {
                session_id: other_target.clone(),
                source: test_display_capture_source("windows:display-shared:1"),
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        close_existing_display_lan_receiver_sessions_for_target(
            &app_state,
            &next_session,
            &test_display_capture_source("windows:display-shared:2"),
        )
        .await;

        let sessions = app_state.sessions.lock().await;
        assert_eq!(
            sessions.get(&old_display).unwrap().lifecycle_state,
            SessionLifecycleState::Closed
        );
        assert!(!sessions.get(&old_display).unwrap().receiver_active);
        assert!(sessions.get(&old_window).unwrap().receiver_active);
        assert_eq!(
            sessions.get(&old_window).unwrap().lifecycle_state,
            SessionLifecycleState::Streaming
        );
        assert!(sessions.get(&other_target).unwrap().receiver_active);
        assert_eq!(
            sessions.get(&other_target).unwrap().lifecycle_state,
            SessionLifecycleState::Streaming
        );
    }

    #[tokio::test]
    async fn active_window_capture_count_ignores_remote_and_inactive_selections() {
        let app_state = Arc::new(AppState::default());
        let active_sender = SessionId("active-sender".to_string());
        let remote_controller = SessionId("remote-controller".to_string());
        let failed_sender = SessionId("failed-sender".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                active_sender.clone(),
                sender_snapshot_for_source(&active_sender, "controller-a"),
            );
            sessions.insert(
                remote_controller.clone(),
                receiver_snapshot_for_target(&remote_controller, "target-a"),
            );
            sessions.insert(
                failed_sender.clone(),
                SessionSnapshot {
                    lifecycle_state: SessionLifecycleState::Failed {
                        message: "failed".to_string(),
                    },
                    sender_active: false,
                    ..sender_snapshot_for_source(&failed_sender, "controller-b")
                },
            );
        }

        for session_id in [&active_sender, &remote_controller, &failed_sender] {
            store_capture_source_selection(
                &app_state,
                session_id,
                CaptureSourceSelection {
                    session_id: session_id.clone(),
                    source: test_window_capture_source("windows:window:0x1111"),
                    status: "selected".to_string(),
                    reason: None,
                },
            )
            .await;
        }

        assert_eq!(active_window_capture_count(&app_state).await, 1);
    }

    #[tokio::test]
    async fn display_mode_restore_uses_original_temporary_mode() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("display-mode-restore-session".to_string());
        app_state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), sender_snapshot(&session_id));
        {
            app_state.display_modes.lock().await.record_change(
                session_id.clone(),
                display_mode("requested", 1920, 1080, 144, false),
                Some(display_mode("current", 2560, 1600, 60, true)),
                display_mode("target", 1920, 1080, 144, true),
                true,
            );
        }

        let change = accept_lan_display_mode_restore_with_mode(
            &app_state,
            &session_id,
            display_mode("current", 2560, 1600, 60, false),
        )
        .await
        .unwrap();

        assert_eq!(change.status, "restored");
        assert_eq!(
            change.active.as_ref().map(|mode| mode.id.as_str()),
            Some("current")
        );
        assert!(app_state
            .display_modes
            .lock()
            .await
            .restore_mode(&session_id)
            .is_none());
    }

    #[tokio::test]
    async fn remote_capture_source_selection_reconciles_controller_profile() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("controller-capture-source-profile-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Streaming,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate_mbps: 20,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );

        store_capture_source_selection(
            &app_state,
            &session_id,
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: mrd_ipc::CaptureSource {
                    id: "windows:display:0".to_string(),
                    platform: "windows".to_string(),
                    source_kind: "display".to_string(),
                    title: "Display 1".to_string(),
                    class_name: "Monitor".to_string(),
                    width: 2560,
                    height: 1600,
                    process_id: 0,
                    app_name: Some("Display".to_string()),
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                },
                status: "selected".to_string(),
                reason: None,
            },
        )
        .await;

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("controller profile reconciled to remote source");
        assert_eq!(negotiation.selected.width, 1728);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(
            negotiation.selected_source_id.as_deref(),
            Some("windows:display:0")
        );
        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(
            app_state
                .capture_sources
                .lock()
                .await
                .get(&session_id)
                .expect("stored remote capture source")
                .source
                .id,
            "windows:display:0"
        );
    }

    #[test]
    fn prepare_frame_for_h264_keeps_cpu_frame_when_dimensions_match() {
        let data = vec![7_u8; 64 * 32 * 4];
        let frame = CapturedFrame::from_cpu(64, 32, FramePixelFormat::Bgra32, 1234, data.clone());
        let profile = MediaProfile {
            width: 64,
            height: 32,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        let prepared = prepare_frame_for_h264(frame, &profile).expect("prepared frame");

        assert_eq!(prepared.width, 64);
        assert_eq!(prepared.height, 32);
        assert_eq!(prepared.pixel_format, FramePixelFormat::Bgra32);
        assert_eq!(prepared.data, data);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lan_capture_stream_fps_requests_headroom() {
        let profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(macos_lan_capture_stream_fps(&profile), 120);

        let high_refresh = MediaProfile {
            fps: 165,
            ..profile
        };
        assert_eq!(macos_lan_capture_stream_fps(&high_refresh), 240);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_capture_pump_repeat_pacing_defaults_to_headroom() {
        let profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(macos_capture_pump_repeat_pacing_fps(&profile), 144);
        assert_eq!(
            macos_capture_pump_repeat_frame_interval(&profile),
            media_frame_interval_for_fps(144)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_capture_pump_repeat_grace_uses_capture_headroom() {
        let profile = MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 10,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(
            macos_capture_pump_repeat_grace_timeout(&profile),
            LAN_CAPTURE_PUMP_REPEAT_GRACE_MAX
        );

        let high_refresh = MediaProfile {
            fps: 165,
            ..profile
        };
        assert_eq!(
            macos_capture_pump_repeat_grace_timeout(&high_refresh),
            media_frame_interval_for_fps(240) / 2
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_capture_pump_waits_for_fresh_frame_before_repeating_latest() {
        let latest_frame = CapturedFrame::from_cpu(1, 1, FramePixelFormat::Bgra32, 1, vec![0; 4]);
        let fresh_frame = CapturedFrame::from_cpu(1, 1, FramePixelFormat::Bgra32, 2, vec![1; 4]);
        let shared = Arc::new((
            StdMutex::new(MacosPumpedLanFrameState {
                frames: VecDeque::new(),
                latest_frame: Some(latest_frame),
                sequence: 1,
                error: None,
            }),
            StdCondvar::new(),
        ));
        let producer_shared = shared.clone();
        let producer = thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            let (lock, cvar) = &*producer_shared;
            let mut state = lock.lock().expect("capture pump state");
            state.latest_frame = Some(fresh_frame.clone());
            state.frames.push_back(fresh_frame);
            state.sequence = state.sequence.wrapping_add(1).max(1);
            cvar.notify_all();
        });
        let mut capture = MacosPumpedLanFrameCapture {
            shared,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            repeat_grace_timeout: Duration::from_millis(50),
        };

        let captured = capture.capture_frame().expect("capture pumped frame");
        producer.join().expect("producer thread");

        assert!(!captured.repeated_latest_frame);
        assert_eq!(captured.frame.timestamp_us, 2);
    }

    #[cfg(windows)]
    #[test]
    fn prepare_frame_for_h264_accepts_exact_d3d11_shared_frame() {
        let frame = CapturedFrame::from_d3d11_shared_bgra(64, 32, 1234, 0x1234, 64 * 4);
        let profile = MediaProfile {
            width: 64,
            height: 32,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        let prepared = prepare_frame_for_h264(frame, &profile).expect("prepared shared frame");

        assert_eq!(prepared.width, 64);
        assert_eq!(prepared.height, 32);
        assert!(prepared.data.is_empty());
        assert!(prepared.d3d11_shared_bgra().is_some());
    }

    #[cfg(windows)]
    #[test]
    fn prepare_frame_for_h264_rejects_scaled_d3d11_shared_frame() {
        let frame = CapturedFrame::from_d3d11_shared_bgra(128, 64, 1234, 0x1234, 128 * 4);
        let profile = MediaProfile {
            width: 64,
            height: 32,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        let error = prepare_frame_for_h264(frame, &profile).expect_err("shared scale rejected");

        assert!(error
            .to_string()
            .contains("requires exact selected profile"));
    }

    #[test]
    fn window_h264_capture_dimensions_makes_odd_profile_dimensions_even_and_non_zero() {
        assert_eq!(window_h264_capture_dimensions(1001, 777), (1000, 776));
        assert_eq!(window_h264_capture_dimensions(1, 1), (2, 2));
    }

    #[test]
    fn window_h264_capture_dimensions_returns_even_dimensions_for_odd_window_profile() {
        let (width, height) = window_h264_capture_dimensions(1001, 777);

        assert_eq!(width % 2, 0);
        assert_eq!(height % 2, 0);
        assert!(width >= 2);
        assert!(height >= 2);
    }

    #[test]
    fn capture_sources_ack_strips_preview_payload_before_udp_fit() {
        let sources = (0..24)
            .map(|index| mrd_ipc::CaptureSource {
                id: format!("windows:window:0x{:X}", index + 0x1000),
                platform: "windows".to_string(),
                source_kind: "window".to_string(),
                title: format!("Target App {index}"),
                class_name: "ApplicationFrameWindow".to_string(),
                width: 1280,
                height: 720,
                process_id: 4242 + index,
                app_name: Some(format!("Target App {index}")),
                bundle_identifier: None,
                preview_data_url: Some(format!("legacy-preview-payload-{}", "A".repeat(8_000))),
                preview_width: Some(240),
                preview_height: Some(135),
            })
            .collect();

        let packet = fit_capture_sources_ack_packet(
            "target-instance".to_string(),
            "capture-source-session".to_string(),
            true,
            Some("listed".to_string()),
            sources,
        );

        assert!(serialized_packet_len(&packet) <= DISCOVERY_SAFE_UDP_PAYLOAD_BYTES);
        let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = packet else {
            panic!("expected capture sources ack");
        };
        assert!(sources
            .iter()
            .all(|source| source.preview_data_url.is_none()));
        assert!(sources.iter().all(|source| source.preview_width.is_none()));
        assert!(sources.iter().all(|source| source.preview_height.is_none()));
    }

    #[test]
    fn dynamic_media_probe_frame_preserves_selected_profile() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let frame = build_media_probe_frame(7, 99_000, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.width, 1920);
        assert_eq!(stats.height, 1080);
        assert_eq!(stats.target_fps, 60);
        assert_eq!(stats.target_bitrate_mbps, 20);
        assert_eq!(stats.payload_bytes, media_payload_bytes(&profile) as u32);
        assert_eq!(stats.format, "compressed_h264_test_pattern");
    }

    #[test]
    fn dynamic_hevc_media_probe_frame_preserves_codec() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let frame = build_media_probe_frame(7, 99_000, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.width, 1920);
        assert_eq!(stats.height, 1080);
        assert_eq!(stats.target_fps, 60);
        assert_eq!(stats.target_bitrate_mbps, 20);
        assert_eq!(stats.payload_bytes, media_payload_bytes(&profile) as u32);
        assert_eq!(stats.format, "compressed_hevc_test_pattern");
    }

    #[test]
    fn dynamic_h265_alias_media_probe_frame_uses_hevc_format() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "H.265".to_string(),
            ..MediaProfile::default()
        };
        let frame = build_media_probe_frame(7, 99_000, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(
            LanAccessUnitCodec::from_profile(&profile),
            LanAccessUnitCodec::Hevc
        );
        assert_eq!(stats.format, "compressed_hevc_test_pattern");
    }

    #[test]
    fn decoded_video_probe_format_accepts_h265_aliases() {
        assert_eq!(decoded_video_probe_format("h265"), "hevc_desktop_frame");
        assert_eq!(decoded_video_probe_format("H.265"), "hevc_desktop_frame");
        assert_eq!(decoded_video_probe_format("h.264"), "h264_desktop_frame");
    }

    #[test]
    fn lan_media_v2_envelope_round_trips_h264_access_unit() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let encoded = encode_lan_media_envelope(LanMediaEnvelope {
            payload_type: LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT,
            codec: LAN_MEDIA_CODEC_H264,
            sequence: 99,
            timestamp_us: 123_456,
            profile: profile.clone(),
            payload: vec![0, 0, 0, 1, 0x67],
        })
        .unwrap();

        let decoded = decode_lan_media_envelope(&encoded).unwrap();

        assert_eq!(decoded.payload_type, LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT);
        assert_eq!(decoded.codec, LAN_MEDIA_CODEC_H264);
        assert_eq!(decoded.sequence, 99);
        assert_eq!(decoded.timestamp_us, 123_456);
        assert_eq!(decoded.profile, profile);
        assert_eq!(decoded.payload, vec![0, 0, 0, 1, 0x67]);
    }

    #[test]
    fn lan_media_v2_envelope_round_trips_hevc_access_unit() {
        let profile = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
            ..MediaProfile::default()
        };
        let encoded = encode_lan_media_envelope(LanMediaEnvelope {
            payload_type: LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
            codec: LAN_MEDIA_CODEC_HEVC,
            sequence: 9,
            timestamp_us: 123_456,
            profile: profile.clone(),
            payload: b"fake-hevc".to_vec(),
        })
        .unwrap();

        let decoded = decode_lan_media_envelope(&encoded).unwrap();

        assert_eq!(decoded.payload_type, LAN_MEDIA_PAYLOAD_ACCESS_UNIT);
        assert_eq!(decoded.codec, LAN_MEDIA_CODEC_HEVC);
        assert_eq!(decoded.profile.codec, "hevc");
        assert_eq!(decoded.profile.codec_profile.as_deref(), Some("main"));
        assert_eq!(decoded.profile.chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(decoded.payload, b"fake-hevc");
        assert_eq!(decoded.profile, profile);
    }

    #[test]
    fn hevc_decode_error_message_does_not_use_h264_or_probe_fallback() {
        struct RejectingDecoder;

        impl VideoDecoder for RejectingDecoder {
            fn push_access_unit(
                &mut self,
                _access_unit: &[u8],
            ) -> std::result::Result<(), mrd_pipeline_core::PipelineError> {
                Err(mrd_pipeline_core::PipelineError::message(
                    "synthetic failure",
                ))
            }

            fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
                Vec::new()
            }
        }

        let mut decoder = RejectingDecoder;
        let error =
            decode_lan_desktop_frame(LanAccessUnitCodec::Hevc, &mut decoder, &[0, 0, 1, 0x26])
                .expect_err("decode should fail")
                .to_string();

        assert!(error.contains("HEVC access unit"));
        assert!(!error.contains("H.264"));
        assert!(!error.contains("invalid magic"));
        assert!(!error.contains("probe fallback"));
    }

    #[test]
    fn lan_media_v2_envelope_rejects_legacy_probe_without_magic_fallback() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let legacy_probe = build_media_probe_frame(1, 1_000, &profile);

        let error = decode_lan_media_envelope(&legacy_probe).expect_err("legacy probe is not v2");

        assert!(error.to_string().contains("invalid magic"));
        assert!(!error.to_string().contains("legacy probe fallback"));
    }

    #[tokio::test]
    async fn lan_media_v3_frame_converts_to_receiver_envelope() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("media-v3-session".to_string());
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            MediaProfileNegotiation {
                requested: profile.clone(),
                selected: profile.clone(),
                status: "accepted".to_string(),
                reason: None,
                selected_source_id: None,
                selected_width: Some(profile.width),
                selected_height: Some(profile.height),
                downgrade_reason: None,
            },
        );

        let converted = quic_media_v3_frame_to_legacy_frame(
            &app_state,
            &session_id,
            QuicMediaFrame {
                payload_type: QuicMediaPayloadType::AccessUnit,
                codec: QuicMediaCodec::H264,
                profile_id: lan_media_profile_id(&profile),
                frame_id: 42,
                timestamp_us: 123_456,
                flags: 1,
                payload: vec![0, 0, 0, 1, 0x65].into(),
            },
            QuicAuReassemblerStats::default(),
        )
        .await
        .unwrap()
        .expect("converted frame");

        assert_eq!(converted.frame_id, 42);
        assert!(converted.is_keyframe);
        let envelope = decode_lan_media_envelope(&converted.payload).unwrap();
        assert_eq!(envelope.payload_type, LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT);
        assert_eq!(envelope.codec, LAN_MEDIA_CODEC_H264);
        assert_eq!(envelope.sequence, 42);
        assert_eq!(envelope.profile, profile);
        assert_eq!(envelope.payload, vec![0, 0, 0, 1, 0x65]);
    }

    #[tokio::test]
    async fn lan_media_v3_frame_converts_hevc_to_receiver_envelope() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("media-v3-hevc-session".to_string());
        let profile = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
            ..MediaProfile::default()
        };
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            MediaProfileNegotiation {
                requested: profile.clone(),
                selected: profile.clone(),
                status: "accepted".to_string(),
                reason: None,
                selected_source_id: None,
                selected_width: Some(profile.width),
                selected_height: Some(profile.height),
                downgrade_reason: None,
            },
        );

        let converted = quic_media_v3_frame_to_legacy_frame(
            &app_state,
            &session_id,
            QuicMediaFrame {
                payload_type: QuicMediaPayloadType::AccessUnit,
                codec: QuicMediaCodec::Hevc,
                profile_id: lan_media_profile_id(&profile),
                frame_id: 12,
                timestamp_us: 77,
                flags: 1,
                payload: b"hevc-au".to_vec().into(),
            },
            QuicAuReassemblerStats::default(),
        )
        .await
        .unwrap()
        .expect("converted frame");

        let envelope = decode_lan_media_envelope(&converted.payload).unwrap();
        assert_eq!(envelope.payload_type, LAN_MEDIA_PAYLOAD_ACCESS_UNIT);
        assert_eq!(envelope.codec, LAN_MEDIA_CODEC_HEVC);
        assert_eq!(envelope.profile.codec, "hevc");
        assert_eq!(envelope.payload, b"hevc-au");
        assert_eq!(envelope.profile, profile);
    }

    #[tokio::test]
    async fn lan_media_v3_profile_mismatch_is_transient_drop() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("media-v3-mismatch-session".to_string());
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let stale_profile = MediaProfile {
            fps: 60,
            ..profile.clone()
        };
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            MediaProfileNegotiation {
                requested: profile.clone(),
                selected: profile,
                status: "accepted".to_string(),
                reason: None,
                selected_source_id: None,
                selected_width: Some(1920),
                selected_height: Some(1080),
                downgrade_reason: None,
            },
        );

        let converted = quic_media_v3_frame_to_legacy_frame(
            &app_state,
            &session_id,
            QuicMediaFrame {
                payload_type: QuicMediaPayloadType::AccessUnit,
                codec: QuicMediaCodec::H264,
                profile_id: lan_media_profile_id(&stale_profile),
                frame_id: 7,
                timestamp_us: 123_456,
                flags: 1,
                payload: vec![0, 0, 0, 1, 0x65].into(),
            },
            QuicAuReassemblerStats::default(),
        )
        .await
        .unwrap();

        assert!(converted.is_none());
        let snapshot = app_state.probes.lock().await.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 0);
        assert_eq!(snapshot.frames_dropped, 1);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn lan_sender_stats_datagram_round_trips_without_media_sequence() {
        let payload = LanSenderStatsPayload {
            sequence: 123,
            frame_count: 122,
            source_id: Some("windows:display-shared:0".to_string()),
            target_fps: 144,
            target_bitrate_mbps: 20,
            metrics: vec![MediaStageMetrics {
                stage: "sender.encode".to_string(),
                p50_ms: Some(1.2),
                p95_ms: Some(2.4),
            }],
            sender_transport: MediaSenderTransportSnapshot {
                capture_source_id: Some("windows:display-shared:0".to_string()),
                capture_source_kind: Some("display_shared".to_string()),
                capture_memory_path: Some("d3d11_shared_bgra".to_string()),
                dynamic_fps_tier: None,
                target_fps: Some(144),
                frames_completed: 122,
                repeated_latest_frames: 3,
                datagram_fragments_attempted: 4,
                datagram_fragments_sent: 3,
                datagram_fragments_delayed: 0,
                datagram_fragments_dropped_by_impairment: 0,
                datagram_fragments_dropped_for_capacity: 1,
                datagram_fragments_dropped_for_budget: 0,
                datagram_frames_cut_short_for_capacity: 1,
                datagram_frames_cut_short_for_budget: 0,
                reliable_fragments_sent: 0,
                reliable_frames_sent: 0,
                ..MediaSenderTransportSnapshot::default()
            },
            test_impairment: None,
        };

        let encoded = encode_lan_sender_stats_datagram(&payload).unwrap();
        let decoded = decode_lan_sender_stats_datagram(&encoded).unwrap();

        assert_eq!(decoded, Some(payload));
        assert_eq!(
            decode_lan_sender_stats_datagram(b"not-stats").unwrap(),
            None
        );
    }

    #[test]
    fn lan_sender_stats_tracker_accumulates_transport_counters() {
        let mut tracker = LanSenderStatsTracker::new(Instant::now());
        tracker.record_datagram_frame(LanSenderDatagramFrameReport {
            fragments_attempted: 5,
            fragments_sent: 3,
            fragments_delayed: 1,
            fragments_dropped_by_impairment: 1,
            fragments_dropped_for_capacity: 1,
            fragments_dropped_for_budget: 0,
            cut_short_for_capacity: true,
            cut_short_for_budget: false,
        });
        tracker.record_datagram_frame(LanSenderDatagramFrameReport {
            fragments_attempted: 4,
            fragments_sent: 2,
            fragments_delayed: 0,
            fragments_dropped_by_impairment: 0,
            fragments_dropped_for_capacity: 0,
            fragments_dropped_for_budget: 2,
            cut_short_for_capacity: false,
            cut_short_for_budget: true,
        });
        tracker.record_reliable_frame(7, true);
        tracker.record_repeated_latest_frame();
        tracker.record_captured_frame(&CapturedFrame::from_cpu(
            1,
            1,
            FramePixelFormat::Bgra32,
            0,
            vec![0; 4],
        ));
        tracker.record_captured_frame(&CapturedFrame::from_cpu(
            2,
            2,
            FramePixelFormat::Nv12,
            0,
            vec![0; 6],
        ));
        tracker.record_encoded_access_unit(1_024, true);
        tracker.record_encoded_access_unit(256, false);
        tracker.frame_completed();

        assert_eq!(tracker.sender_transport.frames_completed, 1);
        assert_eq!(tracker.sender_transport.repeated_latest_frames, 1);
        assert_eq!(tracker.sender_transport.capture_frame_samples, 2);
        assert_eq!(tracker.sender_transport.capture_cpu_frames, 2);
        assert_eq!(tracker.sender_transport.capture_bgra32_frames, 1);
        assert_eq!(tracker.sender_transport.capture_nv12_frames, 1);
        assert_eq!(tracker.sender_transport.access_units_encoded, 2);
        assert_eq!(tracker.sender_transport.keyframes_encoded, 1);
        assert_eq!(tracker.sender_transport.encoded_access_unit_bytes, 1_280);
        assert_eq!(tracker.sender_transport.datagram_fragments_attempted, 9);
        assert_eq!(tracker.sender_transport.datagram_fragments_sent, 5);
        assert_eq!(tracker.sender_transport.datagram_fragments_delayed, 1);
        assert_eq!(
            tracker
                .sender_transport
                .datagram_fragments_dropped_by_impairment,
            1
        );
        assert_eq!(
            tracker
                .sender_transport
                .datagram_fragments_dropped_for_capacity,
            1
        );
        assert_eq!(
            tracker
                .sender_transport
                .datagram_fragments_dropped_for_budget,
            2
        );
        assert_eq!(
            tracker
                .sender_transport
                .datagram_frames_cut_short_for_capacity,
            1
        );
        assert_eq!(
            tracker
                .sender_transport
                .datagram_frames_cut_short_for_budget,
            1
        );
        assert_eq!(tracker.sender_transport.reliable_fragments_sent, 7);
        assert_eq!(tracker.sender_transport.reliable_frames_sent, 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_sender_uses_monitor_specific_backends_for_display_sources() {
        assert_eq!(
            windows_lan_capture_backend("windows:display-shared:0", false),
            WindowsLanCaptureBackend::DxgiShared
        );
        assert_eq!(
            windows_lan_capture_backend("windows:display:0", true),
            WindowsLanCaptureBackend::Winrt
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_selects_winrt_window_shared_for_window_sources() {
        assert_eq!(
            windows_lan_capture_backend("windows:window:0x1234", true),
            WindowsLanCaptureBackend::WinrtWindowShared
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_keeps_window_sources_on_cpu_when_nvenc_h264_is_unavailable() {
        assert_eq!(
            windows_lan_capture_backend("windows:window:0x1234", false),
            WindowsLanCaptureBackend::Winrt
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_window_shared_capture_uses_shared_texture_when_nvenc_h264_is_available() {
        assert!(windows_lan_window_capture_uses_shared_texture(true));
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_window_shared_capture_uses_cpu_texture_when_nvenc_h264_is_unavailable() {
        assert!(!windows_lan_window_capture_uses_shared_texture(false));
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_keeps_dxgi_shared_for_display_shared_sources() {
        assert_eq!(
            windows_lan_capture_backend("windows:display-shared:1", false),
            WindowsLanCaptureBackend::DxgiShared
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_for_profile_keeps_shared_for_full_size_display() {
        assert_eq!(
            windows_lan_capture_backend_for_profile(
                "windows:display-shared:1",
                2560,
                1440,
                &test_media_profile(2560, 1440),
                false
            ),
            WindowsLanCaptureBackend::DxgiShared
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_for_profile_keeps_shared_for_reduced_display() {
        assert_eq!(
            windows_lan_capture_backend_for_profile(
                "windows:display-shared:1",
                2560,
                1440,
                &test_media_profile(1920, 1080),
                false
            ),
            WindowsLanCaptureBackend::DxgiShared
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_for_profile_keeps_shared_for_full_size_window() {
        assert_eq!(
            windows_lan_capture_backend_for_profile(
                "windows:window:0x1234",
                1280,
                720,
                &test_media_profile(1280, 720),
                true
            ),
            WindowsLanCaptureBackend::WinrtWindowShared
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lan_capture_backend_for_profile_uses_scaling_path_for_reduced_window() {
        assert_eq!(
            windows_lan_capture_backend_for_profile(
                "windows:window:0x1234",
                1280,
                720,
                &test_media_profile(960, 540),
                true
            ),
            WindowsLanCaptureBackend::Winrt
        );
    }

    #[cfg(windows)]
    fn test_media_profile(width: u32, height: u32) -> MediaProfile {
        MediaProfile {
            width,
            height,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }
    }

    #[cfg(windows)]
    #[test]
    fn parse_windows_window_source_id_extracts_hwnd() {
        assert_eq!(
            parse_windows_window_source_id("windows:window:0x1234").unwrap(),
            0x1234
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_windows_window_source_id_rejects_display_source() {
        let error = parse_windows_window_source_id("windows:display-shared:1")
            .unwrap_err()
            .to_string();

        assert!(error.contains("window"));
    }

    #[test]
    fn lan_sender_encoder_order_prefers_hardware_before_fallback() {
        let backends = preferred_lan_h264_encoder_backends();
        #[cfg(windows)]
        assert_eq!(backends, ["nvenc_h264", "openh264"]);
        #[cfg(target_os = "macos")]
        assert_eq!(backends, ["videotoolbox_h264", "openh264"]);
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(backends, ["openh264"]);
    }

    #[test]
    fn lan_quic_media_routes_only_keyframes_reliably() {
        assert!(should_send_access_unit_reliably(true, true, 1024, 1_200));
        assert!(!should_send_access_unit_reliably(
            true,
            false,
            32 * 1024 + 1,
            1_200
        ));
        assert!(!should_send_access_unit_reliably(true, false, 1_200, 1_200));
        assert!(!should_send_access_unit_reliably(true, false, 512, 1_200));
        assert!(!should_send_access_unit_reliably(
            false,
            true,
            32 * 1024 + 1,
            1_200
        ));
    }

    #[test]
    fn lan_quic_media_uses_datagrams_by_default_and_reliable_whole_frame_only_when_enabled() {
        let profile_1080p = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let profile_2k = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            2,
            &profile_1080p,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            2,
            &profile_2k,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            1,
            &profile_2k,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            false,
            2,
            &profile_2k,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            false,
            true,
            2,
            &profile_2k,
            None
        ));
        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            2,
            &profile_1080p,
            Some(true)
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            2,
            &profile_2k,
            Some(false)
        ));
    }

    #[test]
    fn lan_quic_media_uses_best_effort_only_for_low_latency_bitrate_tiers() {
        let low_latency = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let high_quality_2k144 = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let high_bitrate = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(use_best_effort_media_datagrams(&low_latency));
        assert!(!use_best_effort_media_datagrams(&high_quality_2k144));
        assert!(!use_best_effort_media_datagrams(&high_bitrate));
    }

    #[test]
    fn high_refresh_datagram_send_budget_requires_reliable_media() {
        let high_refresh = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 144,
            bitrate_mbps: 96,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let low_bitrate = MediaProfile {
            bitrate_mbps: 40,
            ..high_refresh.clone()
        };
        let low_refresh = MediaProfile {
            fps: 60,
            ..high_refresh.clone()
        };

        assert_eq!(
            lan_datagram_frame_send_budget(&high_refresh, true),
            Some(LAN_QUIC_DATAGRAM_SEND_BUDGET)
        );
        assert_eq!(lan_datagram_frame_send_budget(&high_refresh, false), None);
        assert_eq!(lan_datagram_frame_send_budget(&low_bitrate, true), None);
        assert_eq!(lan_datagram_frame_send_budget(&low_refresh, true), None);
    }

    #[test]
    fn high_quality_lan_media_keeps_safe_datagram_size_by_default() {
        let profile = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(
            lan_media_datagram_size(1_500, &profile, true),
            LAN_QUIC_FALLBACK_DATAGRAM_BYTES
        );
        assert_eq!(
            lan_media_datagram_size(1_500, &profile, false),
            LAN_QUIC_FALLBACK_DATAGRAM_BYTES
        );
    }

    #[test]
    fn lan_quic_media_prefers_persistent_reliable_stream_when_available() {
        assert_eq!(
            select_reliable_media_send_mode(true, true),
            LanReliableMediaSendMode::Persistent
        );
        assert_eq!(
            select_reliable_media_send_mode(false, true),
            LanReliableMediaSendMode::Persistent
        );
        assert_eq!(
            select_reliable_media_send_mode(true, false),
            LanReliableMediaSendMode::PerMessage
        );
        assert_eq!(
            select_reliable_media_send_mode(false, false),
            LanReliableMediaSendMode::Disabled
        );
    }

    #[test]
    fn high_refresh_reliable_media_prefers_per_message_streams_to_reduce_hol() {
        let high_bitrate = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let stable_bitrate = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(
            select_reliable_media_send_mode_for_profile(true, true, &high_bitrate),
            LanReliableMediaSendMode::PerMessage
        );
        assert_eq!(
            select_reliable_media_send_mode_for_profile(true, true, &stable_bitrate),
            LanReliableMediaSendMode::PerMessage
        );
        assert_eq!(
            select_reliable_media_send_mode_for_profile(false, true, &high_bitrate),
            LanReliableMediaSendMode::Persistent
        );
    }

    #[test]
    fn reliable_whole_frame_media_env_override_parses_truthy_and_falsey_values() {
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(Some("1")),
            Some(true)
        );
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(Some("true")),
            Some(true)
        );
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(Some("0")),
            Some(false)
        );
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(Some("off")),
            Some(false)
        );
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(Some("")),
            None
        );
        assert_eq!(
            reliable_whole_frame_media_override_from_env_value(None),
            None
        );
    }

    #[test]
    fn render_pacing_env_override_parses_truthy_and_falsey_values() {
        assert_eq!(lan_render_pacing_from_env_value(None), None);
        assert_eq!(lan_render_pacing_from_env_value(Some("")), None);
        assert_eq!(lan_render_pacing_from_env_value(Some("0")), Some(false));
        assert_eq!(lan_render_pacing_from_env_value(Some("off")), Some(false));
        assert_eq!(lan_render_pacing_from_env_value(Some("1")), Some(true));
        assert_eq!(lan_render_pacing_from_env_value(Some("true")), Some(true));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_compressed_proxy_defaults_away_from_high_throughput_hevc() {
        assert!(!macos_render_proxy_compressed_media_enabled_for_values(
            "hevc", 2560, 1440, 144, None
        ));
        assert!(macos_render_proxy_compressed_media_enabled_for_values(
            "h264", 2560, 1440, 144, None
        ));
        assert!(macos_render_proxy_compressed_media_enabled_for_values(
            "hevc", 1920, 1080, 144, None
        ));
        assert!(macos_render_proxy_compressed_media_enabled_for_values(
            "hevc", 2560, 1440, 60, None
        ));
        assert!(macos_render_proxy_compressed_media_enabled_for_values(
            "hevc",
            2560,
            1440,
            144,
            Some(true)
        ));
        assert!(!macos_render_proxy_compressed_media_enabled_for_values(
            "h264",
            2560,
            1440,
            144,
            Some(false)
        ));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn render_queue_policy_env_parses_values() {
        assert_eq!(lan_render_queue_policy_from_env_value(None), None);
        assert_eq!(lan_render_queue_policy_from_env_value(Some("")), None);
        assert_eq!(
            lan_render_queue_policy_from_env_value(Some("latest")),
            Some(LanRenderQueuePolicy::Latest)
        );
        assert_eq!(
            lan_render_queue_policy_from_env_value(Some("low_latency")),
            Some(LanRenderQueuePolicy::Latest)
        );
        assert_eq!(
            lan_render_queue_policy_from_env_value(Some("paced_fifo")),
            Some(LanRenderQueuePolicy::PacedFifo)
        );
        assert_eq!(
            lan_render_queue_policy_from_env_value(Some("fifo")),
            Some(LanRenderQueuePolicy::PacedFifo)
        );
        assert_eq!(
            lan_render_queue_policy_from_env_value(Some("invalid")),
            None
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn render_queue_policy_defaults_by_platform_and_allows_latest_override() {
        let high_fps = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let low_fps = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        #[cfg(windows)]
        let expected_high_fps_default = LanRenderQueuePolicy::PacedFifo;
        #[cfg(target_os = "macos")]
        let expected_high_fps_default = LanRenderQueuePolicy::Latest;

        assert_eq!(
            lan_render_queue_policy_for_profile_with_override(&high_fps, None),
            expected_high_fps_default
        );
        assert_eq!(
            lan_render_queue_policy_for_profile_with_override(&low_fps, None),
            LanRenderQueuePolicy::PacedFifo
        );
        assert_eq!(
            lan_render_queue_policy_for_profile_with_override(
                &high_fps,
                Some(LanRenderQueuePolicy::Latest)
            ),
            LanRenderQueuePolicy::Latest
        );
    }

    #[test]
    fn media_payload_hash_mode_defaults_to_metadata_for_high_fps() {
        let high_fps = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let low_fps = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert_eq!(
            lan_media_payload_hash_mode_for_profile_with_override(&high_fps, None),
            LanMediaPayloadHashMode::Metadata
        );
        assert_eq!(
            lan_media_payload_hash_mode_for_profile_with_override(&low_fps, None),
            LanMediaPayloadHashMode::Full
        );
        assert_eq!(
            lan_media_payload_hash_mode_from_env_value(Some("full")),
            Some(LanMediaPayloadHashMode::Full)
        );
        assert_eq!(
            lan_media_payload_hash_mode_from_env_value(Some("metadata")),
            Some(LanMediaPayloadHashMode::Metadata)
        );
        assert_eq!(
            lan_media_payload_hash_mode_from_env_value(Some("off")),
            Some(LanMediaPayloadHashMode::Disabled)
        );

        let payload = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            lan_media_payload_hash_for_mode(
                LanMediaPayloadHashMode::Full,
                &high_fps,
                42,
                123_456,
                &payload
            ),
            format!("fnv1a64:{:016x}", fnv1a64(&payload))
        );
        assert!(lan_media_payload_hash_for_mode(
            LanMediaPayloadHashMode::Metadata,
            &high_fps,
            42,
            123_456,
            &payload
        )
        .starts_with("fnv1a64:meta:"));
        assert_eq!(
            lan_media_payload_hash_for_mode(
                LanMediaPayloadHashMode::Disabled,
                &high_fps,
                42,
                123_456,
                &payload
            ),
            "fnv1a64:disabled"
        );
    }

    #[test]
    fn lan_keyframe_request_control_datagram_roundtrips() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let datagram =
            encode_lan_keyframe_request_datagram(&profile, 7, LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
                .expect("encode keyframe request");

        assert!(decode_lan_keyframe_request_datagram(&datagram).expect("decode request"));
    }

    #[test]
    fn lan_media_profile_identity_includes_color_fields() {
        let base = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let mut grayscale = base.clone();
        grayscale.color_mode = Some("grayscale".to_string());
        let mut hdr = base.clone();
        hdr.color_pipeline = Some("hdr_main10".to_string());

        assert_ne!(
            lan_media_profile_id(&base),
            lan_media_profile_id(&grayscale)
        );
        assert_ne!(lan_media_profile_id(&base), lan_media_profile_id(&hdr));
        assert_ne!(
            fnv1a64_media_metadata(&base, 7, 123_456, 4096),
            fnv1a64_media_metadata(&grayscale, 7, 123_456, 4096)
        );
        assert!(format_media_profile(&grayscale).contains("color=grayscale"));
        assert!(format_media_profile(&hdr).contains("pipeline=hdr_main10"));
    }

    #[test]
    fn lan_keyframe_request_decoder_ignores_access_units() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let datagram = fragment_media_payload_v3(
            QuicMediaPayloadType::AccessUnit,
            QuicMediaCodec::H264,
            lan_media_profile_id(&profile),
            1,
            123,
            true,
            &[0, 0, 0, 1, 0x65],
            LAN_QUIC_FALLBACK_DATAGRAM_BYTES,
        )
        .expect("fragment access unit")
        .remove(0);

        assert!(!decode_lan_keyframe_request_datagram(&datagram).expect("decode access unit"));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn latest_render_queue_policy_skips_pacing_wait() {
        let high_fps = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(!lan_render_policy_allows_service_pacing(
            LanRenderQueuePolicy::Latest,
            &high_fps,
            false
        ));
        assert!(lan_render_policy_allows_service_pacing(
            LanRenderQueuePolicy::PacedFifo,
            &high_fps,
            false
        ));
        assert!(!lan_render_policy_allows_service_pacing(
            LanRenderQueuePolicy::PacedFifo,
            &high_fps,
            true
        ));
        assert_eq!(
            lan_render_queue_capacity_for_policy(&high_fps, LanRenderQueuePolicy::Latest),
            1
        );
        assert_eq!(
            lan_render_queue_capacity_for_policy(&high_fps, LanRenderQueuePolicy::PacedFifo),
            lan_render_queue_capacity_for_profile(&high_fps)
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn latest_render_queue_policy_takes_latest_and_reports_stale_drops() {
        let mut registry = crate::app_state::MediaRenderQueueRegistry::default();
        let session_id = SessionId("latest-render-policy-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
        let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));

        match registry.enqueue_bounded(session_id.clone(), first, 3) {
            MediaRenderQueueEnqueue::Start(_) => {}
            other => panic!("expected render worker start, got {other:?}"),
        }
        registry.enqueue_bounded(session_id.clone(), second, 3);
        registry.enqueue_bounded(session_id.clone(), third, 3);
        registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3);

        let (next, dropped) = take_next_lan_render_frame_for_policy(
            &mut registry,
            &session_id,
            LanRenderQueuePolicy::Latest,
        );

        assert_eq!(next, Some(fourth));
        assert_eq!(dropped, 2);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn paced_fifo_render_queue_policy_takes_next_without_stale_drops() {
        let mut registry = crate::app_state::MediaRenderQueueRegistry::default();
        let session_id = SessionId("paced-render-policy-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));

        match registry.enqueue_bounded(session_id.clone(), first, 3) {
            MediaRenderQueueEnqueue::Start(_) => {}
            other => panic!("expected render worker start, got {other:?}"),
        }
        registry.enqueue_bounded(session_id.clone(), second.clone(), 3);
        registry.enqueue_bounded(session_id.clone(), third, 3);

        let (next, dropped) = take_next_lan_render_frame_for_policy(
            &mut registry,
            &session_id,
            LanRenderQueuePolicy::PacedFifo,
        );

        assert_eq!(next, Some(second));
        assert_eq!(dropped, 0);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn render_queue_capacity_env_keeps_bounded_burst_backlog() {
        assert_eq!(
            lan_render_queue_capacity_from_env_value(None),
            LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
        );
        assert_eq!(lan_render_queue_capacity_from_env_value(Some("1")), 1);
        assert_eq!(lan_render_queue_capacity_from_env_value(Some("6")), 6);
        assert_eq!(
            lan_render_queue_capacity_from_env_value(Some("128")),
            LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT
        );
        assert_eq!(
            lan_render_queue_capacity_from_env_value(Some("invalid")),
            LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn render_pacing_defaults_to_interruptible_refresh_cap() {
        let high_fps = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let low_fps = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert!(lan_render_pacing_enabled_for_profile(&high_fps));
        assert_eq!(
            lan_render_queue_capacity_for_profile(&high_fps),
            LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES
        );
        assert_eq!(
            lan_render_pacing_target_fps_from_values(high_fps.fps, Some(144)),
            144
        );
        assert_eq!(
            lan_render_pacing_target_fps_from_values(high_fps.fps, Some(240)),
            165
        );
        assert_eq!(
            lan_render_cap_target_fps_for_profile(&high_fps),
            Some(lan_render_pacing_target_fps(&high_fps))
        );
        assert_eq!(
            render_profile_requests_high_resolution_timer(&high_fps),
            lan_render_pacing_target_fps(&high_fps) >= LAN_RENDER_PACING_PRECISE_SLEEP_MIN_FPS
        );
        let precise_guard = render_pacing_precise_sleep_guard(120);
        assert!(precise_guard > Duration::ZERO);
        assert!(precise_guard < render_pacing_frame_interval(120));
        assert_eq!(render_pacing_precise_sleep_guard(60), Duration::ZERO);
        assert_eq!(
            lan_render_pacing_render_start_delay(Duration::from_micros(7_000), 144),
            Duration::from_micros(6_750)
        );
        assert_eq!(
            lan_render_pacing_render_start_delay(Duration::from_micros(7_000), 60),
            Duration::from_micros(7_000)
        );
        assert!(!should_interrupt_render_pacing_sleep(0, 3));
        assert!(should_interrupt_render_pacing_sleep(1, 3));
        assert!(should_interrupt_render_pacing_sleep(2, 3));
        assert!(should_interrupt_render_pacing_sleep(1, 1));
        assert_eq!(
            lan_render_pacing_target_fps_from_values(high_fps.fps, None),
            165
        );
        assert!(!lan_render_pacing_enabled_for_profile(&low_fps));
        assert_eq!(lan_render_queue_capacity_for_profile(&low_fps), 1);
        assert_eq!(lan_render_cap_target_fps_for_profile(&low_fps), None);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn surface_renderer_lock_waits_through_short_contention() {
        let mutex = Arc::new(std::sync::Mutex::new(()));
        let guard = mutex.lock().expect("hold test mutex");
        let waiter = {
            let mutex = mutex.clone();
            std::thread::spawn(move || {
                wait_for_mutex_guard(&mutex, Duration::from_millis(20))
                    .expect("wait for mutex")
                    .is_some()
            })
        };

        std::thread::sleep(Duration::from_millis(2));
        drop(guard);

        assert!(waiter.join().expect("waiter thread"));
    }

    #[test]
    fn below_high_refresh_stability_tier_keeps_delta_frames_on_datagrams_by_default() {
        let stable_bitrate = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 120,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &stable_bitrate,
            None
        ));
    }

    #[test]
    fn high_refresh_stability_tier_keeps_delta_frames_on_datagrams_by_default() {
        let stability_tier = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 64,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &stability_tier,
            None
        ));
    }

    #[test]
    fn ultra_high_bitrate_uses_reliable_whole_frame_by_default() {
        let ultra_high = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let render_capped = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 144,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let high_refresh_2k180 = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 180,
            bitrate_mbps: 100,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let stable_2k144 = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &ultra_high,
            None
        ));
        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &render_capped,
            None
        ));
        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_refresh_2k180,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &stable_2k144,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            false,
            true,
            64,
            &ultra_high,
            None
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            false,
            64,
            &ultra_high,
            None
        ));
    }

    #[test]
    fn reliable_whole_frame_requires_explicit_override() {
        let high_quality_2k120 = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 120,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k120,
            None
        ));
        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k120,
            Some(true)
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k120,
            Some(false)
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            false,
            true,
            64,
            &high_quality_2k120,
            None
        ));
    }

    #[test]
    fn lan_quic_reliable_keyframe_fragments_match_datagram_fragments() {
        let payload = vec![0x33; 4096];
        let fragments = fragment_access_unit(42, 12_345, true, &payload, 1_200).unwrap();
        assert!(fragments.len() > 1);

        let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig {
            frame_timeout: Duration::from_secs(1),
            max_pending_frames: 8,
        });

        assert!(reassembler.push_datagram(&fragments[0]).unwrap().is_none());
        assert!(reassembler.push_datagram(&fragments[0]).unwrap().is_none());

        let mut completed = None;
        for fragment in fragments.iter().skip(1) {
            completed = reassembler.push_datagram(fragment).unwrap();
        }

        let frame = completed.expect("keyframe should complete after all fragments");
        assert_eq!(frame.frame_id, 42);
        assert!(frame.is_keyframe);
        assert_eq!(frame.payload.as_ref(), payload.as_slice());
        assert_eq!(reassembler.stats().duplicate_fragments, 1);
    }

    #[test]
    fn lan_sender_treats_h264_idr_payload_as_keyframe() {
        let idr_annexb = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        let p_slice_annexb = [0, 0, 1, 0x41, 0x9a];
        let idr_avcc = [0, 0, 0, 3, 0x65, 0x88, 0x84];

        assert!(h264_access_unit_is_keyframe(false, &idr_annexb));
        assert!(h264_access_unit_is_keyframe(false, &idr_avcc));
        assert!(!h264_access_unit_is_keyframe(false, &p_slice_annexb));
        assert!(h264_access_unit_is_keyframe(true, &p_slice_annexb));
    }

    #[test]
    fn decoded_frame_to_rgb24_accepts_nv12_decoder_output() {
        let frame = DecodedFrame {
            width: 2,
            height: 2,
            timestamp_us: 0,
            data: DecodedFrameData::CpuNv12 {
                data: vec![235, 235, 235, 235, 128, 128],
                pitch: 2,
            },
        };

        let (width, height, rgb) = decoded_frame_to_rgb24(frame).unwrap();

        assert_eq!((width, height), (2, 2));
        assert_eq!(rgb.len(), 2 * 2 * 3);
        assert!(rgb.iter().all(|channel| *channel >= 250));
    }

    #[test]
    fn media_frame_scheduler_does_not_add_processing_time_to_interval() {
        let start = Instant::now();
        let interval = Duration::from_millis(16);
        let mut next_frame_at = start;

        assert_eq!(
            schedule_next_media_frame(start, &mut next_frame_at, interval),
            None
        );
        assert_eq!(next_frame_at, start + interval);

        assert_eq!(
            schedule_next_media_frame(
                start + Duration::from_millis(15),
                &mut next_frame_at,
                interval
            ),
            Some(start + interval)
        );
        assert_eq!(next_frame_at, start + interval + interval);
    }

    #[test]
    fn media_frame_scheduler_resets_after_large_stall() {
        let start = Instant::now();
        let interval = Duration::from_millis(16);
        let mut next_frame_at = start + interval;
        let now = start + Duration::from_millis(80);

        assert_eq!(
            schedule_next_media_frame(now, &mut next_frame_at, interval),
            None
        );
        assert_eq!(next_frame_at, now + interval);
    }

    #[test]
    fn high_refresh_media_profiles_request_high_resolution_timer() {
        let high_refresh = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let low_refresh = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert!(media_profile_requests_high_resolution_timer(&high_refresh));
        assert!(!media_profile_requests_high_resolution_timer(&low_refresh));
    }

    #[test]
    fn high_refresh_media_profiles_use_precise_sleep_guard() {
        let high_refresh = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let low_refresh = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        let guard = media_frame_precise_sleep_guard(&high_refresh);

        assert!(guard > Duration::ZERO);
        assert!(guard < media_frame_interval(&high_refresh));
        assert_eq!(
            media_frame_precise_sleep_guard(&low_refresh),
            Duration::ZERO
        );
        assert_eq!(
            media_frame_precise_sleep_chunk(Duration::from_millis(5), guard),
            Some(Duration::from_millis(1))
        );
        assert_eq!(media_frame_precise_sleep_chunk(guard, guard), None);
    }

    #[tokio::test]
    async fn media_profile_update_changes_active_quic_session_profile() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("profile-update-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .set(session_id.clone(), vec!["decode.software".to_string()]);

        let negotiation = accept_lan_media_profile_update(
            &app_state,
            &session_id,
            MediaProfile {
                width: 1280,
                height: 720,
                fps: 60,
                bitrate_mbps: 8,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(negotiation.status, "accepted");
        assert_eq!(negotiation.selected.width, 1280);
        assert_eq!(
            app_state
                .media_profiles
                .lock()
                .await
                .get(&session_id)
                .expect("profile update result")
                .selected
                .height,
            720
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn d3d11_shared_preview_failure_still_counts_decoded_frame() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("d3d11-shared-preview-session".to_string());
        let frame = DecodedFrame::from_d3d11_shared_nv12(1920, 1080, 123_456, 1, 2);
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        record_lan_decoded_frames(
            &app_state,
            &session_id,
            vec![frame],
            1024,
            60,
            123_456,
            &profile,
            &[1, 2, 3, 4],
        )
        .await;

        let snapshot = app_state.probes.lock().await.snapshot(&session_id);

        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(snapshot.last_media_sequence, Some(60));
        assert!(snapshot.latest_frame_data_url.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn upload_lan_render_frame_dispatches_macos_compressed_access_units() {
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

        #[derive(Default)]
        struct CompressedDispatchRenderer {
            decoded_uploads: u64,
            h264_upload: Option<(usize, usize, u64, Vec<u8>)>,
            hevc_upload: Option<(usize, usize, u64, Vec<u8>)>,
        }

        impl RendererInstance for CompressedDispatchRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                self.decoded_uploads += 1;
                Ok(())
            }

            fn upload_h264_access_unit(
                &mut self,
                width: usize,
                height: usize,
                timestamp_us: u64,
                payload: bytes::Bytes,
            ) -> Result<(), RenderError> {
                self.h264_upload = Some((width, height, timestamp_us, payload.to_vec()));
                Ok(())
            }

            fn upload_hevc_access_unit(
                &mut self,
                width: usize,
                height: usize,
                timestamp_us: u64,
                payload: bytes::Bytes,
            ) -> Result<(), RenderError> {
                self.hevc_upload = Some((width, height, timestamp_us, payload.to_vec()));
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: self.decoded_uploads,
                    presented_frame_count: self.decoded_uploads,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: None,
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 0,
                    last_height: 0,
                    last_pixel_format: None,
                }
            }
        }

        let mut renderer = CompressedDispatchRenderer::default();

        upload_lan_render_frame(
            &mut renderer,
            MediaRenderFrame::H264AccessUnit {
                width: 640,
                height: 360,
                timestamp_us: 123,
                payload: bytes::Bytes::from_static(b"h264-au"),
            },
        )
        .expect("dispatch H.264 access unit");
        upload_lan_render_frame(
            &mut renderer,
            MediaRenderFrame::HevcAccessUnit {
                width: 1280,
                height: 720,
                timestamp_us: 456,
                payload: bytes::Bytes::from_static(b"hevc-au"),
            },
        )
        .expect("dispatch HEVC access unit");

        assert_eq!(renderer.decoded_uploads, 0);
        assert_eq!(
            renderer.h264_upload,
            Some((640, 360, 123, b"h264-au".to_vec()))
        );
        assert_eq!(
            renderer.hevc_upload,
            Some((1280, 720, 456, b"hevc-au".to_vec()))
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_compressed_proxy_requires_surface_before_claiming_access_units() {
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

        struct NoopSurfaceRenderer;

        impl RendererInstance for NoopSurfaceRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_h264_access_unit(
                &mut self,
                _width: usize,
                _height: usize,
                _timestamp_us: u64,
                _payload: bytes::Bytes,
            ) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_hevc_access_unit(
                &mut self,
                _width: usize,
                _height: usize,
                _timestamp_us: u64,
                _payload: bytes::Bytes,
            ) -> Result<(), RenderError> {
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: 0,
                    presented_frame_count: 0,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: None,
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 0,
                    last_height: 0,
                    last_pixel_format: None,
                }
            }
        }

        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("macos-compressed-surface-gate".to_string());
        let profile = MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 8,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };

        assert!(
            !macos_render_proxy_compressed_media_surface_available(&app_state, &session_id).await
        );
        assert!(!render_lan_h264_access_unit_frame(
            &app_state,
            &session_id,
            bytes::Bytes::from_static(b"h264-au"),
            1,
            123,
            &profile,
        )
        .await
        .expect("missing surface should not error"));
        assert_eq!(
            app_state
                .media_render_queues
                .lock()
                .await
                .pending_depth(&session_id),
            0
        );
        assert_eq!(
            app_state
                .probes
                .lock()
                .await
                .snapshot(&session_id)
                .frames_decoded,
            0
        );

        app_state
            .media_surface_renderers
            .lock()
            .await
            .insert_renderer_for_test(&session_id, "surface-1", Box::new(NoopSurfaceRenderer));

        assert!(
            macos_render_proxy_compressed_media_surface_available(&app_state, &session_id).await
        );
        assert!(render_lan_h264_access_unit_frame(
            &app_state,
            &session_id,
            bytes::Bytes::from_static(b"h264-au"),
            2,
            456,
            &profile,
        )
        .await
        .expect("surface should accept compressed proxy frame"));
        assert_eq!(
            app_state
                .probes
                .lock()
                .await
                .snapshot(&session_id)
                .frames_decoded,
            1
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[tokio::test]
    async fn d3d11_present_skip_is_not_counted_as_presented_frame() {
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

        struct PresentSkipRenderer {
            uploaded: u64,
            skipped: u64,
        }

        impl RendererInstance for PresentSkipRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                self.uploaded += 1;
                self.skipped += 1;
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: self.uploaded,
                    presented_frame_count: 0,
                    present_skipped_count: self.skipped,
                    render_queue_replacements: None,
                    last_present_status: Some("skipped_still_drawing".to_string()),
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 1,
                    last_height: 1,
                    last_pixel_format: None,
                }
            }
        }

        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("present-skip-session".to_string());
        app_state
            .media_surface_renderers
            .lock()
            .await
            .insert_renderer_for_test(
                &session_id,
                "surface-1",
                Box::new(PresentSkipRenderer {
                    uploaded: 0,
                    skipped: 0,
                }),
            );

        let outcome = render_lan_frame_once(
            app_state,
            session_id,
            MediaRenderFrame::Decoded(RenderFrame::from_bgra32(1, 1, vec![0, 0, 0, 255])),
        )
        .await
        .expect("render one frame");

        match outcome {
            LanRenderTaskOutcome::Rendered {
                presented_frames,
                present_skips,
                ..
            } => {
                assert_eq!(presented_frames, 0);
                assert_eq!(present_skips, 1);
            }
            other => panic!("unexpected render outcome: {other:?}"),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn renderer_snapshot_render_queue_replacement_delta_uses_cumulative_counter() {
        use mrd_render::RendererSnapshot;

        fn snapshot(replacements: Option<u64>) -> RendererSnapshot {
            RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: 0,
                presented_frame_count: 0,
                present_skipped_count: 0,
                render_queue_replacements: replacements,
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: None,
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 1,
                last_height: 1,
                last_pixel_format: None,
            }
        }

        assert_eq!(
            renderer_snapshot_render_queue_replacement_delta(
                &snapshot(Some(2)),
                &snapshot(Some(5))
            ),
            3
        );
        assert_eq!(
            renderer_snapshot_render_queue_replacement_delta(&snapshot(None), &snapshot(Some(1))),
            1
        );
    }

    #[tokio::test]
    async fn media_profile_update_preserves_selected_capture_source_aspect_ratio() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("profile-update-aspect-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .set(session_id.clone(), vec!["decode.software".to_string()]);
        app_state.capture_sources.lock().await.set(
            session_id.clone(),
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source: mrd_ipc::CaptureSource {
                    id: "windows:display-shared:0".to_string(),
                    platform: "windows".to_string(),
                    source_kind: "display_shared".to_string(),
                    title: "Display 1".to_string(),
                    class_name: "WinRTMonitorShared".to_string(),
                    width: 2560,
                    height: 1600,
                    process_id: 0,
                    app_name: Some("Display".to_string()),
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                },
                status: "selected".to_string(),
                reason: None,
            },
        );

        let negotiation = accept_lan_media_profile_update(
            &app_state,
            &session_id,
            MediaProfile {
                width: 1920,
                height: 1080,
                fps: 144,
                bitrate_mbps: 20,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            negotiation.selected_source_id.as_deref(),
            Some("windows:display-shared:0")
        );
        assert_eq!(negotiation.selected.width, 1728);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(negotiation.selected.fps, 144);
        assert_eq!(negotiation.selected_width, Some(1728));
        assert_eq!(negotiation.selected_height, Some(1080));
        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(
            negotiation.downgrade_reason.as_deref(),
            Some("matched selected capture source dimensions and aspect ratio")
        );
    }

    #[tokio::test]
    async fn capture_source_reselection_can_restore_requested_profile_after_display_mode_change() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("capture-source-restore-session".to_string());
        app_state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), sender_snapshot(&session_id));
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            negotiate_media_profile(Some(MediaProfile {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate_mbps: 20,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }))
            .unwrap(),
        );

        let source_before_mode_change = mrd_ipc::CaptureSource {
            id: "windows:display-shared:0".to_string(),
            platform: "windows".to_string(),
            source_kind: "display_shared".to_string(),
            title: "Display 1".to_string(),
            class_name: "WinRTMonitorShared".to_string(),
            width: 2560,
            height: 1600,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };
        accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "windows:display-shared:0",
            vec![source_before_mode_change],
        )
        .await
        .unwrap();
        assert_eq!(
            app_state
                .media_profiles
                .lock()
                .await
                .get(&session_id)
                .expect("profile after first source")
                .selected
                .width,
            1728
        );

        let source_after_mode_change = mrd_ipc::CaptureSource {
            id: "windows:display-shared:0".to_string(),
            platform: "windows".to_string(),
            source_kind: "display_shared".to_string(),
            title: "Display 1".to_string(),
            class_name: "WinRTMonitorShared".to_string(),
            width: 1920,
            height: 1080,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };
        accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "windows:display-shared:0",
            vec![source_after_mode_change],
        )
        .await
        .unwrap();

        let negotiation = app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .expect("profile after source refresh");
        assert_eq!(negotiation.selected.width, 1920);
        assert_eq!(negotiation.selected.height, 1080);
        assert_eq!(negotiation.selected_width, Some(1920));
        assert_eq!(negotiation.selected_height, Some(1080));
        assert_eq!(negotiation.status, "accepted");
        assert_eq!(negotiation.downgrade_reason, None);
    }

    #[test]
    fn lan_capture_config_changes_when_profile_dimensions_change() {
        let source_id = "windows:display-shared:0";
        let before = MediaProfile {
            width: 1728,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let after = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let active = lan_capture_config_key(source_id, &before);

        assert!(lan_capture_config_matches(
            Some(&active),
            source_id,
            &before
        ));
        assert!(!lan_capture_config_matches(
            Some(&active),
            source_id,
            &after
        ));
    }

    fn sender_snapshot(session_id: &SessionId) -> SessionSnapshot {
        sender_snapshot_for_source(session_id, "controller-device")
    }

    fn sender_snapshot_for_source(
        session_id: &SessionId,
        source_device_id: &str,
    ) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId(source_device_id.to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        }
    }

    fn receiver_snapshot_for_target(
        session_id: &SessionId,
        target_device_id: &str,
    ) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId(target_device_id.to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Streaming,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        }
    }

    fn test_window_capture_source(id: &str) -> CaptureSource {
        CaptureSource {
            id: id.to_string(),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: "Target App".to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: Some("Target App".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    fn test_display_capture_source(id: &str) -> CaptureSource {
        CaptureSource {
            id: id.to_string(),
            platform: "windows".to_string(),
            source_kind: "display_shared".to_string(),
            title: "Display 1".to_string(),
            class_name: "WinRTMonitorShared".to_string(),
            width: 1920,
            height: 1080,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    fn display_mode(
        id: &str,
        width: u32,
        height: u32,
        refresh_hz: u32,
        is_current: bool,
    ) -> mrd_ipc::DisplayMode {
        mrd_ipc::DisplayMode {
            id: id.to_string(),
            source_id: Some("windows:display-shared:0".to_string()),
            width,
            height,
            refresh_hz,
            bit_depth: Some(32),
            is_current,
        }
    }
}
