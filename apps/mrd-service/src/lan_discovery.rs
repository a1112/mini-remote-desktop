#[cfg(windows)]
use crate::app_state::MediaRenderQueueEnqueue;
use crate::app_state::{AppState, DecodedVideoFrameStats, MediaProbeFrameStats};
use anyhow::{Context, Result};
use mrd_application::ports::SessionSnapshot;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, DisplayMode, DisplayModeChange, LanDiscoverySnapshot,
    LanPeerInfo, MediaProfile, MediaProfileNegotiation, MediaStageMetrics,
    MediaTestImpairmentSnapshot,
};
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame, DecodedFrameData, FramePixelFormat, VideoDecoder, VideoEncoder,
};
use mrd_proto::{DeviceId, SessionId};
#[cfg(windows)]
use mrd_render::RenderFrame;
use mrd_transport_quic_quinn::{
    fragment_access_unit, fragment_media_payload_v3, is_quic_media_v3_datagram, QuicAuFrame,
    QuicAuReassembler, QuicAuReassemblerConfig, QuicAuReassemblerStats, QuicMediaCodec,
    QuicMediaFrame, QuicMediaPayloadType, QuicMediaReassembler, QuinnDatagramEndpoint,
    QuinnServerBootstrap, QuinnServerListener, QUIC_AU_FRAGMENT_HEADER_LEN,
    QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, Notify};
use tokio::time::{interval, sleep_until, timeout, Instant};
#[cfg(windows)]
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

const DEFAULT_DISCOVERY_PORT: u16 = 21116;
const LAN_DISCOVERY_PORT_ENV: &str = "MRD_LAN_DISCOVERY_PORT";
const LAN_DISCOVERY_PROBE_ENDPOINTS_ENV: &str = "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS";
const SERVICE_BUILD_ID_ENV: &str = "MRD_SERVICE_BUILD_ID";
const LAN_TEST_IMPAIRMENT_LOSS_PCT_ENV: &str = "MRD_LAN_TEST_IMPAIRMENT_LOSS_PCT";
const LAN_TEST_IMPAIRMENT_BASE_DELAY_MS_ENV: &str = "MRD_LAN_TEST_IMPAIRMENT_BASE_DELAY_MS";
const LAN_TEST_IMPAIRMENT_JITTER_MS_ENV: &str = "MRD_LAN_TEST_IMPAIRMENT_JITTER_MS";
const LAN_TEST_IMPAIRMENT_MTU_BYTES_ENV: &str = "MRD_LAN_TEST_IMPAIRMENT_MTU_BYTES";
const LAN_TEST_IMPAIRMENT_SEED_ENV: &str = "MRD_LAN_TEST_IMPAIRMENT_SEED";
const LAN_RELIABLE_WHOLE_FRAME_ENV: &str = "MRD_LAN_RELIABLE_WHOLE_FRAME";
const LAN_RENDER_PACING_ENV: &str = "MRD_LAN_RENDER_PACING";
const PROTOCOL_VERSION: u32 = 1;
const ANNOUNCE_INTERVAL_SECS: u64 = 3;
const PEER_TTL_SECS: u64 = 12;
const DISCOVERY_MAGIC: &str = "mrd-lan-discovery-v1";
const DISCOVERY_APP_ID: &str = "rdesk";
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
const LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_BITRATE_MBPS: u32 = 120;
const LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_FPS: u32 = 160;
const LAN_QUIC_RELIABLE_MEDIA_MAX_BYTES: usize = 4 * 1024 * 1024;
const LAN_QUIC_RELIABLE_MEDIA_RETRY_DELAY: Duration = Duration::from_millis(10);
const LAN_MEDIA_HIGH_RESOLUTION_TIMER_MIN_FPS: u32 = 90;
const LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS: u32 = 1;
const LAN_MEDIA_PRECISE_SLEEP_MIN_FPS: u32 = 90;
const LAN_MEDIA_PRECISE_SLEEP_GUARD: Duration = Duration::from_millis(2);
const LAN_RENDER_SURFACE_LOCK_TIMEOUT: Duration = Duration::from_millis(2);
const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";
const LAN_QUIC_MEDIA_PROFILE_TRANSPORT: &str = "quic_datagram_2k144";
const LAN_QUIC_MEDIA_V2_TRANSPORT: &str = "quic_datagram_media_v2";
const LAN_QUIC_MEDIA_V3_TRANSPORT: &str = "quic_datagram_media_v3";
const LAN_QUIC_RELIABLE_MEDIA_TRANSPORT: &str = "quic_stream_media_v2";
const LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT: &str = "quic_stream_media_v3";
const LAN_MEDIA_PROFILE_CONTROL_TRANSPORT: &str = "media_profile_control_v1";
const LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT: &str = "capture_source_control_v1";
const LAN_DISPLAY_MODE_CONTROL_TRANSPORT: &str = "display_mode_control_v1";
const LAN_MEDIA_PROTOCOL_VERSION: u32 = 3;
const LAN_CAPTURE_DXGI_CAPABILITY: &str = "dxgi_capture";
const LAN_ENCODE_NVENC_H264_CAPABILITY: &str = "nvenc_h264";
const LAN_ENCODE_NVENC_HEVC_CAPABILITY: &str = "encode.nvenc_hevc";
const LAN_DECODE_NVDEC_CAPABILITY: &str = "nvdec";
const LAN_DECODE_NVDEC_HEVC_CAPABILITY: &str = "decode.nvdec_hevc";
const LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY: &str = "media.hevc_main_420_8bit";
const LAN_RENDER_D3D11_NATIVE_CAPABILITY: &str = "d3d11_native_render";
const LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY: &str = "render.d3d11_shared_nv12";
const LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS: u32 = 8;
const LAN_MEDIA_SENDER_ERROR_LOG_INTERVAL: u32 = 3;
const LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 8;
const LAN_MEDIA_RECEIVER_DECODE_ERROR_LOG_INTERVAL: u32 = 3;
const LAN_MEDIA_REASSEMBLER_FRAME_TIMEOUT_MS: u64 = 1_500;
const LAN_MEDIA_REASSEMBLER_MAX_PENDING_FRAMES: usize = 256;
const LAN_MEDIA_RECEIVER_REORDER_MAX_PENDING_FRAMES: usize = 8;
const LAN_MEDIA_PROBE_MAGIC: &[u8; 8] = b"MRDMPF01";
const LAN_MEDIA_PROBE_HEADER_BYTES: usize = 56;
const LAN_MEDIA_PROBE_NATIVE_HIGH_FORMAT: &str = "compressed_native_high_test_pattern";
const LAN_MEDIA_PROBE_DYNAMIC_FORMAT: &str = "compressed_h264_test_pattern";
const LAN_MEDIA_PROBE_FORMAT_CODE: u32 = 2;
const LAN_MEDIA_ENVELOPE_MAGIC: &[u8; 8] = b"MRDMV2F1";
const LAN_MEDIA_ENVELOPE_HEADER_BYTES: usize = 48;
const LAN_MEDIA_PAYLOAD_ACCESS_UNIT: u8 = 1;
#[cfg(test)]
const LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT: u8 = LAN_MEDIA_PAYLOAD_ACCESS_UNIT;
const LAN_MEDIA_PAYLOAD_PROBE_FRAME: u8 = 2;
const LAN_MEDIA_SENDER_STATS_MAGIC: &[u8; 8] = b"MRDMSTG1";
const LAN_MEDIA_SENDER_STATS_HEADER_BYTES: usize = 12;
const LAN_MEDIA_SENDER_STATS_INTERVAL: Duration = Duration::from_secs(1);
const LAN_MEDIA_SENDER_STATS_SAMPLE_LIMIT: usize = 240;
const LAN_MEDIA_CODEC_H264: u8 = 1;
const LAN_MEDIA_CODEC_HEVC: u8 = 2;
const LAN_PREVIEW_FRAME_INTERVAL: u64 = 120;
const LAN_PREVIEW_MAX_WIDTH: u32 = 480;
const LAN_PREVIEW_MAX_HEIGHT: u32 = 270;

#[derive(Debug, Clone)]
pub struct LanDiscoveryConfig {
    pub enabled: bool,
    pub discovery_port: u16,
    pub probe_endpoints: Vec<SocketAddr>,
    pub announce_interval: Duration,
    pub peer_ttl: Duration,
}

impl Default for LanDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            probe_endpoints: Vec::new(),
            announce_interval: Duration::from_secs(ANNOUNCE_INTERVAL_SECS),
            peer_ttl: Duration::from_secs(PEER_TTL_SECS),
        }
    }
}

impl LanDiscoveryConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    fn from_env_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut config = Self::default();
        if let Some(port) = lookup(LAN_DISCOVERY_PORT_ENV) {
            let port = port.trim();
            if !port.is_empty() {
                config.discovery_port = port
                    .parse::<u16>()
                    .with_context(|| format!("invalid {LAN_DISCOVERY_PORT_ENV}: {port}"))?;
            }
        }
        if let Some(endpoints) = lookup(LAN_DISCOVERY_PROBE_ENDPOINTS_ENV) {
            config.probe_endpoints = parse_probe_endpoints(&endpoints)?;
        }
        Ok(config)
    }
}

fn parse_probe_endpoints(value: &str) -> Result<Vec<SocketAddr>> {
    let mut endpoints = Vec::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        endpoints.push(
            entry
                .parse::<SocketAddr>()
                .with_context(|| format!("invalid LAN discovery probe endpoint: {entry}"))?,
        );
    }
    Ok(endpoints)
}

