use super::discovery_identity::default_app_id;
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, ControlInputEvent, ControlInputLane, DisplayMode,
    DisplayModeChange, MediaProfile, MediaProfileNegotiation,
};
use serde::{Deserialize, Serialize};

pub(super) const PROTOCOL_VERSION: u32 = 1;
pub(super) const DISCOVERY_PACKET_BUFFER_BYTES: usize = 65_535;
pub(super) const DISCOVERY_SAFE_UDP_PAYLOAD_BYTES: usize = 60_000;

pub(super) const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";
pub(super) const LAN_QUIC_MEDIA_PROFILE_TRANSPORT: &str = "quic_datagram_2k144";
pub(super) const LAN_QUIC_MEDIA_V2_TRANSPORT: &str = "quic_datagram_media_v2";
pub(super) const LAN_QUIC_MEDIA_V3_TRANSPORT: &str = "quic_datagram_media_v3";
pub(super) const LAN_QUIC_RELIABLE_MEDIA_TRANSPORT: &str = "quic_stream_media_v2";
pub(super) const LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT: &str = "quic_stream_media_v3";
pub(super) const LAN_MEDIA_PROFILE_CONTROL_TRANSPORT: &str = "media_profile_control_v1";
pub(super) const LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT: &str = "capture_source_control_v1";
pub(super) const LAN_DISPLAY_MODE_CONTROL_TRANSPORT: &str = "display_mode_control_v1";
pub(super) const LAN_INPUT_CONTROL_TRANSPORT: &str = "input_control_v1";
pub(super) const LAN_MEDIA_PROTOCOL_VERSION: u32 = 3;
pub(super) const LAN_INPUT_CONTROL_CAPABILITY: &str = "control.keyboard_mouse";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum LanDiscoveryPacket {
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
pub(super) struct LanAnnouncement {
    pub(super) magic: String,
    #[serde(default = "default_app_id")]
    pub(super) app_id: String,
    pub(super) instance_id: String,
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) device_type: String,
    pub(super) protocol_version: u32,
    pub(super) discovery_port: u16,
    pub(super) transports: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) service_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) media_protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) media_capabilities: Vec<String>,
    pub(super) timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LanMediaBootstrap {
    pub(super) transport_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) quic: Option<LanQuicBootstrap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LanQuicBootstrap {
    pub(super) listen_addr: String,
    pub(super) server_name: String,
    pub(super) cert_der: Vec<u8>,
}