#[derive(Debug)]
pub struct LanDiscoveryState {
    config: LanDiscoveryConfig,
    instance_id: String,
    running: AtomicBool,
    last_probe_ms: AtomicU64,
    peers: Mutex<HashMap<String, StoredLanPeer>>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LanMediaEnvelope {
    payload_type: u8,
    codec: u8,
    sequence: u64,
    timestamp_us: u64,
    profile: MediaProfile,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanAccessUnitCodec {
    H264,
    Hevc,
}

impl LanAccessUnitCodec {
    fn from_profile(profile: &MediaProfile) -> Self {
        if profile.codec.eq_ignore_ascii_case("hevc") {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct LanSenderStatsPayload {
    sequence: u64,
    frame_count: u64,
    source_id: Option<String>,
    target_fps: u32,
    target_bitrate_mbps: u32,
    metrics: Vec<MediaStageMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test_impairment: Option<MediaTestImpairmentSnapshot>,
}

#[derive(Debug)]
struct LanSenderStatsTracker {
    samples: HashMap<&'static str, VecDeque<f64>>,
    frame_count: u64,
    last_emit: Instant,
}

#[derive(Debug, Clone)]
struct LanMediaTestImpairment {
    loss_pct: f64,
    base_delay: Duration,
    jitter: Duration,
    mtu_bytes: Option<usize>,
    seed: u64,
    rng_state: u64,
    datagrams_sent: u64,
    datagrams_dropped: u64,
    datagrams_delayed: u64,
    datagrams_fragmented_by_mtu: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LanMediaDatagramDecision {
    drop_datagram: bool,
    delay: Duration,
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

impl LanSenderStatsTracker {
    fn new(now: Instant) -> Self {
        Self {
            samples: HashMap::new(),
            frame_count: 0,
            last_emit: now,
        }
    }

    fn record_elapsed(&mut self, stage: &'static str, start: Instant) {
        self.record_ms(stage, start.elapsed().as_secs_f64() * 1000.0);
    }

    fn record_ms(&mut self, stage: &'static str, duration_ms: f64) {
        if !duration_ms.is_finite() || duration_ms < 0.0 {
            return;
        }
        let samples = self.samples.entry(stage).or_default();
        samples.push_back(duration_ms);
        while samples.len() > LAN_MEDIA_SENDER_STATS_SAMPLE_LIMIT {
            samples.pop_front();
        }
    }

    fn frame_completed(&mut self) {
        self.frame_count = self.frame_count.saturating_add(1);
    }

    fn take_stage_metrics(&mut self, now: Instant) -> Option<Vec<MediaStageMetrics>> {
        if now.duration_since(self.last_emit) < LAN_MEDIA_SENDER_STATS_INTERVAL {
            return None;
        }
        self.last_emit = now;
        Some(self.stage_metrics())
    }

    fn stage_metrics(&self) -> Vec<MediaStageMetrics> {
        let mut metrics = self
            .samples
            .iter()
            .map(|(stage, samples)| MediaStageMetrics {
                stage: (*stage).to_string(),
                p50_ms: sender_stats_percentile(samples, 0.50),
                p95_ms: sender_stats_percentile(samples, 0.95),
            })
            .collect::<Vec<_>>();
        metrics.sort_by(|left, right| left.stage.cmp(&right.stage));
        metrics
    }

    fn take_payload(
        &mut self,
        now: Instant,
        sequence: u64,
        source_id: Option<String>,
        profile: &MediaProfile,
        test_impairment: Option<MediaTestImpairmentSnapshot>,
    ) -> Option<LanSenderStatsPayload> {
        let metrics = self.take_stage_metrics(now)?;
        Some(LanSenderStatsPayload {
            sequence,
            frame_count: self.frame_count,
            source_id,
            target_fps: profile.fps,
            target_bitrate_mbps: profile.bitrate_mbps,
            metrics,
            test_impairment,
        })
    }
}

impl LanMediaTestImpairment {
    fn from_env() -> Result<Self> {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    fn from_env_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let loss_pct =
            parse_env_f64(&lookup, LAN_TEST_IMPAIRMENT_LOSS_PCT_ENV, 0.0)?.clamp(0.0, 100.0);
        let base_delay_ms = parse_env_u64(&lookup, LAN_TEST_IMPAIRMENT_BASE_DELAY_MS_ENV, 0)?;
        let jitter_ms = parse_env_u64(&lookup, LAN_TEST_IMPAIRMENT_JITTER_MS_ENV, 0)?;
        let mtu_bytes = lookup(LAN_TEST_IMPAIRMENT_MTU_BYTES_ENV)
            .and_then(|value| {
                let trimmed = value.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            })
            .map(|value| {
                value.parse::<usize>().with_context(|| {
                    format!("invalid {LAN_TEST_IMPAIRMENT_MTU_BYTES_ENV}: {value}")
                })
            })
            .transpose()?;
        let seed = parse_env_u64(&lookup, LAN_TEST_IMPAIRMENT_SEED_ENV, 0x4d52_444c_414e)?;
        Ok(Self {
            loss_pct,
            base_delay: Duration::from_millis(base_delay_ms),
            jitter: Duration::from_millis(jitter_ms),
            mtu_bytes,
            seed,
            rng_state: seed.max(1),
            datagrams_sent: 0,
            datagrams_dropped: 0,
            datagrams_delayed: 0,
            datagrams_fragmented_by_mtu: 0,
        })
    }

    fn enabled(&self) -> bool {
        self.loss_pct > 0.0
            || !self.base_delay.is_zero()
            || !self.jitter.is_zero()
            || self.mtu_bytes.is_some()
    }

    fn effective_datagram_size(&self, negotiated_size: usize) -> usize {
        let minimum = QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN.max(QUIC_AU_FRAGMENT_HEADER_LEN) + 1;
        self.mtu_bytes
            .map(|mtu| mtu.clamp(minimum, negotiated_size))
            .unwrap_or(negotiated_size)
    }

    fn record_mtu_fragmentation(&mut self, negotiated_size: usize) {
        if self.effective_datagram_size(negotiated_size) < negotiated_size {
            self.datagrams_fragmented_by_mtu = self.datagrams_fragmented_by_mtu.saturating_add(1);
        }
    }

    fn next_datagram_decision(&mut self) -> LanMediaDatagramDecision {
        let loss_roll = self.next_unit_f64() * 100.0;
        let drop_datagram = self.loss_pct > 0.0 && loss_roll < self.loss_pct;
        let delay = self.next_delay();

        if drop_datagram {
            self.datagrams_dropped = self.datagrams_dropped.saturating_add(1);
        } else {
            self.datagrams_sent = self.datagrams_sent.saturating_add(1);
        }

        LanMediaDatagramDecision {
            drop_datagram,
            delay,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let jitter_ms = if self.jitter.is_zero() {
            0
        } else {
            let jitter_bound = self.jitter.as_millis() as u64;
            self.next_u64() % (jitter_bound.saturating_add(1))
        };
        let delay = self.base_delay + Duration::from_millis(jitter_ms);
        if !delay.is_zero() {
            self.datagrams_delayed = self.datagrams_delayed.saturating_add(1);
        }
        delay
    }

    fn snapshot(&self) -> Option<MediaTestImpairmentSnapshot> {
        self.enabled().then(|| MediaTestImpairmentSnapshot {
            loss_pct: self.loss_pct,
            base_delay_ms: self.base_delay.as_millis() as u64,
            jitter_ms: self.jitter.as_millis() as u64,
            mtu_bytes: self.mtu_bytes.map(|value| value as u32),
            seed: self.seed,
            datagrams_sent: self.datagrams_sent,
            datagrams_dropped: self.datagrams_dropped,
            datagrams_delayed: self.datagrams_delayed,
            datagrams_fragmented_by_mtu: self.datagrams_fragmented_by_mtu,
        })
    }

    fn next_unit_f64(&mut self) -> f64 {
        (self.next_u64() as f64) / (u64::MAX as f64)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x.max(1);
        self.rng_state
    }
}

fn parse_env_u64(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    default: u64,
) -> Result<u64> {
    let Some(value) = lookup(key) else {
        return Ok(default);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(default);
    }
    value
        .parse::<u64>()
        .with_context(|| format!("invalid {key}: {value}"))
}

fn parse_env_f64(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    default: f64,
) -> Result<f64> {
    let Some(value) = lookup(key) else {
        return Ok(default);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(default);
    }
    value
        .parse::<f64>()
        .with_context(|| format!("invalid {key}: {value}"))
}

fn sender_stats_percentile(samples: &VecDeque<f64>, quantile: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.iter().copied().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let last = sorted.len().saturating_sub(1);
    let index = ((last as f64) * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted.get(index).copied()
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
    ensure_peer_supports_requested_media(target_device_id, transport_kind, &peer_transports)?;
    close_existing_lan_receiver_sessions_for_target(app_state, target_device_id, session_id).await;

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
                                lifecycle_state: "connecting".to_string(),
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
    ensure_peer_supports_requested_media(&peer_device_id, "quic", &peer_transports)?;

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
                change.context("LAN peer accepted display mode set without change")
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
                change.context("LAN peer accepted display mode restore without change")
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
    close_existing_lan_sender_sessions(app_state, &session_id).await;

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
                lifecycle_state: "listening".to_string(),
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
        if snapshot.lifecycle_state == "closed" || snapshot.lifecycle_state == "failed" {
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
    Ok(app_state.display_modes.lock().await.record_change(
        session_id.clone(),
        mode,
        previous,
        active,
        restore_after_session,
    ))
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
    Ok(app_state.display_modes.lock().await.record_change(
        session_id.clone(),
        requested,
        previous,
        active,
        restore_after_session,
    ))
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
    let mut profiles = app_state.media_profiles.lock().await;
    let mut negotiation = profiles
        .get(session_id)
        .unwrap_or_else(default_media_profile_negotiation);
    reconcile_negotiation_to_capture_source(&mut negotiation, source);

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
    if snapshot.lifecycle_state == "closed" || snapshot.lifecycle_state == "failed" {
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

    Some(LanAnnouncement {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        device_id,
        device_name,
        device_type: "rdesk".to_string(),
        protocol_version: PROTOCOL_VERSION,
        discovery_port: app_state.lan_discovery.discovery_port(),
        transports: vec![
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
        ],
        service_build_id: Some(service_build_id()),
        media_protocol_version: Some(LAN_MEDIA_PROTOCOL_VERSION),
        media_capabilities: lan_media_capabilities(),
        timestamp_ms: now_ms(),
    })
}

fn service_build_id() -> String {
    service_build_id_from_lookup(|key| std::env::var(key).ok())
}

fn service_build_id_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> String {
    if let Some(value) = lookup(SERVICE_BUILD_ID_ENV) {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}

fn lan_media_capabilities() -> Vec<String> {
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
            LAN_DECODE_NVDEC_CAPABILITY.to_string(),
            LAN_DECODE_NVDEC_HEVC_CAPABILITY.to_string(),
            LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_NATIVE_CAPABILITY.to_string(),
            LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY.to_string(),
            crate::display_mode::capability_name().to_string(),
        ]);
    }
    #[cfg(not(windows))]
    {
        capabilities.extend([
            "pipewire_capture".to_string(),
            "openh264_fallback".to_string(),
            "software_decode".to_string(),
        ]);
    }
    capabilities
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
                    lifecycle_state: "streaming".to_string(),
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

async fn close_existing_lan_receiver_sessions_for_target(
    app_state: &Arc<AppState>,
    target_device_id: &DeviceId,
    next_session_id: &SessionId,
) {
    let stale_sessions = {
        let sessions = app_state.sessions.lock().await;
        sessions
            .list_all()
            .into_iter()
            .filter(|snapshot| {
                snapshot.session_id != *next_session_id
                    && snapshot.target_device_id.as_ref() == Some(target_device_id)
                    && snapshot.receiver_active
                    && !matches!(snapshot.lifecycle_state.as_str(), "closed" | "failed")
            })
            .map(|snapshot| snapshot.session_id)
            .collect::<Vec<_>>()
    };
    close_lan_media_sessions(
        app_state,
        stale_sessions,
        "replaced by newer receiver session",
    )
    .await;
}

async fn close_existing_lan_sender_sessions(
    app_state: &Arc<AppState>,
    next_session_id: &SessionId,
) {
    let stale_sessions = {
        let sessions = app_state.sessions.lock().await;
        sessions
            .list_all()
            .into_iter()
            .filter(|snapshot| {
                snapshot.session_id != *next_session_id
                    && snapshot.sender_active
                    && normalize_transport_kind(&snapshot.transport) == "quic"
                    && !matches!(snapshot.lifecycle_state.as_str(), "closed" | "failed")
            })
            .map(|snapshot| snapshot.session_id)
            .collect::<Vec<_>>()
    };
    close_lan_media_sessions(
        app_state,
        stale_sessions,
        "replaced by newer sender session",
    )
    .await;
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
                        lifecycle_state: "closed".to_string(),
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
        #[cfg(windows)]
        app_state
            .media_surface_renderers
            .lock()
            .await
            .detach_session(&session_id);
        #[cfg(windows)]
        app_state
            .media_render_queues
            .lock()
            .await
            .remove(&session_id);
        app_state.media_pipelines.lock().await.remove(&session_id);
    }
}

fn ensure_peer_supports_requested_media(
    target_device_id: &DeviceId,
    transport_kind: &str,
    peer_transports: &[String],
) -> Result<()> {
    let transport = normalize_transport_kind(transport_kind);
    if transport == "quic" {
        let required = [
            LAN_QUIC_MEDIA_TRANSPORT,
            LAN_QUIC_MEDIA_PROFILE_TRANSPORT,
            LAN_QUIC_MEDIA_V2_TRANSPORT,
            LAN_MEDIA_PROFILE_CONTROL_TRANSPORT,
        ];
        let missing = required
            .iter()
            .filter(|required_transport| {
                !peer_transports
                    .iter()
                    .any(|peer_transport| peer_transport.eq_ignore_ascii_case(required_transport))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            anyhow::bail!(
                "LAN peer does not advertise required media controls [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
                missing.join(", "),
                target_device_id.0,
                format_peer_transports(peer_transports)
            );
        }
    }
    Ok(())
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

fn format_peer_transports(peer_transports: &[String]) -> String {
    if peer_transports.is_empty() {
        "none".to_string()
    } else {
        peer_transports.join(", ")
    }
}

fn fit_capture_sources_ack_packet(
    instance_id: String,
    session_id: String,
    accepted: bool,
    message: Option<String>,
    sources: Vec<CaptureSource>,
) -> LanDiscoveryPacket {
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

        if let Some(index) = largest_preview_source_index(sources) {
            sources[index].preview_data_url = None;
            sources[index].preview_width = None;
            sources[index].preview_height = None;
            continue;
        }

        if sources.len() > 1 {
            sources.pop();
            continue;
        }

        break;
    }

    packet
}

fn serialized_packet_len(packet: &LanDiscoveryPacket) -> usize {
    serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn largest_preview_source_index(sources: &[CaptureSource]) -> Option<usize> {
    sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| {
            source
                .preview_data_url
                .as_ref()
                .map(|preview| (index, preview.len()))
        })
        .max_by_key(|(_, preview_len)| *preview_len)
        .map(|(index, _)| index)
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct LanCaptureConfigKey {
    source_id: String,
    width: u32,
    height: u32,
}

fn lan_capture_config_key(source_id: &str, profile: &MediaProfile) -> LanCaptureConfigKey {
    LanCaptureConfigKey {
        source_id: source_id.to_string(),
        width: profile.width,
        height: profile.height,
    }
}

fn lan_capture_config_matches(
    active: Option<&LanCaptureConfigKey>,
    source_id: &str,
    profile: &MediaProfile,
) -> bool {
    active
        .map(|config| {
            config.source_id == source_id
                && config.width == profile.width
                && config.height == profile.height
        })
        .unwrap_or(false)
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

    let mut frame_id = 1_u64;
    let mut active_capture_config: Option<LanCaptureConfigKey> = None;
    let mut capture: Option<LanFrameCapture> = None;
    let mut encoder: Option<LanSenderEncoder> = None;
    let mut encoder_config: Option<(usize, usize, u32, u32, LanAccessUnitCodec)> = None;
    let mut consecutive_frame_errors = 0_u32;
    let mut next_frame_at = Instant::now();
    let mut active_frame_interval = Duration::ZERO;
    let mut media_timer_resolution = MediaTimerResolution::default();
    let mut sender_stats = LanSenderStatsTracker::new(Instant::now());
    let mut test_impairment = LanMediaTestImpairment::from_env()?;
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
        let loop_started = Instant::now();
        if !session_allows_media(&app_state, &session_id).await {
            return Ok(());
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
        let frame_interval = media_frame_interval(&profile);
        if active_frame_interval != frame_interval {
            active_frame_interval = frame_interval;
            next_frame_at = Instant::now() + frame_interval;
        }
        if let Some(delay_until) =
            schedule_next_media_frame(Instant::now(), &mut next_frame_at, frame_interval)
        {
            sleep_until_media_frame(delay_until, &profile).await;
        }

        let source_id = selected_capture_source_id(&app_state, &session_id).await?;
        if !lan_capture_config_matches(active_capture_config.as_ref(), &source_id, &profile) {
            match create_lan_frame_capture(&source_id, &profile).await {
                Ok(next_capture) => {
                    capture = Some(next_capture);
                    encoder = None;
                    encoder_config = None;
                    active_capture_config = Some(lan_capture_config_key(&source_id, &profile));
                    consecutive_frame_errors = 0;
                    set_session_last_error(&app_state, &session_id, None).await;
                }
                Err(error) => {
                    capture = None;
                    encoder = None;
                    encoder_config = None;
                    active_capture_config = None;
                    handle_media_sender_frame_error(
                        &app_state,
                        &session_id,
                        &source_id,
                        &mut consecutive_frame_errors,
                        format!("failed to create LAN capture source: {error:#}"),
                        false,
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
        let raw_frame = match raw_frame_result {
            Ok(frame) => frame,
            Err(error) => {
                let error_source_id = active_capture_config
                    .as_ref()
                    .map(|config| config.source_id.as_str())
                    .unwrap_or("<unknown>")
                    .to_string();
                capture = None;
                encoder = None;
                encoder_config = None;
                active_capture_config = None;
                handle_media_sender_frame_error(
                    &app_state,
                    &session_id,
                    &error_source_id,
                    &mut consecutive_frame_errors,
                    format!("{error:#}"),
                    false,
                )
                .await?;
                continue;
            }
        };
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
        );
        if encoder_config != Some(expected_encoder_config) {
            let encoder_create_started = Instant::now();
            match create_lan_encoder(
                requested_codec,
                frame.width,
                frame.height,
                profile.fps,
                profile.bitrate_mbps.saturating_mul(1_000_000).max(1),
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
                }
                sender_stats.record_elapsed("sender.send_reliable", reliable_send_started);
            } else {
                let best_effort_datagrams = use_best_effort_media_datagrams(&profile);
                let datagram_send_started = Instant::now();
                for fragment in &fragments {
                    let decision = test_impairment.next_datagram_decision();
                    if decision.drop_datagram {
                        continue;
                    }
                    if !decision.delay.is_zero() {
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
                    )
                    .await;
                    if let Err(error) = send_fragment_result {
                        send_result = Err(error).with_context(|| {
                            format!("failed to send LAN QUIC media frame {}", frame_id)
                        });
                        break;
                    }
                }
                sender_stats.record_elapsed("sender.send_datagram", datagram_send_started);

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
                &profile,
                test_impairment.snapshot(),
            ) {
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
) -> Result<LanSenderEncoder> {
    match requested_codec {
        LanAccessUnitCodec::Hevc => match create_lan_hevc_encoder(width, height, fps, bitrate) {
            Ok((backend, encoder)) => Ok(LanSenderEncoder {
                codec: LanAccessUnitCodec::Hevc,
                backend,
                encoder,
            }),
            Err(hevc_error) => {
                let (backend, encoder) = create_lan_h264_encoder(width, height, fps, bitrate)
                    .with_context(|| {
                        format!("HEVC unavailable ({hevc_error}); H.264 fallback also failed")
                    })?;
                Ok(LanSenderEncoder {
                    codec: LanAccessUnitCodec::H264,
                    backend,
                    encoder,
                })
            }
        },
        LanAccessUnitCodec::H264 => {
            let (backend, encoder) = create_lan_h264_encoder(width, height, fps, bitrate)?;
            Ok(LanSenderEncoder {
                codec: LanAccessUnitCodec::H264,
                backend,
                encoder,
            })
        }
    }
}

#[cfg(windows)]
fn create_lan_hevc_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    match mrd_encode_nvenc::NvencHevcEncoder::new_max_speed_with_bitrate(
        width, height, fps, bitrate,
    ) {
        Ok(encoder) => Ok((
            "nvenc_hevc_p1_ultra_low_latency",
            Box::new(encoder) as Box<dyn VideoEncoder + Send>,
        )),
        Err(max_speed_error) => {
            mrd_encode_nvenc::NvencHevcEncoder::new_main_with_bitrate(width, height, fps, bitrate)
                .map(|encoder| {
                    (
                        "nvenc_hevc",
                        Box::new(encoder) as Box<dyn VideoEncoder + Send>,
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

#[cfg(not(windows))]
fn create_lan_hevc_encoder(
    _width: usize,
    _height: usize,
    _fps: u32,
    _bitrate: u32,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    anyhow::bail!("NVENC HEVC is unavailable on this platform")
}

fn create_lan_h264_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let mut last_error = None;
    for backend in preferred_lan_h264_encoder_backends() {
        let encoder: Result<Box<dyn VideoEncoder + Send>> = match *backend {
            #[cfg(windows)]
            "nvenc_h264" => mrd_encode_nvenc::NvencH264Encoder::new_max_speed_with_bitrate(
                width, height, fps, bitrate,
            )
            .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
            .map_err(|error| anyhow::anyhow!(error.to_string())),
            "openh264" => OpenH264Encoder::new_with_bitrate(width, height, fps, bitrate)
                .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
                .map_err(|error| anyhow::anyhow!(error.to_string())),
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

#[cfg(not(windows))]
fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["openh264"]
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

fn should_log_media_sender_frame_error(consecutive_frame_errors: u32) -> bool {
    consecutive_frame_errors == 1
        || consecutive_frame_errors >= LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS
        || consecutive_frame_errors % LAN_MEDIA_SENDER_ERROR_LOG_INTERVAL == 0
}

fn should_log_media_receiver_decode_error(consecutive_decode_errors: u32) -> bool {
    consecutive_decode_errors == 1
        || consecutive_decode_errors == LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS
        || consecutive_decode_errors % LAN_MEDIA_RECEIVER_DECODE_ERROR_LOG_INTERVAL == 0
}

struct LanMediaFrameOrderer {
    next_frame_id: Option<u32>,
    max_pending_frames: usize,
    pending: BTreeMap<u32, QuicAuFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanReliableMediaSendMode {
    Disabled,
    PerMessage,
    Persistent,
}

impl LanMediaFrameOrderer {
    fn new(max_pending_frames: usize) -> Self {
        Self {
            next_frame_id: None,
            max_pending_frames: max_pending_frames.max(1),
            pending: BTreeMap::new(),
        }
    }

    fn push(&mut self, frame: QuicAuFrame) -> Vec<QuicAuFrame> {
        if self
            .next_frame_id
            .is_some_and(|next_frame_id| frame.frame_id < next_frame_id)
        {
            return Vec::new();
        }

        self.next_frame_id.get_or_insert(frame.frame_id);
        self.pending.entry(frame.frame_id).or_insert(frame);

        let mut ready = self.drain_contiguous();
        if ready.is_empty() && self.pending.len() > self.max_pending_frames {
            if let Some(next_frame_id) = self.pending.keys().next().copied() {
                self.next_frame_id = Some(next_frame_id);
                ready = self.drain_contiguous();
            }
        }
        ready
    }

    fn drain_contiguous(&mut self) -> Vec<QuicAuFrame> {
        let mut ready = Vec::new();
        while let Some(next_frame_id) = self.next_frame_id {
            let Some(frame) = self.pending.remove(&next_frame_id) else {
                break;
            };
            self.next_frame_id = Some(next_frame_id.wrapping_add(1));
            ready.push(frame);
        }
        ready
    }
}

fn lan_media_reassembler_config() -> QuicAuReassemblerConfig {
    QuicAuReassemblerConfig {
        frame_timeout: Duration::from_millis(LAN_MEDIA_REASSEMBLER_FRAME_TIMEOUT_MS),
        max_pending_frames: LAN_MEDIA_REASSEMBLER_MAX_PENDING_FRAMES,
    }
}

fn should_send_access_unit_reliably(
    reliable_media_supported: bool,
    is_keyframe: bool,
    _payload_len: usize,
    _max_datagram_size: usize,
) -> bool {
    if !reliable_media_supported {
        return false;
    }

    is_keyframe
}

fn select_reliable_media_send_mode(
    reliable_media_supported: bool,
    persistent_media_supported: bool,
) -> LanReliableMediaSendMode {
    if persistent_media_supported {
        LanReliableMediaSendMode::Persistent
    } else if reliable_media_supported {
        LanReliableMediaSendMode::PerMessage
    } else {
        LanReliableMediaSendMode::Disabled
    }
}

fn select_reliable_media_send_mode_for_profile(
    reliable_media_supported: bool,
    persistent_media_supported: bool,
    profile: &MediaProfile,
) -> LanReliableMediaSendMode {
    if reliable_media_supported
        && profile.bitrate_mbps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_MIN_BITRATE_MBPS
    {
        LanReliableMediaSendMode::PerMessage
    } else {
        select_reliable_media_send_mode(reliable_media_supported, persistent_media_supported)
    }
}

fn use_best_effort_media_datagrams(profile: &MediaProfile) -> bool {
    profile.bitrate_mbps <= LAN_QUIC_BEST_EFFORT_DATAGRAM_MAX_BITRATE_MBPS
}

fn lan_media_datagram_size(
    negotiated_max_datagram_size: usize,
    profile: &MediaProfile,
    high_quality_datagram_supported: bool,
) -> usize {
    let minimum = QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN.max(QUIC_AU_FRAGMENT_HEADER_LEN) + 1;
    let safe_cap = if high_quality_datagram_supported && !use_best_effort_media_datagrams(profile) {
        LAN_QUIC_LAN_HIGH_QUALITY_DATAGRAM_BYTES
    } else {
        LAN_QUIC_FALLBACK_DATAGRAM_BYTES
    };
    negotiated_max_datagram_size.min(safe_cap).max(minimum)
}

async fn send_lan_media_datagram(
    endpoint: &QuinnDatagramEndpoint,
    fragment: bytes::Bytes,
    wait_for_capacity: bool,
) -> Result<()> {
    if !wait_for_capacity {
        return endpoint
            .send_datagram(fragment)
            .context("failed to send LAN QUIC media datagram");
    }

    match endpoint.send_datagram(fragment.clone()) {
        Ok(()) => Ok(()),
        Err(_) => endpoint
            .send_datagram_wait(fragment)
            .await
            .context("failed to send LAN QUIC media datagram after waiting for capacity"),
    }
}

async fn send_lan_reliable_media_fragment(
    endpoint: &QuinnDatagramEndpoint,
    mode: LanReliableMediaSendMode,
    fragment: bytes::Bytes,
) -> Result<()> {
    match mode {
        LanReliableMediaSendMode::Disabled => {
            anyhow::bail!("LAN reliable media send requested while reliable media is disabled")
        }
        LanReliableMediaSendMode::PerMessage => {
            endpoint
                .send_reliable_message(fragment)
                .await
                .context("failed to send per-message reliable LAN media fragment")?;
        }
        LanReliableMediaSendMode::Persistent => {
            endpoint
                .send_reliable_message_persistent(fragment)
                .await
                .context("failed to send persistent reliable LAN media fragment")?;
        }
    }
    Ok(())
}

fn should_send_access_unit_as_reliable_frame(
    reliable_media_supported: bool,
    media_v3_supported: bool,
    _fragment_count: usize,
    profile: &MediaProfile,
    reliable_whole_frame_override: Option<bool>,
) -> bool {
    if !reliable_media_supported || !media_v3_supported {
        return false;
    }
    if let Some(enabled) = reliable_whole_frame_override {
        return enabled;
    }

    should_default_to_reliable_whole_frame(profile)
}

fn should_default_to_reliable_whole_frame(profile: &MediaProfile) -> bool {
    profile.bitrate_mbps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_BITRATE_MBPS
        && profile.fps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_FPS
}

fn reliable_whole_frame_media_override() -> Option<bool> {
    reliable_whole_frame_media_override_from_env_value(
        std::env::var(LAN_RELIABLE_WHOLE_FRAME_ENV).ok().as_deref(),
    )
}

fn reliable_whole_frame_media_override_from_env_value(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        "" => None,
        _ => None,
    }
}

#[cfg(windows)]
fn lan_render_pacing_enabled() -> bool {
    lan_render_pacing_from_env_value(std::env::var(LAN_RENDER_PACING_ENV).ok().as_deref())
}

fn lan_render_pacing_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

fn h264_access_unit_is_keyframe(metadata_is_keyframe: bool, payload: &[u8]) -> bool {
    metadata_is_keyframe
        || h264_annexb_nal_types(payload)
            .into_iter()
            .any(|nal_type| nal_type == 5)
        || h264_avcc_nal_types(payload)
            .into_iter()
            .any(|nal_type| nal_type == 5)
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
    if matches!(snapshot.lifecycle_state.as_str(), "closed" | "failed") {
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
            match media_v3_reassembler
                .push_datagram(&media_message)
                .context("failed to reassemble LAN QUIC media v3 frame")?
            {
                Some(frame) => {
                    quic_media_v3_frame_to_legacy_frame(
                        &app_state,
                        &session_id,
                        frame,
                        media_v3_reassembler.stats(),
                    )
                    .await?
                }
                None => None,
            }
        } else {
            reassembler
                .push_datagram(&media_message)
                .context("failed to reassemble LAN QUIC media v2 frame")?
        };
        receiver_stats.record_elapsed("receiver.reassemble", reassemble_started);

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
                            continue;
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
        if let Some(metrics) = receiver_stats.take_stage_metrics(Instant::now()) {
            app_state
                .media_pipelines
                .lock()
                .await
                .set_stage_metrics(session_id.clone(), metrics);
        }
    }
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
        #[cfg(windows)]
        if let Err(error) = render_lan_decoded_frame(app_state, session_id, &decoded_frame).await {
            tracing::warn!(
                %error,
                session_id = %session_id.0,
                sequence,
                "LAN media receiver failed to present decoded frame"
            );
        }

        let width = decoded_frame.width as u32;
        let height = decoded_frame.height as u32;
        let decoded_pixel_format = decoded_frame_pixel_format(&decoded_frame);
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
        let payload_hash = format!("fnv1a64:{:016x}", fnv1a64(encoded_payload));
        let preview_frame = if should_update_lan_preview(sequence) {
            match decoded_frame_to_preview_rgb24(decoded_frame) {
                Ok(preview_frame) => Some(preview_frame),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id = %session_id.0,
                        sequence,
                        "LAN media preview frame was not CPU-readable"
                    );
                    None
                }
            }
        } else {
            None
        };
        let (preview_width, preview_height, preview_rgb24) = preview_frame
            .map(|(width, height, rgb24)| (Some(width), Some(height), Some(rgb24)))
            .unwrap_or((None, None, None));

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
                pixel_format: preview_rgb24
                    .as_ref()
                    .map(|_| "rgb24".to_string())
                    .unwrap_or(decoded_pixel_format),
                payload_hash,
                preview_width,
                preview_height,
                rgb24: preview_rgb24,
            },
            now_ms(),
        );
    }
}

#[cfg(windows)]
enum LanRenderTaskOutcome {
    Rendered { duration_ms: f64 },
    Dropped,
    Idle,
}

#[cfg(windows)]
async fn render_lan_decoded_frame(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    decoded_frame: &DecodedFrame,
) -> Result<()> {
    let render_frame = decoded_frame_to_render_frame(decoded_frame.clone())?;
    let (enqueue, enqueue_gap_ms) = {
        let mut render_queues = app_state.media_render_queues.lock().await;
        let now = Instant::now();
        let enqueue_gap_ms = render_queues
            .record_enqueued(session_id, now)
            .map(duration_as_millis);
        let enqueue = render_queues.enqueue_latest(session_id.clone(), render_frame);
        (enqueue, enqueue_gap_ms)
    };
    if let Some(enqueue_gap_ms) = enqueue_gap_ms {
        app_state
            .media_pipelines
            .lock()
            .await
            .record_stage_duration_ms(session_id.clone(), "render_enqueue_gap", enqueue_gap_ms);
    }
    match enqueue {
        MediaRenderQueueEnqueue::Start(frame) => {
            spawn_lan_render_worker(app_state.clone(), session_id.clone(), frame);
        }
        MediaRenderQueueEnqueue::Queued { replaced } => {
            let mut pipelines = app_state.media_pipelines.lock().await;
            pipelines.record_queue_depth(session_id.clone(), 1);
            if replaced {
                pipelines.increment_render_queue_replacements(session_id.clone(), 1);
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_lan_render_worker(
    app_state: Arc<AppState>,
    session_id: SessionId,
    first_frame: RenderFrame,
) {
    tokio::spawn(async move {
        let mut frame = first_frame;
        loop {
            pace_lan_render_frame(&app_state, &session_id).await;
            match render_lan_frame_once(app_state.clone(), session_id.clone(), frame).await {
                Ok(LanRenderTaskOutcome::Rendered { duration_ms }) => {
                    app_state
                        .media_pipelines
                        .lock()
                        .await
                        .record_stage_duration_ms(
                            session_id.clone(),
                            "render_present",
                            duration_ms,
                        );
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

            let next_frame = app_state
                .media_render_queues
                .lock()
                .await
                .take_next_or_finish(&session_id);
            match next_frame {
                Some(next_frame) => {
                    app_state
                        .media_pipelines
                        .lock()
                        .await
                        .record_queue_depth(session_id.clone(), 0);
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
    });
}

#[cfg(windows)]
async fn pace_lan_render_frame(app_state: &Arc<AppState>, session_id: &SessionId) {
    if !lan_render_pacing_enabled() {
        return;
    }

    let fps = selected_media_profile(app_state, session_id).await.fps;
    let delay =
        app_state
            .media_render_queues
            .lock()
            .await
            .pacing_delay(session_id, fps, Instant::now());
    if delay < Duration::from_micros(500) {
        return;
    }

    let started = Instant::now();
    sleep_until_lan_render_frame(started + delay).await;
    app_state
        .media_pipelines
        .lock()
        .await
        .record_stage_duration_ms(
            session_id.clone(),
            "render_pacing_wait",
            started.elapsed().as_secs_f64() * 1000.0,
        );
}

#[cfg(windows)]
async fn sleep_until_lan_render_frame(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }

        if deadline > now + Duration::from_micros(750) {
            tokio::task::yield_now().await;
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(windows)]
async fn render_lan_frame_once(
    app_state: Arc<AppState>,
    session_id: SessionId,
    frame: RenderFrame,
) -> Result<LanRenderTaskOutcome> {
    let started = Instant::now();
    let mut renderers = match app_state.media_surface_renderers.try_lock() {
        Ok(renderers) => renderers,
        Err(_) => match timeout(
            LAN_RENDER_SURFACE_LOCK_TIMEOUT,
            app_state.media_surface_renderers.lock(),
        )
        .await
        {
            Ok(renderers) => renderers,
            Err(_) => return Ok(LanRenderTaskOutcome::Dropped),
        },
    };
    let rendered = renderers
        .render_frame(&session_id, &frame)
        .map_err(anyhow::Error::msg)?;
    if rendered > 0 {
        Ok(LanRenderTaskOutcome::Rendered {
            duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    } else {
        Ok(LanRenderTaskOutcome::Idle)
    }
}

#[cfg(windows)]
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
        DecodedFrameData::CpuNv12 { .. } => {
            let (width, height, rgb24) = decoded_frame_to_rgb24(frame)?;
            Ok(RenderFrame::from_rgb24(
                width as usize,
                height as usize,
                rgb24,
            ))
        }
        DecodedFrameData::CpuP010 { .. } => {
            anyhow::bail!("CPU P010 decoded frames are not supported by the D3D11 renderer yet")
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
        match mrd_decode::create_decoder(backend) {
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
        let mut decoder = match mrd_decode::create_decoder(backend) {
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

fn preferred_lan_receiver_decoder_candidates(codec: LanAccessUnitCodec) -> Vec<&'static str> {
    let preferred = std::env::var("MRD_LAN_RECEIVER_DECODER")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match (codec, preferred.as_str()) {
        (LanAccessUnitCodec::H264, "software" | "h264_software" | "openh264") => {
            vec!["h264_software"]
        }
        (LanAccessUnitCodec::H264, "nvdec" | "nvdec_d3d11_shared" | "d3d11_shared") => {
            vec!["nvdec_d3d11_shared", "nvdec"]
        }
        (LanAccessUnitCodec::H264, "nvdec_cpu" | "nvdec_cpu_nv12") => vec!["nvdec"],
        (
            LanAccessUnitCodec::Hevc,
            "nvdec"
            | "hevc"
            | "nvdec_hevc_d3d11_shared"
            | "nvdec_d3d11_shared_hevc"
            | "d3d11_shared",
        ) => {
            vec!["nvdec_hevc_d3d11_shared", "nvdec_hevc"]
        }
        (LanAccessUnitCodec::Hevc, "nvdec_cpu" | "nvdec_cpu_nv12" | "nvdec_hevc") => {
            vec!["nvdec_hevc"]
        }
        _ => default_lan_receiver_decoder_candidates(codec).to_vec(),
    }
}

fn lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
    preferred_backend: Option<&'static str>,
) -> Vec<&'static str> {
    prioritize_lan_receiver_decoder_candidates(
        preferred_lan_receiver_decoder_candidates(codec),
        preferred_backend,
    )
}

fn prioritize_lan_receiver_decoder_candidates(
    candidates: Vec<&'static str>,
    preferred_backend: Option<&'static str>,
) -> Vec<&'static str> {
    let Some(preferred_backend) = preferred_backend else {
        return candidates;
    };
    if !candidates.contains(&preferred_backend) {
        return candidates;
    }

    let mut prioritized = vec![preferred_backend];
    prioritized.extend(
        candidates
            .into_iter()
            .filter(|backend| *backend != preferred_backend),
    );
    prioritized
}

#[cfg(windows)]
fn default_lan_receiver_decoder_candidates(codec: LanAccessUnitCodec) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["nvdec_d3d11_shared", "nvdec", "h264_software"],
        LanAccessUnitCodec::Hevc => &["nvdec_hevc_d3d11_shared", "nvdec_hevc"],
    }
}

#[cfg(target_os = "linux")]
fn default_lan_receiver_decoder_candidates(codec: LanAccessUnitCodec) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["linux_h264", "h264_software"],
        LanAccessUnitCodec::Hevc => &["linux_hevc"],
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn default_lan_receiver_decoder_candidates(codec: LanAccessUnitCodec) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["h264_software"],
        LanAccessUnitCodec::Hevc => &[],
    }
}

async fn session_allows_media(app_state: &Arc<AppState>, session_id: &SessionId) -> bool {
    let sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id) else {
        return false;
    };
    !matches!(snapshot.lifecycle_state.as_str(), "closed" | "failed")
}

async fn mark_session_failed(app_state: &Arc<AppState>, session_id: &SessionId, reason: String) {
    let mut sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id).cloned() else {
        return;
    };
    if snapshot.lifecycle_state == "closed" {
        return;
    }
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
            lifecycle_state: "failed".to_string(),
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
    DxgiDesktop(mrd_capture_dxgi::DxgiDesktopCapture),
    #[cfg(windows)]
    Winrt(mrd_capture_winrt::WinrtCapture),
    #[cfg(target_os = "macos")]
    Macos(mrd_capture_macos::MacosScreenCapture),
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
            LanFrameCapture::DxgiDesktop(capture) => {
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
        return Ok(TEST_SYNTHETIC_CAPTURE_SOURCE_ID.to_string());
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
        return create_windows_lan_frame_capture(source_id, _profile);
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(LanFrameCapture::Macos(
            crate::capture_source::create_frame_capture(source_id)?,
        ));
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

#[cfg(windows)]
fn create_windows_lan_frame_capture(
    source_id: &str,
    profile: &MediaProfile,
) -> Result<LanFrameCapture> {
    match windows_lan_capture_backend(source_id) {
        "dxgi_shared" => {
            let mut capture = mrd_capture_dxgi::DxgiSharedTextureCapture::new_primary()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            capture.set_target_dimensions(profile.width as usize, profile.height as usize);
            Ok(LanFrameCapture::DxgiShared(capture))
        }
        "dxgi" => Ok(LanFrameCapture::DxgiDesktop(
            mrd_capture_dxgi::DxgiDesktopCapture::new_primary()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
        )),
        _ => Ok(LanFrameCapture::Winrt(
            crate::capture_source::create_frame_capture(source_id)?,
        )),
    }
}

#[cfg(windows)]
fn windows_lan_capture_backend(source_id: &str) -> &'static str {
    let normalized = source_id.trim().to_ascii_lowercase();
    if normalized.starts_with("windows:display-shared:") {
        "dxgi_shared"
    } else if normalized.starts_with("windows:display:") {
        "dxgi"
    } else {
        "winrt"
    }
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

fn even_dimension(value: usize) -> usize {
    value & !1
}

fn frame_bytes_per_pixel(pixel_format: FramePixelFormat) -> usize {
    match pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
    }
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

fn describe_lan_access_unit(codec: LanAccessUnitCodec, payload: &[u8]) -> String {
    match codec {
        LanAccessUnitCodec::H264 => describe_h264_access_unit(payload),
        LanAccessUnitCodec::Hevc => describe_hevc_access_unit(payload),
    }
}

fn describe_hevc_access_unit(payload: &[u8]) -> String {
    let prefix_hex = payload
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "payload_bytes={}, prefix_hex=[{}]",
        payload.len(),
        prefix_hex
    )
}

fn describe_h264_access_unit(payload: &[u8]) -> String {
    let prefix_hex = payload
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let annexb_nals = h264_annexb_nal_types(payload);
    let avcc_nals = if annexb_nals.is_empty() {
        h264_avcc_nal_types(payload)
    } else {
        Vec::new()
    };

    format!(
        "payload_bytes={}, prefix_hex=[{}], annexb_nals=[{}], avcc_nals=[{}]",
        payload.len(),
        prefix_hex,
        annexb_nals
            .iter()
            .map(|nal| nal.to_string())
            .collect::<Vec<_>>()
            .join(","),
        avcc_nals
            .iter()
            .map(|nal| nal.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn h264_annexb_nal_types(payload: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut offset = 0usize;
    while let Some((start, start_len)) = find_h264_start_code(payload, offset) {
        let nal_header = start + start_len;
        if let Some(&header) = payload.get(nal_header) {
            types.push(header & 0x1f);
        }
        offset = nal_header.saturating_add(1);
    }
    types
}

fn h264_avcc_nal_types(payload: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut offset = 0usize;
    while offset + 4 <= payload.len() {
        let nal_len = u32::from_be_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]) as usize;
        offset += 4;
        if nal_len == 0 || offset + nal_len > payload.len() {
            return Vec::new();
        }
        types.push(payload[offset] & 0x1f);
        offset += nal_len;
    }
    if offset == payload.len() {
        types
    } else {
        Vec::new()
    }
}

fn find_h264_start_code(payload: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index + 3 <= payload.len() {
        if payload[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        if index + 4 <= payload.len() && payload[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        index += 1;
    }
    None
}

fn should_update_lan_preview(sequence: u64) -> bool {
    sequence <= 1 || sequence % LAN_PREVIEW_FRAME_INTERVAL == 0
}

fn decoded_frame_pixel_format(frame: &DecodedFrame) -> String {
    match &frame.data {
        DecodedFrameData::CpuRgb24(_) => "cpu_rgb24",
        DecodedFrameData::CpuBgra32(_) => "cpu_bgra32",
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
        _ => anyhow::bail!("decoded frame is not CPU RGB/BGRA/NV12 backed"),
    };

    Ok((frame.width as u32, frame.height as u32, rgb))
}

fn decoded_frame_to_preview_rgb24(frame: DecodedFrame) -> Result<(u32, u32, Vec<u8>)> {
    let (width, height, rgb) = decoded_frame_to_rgb24(frame)?;
    let (target_width, target_height) =
        preview_dimensions(width, height, LAN_PREVIEW_MAX_WIDTH, LAN_PREVIEW_MAX_HEIGHT);
    if target_width == width && target_height == height {
        return Ok((width, height, rgb));
    }

    let source_width = width as usize;
    let source_height = height as usize;
    let target_width = target_width as usize;
    let target_height = target_height as usize;
    let mut scaled = Vec::with_capacity(target_width * target_height * 3);
    for y in 0..target_height {
        let source_y = y * source_height / target_height;
        for x in 0..target_width {
            let source_x = x * source_width / target_width;
            let offset = (source_y * source_width + source_x) * 3;
            scaled.extend_from_slice(&rgb[offset..offset + 3]);
        }
    }

    Ok((target_width as u32, target_height as u32, scaled))
}

fn preview_dimensions(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (1, 1);
    }
    if width <= max_width && height <= max_height {
        return (width, height);
    }

    let scale = (max_width.max(1) as f64 / width as f64)
        .min(max_height.max(1) as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
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

fn clamp_yuv_to_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
fn build_media_probe_frame(sequence: u64, timestamp_us: u64, profile: &MediaProfile) -> Vec<u8> {
    let media_payload = build_probe_compressed_pattern(sequence, profile);
    let payload_hash = fnv1a64(&media_payload);
    let mut frame = Vec::with_capacity(LAN_MEDIA_PROBE_HEADER_BYTES + media_payload.len());
    frame.extend_from_slice(LAN_MEDIA_PROBE_MAGIC);
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(&timestamp_us.to_le_bytes());
    frame.extend_from_slice(&profile.width.to_le_bytes());
    frame.extend_from_slice(&profile.height.to_le_bytes());
    frame.extend_from_slice(&LAN_MEDIA_PROBE_FORMAT_CODE.to_le_bytes());
    frame.extend_from_slice(&(media_payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload_hash.to_le_bytes());
    frame.extend_from_slice(&profile.fps.to_le_bytes());
    frame.extend_from_slice(&profile.bitrate_mbps.to_le_bytes());
    frame.extend_from_slice(&media_payload);
    frame
}

fn decode_media_probe_frame(frame: &[u8]) -> Result<MediaProbeFrameStats> {
    if frame.len() < LAN_MEDIA_PROBE_HEADER_BYTES {
        anyhow::bail!("media probe frame is too small");
    }
    if &frame[..LAN_MEDIA_PROBE_MAGIC.len()] != LAN_MEDIA_PROBE_MAGIC {
        anyhow::bail!("media probe frame has invalid magic");
    }

    let sequence = u64::from_le_bytes(frame[8..16].try_into().unwrap());
    let timestamp_us = u64::from_le_bytes(frame[16..24].try_into().unwrap());
    let width = u32::from_le_bytes(frame[24..28].try_into().unwrap());
    let height = u32::from_le_bytes(frame[28..32].try_into().unwrap());
    let format_code = u32::from_le_bytes(frame[32..36].try_into().unwrap());
    let payload_len = u32::from_le_bytes(frame[36..40].try_into().unwrap()) as usize;
    let expected_hash = u64::from_le_bytes(frame[40..48].try_into().unwrap());
    let target_fps = u32::from_le_bytes(frame[48..52].try_into().unwrap());
    let target_bitrate_mbps = u32::from_le_bytes(frame[52..56].try_into().unwrap());

    let Some(expected_len) = LAN_MEDIA_PROBE_HEADER_BYTES.checked_add(payload_len) else {
        anyhow::bail!("media probe frame payload length overflow");
    };
    if frame.len() != expected_len {
        anyhow::bail!(
            "media probe frame payload length mismatch: expected {}, got {}",
            expected_len,
            frame.len()
        );
    }
    if format_code != LAN_MEDIA_PROBE_FORMAT_CODE {
        anyhow::bail!("unsupported media probe format code: {format_code}");
    }

    let media_payload = &frame[LAN_MEDIA_PROBE_HEADER_BYTES..];
    let actual_hash = fnv1a64(media_payload);
    if actual_hash != expected_hash {
        anyhow::bail!("media probe payload hash mismatch");
    }

    Ok(MediaProbeFrameStats {
        bytes_received: frame.len() as u64,
        sequence,
        timestamp_us,
        width,
        height,
        target_fps,
        target_bitrate_mbps,
        payload_bytes: payload_len as u32,
        format: media_probe_format(width, height, target_fps, target_bitrate_mbps).to_string(),
        payload_hash: format!("fnv1a64:{actual_hash:016x}"),
    })
}

fn send_lan_sender_stats_datagram(
    endpoint: &QuinnDatagramEndpoint,
    max_datagram_size: usize,
    payload: &LanSenderStatsPayload,
) -> Result<()> {
    let datagram = encode_lan_sender_stats_datagram(payload)?;
    if datagram.len() > max_datagram_size {
        anyhow::bail!(
            "LAN sender stats datagram too large: {} > {}",
            datagram.len(),
            max_datagram_size
        );
    }
    endpoint
        .send_datagram(datagram.into())
        .context("failed to send LAN sender stats datagram")
}

fn encode_lan_sender_stats_datagram(payload: &LanSenderStatsPayload) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(payload).context("failed to encode LAN sender stats payload")?;
    let payload_len =
        u32::try_from(json.len()).context("LAN sender stats payload exceeds u32 length")?;
    let mut frame = Vec::with_capacity(LAN_MEDIA_SENDER_STATS_HEADER_BYTES + json.len());
    frame.extend_from_slice(LAN_MEDIA_SENDER_STATS_MAGIC);
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&json);
    Ok(frame)
}

fn decode_lan_sender_stats_datagram(frame: &[u8]) -> Result<Option<LanSenderStatsPayload>> {
    if !frame.starts_with(LAN_MEDIA_SENDER_STATS_MAGIC) {
        return Ok(None);
    }
    if frame.len() < LAN_MEDIA_SENDER_STATS_HEADER_BYTES {
        anyhow::bail!("LAN sender stats datagram is too small");
    }
    let payload_len = u32::from_le_bytes(frame[8..12].try_into().unwrap()) as usize;
    let Some(expected_len) = LAN_MEDIA_SENDER_STATS_HEADER_BYTES.checked_add(payload_len) else {
        anyhow::bail!("LAN sender stats datagram payload length overflow");
    };
    if frame.len() != expected_len {
        anyhow::bail!(
            "LAN sender stats datagram payload length mismatch: expected {}, got {}",
            expected_len,
            frame.len()
        );
    }
    let payload = serde_json::from_slice(&frame[LAN_MEDIA_SENDER_STATS_HEADER_BYTES..])
        .context("failed to decode LAN sender stats payload")?;
    Ok(Some(payload))
}

fn encode_lan_media_envelope(envelope: LanMediaEnvelope) -> Result<Vec<u8>> {
    let payload_len = u32::try_from(envelope.payload.len())
        .context("LAN media v2 envelope payload exceeds u32 length")?;
    let mut frame = Vec::with_capacity(LAN_MEDIA_ENVELOPE_HEADER_BYTES + envelope.payload.len());
    frame.extend_from_slice(LAN_MEDIA_ENVELOPE_MAGIC);
    frame.push(envelope.payload_type);
    frame.push(envelope.codec);
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&envelope.sequence.to_le_bytes());
    frame.extend_from_slice(&envelope.timestamp_us.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.width.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.height.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.fps.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.bitrate_mbps.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&envelope.payload);
    Ok(frame)
}

fn decode_lan_media_envelope(frame: &[u8]) -> Result<LanMediaEnvelope> {
    if frame.len() < LAN_MEDIA_ENVELOPE_HEADER_BYTES {
        anyhow::bail!("LAN media v2 envelope is too small");
    }
    if &frame[..LAN_MEDIA_ENVELOPE_MAGIC.len()] != LAN_MEDIA_ENVELOPE_MAGIC {
        anyhow::bail!("LAN media v2 envelope has invalid magic");
    }
    let payload_type = frame[8];
    let codec = frame[9];
    let sequence = u64::from_le_bytes(frame[12..20].try_into().unwrap());
    let timestamp_us = u64::from_le_bytes(frame[20..28].try_into().unwrap());
    let width = u32::from_le_bytes(frame[28..32].try_into().unwrap());
    let height = u32::from_le_bytes(frame[32..36].try_into().unwrap());
    let fps = u32::from_le_bytes(frame[36..40].try_into().unwrap());
    let bitrate_mbps = u32::from_le_bytes(frame[40..44].try_into().unwrap());
    let payload_len = u32::from_le_bytes(frame[44..48].try_into().unwrap()) as usize;
    let Some(expected_len) = LAN_MEDIA_ENVELOPE_HEADER_BYTES.checked_add(payload_len) else {
        anyhow::bail!("LAN media v2 envelope payload length overflow");
    };
    if frame.len() != expected_len {
        anyhow::bail!(
            "LAN media v2 envelope payload length mismatch: expected {}, got {}",
            expected_len,
            frame.len()
        );
    }
    if width == 0 || height == 0 || fps == 0 || bitrate_mbps == 0 {
        anyhow::bail!("LAN media v2 envelope contains an invalid media profile");
    }
    Ok(LanMediaEnvelope {
        payload_type,
        codec,
        sequence,
        timestamp_us,
        profile: lan_media_profile_from_envelope(width, height, fps, bitrate_mbps, codec),
        payload: frame[LAN_MEDIA_ENVELOPE_HEADER_BYTES..].to_vec(),
    })
}

fn lan_media_profile_from_envelope(
    width: u32,
    height: u32,
    fps: u32,
    bitrate_mbps: u32,
    codec: u8,
) -> MediaProfile {
    let mut profile = MediaProfile {
        width,
        height,
        fps,
        bitrate_mbps,
        codec: lan_media_codec_name(codec).to_string(),
        ..MediaProfile::default()
    };
    apply_lan_media_profile_defaults(&mut profile);
    profile
}

fn lan_media_codec_name(codec: u8) -> &'static str {
    match codec {
        LAN_MEDIA_CODEC_H264 => "h264",
        LAN_MEDIA_CODEC_HEVC => "hevc",
        _ => "unknown",
    }
}

fn lan_media_profile_id(profile: &MediaProfile) -> u32 {
    let mut bytes = Vec::with_capacity(20 + profile.codec.len());
    bytes.extend_from_slice(&profile.width.to_le_bytes());
    bytes.extend_from_slice(&profile.height.to_le_bytes());
    bytes.extend_from_slice(&profile.fps.to_le_bytes());
    bytes.extend_from_slice(&profile.bitrate_mbps.to_le_bytes());
    bytes.extend_from_slice(profile.codec.as_bytes());
    fnv1a64(&bytes) as u32
}

#[cfg(test)]
fn build_probe_compressed_pattern(sequence: u64, profile: &MediaProfile) -> Vec<u8> {
    let mut payload = vec![0_u8; media_payload_bytes(profile)];
    for (offset, byte) in payload.iter_mut().enumerate() {
        let lane = (offset as u64).wrapping_mul(31);
        *byte = lane
            .wrapping_add(sequence.wrapping_mul(17))
            .wrapping_add((offset as u64 >> 8) * 13) as u8;
    }
    payload
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

fn default_media_profile() -> MediaProfile {
    let mut profile = MediaProfile {
        width: LAN_MEDIA_TARGET_WIDTH,
        height: LAN_MEDIA_TARGET_HEIGHT,
        fps: LAN_MEDIA_TARGET_FPS,
        bitrate_mbps: LAN_MEDIA_TARGET_BITRATE_MBPS,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    apply_lan_media_profile_defaults(&mut profile);
    profile
}

fn default_media_profile_negotiation() -> MediaProfileNegotiation {
    let profile = default_media_profile();
    MediaProfileNegotiation {
        requested: profile.clone(),
        selected: profile,
        status: "accepted".to_string(),
        reason: None,
        selected_source_id: None,
        selected_width: None,
        selected_height: None,
        downgrade_reason: None,
    }
}

fn negotiate_media_profile(
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    let requested = requested_profile.unwrap_or_else(default_media_profile);
    validate_media_profile(&requested)?;

    let mut selected = requested.clone();
    selected.width = selected.width.min(LAN_MEDIA_TARGET_WIDTH);
    selected.height = selected.height.min(LAN_MEDIA_TARGET_HEIGHT);
    selected.fps = selected.fps.min(LAN_MEDIA_MAX_FPS);
    selected.bitrate_mbps = selected.bitrate_mbps.min(LAN_MEDIA_TARGET_BITRATE_MBPS);
    normalize_lan_media_profile(&mut selected);

    let changed = selected != requested;
    Ok(MediaProfileNegotiation {
        requested,
        selected: selected.clone(),
        status: if changed { "downgraded" } else { "accepted" }.to_string(),
        reason: if changed {
            Some("clamped to LAN QUIC media capability".to_string())
        } else {
            None
        },
        selected_source_id: None,
        selected_width: Some(selected.width),
        selected_height: Some(selected.height),
        downgrade_reason: if changed {
            Some("clamped to LAN QUIC media capability".to_string())
        } else {
            None
        },
    })
}

fn validate_media_profile(profile: &MediaProfile) -> Result<()> {
    if profile.width == 0 || profile.height == 0 || profile.fps == 0 || profile.bitrate_mbps == 0 {
        anyhow::bail!("media profile width, height, fps and bitrate must be greater than zero");
    }
    Ok(())
}

fn normalize_lan_media_profile(profile: &mut MediaProfile) {
    profile.codec = profile.codec.trim().to_ascii_lowercase();
    if profile.codec != "h264" && profile.codec != "hevc" {
        profile.codec = "h264".to_string();
        profile.codec_profile = None;
        profile.bit_depth = None;
        profile.chroma_subsampling = None;
        profile.pixel_format = None;
        profile.hdr_enabled = None;
        return;
    }
    apply_lan_media_profile_defaults(profile);
}

fn lan_runtime_media_profile(
    selected_profile: &MediaProfile,
    codec: LanAccessUnitCodec,
) -> MediaProfile {
    let mut profile = selected_profile.clone();
    profile.codec = codec.name().to_string();
    if codec == LanAccessUnitCodec::H264 {
        profile.codec_profile = Some("high".to_string());
        profile.bit_depth = Some(8);
        profile.chroma_subsampling = Some("4:2:0".to_string());
        profile.pixel_format = Some("nv12".to_string());
        profile.hdr_enabled = Some(false);
    } else {
        apply_lan_media_profile_defaults(&mut profile);
    }
    profile
}

fn apply_lan_media_profile_defaults(profile: &mut MediaProfile) {
    if profile.codec.eq_ignore_ascii_case("hevc") {
        profile.codec = "hevc".to_string();
        if profile.codec_profile.is_none() {
            profile.codec_profile = Some("main".to_string());
        }
        if profile.bit_depth.is_none() {
            profile.bit_depth = Some(8);
        }
        if profile.chroma_subsampling.is_none() {
            profile.chroma_subsampling = Some("4:2:0".to_string());
        }
        if profile.pixel_format.is_none() {
            profile.pixel_format = Some("nv12".to_string());
        }
        if profile.hdr_enabled.is_none() {
            profile.hdr_enabled = Some(false);
        }
    }
}

fn media_frame_interval(profile: &MediaProfile) -> Duration {
    Duration::from_micros((1_000_000 / u64::from(profile.fps.max(1))).max(1))
}

fn media_profile_requests_high_resolution_timer(profile: &MediaProfile) -> bool {
    profile.fps >= LAN_MEDIA_HIGH_RESOLUTION_TIMER_MIN_FPS
}

fn media_frame_precise_sleep_guard(profile: &MediaProfile) -> Duration {
    if profile.fps < LAN_MEDIA_PRECISE_SLEEP_MIN_FPS {
        return Duration::ZERO;
    }

    LAN_MEDIA_PRECISE_SLEEP_GUARD.min(media_frame_interval(profile) / 2)
}

async fn sleep_until_media_frame(delay_until: Instant, profile: &MediaProfile) {
    let guard = media_frame_precise_sleep_guard(profile);
    if guard.is_zero() {
        sleep_until(delay_until).await;
        return;
    }

    let now = Instant::now();
    if delay_until > now + guard {
        sleep_until(delay_until - guard).await;
    }

    loop {
        if Instant::now() >= delay_until {
            break;
        }
        std::hint::spin_loop();
    }
}

#[derive(Default)]
struct MediaTimerResolution {
    requested: bool,
    #[cfg(windows)]
    period: Option<WindowsMediaTimerPeriod>,
}

impl MediaTimerResolution {
    fn update_for_profile(&mut self, profile: &MediaProfile) {
        if media_profile_requests_high_resolution_timer(profile) {
            self.request();
        } else {
            self.release();
        }
    }

    fn request(&mut self) {
        if self.requested {
            return;
        }
        #[cfg(windows)]
        {
            match WindowsMediaTimerPeriod::begin(LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS) {
                Some(period) => {
                    self.period = Some(period);
                    self.requested = true;
                }
                None => {
                    tracing::debug!(
                        period_ms = LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS,
                        "failed to request high resolution media timer"
                    );
                }
            }
        }
        #[cfg(not(windows))]
        {
            self.requested = true;
        }
    }

    fn release(&mut self) {
        #[cfg(windows)]
        {
            self.period = None;
        }
        self.requested = false;
    }
}

#[cfg(windows)]
struct WindowsMediaTimerPeriod {
    period_ms: u32,
}

#[cfg(windows)]
impl WindowsMediaTimerPeriod {
    fn begin(period_ms: u32) -> Option<Self> {
        let result = unsafe { timeBeginPeriod(period_ms) };
        if result == 0 {
            Some(Self { period_ms })
        } else {
            None
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsMediaTimerPeriod {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(self.period_ms);
        }
    }
}

fn schedule_next_media_frame(
    now: Instant,
    next_frame_at: &mut Instant,
    frame_interval: Duration,
) -> Option<Instant> {
    if now >= *next_frame_at && now.duration_since(*next_frame_at) > frame_interval {
        *next_frame_at = now;
    }

    let delay_until = (*next_frame_at > now).then_some(*next_frame_at);
    *next_frame_at += frame_interval;
    delay_until
}

#[cfg(test)]
fn media_payload_bytes(profile: &MediaProfile) -> usize {
    ((profile.bitrate_mbps as usize * 1_000_000 / 8) / profile.fps.max(1) as usize).max(1)
}

fn media_probe_format(
    width: u32,
    height: u32,
    target_fps: u32,
    target_bitrate_mbps: u32,
) -> &'static str {
    if width == LAN_MEDIA_TARGET_WIDTH
        && height == LAN_MEDIA_TARGET_HEIGHT
        && target_fps == LAN_MEDIA_TARGET_FPS
        && target_bitrate_mbps == LAN_MEDIA_TARGET_BITRATE_MBPS
    {
        LAN_MEDIA_PROBE_NATIVE_HIGH_FORMAT
    } else {
        LAN_MEDIA_PROBE_DYNAMIC_FORMAT
    }
}

fn decoded_video_probe_format(codec: &str) -> String {
    match codec.trim().to_ascii_lowercase().as_str() {
        "hevc" | "h265" => "hevc_desktop_frame".to_string(),
        "av1" => "av1_desktop_frame".to_string(),
        _ => "h264_desktop_frame".to_string(),
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn normalize_transport_kind(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "quic_quinn" {
        "quic".to_string()
    } else {
        normalized
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn duration_as_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn new_instance_id() -> String {
    format!("mrd-{}-{}", std::process::id(), now_ms())
}

fn default_app_id() -> String {
    DISCOVERY_APP_ID.to_string()
}

fn is_valid_discovery_packet(magic: &str, app_id: &str) -> bool {
    magic == DISCOVERY_MAGIC && app_id.eq_ignore_ascii_case(DISCOVERY_APP_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(peer
            .media_capabilities
            .contains(&LAN_CAPTURE_DXGI_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_ENCODE_NVENC_H264_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_ENCODE_NVENC_HEVC_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_DECODE_NVDEC_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_DECODE_NVDEC_HEVC_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_RENDER_D3D11_NATIVE_CAPABILITY.to_string()));
        assert!(peer
            .media_capabilities
            .contains(&LAN_RENDER_D3D11_SHARED_NV12_CAPABILITY.to_string()));
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
                assert!(quic.listen_addr.ends_with(":0") == false);
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
        assert_eq!(snapshot.lifecycle_state, "listening");
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
        if let Some(data_url) = snapshot.latest_frame_data_url.as_deref() {
            assert!(data_url.starts_with("data:image/png;base64,"));
        }
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
        assert_eq!(session_snapshot.lifecycle_state, "streaming");
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
    fn media_probe_frame_uses_native_high_compressed_profile() {
        let profile = default_media_profile();
        let frame = build_media_probe_frame(42, 123_456, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.sequence, 42);
        assert_eq!(stats.width, 2560);
        assert_eq!(stats.height, 1600);
        assert_eq!(stats.target_fps, 165);
        assert_eq!(stats.target_bitrate_mbps, 120);
        assert_eq!(stats.format, "compressed_native_high_test_pattern");
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
    fn lan_media_frame_orderer_skips_gap_when_pending_limit_is_reached() {
        let mut orderer = LanMediaFrameOrderer::new(2);

        assert_eq!(
            frame_ids(&orderer.push(test_quic_au_frame(10, true))),
            vec![10]
        );
        assert!(orderer.push(test_quic_au_frame(12, false)).is_empty());
        assert!(orderer.push(test_quic_au_frame(13, false)).is_empty());
        let ready = orderer.push(test_quic_au_frame(14, false));

        assert_eq!(frame_ids(&ready), vec![12, 13, 14]);
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
    fn windows_receiver_decoder_defaults_to_d3d11_shared_nvdec() {
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264),
            &["nvdec_d3d11_shared", "nvdec", "h264_software"]
        );
        assert_eq!(
            default_lan_receiver_decoder_candidates(LanAccessUnitCodec::Hevc),
            &["nvdec_hevc_d3d11_shared", "nvdec_hevc"]
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
                lifecycle_state: "listening".to_string(),
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
                lifecycle_state: "listening".to_string(),
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
                lifecycle_state: "listening".to_string(),
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
                lifecycle_state: "streaming".to_string(),
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
    fn capture_sources_ack_trims_preview_payload_to_udp_budget() {
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
                preview_data_url: Some(format!("data:image/png;base64,{}", "A".repeat(8_000))),
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
            .any(|source| source.preview_data_url.is_some()));
        assert!(sources
            .iter()
            .any(|source| source.preview_data_url.is_none()));
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

    #[cfg(windows)]
    #[test]
    fn windows_lan_sender_prefers_dxgi_for_display_sources() {
        assert_eq!(
            windows_lan_capture_backend("windows:display-shared:0"),
            "dxgi_shared"
        );
        assert_eq!(windows_lan_capture_backend("windows:display:0"), "dxgi");
        assert_eq!(
            windows_lan_capture_backend("windows:window:0x1234"),
            "winrt"
        );
    }

    #[test]
    fn lan_sender_encoder_order_prefers_hardware_before_fallback() {
        let backends = preferred_lan_h264_encoder_backends();
        #[cfg(windows)]
        assert_eq!(backends, ["nvenc_h264", "openh264"]);
        #[cfg(not(windows))]
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
    fn high_quality_reliable_media_prefers_per_message_streams_to_reduce_hol() {
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
    fn render_pacing_env_override_is_opt_in_only() {
        assert!(!lan_render_pacing_from_env_value(None));
        assert!(!lan_render_pacing_from_env_value(Some("")));
        assert!(!lan_render_pacing_from_env_value(Some("0")));
        assert!(!lan_render_pacing_from_env_value(Some("off")));
        assert!(lan_render_pacing_from_env_value(Some("1")));
        assert!(lan_render_pacing_from_env_value(Some("true")));
    }

    #[test]
    fn stable_high_quality_media_keeps_delta_frames_on_datagrams_by_default() {
        let stable_bitrate = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
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
    fn ultra_high_bitrate_fps_uses_reliable_whole_frame_by_default() {
        let ultra_high = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
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
        let high_quality_2k144 = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };

        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k144,
            None
        ));
        assert!(should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k144,
            Some(true)
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            true,
            true,
            64,
            &high_quality_2k144,
            Some(false)
        ));
        assert!(!should_send_access_unit_as_reliable_frame(
            false,
            true,
            64,
            &high_quality_2k144,
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
    fn decoded_preview_downscales_large_frames() {
        let frame = DecodedFrame {
            width: 1920,
            height: 1080,
            timestamp_us: 0,
            data: DecodedFrameData::CpuRgb24(vec![128; 1920 * 1080 * 3]),
        };

        let (width, height, rgb) = decoded_frame_to_preview_rgb24(frame).unwrap();

        assert_eq!((width, height), (480, 270));
        assert_eq!(rgb.len(), 480 * 270 * 3);
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
                lifecycle_state: "listening".to_string(),
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );

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
            LAN_PREVIEW_FRAME_INTERVAL,
            123_456,
            &profile,
            &[1, 2, 3, 4],
        )
        .await;

        let snapshot = app_state.probes.lock().await.snapshot(&session_id);

        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(
            snapshot.last_media_sequence,
            Some(LAN_PREVIEW_FRAME_INTERVAL)
        );
        assert!(snapshot.latest_frame_data_url.is_none());
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
                lifecycle_state: "listening".to_string(),
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );
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
            lifecycle_state: "listening".to_string(),
            last_error: None,
            sender_active: true,
            receiver_active: false,
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
