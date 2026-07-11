use super::discovery_identity::default_app_id;
use super::discovery_identity::{DISCOVERY_APP_ID, DISCOVERY_MAGIC};
use mrd_identity::{public_key_id, verify_context_bytes, DeviceIdentity};
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, ControlInputEvent, ControlInputLane, DisplayMode,
    DisplayModeChange, MediaProfile, MediaProfileNegotiation, RemoteAccessMode,
    RemoteDevicePowerAction, RemoteFailure, RemotePermissionScope, RemoteReasonCode,
};
use mrd_transport_quic_quinn::certificate_fingerprint_sha256;
use ring::digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, net::SocketAddr};

#[cfg(test)]
pub(super) const PROTOCOL_VERSION: u32 = 1;
pub const SIGNED_LAN_PROTOCOL_VERSION: u32 = 3;
pub(super) const DISCOVERY_PACKET_BUFFER_BYTES: usize = 65_535;
pub(super) const DISCOVERY_SAFE_UDP_PAYLOAD_BYTES: usize = 60_000;

const ANNOUNCEMENT_SIGNATURE_CONTEXT: &str = "MRD_LAN_ANNOUNCEMENT_V3";
const SESSION_REQUEST_SIGNATURE_CONTEXT: &str = "MRD_LAN_SESSION_REQUEST_V3";
const SESSION_GRANT_SIGNATURE_CONTEXT: &str = "MRD_LAN_SESSION_GRANT_V3";
const SESSION_BOOTSTRAP_SIGNATURE_CONTEXT: &str = "MRD_LAN_SESSION_BOOTSTRAP_V3";
const QUIC_CONTROLLER_PROOF_SIGNATURE_CONTEXT: &str = "MRD_LAN_QUIC_CONTROLLER_PROOF_V3";
const ANNOUNCEMENT_MAX_LIFETIME_MS: u64 = 15_000;
const SESSION_REQUEST_MAX_LIFETIME_MS: u64 = 60_000;
const SESSION_GRANT_MAX_LIFETIME_MS: u64 = 600_000;
const SESSION_BOOTSTRAP_MAX_LIFETIME_MS: u64 = 5_000;
const QUIC_CONTROLLER_CHALLENGE_MAX_LIFETIME_MS: u64 = 5_000;
const ALLOWED_CLOCK_SKEW_MS: u64 = 2_000;

pub(super) const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";
pub(super) const LAN_QUIC_MEDIA_PROFILE_TRANSPORT: &str = "quic_datagram_2k144";
pub(super) const LAN_QUIC_MEDIA_V2_TRANSPORT: &str = "quic_datagram_media_v2";
pub(super) const LAN_QUIC_MEDIA_V3_TRANSPORT: &str = "quic_datagram_media_v3";
pub(super) const LAN_QUIC_RELIABLE_MEDIA_TRANSPORT: &str = "quic_stream_media_v2";
pub(super) const LAN_QUIC_PERSISTENT_MEDIA_TRANSPORT: &str = "quic_stream_media_v3";
pub(super) const LAN_MEDIA_PROFILE_CONTROL_TRANSPORT: &str = "media_profile_control_v1";
pub(super) const LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT: &str = "capture_source_control_v1";
pub(super) const LAN_DISPLAY_MODE_CONTROL_TRANSPORT: &str = "display_mode_control_v1";
pub(super) const LAN_INPUT_CONTROL_TRANSPORT: &str = "input_control_v2";
pub(super) const LAN_REMOTE_POWER_CONTROL_TRANSPORT: &str = "remote_power_control_v1";
pub(super) const LAN_MEDIA_PROTOCOL_VERSION: u32 = 3;
pub(super) const LAN_INPUT_CONTROL_CAPABILITY: &str = "control.keyboard_mouse";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// Keeping packet payloads inline preserves the established public enum API; even the largest
// signed bootstrap remains below one KiB, while its variable-sized certificate stays in a Vec.
#[allow(clippy::large_enum_variant)]
pub enum LanDiscoveryPacket {
    Probe {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        device_id: Option<String>,
        timestamp_ms: u64,
    },
    Announce(LanAnnouncement),
    SignedAnnounce(SignedLanAnnouncement),
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
    SignedRemoteSessionRequest(SignedLanSessionRequest),
    RemoteSessionAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<LegacyLanMediaBootstrap>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_profile: Option<MediaProfileNegotiation>,
        timestamp_ms: u64,
    },
    SignedRemoteSessionBootstrap(SignedLanSessionBootstrap),
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
    RemoteDevicePowerAction {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        source_device_id: String,
        action: RemoteDevicePowerAction,
        timestamp_ms: u64,
    },
    RemoteDevicePowerActionAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        device_id: String,
        action: RemoteDevicePowerAction,
        accepted: bool,
        message: Option<String>,
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanAnnouncement {
    pub magic: String,
    #[serde(default = "default_app_id")]
    pub app_id: String,
    pub instance_id: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub protocol_version: u32,
    pub discovery_port: u16,
    pub transports: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyLanMediaBootstrap {
    pub transport_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic: Option<LegacyLanQuicBootstrap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyLanQuicBootstrap {
    pub listen_addr: String,
    pub server_name: String,
    pub cert_der: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanMediaBootstrap {
    pub transport_kind: String,
    #[serde(default)]
    pub quic: Option<LanQuicBootstrap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanQuicBootstrap {
    pub listen_addr: String,
    pub server_name: String,
    pub certificate_fingerprint_sha256: [u8; 32],
    pub cert_der: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanProtocolError {
    PayloadEncoding,
    SigningFailed,
    InvalidNamespace,
    UnsupportedProtocol,
    InvalidPayload,
    InvalidKeyBinding,
    InvalidKeyEpoch,
    InvalidLifetime,
    NotYetValid,
    Expired,
    InvalidNonce,
    CapabilityMismatch,
    InvalidSignature,
    PeerBindingMismatch,
    CertificateFingerprintMismatch,
    InvalidBootstrap,
}

impl LanProtocolError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::PayloadEncoding => "E_LAN_PAYLOAD_ENCODING",
            Self::SigningFailed => "E_LAN_SIGNING_FAILED",
            Self::InvalidNamespace => "E_LAN_INVALID_NAMESPACE",
            Self::UnsupportedProtocol => "E_LAN_UNSUPPORTED_PROTOCOL",
            Self::InvalidPayload => "E_LAN_INVALID_PAYLOAD",
            Self::InvalidKeyBinding => "E_LAN_INVALID_KEY_BINDING",
            Self::InvalidKeyEpoch => "E_LAN_INVALID_KEY_EPOCH",
            Self::InvalidLifetime => "E_LAN_INVALID_LIFETIME",
            Self::NotYetValid => "E_LAN_NOT_YET_VALID",
            Self::Expired => "E_LAN_EXPIRED",
            Self::InvalidNonce => "E_LAN_INVALID_NONCE",
            Self::CapabilityMismatch => "E_LAN_CAPABILITY_MISMATCH",
            Self::InvalidSignature => "E_LAN_INVALID_SIGNATURE",
            Self::PeerBindingMismatch => "E_LAN_PEER_BINDING_MISMATCH",
            Self::CertificateFingerprintMismatch => "E_LAN_CERTIFICATE_FINGERPRINT_MISMATCH",
            Self::InvalidBootstrap => "E_LAN_INVALID_BOOTSTRAP",
        }
    }

    pub const fn remote_reason_code(self) -> RemoteReasonCode {
        match self {
            Self::InvalidKeyBinding
            | Self::InvalidKeyEpoch
            | Self::InvalidSignature
            | Self::PeerBindingMismatch => RemoteReasonCode::IdentityMismatch,
            Self::CertificateFingerprintMismatch => RemoteReasonCode::CertificateBindingMismatch,
            Self::InvalidNonce => RemoteReasonCode::ReplayDetected,
            Self::PayloadEncoding
            | Self::SigningFailed
            | Self::InvalidNamespace
            | Self::UnsupportedProtocol
            | Self::InvalidPayload
            | Self::InvalidLifetime
            | Self::NotYetValid
            | Self::Expired
            | Self::CapabilityMismatch
            | Self::InvalidBootstrap => RemoteReasonCode::ProtocolDowngradeBlocked,
        }
    }

    pub fn remote_reason_code_from_diagnostic(diagnostic: &str) -> Option<RemoteReasonCode> {
        let diagnostic = diagnostic.to_ascii_uppercase();
        [
            Self::PayloadEncoding,
            Self::SigningFailed,
            Self::InvalidNamespace,
            Self::UnsupportedProtocol,
            Self::InvalidPayload,
            Self::InvalidKeyBinding,
            Self::InvalidKeyEpoch,
            Self::InvalidLifetime,
            Self::NotYetValid,
            Self::Expired,
            Self::InvalidNonce,
            Self::CapabilityMismatch,
            Self::InvalidSignature,
            Self::PeerBindingMismatch,
            Self::CertificateFingerprintMismatch,
            Self::InvalidBootstrap,
        ]
        .into_iter()
        .find(|error| diagnostic.contains(error.code()))
        .map(Self::remote_reason_code)
    }
}

impl fmt::Display for LanProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for LanProtocolError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedLanAnnouncementPayload {
    pub announcement: LanAnnouncement,
    pub discovery_endpoint: SocketAddr,
    pub signer_key_id: String,
    pub signer_key_epoch: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
    pub capability_hash: [u8; 32],
}

#[derive(Serialize)]
struct SignedLanAnnouncementPayloadRef<'a> {
    announcement: AuthenticatedLanAnnouncementRef<'a>,
    discovery_endpoint: SocketAddr,
    signer_key_id: &'a str,
    signer_key_epoch: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
    capability_hash: &'a [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedLanAnnouncementPayloadWire {
    announcement: AuthenticatedLanAnnouncement,
    discovery_endpoint: SocketAddr,
    signer_key_id: String,
    signer_key_epoch: u64,
    expires_at_ms: u64,
    nonce: [u8; 16],
    capability_hash: [u8; 32],
}

#[derive(Serialize)]
struct AuthenticatedLanAnnouncementRef<'a> {
    magic: &'a str,
    app_id: &'a str,
    instance_id: &'a str,
    device_id: &'a str,
    device_name: &'a str,
    device_type: &'a str,
    protocol_version: u32,
    discovery_port: u16,
    transports: &'a [String],
    service_build_id: Option<&'a str>,
    media_protocol_version: Option<u32>,
    media_capabilities: &'a [String],
    mac_address: Option<&'a str>,
    timestamp_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedLanAnnouncement {
    magic: String,
    app_id: String,
    instance_id: String,
    device_id: String,
    device_name: String,
    device_type: String,
    protocol_version: u32,
    discovery_port: u16,
    transports: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    service_build_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    media_protocol_version: Option<u32>,
    media_capabilities: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    mac_address: Option<String>,
    timestamp_ms: u64,
}

impl<'a> From<&'a LanAnnouncement> for AuthenticatedLanAnnouncementRef<'a> {
    fn from(announcement: &'a LanAnnouncement) -> Self {
        Self {
            magic: &announcement.magic,
            app_id: &announcement.app_id,
            instance_id: &announcement.instance_id,
            device_id: &announcement.device_id,
            device_name: &announcement.device_name,
            device_type: &announcement.device_type,
            protocol_version: announcement.protocol_version,
            discovery_port: announcement.discovery_port,
            transports: &announcement.transports,
            service_build_id: announcement.service_build_id.as_deref(),
            media_protocol_version: announcement.media_protocol_version,
            media_capabilities: &announcement.media_capabilities,
            mac_address: announcement.mac_address.as_deref(),
            timestamp_ms: announcement.timestamp_ms,
        }
    }
}

impl From<AuthenticatedLanAnnouncement> for LanAnnouncement {
    fn from(announcement: AuthenticatedLanAnnouncement) -> Self {
        Self {
            magic: announcement.magic,
            app_id: announcement.app_id,
            instance_id: announcement.instance_id,
            device_id: announcement.device_id,
            device_name: announcement.device_name,
            device_type: announcement.device_type,
            protocol_version: announcement.protocol_version,
            discovery_port: announcement.discovery_port,
            transports: announcement.transports,
            service_build_id: announcement.service_build_id,
            media_protocol_version: announcement.media_protocol_version,
            media_capabilities: announcement.media_capabilities,
            mac_address: announcement.mac_address,
            timestamp_ms: announcement.timestamp_ms,
        }
    }
}

impl Serialize for SignedLanAnnouncementPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SignedLanAnnouncementPayloadRef {
            announcement: (&self.announcement).into(),
            discovery_endpoint: self.discovery_endpoint,
            signer_key_id: &self.signer_key_id,
            signer_key_epoch: self.signer_key_epoch,
            expires_at_ms: self.expires_at_ms,
            nonce: &self.nonce,
            capability_hash: &self.capability_hash,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SignedLanAnnouncementPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SignedLanAnnouncementPayloadWire::deserialize(deserializer)?;
        Ok(Self {
            announcement: wire.announcement.into(),
            discovery_endpoint: wire.discovery_endpoint,
            signer_key_id: wire.signer_key_id,
            signer_key_epoch: wire.signer_key_epoch,
            expires_at_ms: wire.expires_at_ms,
            nonce: wire.nonce,
            capability_hash: wire.capability_hash,
        })
    }
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Serialize)]
struct AuthenticatedMediaProfileRef<'a> {
    width: u32,
    height: u32,
    fps: u32,
    bitrate_mbps: u32,
    codec: &'a str,
    codec_profile: Option<&'a str>,
    bit_depth: Option<u8>,
    chroma_subsampling: Option<&'a str>,
    pixel_format: Option<&'a str>,
    hdr_enabled: Option<bool>,
    color_mode: Option<&'a str>,
    color_pipeline: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedMediaProfile {
    width: u32,
    height: u32,
    fps: u32,
    bitrate_mbps: u32,
    codec: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    codec_profile: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    bit_depth: Option<u8>,
    #[serde(deserialize_with = "deserialize_required_option")]
    chroma_subsampling: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pixel_format: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    hdr_enabled: Option<bool>,
    #[serde(deserialize_with = "deserialize_required_option")]
    color_mode: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    color_pipeline: Option<String>,
}

impl<'a> From<&'a MediaProfile> for AuthenticatedMediaProfileRef<'a> {
    fn from(profile: &'a MediaProfile) -> Self {
        Self {
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            bitrate_mbps: profile.bitrate_mbps,
            codec: &profile.codec,
            codec_profile: profile.codec_profile.as_deref(),
            bit_depth: profile.bit_depth,
            chroma_subsampling: profile.chroma_subsampling.as_deref(),
            pixel_format: profile.pixel_format.as_deref(),
            hdr_enabled: profile.hdr_enabled,
            color_mode: profile.color_mode.as_deref(),
            color_pipeline: profile.color_pipeline.as_deref(),
        }
    }
}

impl From<AuthenticatedMediaProfile> for MediaProfile {
    fn from(profile: AuthenticatedMediaProfile) -> Self {
        Self {
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            bitrate_mbps: profile.bitrate_mbps,
            codec: profile.codec,
            codec_profile: profile.codec_profile,
            bit_depth: profile.bit_depth,
            chroma_subsampling: profile.chroma_subsampling,
            pixel_format: profile.pixel_format,
            hdr_enabled: profile.hdr_enabled,
            color_mode: profile.color_mode,
            color_pipeline: profile.color_pipeline,
        }
    }
}

#[derive(Serialize)]
struct AuthenticatedMediaProfileNegotiationRef<'a> {
    requested: AuthenticatedMediaProfileRef<'a>,
    selected: AuthenticatedMediaProfileRef<'a>,
    status: &'a str,
    reason: Option<&'a str>,
    selected_source_id: Option<&'a str>,
    selected_width: Option<u32>,
    selected_height: Option<u32>,
    downgrade_reason: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedMediaProfileNegotiation {
    requested: AuthenticatedMediaProfile,
    selected: AuthenticatedMediaProfile,
    status: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    reason: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    selected_source_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    selected_width: Option<u32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    selected_height: Option<u32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    downgrade_reason: Option<String>,
}

impl<'a> From<&'a MediaProfileNegotiation> for AuthenticatedMediaProfileNegotiationRef<'a> {
    fn from(negotiation: &'a MediaProfileNegotiation) -> Self {
        Self {
            requested: (&negotiation.requested).into(),
            selected: (&negotiation.selected).into(),
            status: &negotiation.status,
            reason: negotiation.reason.as_deref(),
            selected_source_id: negotiation.selected_source_id.as_deref(),
            selected_width: negotiation.selected_width,
            selected_height: negotiation.selected_height,
            downgrade_reason: negotiation.downgrade_reason.as_deref(),
        }
    }
}

impl From<AuthenticatedMediaProfileNegotiation> for MediaProfileNegotiation {
    fn from(negotiation: AuthenticatedMediaProfileNegotiation) -> Self {
        Self {
            requested: negotiation.requested.into(),
            selected: negotiation.selected.into(),
            status: negotiation.status,
            reason: negotiation.reason,
            selected_source_id: negotiation.selected_source_id,
            selected_width: negotiation.selected_width,
            selected_height: negotiation.selected_height,
            downgrade_reason: negotiation.downgrade_reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanAnnouncement {
    pub payload: SignedLanAnnouncementPayload,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanUnattendedProof {
    pub access_epoch: u64,
    pub proof: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanSessionRequest {
    pub magic: String,
    pub app_id: String,
    pub protocol_version: u32,
    pub instance_id: String,
    pub session_id: String,
    pub source_device_id: String,
    pub source_device_name: String,
    pub source_key_id: String,
    pub source_key_epoch: u64,
    pub target_device_id: String,
    pub target_key_id: String,
    pub target_key_epoch: u64,
    pub transport_kind: String,
    pub source_discovery_port: Option<u16>,
    pub source_endpoint: SocketAddr,
    pub source_media_capabilities: Vec<String>,
    pub requested_media_profile: Option<MediaProfile>,
    pub access_mode: RemoteAccessMode,
    pub requested_scopes: Vec<RemotePermissionScope>,
    pub unattended_proof: Option<LanUnattendedProof>,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
}

#[derive(Serialize)]
struct LanSessionRequestRef<'a> {
    magic: &'a str,
    app_id: &'a str,
    protocol_version: u32,
    instance_id: &'a str,
    session_id: &'a str,
    source_device_id: &'a str,
    source_device_name: &'a str,
    source_key_id: &'a str,
    source_key_epoch: u64,
    target_device_id: &'a str,
    target_key_id: &'a str,
    target_key_epoch: u64,
    transport_kind: &'a str,
    source_discovery_port: Option<u16>,
    source_endpoint: SocketAddr,
    source_media_capabilities: &'a [String],
    requested_media_profile: Option<AuthenticatedMediaProfileRef<'a>>,
    access_mode: RemoteAccessMode,
    requested_scopes: &'a [RemotePermissionScope],
    unattended_proof: Option<&'a LanUnattendedProof>,
    timestamp_ms: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanSessionRequestWire {
    magic: String,
    app_id: String,
    protocol_version: u32,
    instance_id: String,
    session_id: String,
    source_device_id: String,
    source_device_name: String,
    source_key_id: String,
    source_key_epoch: u64,
    target_device_id: String,
    target_key_id: String,
    target_key_epoch: u64,
    transport_kind: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    source_discovery_port: Option<u16>,
    source_endpoint: SocketAddr,
    source_media_capabilities: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    requested_media_profile: Option<AuthenticatedMediaProfile>,
    access_mode: RemoteAccessMode,
    requested_scopes: Vec<RemotePermissionScope>,
    #[serde(deserialize_with = "deserialize_required_option")]
    unattended_proof: Option<LanUnattendedProof>,
    timestamp_ms: u64,
    expires_at_ms: u64,
    nonce: [u8; 16],
}

impl Serialize for LanSessionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        LanSessionRequestRef {
            magic: &self.magic,
            app_id: &self.app_id,
            protocol_version: self.protocol_version,
            instance_id: &self.instance_id,
            session_id: &self.session_id,
            source_device_id: &self.source_device_id,
            source_device_name: &self.source_device_name,
            source_key_id: &self.source_key_id,
            source_key_epoch: self.source_key_epoch,
            target_device_id: &self.target_device_id,
            target_key_id: &self.target_key_id,
            target_key_epoch: self.target_key_epoch,
            transport_kind: &self.transport_kind,
            source_discovery_port: self.source_discovery_port,
            source_endpoint: self.source_endpoint,
            source_media_capabilities: &self.source_media_capabilities,
            requested_media_profile: self
                .requested_media_profile
                .as_ref()
                .map(AuthenticatedMediaProfileRef::from),
            access_mode: self.access_mode,
            requested_scopes: &self.requested_scopes,
            unattended_proof: self.unattended_proof.as_ref(),
            timestamp_ms: self.timestamp_ms,
            expires_at_ms: self.expires_at_ms,
            nonce: &self.nonce,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LanSessionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LanSessionRequestWire::deserialize(deserializer)?;
        Ok(Self {
            magic: wire.magic,
            app_id: wire.app_id,
            protocol_version: wire.protocol_version,
            instance_id: wire.instance_id,
            session_id: wire.session_id,
            source_device_id: wire.source_device_id,
            source_device_name: wire.source_device_name,
            source_key_id: wire.source_key_id,
            source_key_epoch: wire.source_key_epoch,
            target_device_id: wire.target_device_id,
            target_key_id: wire.target_key_id,
            target_key_epoch: wire.target_key_epoch,
            transport_kind: wire.transport_kind,
            source_discovery_port: wire.source_discovery_port,
            source_endpoint: wire.source_endpoint,
            source_media_capabilities: wire.source_media_capabilities,
            requested_media_profile: wire.requested_media_profile.map(Into::into),
            access_mode: wire.access_mode,
            requested_scopes: wire.requested_scopes,
            unattended_proof: wire.unattended_proof,
            timestamp_ms: wire.timestamp_ms,
            expires_at_ms: wire.expires_at_ms,
            nonce: wire.nonce,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanSessionRequest {
    pub payload: LanSessionRequest,
    pub capability_hash: [u8; 32],
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanSessionGrantPayload {
    pub session_id: String,
    pub controller_key_id: String,
    pub controller_key_epoch: u64,
    pub target_key_id: String,
    pub target_key_epoch: u64,
    pub access_mode: RemoteAccessMode,
    pub granted_scopes: Vec<RemotePermissionScope>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub policy_revision: u64,
    pub route_constraint: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub profile_constraint: Option<[u8; 32]>,
    pub request_nonce: [u8; 16],
    pub grant_nonce: [u8; 16],
    #[serde(deserialize_with = "deserialize_required_option")]
    pub windows_session_id: Option<u32>,
    pub transport_fingerprint_sha256: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanSessionGrant {
    pub payload: LanSessionGrantPayload,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Server-generated, single-connection challenge that binds the QUIC channel
/// to the exact signed session grant and ephemeral server certificate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanQuicControllerChallenge {
    pub protocol_version: u32,
    pub session_id: String,
    pub grant_id: [u8; 32],
    pub transport_fingerprint_sha256: [u8; 32],
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanQuicControllerProofPayload {
    pub controller_key_id: String,
    pub controller_key_epoch: u64,
    pub challenge: LanQuicControllerChallenge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanQuicControllerProof {
    pub payload: LanQuicControllerProofPayload,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanSessionBootstrap {
    pub magic: String,
    pub app_id: String,
    pub protocol_version: u32,
    pub instance_id: String,
    pub session_id: String,
    pub controller_key_id: String,
    pub controller_key_epoch: u64,
    pub target_key_id: String,
    pub target_key_epoch: u64,
    pub request_nonce: [u8; 16],
    pub accepted: bool,
    pub message: Option<String>,
    pub failure: Option<RemoteFailure>,
    pub grant: Option<SignedLanSessionGrant>,
    pub media: Option<LanMediaBootstrap>,
    pub media_profile: Option<MediaProfileNegotiation>,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
}

#[derive(Serialize)]
struct LanSessionBootstrapRef<'a> {
    magic: &'a str,
    app_id: &'a str,
    protocol_version: u32,
    instance_id: &'a str,
    session_id: &'a str,
    controller_key_id: &'a str,
    controller_key_epoch: u64,
    target_key_id: &'a str,
    target_key_epoch: u64,
    request_nonce: &'a [u8; 16],
    accepted: bool,
    message: Option<&'a str>,
    failure: Option<&'a RemoteFailure>,
    grant: Option<&'a SignedLanSessionGrant>,
    media: Option<&'a LanMediaBootstrap>,
    media_profile: Option<AuthenticatedMediaProfileNegotiationRef<'a>>,
    timestamp_ms: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanSessionBootstrapWire {
    magic: String,
    app_id: String,
    protocol_version: u32,
    instance_id: String,
    session_id: String,
    controller_key_id: String,
    controller_key_epoch: u64,
    target_key_id: String,
    target_key_epoch: u64,
    request_nonce: [u8; 16],
    accepted: bool,
    #[serde(deserialize_with = "deserialize_required_option")]
    message: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    failure: Option<AuthenticatedRemoteFailure>,
    #[serde(deserialize_with = "deserialize_required_option")]
    grant: Option<SignedLanSessionGrant>,
    #[serde(deserialize_with = "deserialize_required_option")]
    media: Option<AuthenticatedLanMediaBootstrap>,
    #[serde(deserialize_with = "deserialize_required_option")]
    media_profile: Option<AuthenticatedMediaProfileNegotiation>,
    timestamp_ms: u64,
    expires_at_ms: u64,
    nonce: [u8; 16],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedLanMediaBootstrap {
    transport_kind: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    quic: Option<AuthenticatedLanQuicBootstrap>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedLanQuicBootstrap {
    listen_addr: String,
    server_name: String,
    certificate_fingerprint_sha256: [u8; 32],
    cert_der: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedRemoteFailure {
    code: RemoteReasonCode,
    message: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    suggested_action: Option<String>,
}

impl Serialize for LanSessionBootstrap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        LanSessionBootstrapRef {
            magic: &self.magic,
            app_id: &self.app_id,
            protocol_version: self.protocol_version,
            instance_id: &self.instance_id,
            session_id: &self.session_id,
            controller_key_id: &self.controller_key_id,
            controller_key_epoch: self.controller_key_epoch,
            target_key_id: &self.target_key_id,
            target_key_epoch: self.target_key_epoch,
            request_nonce: &self.request_nonce,
            accepted: self.accepted,
            message: self.message.as_deref(),
            failure: self.failure.as_ref(),
            grant: self.grant.as_ref(),
            media: self.media.as_ref(),
            media_profile: self
                .media_profile
                .as_ref()
                .map(AuthenticatedMediaProfileNegotiationRef::from),
            timestamp_ms: self.timestamp_ms,
            expires_at_ms: self.expires_at_ms,
            nonce: &self.nonce,
        }
        .serialize(serializer)
    }
}

impl From<AuthenticatedRemoteFailure> for RemoteFailure {
    fn from(failure: AuthenticatedRemoteFailure) -> Self {
        Self {
            code: failure.code,
            message: failure.message,
            suggested_action: failure.suggested_action,
        }
    }
}

impl From<AuthenticatedLanQuicBootstrap> for LanQuicBootstrap {
    fn from(quic: AuthenticatedLanQuicBootstrap) -> Self {
        Self {
            listen_addr: quic.listen_addr,
            server_name: quic.server_name,
            certificate_fingerprint_sha256: quic.certificate_fingerprint_sha256,
            cert_der: quic.cert_der,
        }
    }
}

impl From<AuthenticatedLanMediaBootstrap> for LanMediaBootstrap {
    fn from(media: AuthenticatedLanMediaBootstrap) -> Self {
        Self {
            transport_kind: media.transport_kind,
            quic: media.quic.map(Into::into),
        }
    }
}

impl<'de> Deserialize<'de> for LanSessionBootstrap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LanSessionBootstrapWire::deserialize(deserializer)?;
        Ok(Self {
            magic: wire.magic,
            app_id: wire.app_id,
            protocol_version: wire.protocol_version,
            instance_id: wire.instance_id,
            session_id: wire.session_id,
            controller_key_id: wire.controller_key_id,
            controller_key_epoch: wire.controller_key_epoch,
            target_key_id: wire.target_key_id,
            target_key_epoch: wire.target_key_epoch,
            request_nonce: wire.request_nonce,
            accepted: wire.accepted,
            message: wire.message,
            failure: wire.failure.map(Into::into),
            grant: wire.grant,
            media: wire.media.map(Into::into),
            media_profile: wire.media_profile.map(Into::into),
            timestamp_ms: wire.timestamp_ms,
            expires_at_ms: wire.expires_at_ms,
            nonce: wire.nonce,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanSessionBootstrap {
    pub payload: LanSessionBootstrap,
    pub capability_hash: [u8; 32],
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl SignedLanAnnouncement {
    pub fn sign(
        identity: &DeviceIdentity,
        signer_key_epoch: u64,
        announcement: LanAnnouncement,
        discovery_endpoint: SocketAddr,
        expires_at_ms: u64,
        nonce: [u8; 16],
    ) -> Result<Self, LanProtocolError> {
        let capability_hash = announcement_commitment_hash(&announcement, discovery_endpoint)?;
        let payload = SignedLanAnnouncementPayload {
            announcement,
            discovery_endpoint,
            signer_key_id: identity.key_id().to_string(),
            signer_key_epoch,
            expires_at_ms,
            nonce,
            capability_hash,
        };
        validate_announcement_payload(&payload)?;
        let canonical = announcement_signature_bytes(&payload)?;
        let signature = identity
            .sign_context_bytes(ANNOUNCEMENT_SIGNATURE_CONTEXT, &canonical)
            .map_err(|_| LanProtocolError::SigningFailed)?;
        Ok(Self {
            payload,
            public_key: identity.public_key().to_vec(),
            signature,
        })
    }

    pub fn verify(&self, now_ms: u64) -> Result<(), LanProtocolError> {
        validate_announcement_payload(&self.payload)?;
        validate_current_time(
            self.payload.announcement.timestamp_ms,
            self.payload.expires_at_ms,
            now_ms,
        )?;
        validate_public_key_binding(&self.public_key, &self.payload.signer_key_id)?;
        let expected_hash = announcement_commitment_hash(
            &self.payload.announcement,
            self.payload.discovery_endpoint,
        )?;
        if self.payload.capability_hash != expected_hash {
            return Err(LanProtocolError::CapabilityMismatch);
        }
        let canonical = announcement_signature_bytes(&self.payload)?;
        verify_context_bytes(
            &self.public_key,
            ANNOUNCEMENT_SIGNATURE_CONTEXT,
            &canonical,
            &self.signature,
        )
        .map_err(|_| LanProtocolError::InvalidSignature)
    }
}

impl SignedLanSessionRequest {
    pub fn sign(
        identity: &DeviceIdentity,
        payload: LanSessionRequest,
    ) -> Result<Self, LanProtocolError> {
        validate_session_request(&payload)?;
        if payload.source_key_id != identity.key_id() {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        let capability_hash = session_request_commitment_hash(&payload)?;
        let canonical = session_request_signature_bytes(&payload, &capability_hash)?;
        let signature = identity
            .sign_context_bytes(SESSION_REQUEST_SIGNATURE_CONTEXT, &canonical)
            .map_err(|_| LanProtocolError::SigningFailed)?;
        Ok(Self {
            payload,
            capability_hash,
            public_key: identity.public_key().to_vec(),
            signature,
        })
    }

    pub fn verify_for_target(
        &self,
        now_ms: u64,
        expected_target_key_id: &str,
        expected_target_key_epoch: u64,
    ) -> Result<(), LanProtocolError> {
        validate_session_request(&self.payload)?;
        validate_current_time(
            self.payload.timestamp_ms,
            self.payload.expires_at_ms,
            now_ms,
        )?;
        if expected_target_key_epoch == 0 {
            return Err(LanProtocolError::InvalidKeyEpoch);
        }
        if self.payload.target_key_id != expected_target_key_id
            || self.payload.target_key_epoch != expected_target_key_epoch
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        validate_public_key_binding(&self.public_key, &self.payload.source_key_id)?;
        let expected_hash = session_request_commitment_hash(&self.payload)?;
        if self.capability_hash != expected_hash {
            return Err(LanProtocolError::CapabilityMismatch);
        }
        let canonical = session_request_signature_bytes(&self.payload, &self.capability_hash)?;
        verify_context_bytes(
            &self.public_key,
            SESSION_REQUEST_SIGNATURE_CONTEXT,
            &canonical,
            &self.signature,
        )
        .map_err(|_| LanProtocolError::InvalidSignature)
    }
}

impl SignedLanSessionGrant {
    pub fn sign(
        identity: &DeviceIdentity,
        payload: LanSessionGrantPayload,
    ) -> Result<Self, LanProtocolError> {
        validate_session_grant(&payload)?;
        if payload.target_key_id != identity.key_id() {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        let canonical = session_grant_signature_bytes(&payload)?;
        let signature = identity
            .sign_context_bytes(SESSION_GRANT_SIGNATURE_CONTEXT, &canonical)
            .map_err(|_| LanProtocolError::SigningFailed)?;
        Ok(Self {
            payload,
            public_key: identity.public_key().to_vec(),
            signature,
        })
    }

    pub fn verify(
        &self,
        now_ms: u64,
        expected_target_public_key: &[u8],
        expected_target_key_epoch: u64,
    ) -> Result<(), LanProtocolError> {
        if expected_target_public_key.len() != 32 {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        if expected_target_key_epoch == 0 {
            return Err(LanProtocolError::InvalidKeyEpoch);
        }
        let expected_target_key_id = public_key_id(expected_target_public_key);
        self.verify_signature(now_ms)?;
        if self.public_key != expected_target_public_key
            || self.payload.target_key_id != expected_target_key_id
            || self.payload.target_key_epoch != expected_target_key_epoch
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        Ok(())
    }

    pub fn verify_for_request(
        &self,
        now_ms: u64,
        request: &SignedLanSessionRequest,
        expected_target_public_key: &[u8],
        expected_target_key_epoch: u64,
    ) -> Result<(), LanProtocolError> {
        if expected_target_public_key.len() != 32 {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        let expected_target_key_id = public_key_id(expected_target_public_key);
        request.verify_for_target(now_ms, &expected_target_key_id, expected_target_key_epoch)?;
        self.verify(
            now_ms,
            expected_target_public_key,
            expected_target_key_epoch,
        )?;
        if self.payload.session_id != request.payload.session_id
            || self.payload.controller_key_id != request.payload.source_key_id
            || self.payload.controller_key_epoch != request.payload.source_key_epoch
            || self.payload.target_key_id != request.payload.target_key_id
            || self.payload.target_key_epoch != request.payload.target_key_epoch
            || self.payload.target_key_epoch != expected_target_key_epoch
            || self.payload.access_mode != request.payload.access_mode
            || self.payload.route_constraint != request.payload.transport_kind
            || self.payload.request_nonce != request.payload.nonce
            || self
                .payload
                .issued_at_ms
                .saturating_add(ALLOWED_CLOCK_SKEW_MS)
                < request.payload.timestamp_ms
            || self
                .payload
                .granted_scopes
                .iter()
                .any(|scope| !request.payload.requested_scopes.contains(scope))
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        Ok(())
    }

    pub fn grant_id(&self) -> Result<[u8; 32], LanProtocolError> {
        canonical_hash(&SessionGrantIdCommitment {
            schema_version: SIGNED_LAN_PROTOCOL_VERSION,
            kind: "signed_session_grant_id",
            grant: self,
        })
    }

    fn verify_signature(&self, now_ms: u64) -> Result<(), LanProtocolError> {
        validate_session_grant(&self.payload)?;
        validate_current_time(
            self.payload.issued_at_ms,
            self.payload.expires_at_ms,
            now_ms,
        )?;
        validate_public_key_binding(&self.public_key, &self.payload.target_key_id)?;
        let canonical = session_grant_signature_bytes(&self.payload)?;
        verify_context_bytes(
            &self.public_key,
            SESSION_GRANT_SIGNATURE_CONTEXT,
            &canonical,
            &self.signature,
        )
        .map_err(|_| LanProtocolError::InvalidSignature)
    }
}

impl LanQuicControllerChallenge {
    pub fn verify_binding(
        &self,
        now_ms: u64,
        expected_session_id: &str,
        expected_grant_id: &[u8; 32],
        expected_transport_fingerprint_sha256: &[u8; 32],
    ) -> Result<(), LanProtocolError> {
        validate_quic_controller_challenge(self)?;
        validate_current_time(self.issued_at_ms, self.expires_at_ms, now_ms)?;
        if self.session_id != expected_session_id
            || &self.grant_id != expected_grant_id
            || &self.transport_fingerprint_sha256 != expected_transport_fingerprint_sha256
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        Ok(())
    }
}

impl SignedLanQuicControllerProof {
    pub fn sign(
        identity: &DeviceIdentity,
        controller_key_epoch: u64,
        challenge: LanQuicControllerChallenge,
    ) -> Result<Self, LanProtocolError> {
        validate_quic_controller_challenge(&challenge)?;
        if controller_key_epoch == 0 {
            return Err(LanProtocolError::InvalidKeyEpoch);
        }
        let payload = LanQuicControllerProofPayload {
            controller_key_id: identity.key_id().to_string(),
            controller_key_epoch,
            challenge,
        };
        let canonical = quic_controller_proof_signature_bytes(&payload)?;
        let signature = identity
            .sign_context_bytes(QUIC_CONTROLLER_PROOF_SIGNATURE_CONTEXT, &canonical)
            .map_err(|_| LanProtocolError::SigningFailed)?;
        Ok(Self {
            payload,
            public_key: identity.public_key().to_vec(),
            signature,
        })
    }

    pub fn verify(
        &self,
        now_ms: u64,
        expected_controller_public_key: &[u8],
        expected_controller_key_epoch: u64,
        expected_challenge: &LanQuicControllerChallenge,
    ) -> Result<(), LanProtocolError> {
        if expected_controller_public_key.len() != 32 || expected_controller_key_epoch == 0 {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        expected_challenge.verify_binding(
            now_ms,
            &expected_challenge.session_id,
            &expected_challenge.grant_id,
            &expected_challenge.transport_fingerprint_sha256,
        )?;
        if self.payload.challenge != *expected_challenge
            || self.payload.controller_key_epoch != expected_controller_key_epoch
            || self.payload.controller_key_id != public_key_id(expected_controller_public_key)
            || self.public_key != expected_controller_public_key
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        validate_public_key_binding(&self.public_key, &self.payload.controller_key_id)?;
        let canonical = quic_controller_proof_signature_bytes(&self.payload)?;
        verify_context_bytes(
            &self.public_key,
            QUIC_CONTROLLER_PROOF_SIGNATURE_CONTEXT,
            &canonical,
            &self.signature,
        )
        .map_err(|_| LanProtocolError::InvalidSignature)
    }
}

impl SignedLanSessionBootstrap {
    pub fn sign(
        identity: &DeviceIdentity,
        payload: LanSessionBootstrap,
    ) -> Result<Self, LanProtocolError> {
        validate_session_bootstrap(&payload)?;
        if payload.target_key_id != identity.key_id() {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        if let Some(grant) = payload.grant.as_ref() {
            grant.verify_signature(payload.timestamp_ms)?;
            if grant.public_key != identity.public_key()
                || grant.payload.target_key_id != payload.target_key_id
                || grant.payload.target_key_epoch != payload.target_key_epoch
            {
                return Err(LanProtocolError::PeerBindingMismatch);
            }
        }
        let capability_hash = session_bootstrap_commitment_hash(&payload)?;
        let canonical = session_bootstrap_signature_bytes(&payload, &capability_hash)?;
        let signature = identity
            .sign_context_bytes(SESSION_BOOTSTRAP_SIGNATURE_CONTEXT, &canonical)
            .map_err(|_| LanProtocolError::SigningFailed)?;
        Ok(Self {
            payload,
            capability_hash,
            public_key: identity.public_key().to_vec(),
            signature,
        })
    }

    pub fn verify_for_request(
        &self,
        now_ms: u64,
        request: &SignedLanSessionRequest,
        expected_target_public_key: &[u8],
        expected_target_key_epoch: u64,
    ) -> Result<(), LanProtocolError> {
        if expected_target_public_key.len() != 32 {
            return Err(LanProtocolError::InvalidKeyBinding);
        }
        let expected_target_key_id = public_key_id(expected_target_public_key);
        request.verify_for_target(now_ms, &expected_target_key_id, expected_target_key_epoch)?;
        validate_session_bootstrap(&self.payload)?;
        validate_current_time(
            self.payload.timestamp_ms,
            self.payload.expires_at_ms,
            now_ms,
        )?;
        if self.public_key != expected_target_public_key
            || self.payload.target_key_id != expected_target_key_id
            || self.payload.target_key_epoch != expected_target_key_epoch
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        validate_public_key_binding(&self.public_key, &self.payload.target_key_id)?;
        if self.payload.session_id != request.payload.session_id
            || self.payload.controller_key_id != request.payload.source_key_id
            || self.payload.controller_key_epoch != request.payload.source_key_epoch
            || self.payload.target_key_id != request.payload.target_key_id
            || self.payload.target_key_epoch != request.payload.target_key_epoch
            || self.payload.request_nonce != request.payload.nonce
            || self
                .payload
                .timestamp_ms
                .saturating_add(ALLOWED_CLOCK_SKEW_MS)
                < request.payload.timestamp_ms
        {
            return Err(LanProtocolError::PeerBindingMismatch);
        }
        if self.payload.accepted {
            let media = self
                .payload
                .media
                .as_ref()
                .ok_or(LanProtocolError::InvalidBootstrap)?;
            let grant = self
                .payload
                .grant
                .as_ref()
                .ok_or(LanProtocolError::InvalidBootstrap)?;
            if media.transport_kind != request.payload.transport_kind {
                return Err(LanProtocolError::PeerBindingMismatch);
            }
            grant.verify_for_request(
                now_ms,
                request,
                expected_target_public_key,
                expected_target_key_epoch,
            )?;
            if !grant
                .payload
                .granted_scopes
                .contains(&RemotePermissionScope::ScreenView)
                || grant.payload.route_constraint != media.transport_kind
                || self.payload.timestamp_ms < grant.payload.issued_at_ms
                || self.payload.expires_at_ms
                    > grant
                        .payload
                        .expires_at_ms
                        .saturating_add(ALLOWED_CLOCK_SKEW_MS)
            {
                return Err(LanProtocolError::PeerBindingMismatch);
            }
            if let Some(requested_profile) = request.payload.requested_media_profile.as_ref() {
                let negotiated_request = self
                    .payload
                    .media_profile
                    .as_ref()
                    .map(|negotiation| &negotiation.requested);
                if negotiated_request != Some(requested_profile) {
                    return Err(LanProtocolError::PeerBindingMismatch);
                }
            }
            let expected_profile_constraint = self
                .payload
                .media_profile
                .as_ref()
                .map(media_profile_constraint_hash)
                .transpose()?;
            if grant.payload.profile_constraint != expected_profile_constraint {
                return Err(LanProtocolError::PeerBindingMismatch);
            }
            let transport_fingerprint = media
                .quic
                .as_ref()
                .map(|quic| quic.certificate_fingerprint_sha256)
                .ok_or(LanProtocolError::InvalidBootstrap)?;
            if grant.payload.transport_fingerprint_sha256 != transport_fingerprint {
                return Err(LanProtocolError::CertificateFingerprintMismatch);
            }
        }
        let expected_hash = session_bootstrap_commitment_hash(&self.payload)?;
        if self.capability_hash != expected_hash {
            return Err(LanProtocolError::CapabilityMismatch);
        }
        let canonical = session_bootstrap_signature_bytes(&self.payload, &self.capability_hash)?;
        verify_context_bytes(
            &self.public_key,
            SESSION_BOOTSTRAP_SIGNATURE_CONTEXT,
            &canonical,
            &self.signature,
        )
        .map_err(|_| LanProtocolError::InvalidSignature)
    }
}

fn validate_namespace(
    magic: &str,
    app_id: &str,
    protocol_version: u32,
) -> Result<(), LanProtocolError> {
    if magic != DISCOVERY_MAGIC || app_id != DISCOVERY_APP_ID {
        return Err(LanProtocolError::InvalidNamespace);
    }
    if protocol_version != SIGNED_LAN_PROTOCOL_VERSION {
        return Err(LanProtocolError::UnsupportedProtocol);
    }
    Ok(())
}

fn validate_required(value: &str) -> Result<(), LanProtocolError> {
    if value.trim().is_empty() {
        Err(LanProtocolError::InvalidPayload)
    } else {
        Ok(())
    }
}

fn validate_nonce(nonce: &[u8; 16]) -> Result<(), LanProtocolError> {
    if nonce.iter().all(|byte| *byte == 0) {
        Err(LanProtocolError::InvalidNonce)
    } else {
        Ok(())
    }
}

fn validate_lifetime(
    issued_at_ms: u64,
    expires_at_ms: u64,
    maximum_lifetime_ms: u64,
) -> Result<(), LanProtocolError> {
    let lifetime = expires_at_ms
        .checked_sub(issued_at_ms)
        .ok_or(LanProtocolError::InvalidLifetime)?;
    if lifetime == 0 || lifetime > maximum_lifetime_ms {
        return Err(LanProtocolError::InvalidLifetime);
    }
    Ok(())
}

fn validate_current_time(
    issued_at_ms: u64,
    expires_at_ms: u64,
    now_ms: u64,
) -> Result<(), LanProtocolError> {
    if issued_at_ms > now_ms.saturating_add(ALLOWED_CLOCK_SKEW_MS) {
        return Err(LanProtocolError::NotYetValid);
    }
    if now_ms > expires_at_ms.saturating_add(ALLOWED_CLOCK_SKEW_MS) {
        return Err(LanProtocolError::Expired);
    }
    Ok(())
}

fn validate_public_key_binding(
    public_key: &[u8],
    claimed_key_id: &str,
) -> Result<(), LanProtocolError> {
    if public_key.len() != 32 || public_key_id(public_key) != claimed_key_id {
        return Err(LanProtocolError::InvalidKeyBinding);
    }
    Ok(())
}

fn validate_announcement_payload(
    payload: &SignedLanAnnouncementPayload,
) -> Result<(), LanProtocolError> {
    validate_announcement_structure(payload)?;
    validate_lifetime(
        payload.announcement.timestamp_ms,
        payload.expires_at_ms,
        ANNOUNCEMENT_MAX_LIFETIME_MS,
    )
}

fn validate_announcement_structure(
    payload: &SignedLanAnnouncementPayload,
) -> Result<(), LanProtocolError> {
    let announcement = &payload.announcement;
    validate_namespace(
        &announcement.magic,
        &announcement.app_id,
        announcement.protocol_version,
    )?;
    validate_required(&announcement.instance_id)?;
    validate_required(&announcement.device_id)?;
    validate_required(&announcement.device_name)?;
    validate_required(&announcement.device_type)?;
    validate_required(&payload.signer_key_id)?;
    if announcement.discovery_port == 0 {
        return Err(LanProtocolError::InvalidPayload);
    }
    if payload.discovery_endpoint.port() != announcement.discovery_port
        || payload.discovery_endpoint.ip().is_unspecified()
        || payload.discovery_endpoint.ip().is_multicast()
    {
        return Err(LanProtocolError::InvalidPayload);
    }
    if payload.signer_key_epoch == 0 {
        return Err(LanProtocolError::InvalidKeyEpoch);
    }
    validate_nonce(&payload.nonce)
}

fn validate_session_request(payload: &LanSessionRequest) -> Result<(), LanProtocolError> {
    validate_session_request_structure(payload)?;
    validate_lifetime(
        payload.timestamp_ms,
        payload.expires_at_ms,
        SESSION_REQUEST_MAX_LIFETIME_MS,
    )
}

fn validate_session_request_structure(payload: &LanSessionRequest) -> Result<(), LanProtocolError> {
    validate_namespace(&payload.magic, &payload.app_id, payload.protocol_version)?;
    validate_required(&payload.instance_id)?;
    validate_required(&payload.session_id)?;
    validate_required(&payload.source_device_id)?;
    validate_required(&payload.source_device_name)?;
    validate_required(&payload.source_key_id)?;
    validate_required(&payload.target_device_id)?;
    validate_required(&payload.target_key_id)?;
    validate_required(&payload.transport_kind)?;
    if payload.source_key_epoch == 0 || payload.target_key_epoch == 0 {
        return Err(LanProtocolError::InvalidKeyEpoch);
    }
    if payload.source_discovery_port == Some(0) {
        return Err(LanProtocolError::InvalidPayload);
    }
    if payload.source_endpoint.port() == 0
        || payload.source_endpoint.ip().is_unspecified()
        || payload.source_endpoint.ip().is_multicast()
    {
        return Err(LanProtocolError::InvalidPayload);
    }
    if payload.requested_scopes.is_empty()
        || payload
            .requested_scopes
            .iter()
            .enumerate()
            .any(|(index, scope)| payload.requested_scopes[..index].contains(scope))
    {
        return Err(LanProtocolError::InvalidPayload);
    }
    match (payload.access_mode, payload.unattended_proof.as_ref()) {
        (RemoteAccessMode::Attended, Some(_)) => return Err(LanProtocolError::InvalidPayload),
        (RemoteAccessMode::Unattended, Some(proof))
            if proof.access_epoch == 0 || proof.proof.len() != 32 =>
        {
            return Err(LanProtocolError::InvalidPayload);
        }
        _ => {}
    }
    validate_nonce(&payload.nonce)
}

fn validate_session_bootstrap(payload: &LanSessionBootstrap) -> Result<(), LanProtocolError> {
    validate_session_bootstrap_structure(payload)?;
    validate_lifetime(
        payload.timestamp_ms,
        payload.expires_at_ms,
        SESSION_BOOTSTRAP_MAX_LIFETIME_MS,
    )
}

fn validate_session_grant(payload: &LanSessionGrantPayload) -> Result<(), LanProtocolError> {
    validate_required(&payload.session_id)?;
    validate_required(&payload.controller_key_id)?;
    validate_required(&payload.target_key_id)?;
    validate_required(&payload.route_constraint)?;
    if payload.controller_key_epoch == 0 || payload.target_key_epoch == 0 {
        return Err(LanProtocolError::InvalidKeyEpoch);
    }
    if payload.granted_scopes.is_empty()
        || payload
            .granted_scopes
            .iter()
            .enumerate()
            .any(|(index, scope)| payload.granted_scopes[..index].contains(scope))
        || payload.policy_revision == 0
        || payload.windows_session_id == Some(0)
        || payload
            .transport_fingerprint_sha256
            .iter()
            .all(|byte| *byte == 0)
        || payload
            .profile_constraint
            .is_some_and(|hash| hash.iter().all(|byte| *byte == 0))
    {
        return Err(LanProtocolError::InvalidPayload);
    }
    validate_nonce(&payload.request_nonce)?;
    validate_nonce(&payload.grant_nonce)?;
    validate_lifetime(
        payload.issued_at_ms,
        payload.expires_at_ms,
        SESSION_GRANT_MAX_LIFETIME_MS,
    )
}

fn validate_quic_controller_challenge(
    challenge: &LanQuicControllerChallenge,
) -> Result<(), LanProtocolError> {
    if challenge.protocol_version != SIGNED_LAN_PROTOCOL_VERSION {
        return Err(LanProtocolError::UnsupportedProtocol);
    }
    validate_required(&challenge.session_id)?;
    if challenge.grant_id.iter().all(|byte| *byte == 0)
        || challenge
            .transport_fingerprint_sha256
            .iter()
            .all(|byte| *byte == 0)
    {
        return Err(LanProtocolError::InvalidPayload);
    }
    validate_nonce(&challenge.nonce)?;
    validate_lifetime(
        challenge.issued_at_ms,
        challenge.expires_at_ms,
        QUIC_CONTROLLER_CHALLENGE_MAX_LIFETIME_MS,
    )
}

fn validate_session_bootstrap_structure(
    payload: &LanSessionBootstrap,
) -> Result<(), LanProtocolError> {
    validate_namespace(&payload.magic, &payload.app_id, payload.protocol_version)?;
    validate_required(&payload.instance_id)?;
    validate_required(&payload.session_id)?;
    validate_required(&payload.controller_key_id)?;
    validate_required(&payload.target_key_id)?;
    if payload.controller_key_epoch == 0 || payload.target_key_epoch == 0 {
        return Err(LanProtocolError::InvalidKeyEpoch);
    }
    validate_nonce(&payload.request_nonce)?;
    validate_nonce(&payload.nonce)?;
    match (
        payload.accepted,
        payload.media.as_ref(),
        payload.grant.as_ref(),
        payload.failure.as_ref(),
    ) {
        (true, Some(media), Some(grant), None) => {
            if !grant
                .payload
                .granted_scopes
                .contains(&RemotePermissionScope::ScreenView)
            {
                return Err(LanProtocolError::InvalidBootstrap);
            }
            validate_media_bootstrap(media)?;
        }
        (false, None, None, Some(failure)) => {
            validate_required(&failure.message)?;
            if payload.message.is_some() || payload.media_profile.is_some() {
                return Err(LanProtocolError::InvalidBootstrap);
            }
        }
        _ => return Err(LanProtocolError::InvalidBootstrap),
    }
    Ok(())
}

fn validate_media_bootstrap(media: &LanMediaBootstrap) -> Result<(), LanProtocolError> {
    validate_required(&media.transport_kind)?;
    if media.transport_kind.eq_ignore_ascii_case("quic") && media.quic.is_none() {
        return Err(LanProtocolError::InvalidBootstrap);
    }
    if let Some(quic) = &media.quic {
        validate_required(&quic.listen_addr)?;
        validate_required(&quic.server_name)?;
        let listen_addr = quic
            .listen_addr
            .parse::<std::net::SocketAddr>()
            .map_err(|_| LanProtocolError::InvalidBootstrap)?;
        if listen_addr.port() == 0 || quic.cert_der.is_empty() {
            return Err(LanProtocolError::InvalidBootstrap);
        }
        if certificate_fingerprint_sha256(&quic.cert_der) != quic.certificate_fingerprint_sha256 {
            return Err(LanProtocolError::CertificateFingerprintMismatch);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct AnnouncementCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    announcement: &'a LanAnnouncement,
    discovery_endpoint: SocketAddr,
}

#[derive(Serialize)]
struct SessionRequestCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    request: &'a LanSessionRequest,
}

#[derive(Serialize)]
struct UnattendedTranscriptCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    request: &'a LanSessionRequest,
}

#[derive(Serialize)]
struct SessionBootstrapCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    bootstrap: &'a LanSessionBootstrap,
}

#[derive(Serialize)]
struct SessionGrantSignatureCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    payload: &'a LanSessionGrantPayload,
}

#[derive(Serialize)]
struct SessionGrantIdCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    grant: &'a SignedLanSessionGrant,
}

#[derive(Serialize)]
struct QuicControllerProofSignatureCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    payload: &'a LanQuicControllerProofPayload,
}

#[derive(Serialize)]
struct MediaProfileConstraintCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    negotiation: AuthenticatedMediaProfileNegotiationRef<'a>,
}

#[derive(Serialize)]
struct AnnouncementSignatureCommitment<'a> {
    schema_version: u32,
    signer_key_id: &'a str,
    signer_key_epoch: u64,
    discovery_endpoint: SocketAddr,
    capability_hash: &'a [u8; 32],
    issued_at_ms: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
}

#[derive(Serialize)]
struct SessionRequestSignatureCommitment<'a> {
    schema_version: u32,
    source_key_id: &'a str,
    source_key_epoch: u64,
    target_key_id: &'a str,
    target_key_epoch: u64,
    source_endpoint: SocketAddr,
    capability_hash: &'a [u8; 32],
    issued_at_ms: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
}

#[derive(Serialize)]
struct SessionBootstrapSignatureCommitment<'a> {
    schema_version: u32,
    controller_key_id: &'a str,
    controller_key_epoch: u64,
    target_key_id: &'a str,
    target_key_epoch: u64,
    request_nonce: &'a [u8; 16],
    capability_hash: &'a [u8; 32],
    issued_at_ms: u64,
    expires_at_ms: u64,
    nonce: &'a [u8; 16],
}

fn announcement_commitment_hash(
    announcement: &LanAnnouncement,
    discovery_endpoint: SocketAddr,
) -> Result<[u8; 32], LanProtocolError> {
    canonical_hash(&AnnouncementCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "announcement",
        announcement,
        discovery_endpoint,
    })
}

fn session_request_commitment_hash(
    request: &LanSessionRequest,
) -> Result<[u8; 32], LanProtocolError> {
    canonical_hash(&SessionRequestCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "session_request",
        request,
    })
}

pub fn unattended_transcript_bytes(
    request: &LanSessionRequest,
) -> Result<Vec<u8>, LanProtocolError> {
    let mut proof_free_request = request.clone();
    if let Some(proof) = proof_free_request.unattended_proof.as_mut() {
        proof.proof.clear();
    }
    canonical_bytes(&UnattendedTranscriptCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "unattended_session_request",
        request: &proof_free_request,
    })
}

pub fn media_profile_constraint_hash(
    negotiation: &MediaProfileNegotiation,
) -> Result<[u8; 32], LanProtocolError> {
    canonical_hash(&MediaProfileConstraintCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "media_profile_constraint",
        negotiation: negotiation.into(),
    })
}

fn session_bootstrap_commitment_hash(
    bootstrap: &LanSessionBootstrap,
) -> Result<[u8; 32], LanProtocolError> {
    canonical_hash(&SessionBootstrapCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "session_bootstrap",
        bootstrap,
    })
}

fn announcement_signature_bytes(
    payload: &SignedLanAnnouncementPayload,
) -> Result<Vec<u8>, LanProtocolError> {
    canonical_bytes(&AnnouncementSignatureCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        signer_key_id: &payload.signer_key_id,
        signer_key_epoch: payload.signer_key_epoch,
        discovery_endpoint: payload.discovery_endpoint,
        capability_hash: &payload.capability_hash,
        issued_at_ms: payload.announcement.timestamp_ms,
        expires_at_ms: payload.expires_at_ms,
        nonce: &payload.nonce,
    })
}

fn session_request_signature_bytes(
    payload: &LanSessionRequest,
    capability_hash: &[u8; 32],
) -> Result<Vec<u8>, LanProtocolError> {
    canonical_bytes(&SessionRequestSignatureCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        source_key_id: &payload.source_key_id,
        source_key_epoch: payload.source_key_epoch,
        target_key_id: &payload.target_key_id,
        target_key_epoch: payload.target_key_epoch,
        source_endpoint: payload.source_endpoint,
        capability_hash,
        issued_at_ms: payload.timestamp_ms,
        expires_at_ms: payload.expires_at_ms,
        nonce: &payload.nonce,
    })
}

fn session_bootstrap_signature_bytes(
    payload: &LanSessionBootstrap,
    capability_hash: &[u8; 32],
) -> Result<Vec<u8>, LanProtocolError> {
    canonical_bytes(&SessionBootstrapSignatureCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        controller_key_id: &payload.controller_key_id,
        controller_key_epoch: payload.controller_key_epoch,
        target_key_id: &payload.target_key_id,
        target_key_epoch: payload.target_key_epoch,
        request_nonce: &payload.request_nonce,
        capability_hash,
        issued_at_ms: payload.timestamp_ms,
        expires_at_ms: payload.expires_at_ms,
        nonce: &payload.nonce,
    })
}

fn session_grant_signature_bytes(
    payload: &LanSessionGrantPayload,
) -> Result<Vec<u8>, LanProtocolError> {
    canonical_bytes(&SessionGrantSignatureCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "session_grant",
        payload,
    })
}

fn quic_controller_proof_signature_bytes(
    payload: &LanQuicControllerProofPayload,
) -> Result<Vec<u8>, LanProtocolError> {
    canonical_bytes(&QuicControllerProofSignatureCommitment {
        schema_version: SIGNED_LAN_PROTOCOL_VERSION,
        kind: "quic_controller_proof",
        payload,
    })
}

fn canonical_hash(value: &impl Serialize) -> Result<[u8; 32], LanProtocolError> {
    let bytes = canonical_bytes(value)?;
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(digest::digest(&digest::SHA256, &bytes).as_ref());
    Ok(hash)
}

fn canonical_bytes(value: &impl Serialize) -> Result<Vec<u8>, LanProtocolError> {
    serde_json::to_vec(value).map_err(|_| LanProtocolError::PayloadEncoding)
}

#[cfg(test)]
mod signed_protocol_tests {
    use super::*;
    use mrd_identity::{DeviceIdentity, UnattendedCredential};
    use ring::rand::SystemRandom;

    const ISSUED_AT_MS: u64 = 10_000;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::generate(&SystemRandom::new()).expect("test identity")
    }

    #[test]
    fn quic_controller_proof_requires_expected_controller_key_and_exact_challenge() {
        let controller = identity();
        let attacker = identity();
        let challenge = LanQuicControllerChallenge {
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            session_id: "proof-session".to_string(),
            grant_id: [0x31; 32],
            transport_fingerprint_sha256: [0x42; 32],
            issued_at_ms: ISSUED_AT_MS,
            expires_at_ms: ISSUED_AT_MS + 5_000,
            nonce: [0x53; 16],
        };

        let valid = SignedLanQuicControllerProof::sign(&controller, 7, challenge.clone())
            .expect("controller proof");
        valid
            .verify(ISSUED_AT_MS + 1, controller.public_key(), 7, &challenge)
            .expect("expected controller proof verifies");

        let wrong_key = SignedLanQuicControllerProof::sign(&attacker, 7, challenge.clone())
            .expect("attacker proof");
        assert_eq!(
            wrong_key.verify(ISSUED_AT_MS + 1, controller.public_key(), 7, &challenge),
            Err(LanProtocolError::PeerBindingMismatch)
        );

        let mut different_challenge = challenge.clone();
        different_challenge.nonce = [0x64; 16];
        assert_eq!(
            valid.verify(
                ISSUED_AT_MS + 1,
                controller.public_key(),
                7,
                &different_challenge,
            ),
            Err(LanProtocolError::PeerBindingMismatch)
        );
    }

    fn announcement() -> LanAnnouncement {
        LanAnnouncement {
            magic: super::super::discovery_identity::DISCOVERY_MAGIC.to_string(),
            app_id: super::super::discovery_identity::DISCOVERY_APP_ID.to_string(),
            instance_id: "signed-protocol-instance".to_string(),
            device_id: "signed-protocol-device".to_string(),
            device_name: "Signed Protocol Device".to_string(),
            device_type: "rdesk".to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            discovery_port: 21_116,
            transports: vec!["quic".to_string()],
            service_build_id: Some("signed-protocol-test".to_string()),
            media_protocol_version: Some(3),
            media_capabilities: vec!["decode.software".to_string()],
            mac_address: None,
            timestamp_ms: ISSUED_AT_MS,
        }
    }

    fn discovery_endpoint() -> SocketAddr {
        "192.168.1.50:21116".parse().expect("test endpoint")
    }

    #[test]
    fn authorization_wire_advances_the_signed_protocol_domain() {
        assert_eq!(SIGNED_LAN_PROTOCOL_VERSION, 3);
        for context in [
            ANNOUNCEMENT_SIGNATURE_CONTEXT,
            SESSION_REQUEST_SIGNATURE_CONTEXT,
            SESSION_GRANT_SIGNATURE_CONTEXT,
            SESSION_BOOTSTRAP_SIGNATURE_CONTEXT,
        ] {
            assert!(
                context.ends_with("_V3"),
                "all signed authorization domains must advance together"
            );
        }
    }

    fn session_request(controller: &DeviceIdentity, target: &DeviceIdentity) -> LanSessionRequest {
        LanSessionRequest {
            magic: super::super::discovery_identity::DISCOVERY_MAGIC.to_string(),
            app_id: super::super::discovery_identity::DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "controller-instance".to_string(),
            session_id: "authorization-session".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            source_key_id: controller.key_id().to_string(),
            source_key_epoch: 3,
            target_device_id: "target-device".to_string(),
            target_key_id: target.key_id().to_string(),
            target_key_epoch: 7,
            transport_kind: "quic".to_string(),
            source_discovery_port: None,
            source_endpoint: discovery_endpoint(),
            source_media_capabilities: vec!["decode.software".to_string()],
            requested_media_profile: None,
            access_mode: RemoteAccessMode::Unattended,
            requested_scopes: vec![
                RemotePermissionScope::ScreenView,
                RemotePermissionScope::InputPointer,
            ],
            unattended_proof: None,
            timestamp_ms: ISSUED_AT_MS,
            expires_at_ms: ISSUED_AT_MS + 5_000,
            nonce: [1; 16],
        }
    }

    #[test]
    fn signed_session_request_wire_carries_authorization_metadata() {
        let wire = serde_json::json!({
            "magic": super::super::discovery_identity::DISCOVERY_MAGIC,
            "app_id": super::super::discovery_identity::DISCOVERY_APP_ID,
            "protocol_version": SIGNED_LAN_PROTOCOL_VERSION,
            "instance_id": "controller-instance",
            "session_id": "authorization-session",
            "source_device_id": "controller-device",
            "source_device_name": "Controller",
            "source_key_id": "controller-key",
            "source_key_epoch": 3,
            "target_device_id": "target-device",
            "target_key_id": "target-key",
            "target_key_epoch": 7,
            "transport_kind": "quic",
            "source_discovery_port": null,
            "source_endpoint": "192.168.1.50:21116",
            "source_media_capabilities": ["decode.software"],
            "requested_media_profile": null,
            "access_mode": "unattended",
            "requested_scopes": ["screen.view", "input.pointer"],
            "unattended_proof": {
                "access_epoch": 11,
                "proof": [9, 8, 7, 6]
            },
            "timestamp_ms": ISSUED_AT_MS,
            "expires_at_ms": ISSUED_AT_MS + 30_000,
            "nonce": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
        });

        assert!(
            serde_json::from_value::<LanSessionRequest>(wire.clone()).is_ok(),
            "authorization metadata must be part of the strict signed request wire"
        );

        let mut missing_proof_option = wire;
        missing_proof_option
            .as_object_mut()
            .expect("request object")
            .remove("unattended_proof");
        assert!(serde_json::from_value::<LanSessionRequest>(missing_proof_option).is_err());
    }

    #[test]
    fn unattended_proof_is_transcript_bound_and_covered_by_request_signature() {
        let controller = identity();
        let target = identity();
        let credential =
            UnattendedCredential::generate(&SystemRandom::new()).expect("unattended credential");
        let mut request = session_request(&controller, &target);
        request.unattended_proof = Some(LanUnattendedProof {
            access_epoch: 11,
            proof: Vec::new(),
        });

        let transcript = unattended_transcript_bytes(&request).expect("canonical transcript");
        let proof = credential.prove(&transcript, request.nonce);
        request
            .unattended_proof
            .as_mut()
            .expect("proof metadata")
            .proof = proof;

        assert_eq!(
            unattended_transcript_bytes(&request).expect("proof-free canonical transcript"),
            transcript,
            "the proof must not recursively include itself in its transcript"
        );
        let mut rotated_epoch = request.clone();
        rotated_epoch
            .unattended_proof
            .as_mut()
            .expect("proof metadata")
            .access_epoch = 12;
        assert_ne!(
            unattended_transcript_bytes(&rotated_epoch).expect("rotated epoch transcript"),
            transcript,
            "the unattended possession proof must bind its credential epoch"
        );
        let signed = SignedLanSessionRequest::sign(&controller, request)
            .expect("controller signs unattended request");
        signed
            .verify_for_target(
                ISSUED_AT_MS,
                target.key_id(),
                signed.payload.target_key_epoch,
            )
            .expect("bound proof verifies with the signed request");

        let mut substituted = signed;
        substituted
            .payload
            .unattended_proof
            .as_mut()
            .expect("proof metadata")
            .proof[0] ^= 0xff;
        assert_eq!(
            substituted.verify_for_target(ISSUED_AT_MS, target.key_id(), 7),
            Err(LanProtocolError::CapabilityMismatch),
            "replacing the unattended proof must invalidate the signed request commitment"
        );
    }

    #[test]
    fn signed_session_request_rejects_ambiguous_authorization_metadata() {
        let controller = identity();
        let target = identity();

        let mut no_scopes = session_request(&controller, &target);
        no_scopes.requested_scopes.clear();
        assert_eq!(
            SignedLanSessionRequest::sign(&controller, no_scopes),
            Err(LanProtocolError::InvalidPayload)
        );

        let mut duplicate_scope = session_request(&controller, &target);
        duplicate_scope
            .requested_scopes
            .push(RemotePermissionScope::ScreenView);
        assert_eq!(
            SignedLanSessionRequest::sign(&controller, duplicate_scope),
            Err(LanProtocolError::InvalidPayload)
        );

        let mut attended_with_proof = session_request(&controller, &target);
        attended_with_proof.access_mode = RemoteAccessMode::Attended;
        attended_with_proof.unattended_proof = Some(LanUnattendedProof {
            access_epoch: 1,
            proof: vec![3; 32],
        });
        assert_eq!(
            SignedLanSessionRequest::sign(&controller, attended_with_proof),
            Err(LanProtocolError::InvalidPayload)
        );

        let mut malformed_unattended_proof = session_request(&controller, &target);
        malformed_unattended_proof.unattended_proof = Some(LanUnattendedProof {
            access_epoch: 0,
            proof: vec![3; 31],
        });
        assert_eq!(
            SignedLanSessionRequest::sign(&controller, malformed_unattended_proof),
            Err(LanProtocolError::InvalidPayload)
        );
    }

    #[test]
    fn signed_session_request_allows_a_bounded_consent_window() {
        let controller = identity();
        let target = identity();
        let mut request = session_request(&controller, &target);
        request.access_mode = RemoteAccessMode::Attended;
        request.expires_at_ms = request.timestamp_ms + 30_000;

        SignedLanSessionRequest::sign(&controller, request)
            .expect("consent request remains signed for its bounded authorization window");
    }

    #[test]
    fn target_signed_grant_binds_request_identity_and_scope_subset() {
        let controller = identity();
        let target = identity();
        let mut request = session_request(&controller, &target);
        request.access_mode = RemoteAccessMode::Attended;
        request.expires_at_ms = request.timestamp_ms + 30_000;
        let request = SignedLanSessionRequest::sign(&controller, request)
            .expect("controller signs attended request");
        let payload = LanSessionGrantPayload {
            session_id: request.payload.session_id.clone(),
            controller_key_id: request.payload.source_key_id.clone(),
            controller_key_epoch: request.payload.source_key_epoch,
            target_key_id: request.payload.target_key_id.clone(),
            target_key_epoch: request.payload.target_key_epoch,
            access_mode: RemoteAccessMode::Attended,
            granted_scopes: vec![RemotePermissionScope::ScreenView],
            issued_at_ms: ISSUED_AT_MS + 100,
            expires_at_ms: ISSUED_AT_MS + 60_000,
            policy_revision: 9,
            route_constraint: "quic".to_string(),
            profile_constraint: None,
            request_nonce: request.payload.nonce,
            grant_nonce: [2; 16],
            windows_session_id: Some(1),
            transport_fingerprint_sha256: [7; 32],
        };
        let grant =
            SignedLanSessionGrant::sign(&target, payload).expect("target signs exact grant");

        grant
            .verify(
                ISSUED_AT_MS + 100,
                target.public_key(),
                request.payload.target_key_epoch,
            )
            .expect("grant signature and target epoch verify independently");
        assert_eq!(
            grant.verify(ISSUED_AT_MS + 100, target.public_key(), 8),
            Err(LanProtocolError::PeerBindingMismatch)
        );
        grant
            .verify_for_request(
                ISSUED_AT_MS + 100,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            )
            .expect("grant is bound to the signed request");
        assert_eq!(
            grant.grant_id().expect("grant id"),
            grant.clone().grant_id().expect("stable grant id")
        );
        assert_ne!(grant.grant_id().expect("grant id"), [0; 32]);

        let wire = serde_json::to_value(&grant).expect("grant wire");
        for required_option in ["profile_constraint", "windows_session_id"] {
            let mut missing = wire.clone();
            missing
                .get_mut("payload")
                .and_then(serde_json::Value::as_object_mut)
                .expect("grant payload")
                .remove(required_option);
            assert!(
                serde_json::from_value::<SignedLanSessionGrant>(missing).is_err(),
                "signed grant accepted missing required option {required_option}"
            );
        }
        let mut unknown = wire;
        unknown
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .expect("grant payload")
            .insert("legacy_full_access".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<SignedLanSessionGrant>(unknown).is_err());

        let mut widened_payload = grant.payload.clone();
        widened_payload
            .granted_scopes
            .push(RemotePermissionScope::InputKeyboard);
        let widened = SignedLanSessionGrant::sign(&target, widened_payload)
            .expect("target signature remains cryptographically valid");
        assert_eq!(
            widened.verify_for_request(
                ISSUED_AT_MS + 100,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            ),
            Err(LanProtocolError::PeerBindingMismatch),
            "a signed grant must not widen the scopes in its bound request"
        );
    }

    #[test]
    fn target_signed_grant_requires_a_concrete_policy_revision() {
        let controller = identity();
        let target = identity();
        let request = session_request(&controller, &target);
        let payload = LanSessionGrantPayload {
            session_id: request.session_id,
            controller_key_id: request.source_key_id,
            controller_key_epoch: request.source_key_epoch,
            target_key_id: request.target_key_id,
            target_key_epoch: request.target_key_epoch,
            access_mode: request.access_mode,
            granted_scopes: vec![RemotePermissionScope::ScreenView],
            issued_at_ms: ISSUED_AT_MS,
            expires_at_ms: ISSUED_AT_MS + 60_000,
            policy_revision: 0,
            route_constraint: request.transport_kind,
            profile_constraint: None,
            request_nonce: request.nonce,
            grant_nonce: [2; 16],
            windows_session_id: Some(1),
            transport_fingerprint_sha256: [7; 32],
        };

        assert_eq!(
            SignedLanSessionGrant::sign(&target, payload),
            Err(LanProtocolError::InvalidPayload),
            "zero must not stand in for a bound policy revision"
        );
    }

    #[test]
    fn profile_constraint_hash_commits_to_the_full_negotiation() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let negotiation = MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: Some("display-1".to_string()),
            selected_width: Some(1920),
            selected_height: Some(1080),
            downgrade_reason: None,
        };
        let committed =
            media_profile_constraint_hash(&negotiation).expect("profile constraint hash");

        let mut substituted = negotiation;
        substituted.selected.fps = 30;
        assert_ne!(
            media_profile_constraint_hash(&substituted).expect("substituted profile hash"),
            committed,
            "the grant profile commitment must cover negotiated media values"
        );
    }

    #[test]
    fn accepted_bootstrap_verifies_embedded_grant_profile_and_transport_bindings() {
        let controller = identity();
        let target = identity();
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        };
        let negotiation = MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile.clone(),
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: Some("display-1".to_string()),
            selected_width: Some(1920),
            selected_height: Some(1080),
            downgrade_reason: None,
        };
        let mut request_payload = session_request(&controller, &target);
        request_payload.access_mode = RemoteAccessMode::Attended;
        request_payload.requested_media_profile = Some(profile);
        request_payload.expires_at_ms = request_payload.timestamp_ms + 30_000;
        let request = SignedLanSessionRequest::sign(&controller, request_payload)
            .expect("controller signs attended request");
        let cert_der = vec![1, 2, 3, 4, 5];
        let fingerprint = certificate_fingerprint_sha256(&cert_der);
        let grant = SignedLanSessionGrant::sign(
            &target,
            LanSessionGrantPayload {
                session_id: request.payload.session_id.clone(),
                controller_key_id: request.payload.source_key_id.clone(),
                controller_key_epoch: request.payload.source_key_epoch,
                target_key_id: request.payload.target_key_id.clone(),
                target_key_epoch: request.payload.target_key_epoch,
                access_mode: request.payload.access_mode,
                granted_scopes: vec![RemotePermissionScope::ScreenView],
                issued_at_ms: ISSUED_AT_MS + 100,
                expires_at_ms: ISSUED_AT_MS + 60_000,
                policy_revision: 9,
                route_constraint: "quic".to_string(),
                profile_constraint: Some(
                    media_profile_constraint_hash(&negotiation).expect("profile hash"),
                ),
                request_nonce: request.payload.nonce,
                grant_nonce: [2; 16],
                windows_session_id: Some(1),
                transport_fingerprint_sha256: fingerprint,
            },
        )
        .expect("target signs grant");
        let payload = LanSessionBootstrap {
            magic: super::super::discovery_identity::DISCOVERY_MAGIC.to_string(),
            app_id: super::super::discovery_identity::DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "target-instance".to_string(),
            session_id: request.payload.session_id.clone(),
            controller_key_id: request.payload.source_key_id.clone(),
            controller_key_epoch: request.payload.source_key_epoch,
            target_key_id: request.payload.target_key_id.clone(),
            target_key_epoch: request.payload.target_key_epoch,
            request_nonce: request.payload.nonce,
            accepted: true,
            message: None,
            failure: None,
            grant: Some(grant),
            media: Some(LanMediaBootstrap {
                transport_kind: "quic".to_string(),
                quic: Some(LanQuicBootstrap {
                    listen_addr: "127.0.0.1:21116".to_string(),
                    server_name: "localhost".to_string(),
                    certificate_fingerprint_sha256: fingerprint,
                    cert_der,
                }),
            }),
            media_profile: Some(negotiation),
            timestamp_ms: ISSUED_AT_MS + 200,
            expires_at_ms: ISSUED_AT_MS + 1_200,
            nonce: [3; 16],
        };
        let signed = SignedLanSessionBootstrap::sign(&target, payload)
            .expect("target signs authorized bootstrap");
        signed
            .verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            )
            .expect("bootstrap and embedded grant bindings verify");

        let request_wire = serde_json::to_value(&request).expect("request wire");
        let requested_profile = request_wire
            .get("payload")
            .and_then(|payload| payload.get("requested_media_profile"))
            .and_then(serde_json::Value::as_object)
            .expect("requested profile object");
        for required_option in [
            "codec_profile",
            "bit_depth",
            "chroma_subsampling",
            "pixel_format",
            "hdr_enabled",
            "color_mode",
            "color_pipeline",
        ] {
            assert!(
                requested_profile.contains_key(required_option),
                "signed request profile omitted required option {required_option}"
            );
        }

        let bootstrap_wire = serde_json::to_value(&signed).expect("bootstrap wire");
        let selected_profile = bootstrap_wire
            .get("payload")
            .and_then(|payload| payload.get("media_profile"))
            .and_then(|negotiation| negotiation.get("selected"))
            .and_then(serde_json::Value::as_object)
            .expect("selected profile object");
        assert!(
            selected_profile.contains_key("codec_profile"),
            "signed bootstrap profile must serialize absent options as explicit null"
        );
        let mut unknown_nested_profile = bootstrap_wire;
        unknown_nested_profile
            .get_mut("payload")
            .and_then(|payload| payload.get_mut("media_profile"))
            .and_then(|negotiation| negotiation.get_mut("selected"))
            .and_then(serde_json::Value::as_object_mut)
            .expect("selected profile object")
            .insert(
                "legacy_quality_override".to_string(),
                serde_json::json!(true),
            );
        assert!(
            serde_json::from_value::<SignedLanSessionBootstrap>(unknown_nested_profile).is_err(),
            "unknown nested fields must not disappear before commitment verification"
        );

        let mut late_grant_payload = signed.payload.clone();
        let mut grant_payload = late_grant_payload
            .grant
            .as_ref()
            .expect("grant")
            .payload
            .clone();
        grant_payload.issued_at_ms = late_grant_payload.timestamp_ms + 1;
        late_grant_payload.grant = Some(
            SignedLanSessionGrant::sign(&target, grant_payload).expect("target signs late grant"),
        );
        let late_grant = SignedLanSessionBootstrap::sign(&target, late_grant_payload)
            .expect("outer signature is valid");
        assert_eq!(
            late_grant.verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            ),
            Err(LanProtocolError::PeerBindingMismatch),
            "the grant must already exist when the bootstrap is issued"
        );

        let mut no_screen_view_payload = signed.payload.clone();
        let mut grant_payload = no_screen_view_payload
            .grant
            .as_ref()
            .expect("grant")
            .payload
            .clone();
        grant_payload.granted_scopes = vec![RemotePermissionScope::InputPointer];
        no_screen_view_payload.grant = Some(
            SignedLanSessionGrant::sign(&target, grant_payload)
                .expect("target signs scope-reduced grant"),
        );
        assert_eq!(
            SignedLanSessionBootstrap::sign(&target, no_screen_view_payload),
            Err(LanProtocolError::InvalidBootstrap),
            "the target must not sign an accepted media bootstrap without screen.view"
        );

        let mut substituted_request_profile = signed.payload.clone();
        substituted_request_profile
            .media_profile
            .as_mut()
            .expect("media profile")
            .requested
            .fps = 30;
        let substituted_profile_hash = media_profile_constraint_hash(
            substituted_request_profile
                .media_profile
                .as_ref()
                .expect("media profile"),
        )
        .expect("substituted profile hash");
        let mut grant_payload = substituted_request_profile
            .grant
            .as_ref()
            .expect("grant")
            .payload
            .clone();
        grant_payload.profile_constraint = Some(substituted_profile_hash);
        substituted_request_profile.grant = Some(
            SignedLanSessionGrant::sign(&target, grant_payload)
                .expect("target re-signs substituted profile grant"),
        );
        let substituted_request_profile =
            SignedLanSessionBootstrap::sign(&target, substituted_request_profile)
                .expect("outer signature is valid");
        assert_eq!(
            substituted_request_profile.verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            ),
            Err(LanProtocolError::PeerBindingMismatch),
            "the negotiation must preserve the controller-signed requested profile"
        );

        let mut profile_substitution = signed.payload.clone();
        profile_substitution
            .media_profile
            .as_mut()
            .expect("media profile")
            .selected
            .fps = 30;
        let profile_substitution = SignedLanSessionBootstrap::sign(&target, profile_substitution)
            .expect("outer signature is valid");
        assert_eq!(
            profile_substitution.verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            ),
            Err(LanProtocolError::PeerBindingMismatch)
        );

        let mut certificate_substitution = signed.payload;
        let substituted_cert = vec![9, 8, 7, 6];
        let substituted_fingerprint = certificate_fingerprint_sha256(&substituted_cert);
        let quic = certificate_substitution
            .media
            .as_mut()
            .and_then(|media| media.quic.as_mut())
            .expect("QUIC bootstrap");
        quic.cert_der = substituted_cert;
        quic.certificate_fingerprint_sha256 = substituted_fingerprint;
        let certificate_substitution =
            SignedLanSessionBootstrap::sign(&target, certificate_substitution)
                .expect("outer signature and certificate self-fingerprint are valid");
        assert_eq!(
            certificate_substitution.verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            ),
            Err(LanProtocolError::CertificateFingerprintMismatch)
        );
    }

    #[test]
    fn denied_bootstrap_carries_only_a_strict_stable_failure() {
        let controller = identity();
        let target = identity();
        let mut request_payload = session_request(&controller, &target);
        request_payload.access_mode = RemoteAccessMode::Attended;
        request_payload.expires_at_ms = request_payload.timestamp_ms + 30_000;
        let request = SignedLanSessionRequest::sign(&controller, request_payload)
            .expect("controller signs request");
        let payload = LanSessionBootstrap {
            magic: super::super::discovery_identity::DISCOVERY_MAGIC.to_string(),
            app_id: super::super::discovery_identity::DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "target-instance".to_string(),
            session_id: request.payload.session_id.clone(),
            controller_key_id: request.payload.source_key_id.clone(),
            controller_key_epoch: request.payload.source_key_epoch,
            target_key_id: request.payload.target_key_id.clone(),
            target_key_epoch: request.payload.target_key_epoch,
            request_nonce: request.payload.nonce,
            accepted: false,
            message: None,
            failure: Some(RemoteFailure {
                code: RemoteReasonCode::ConsentDenied,
                message: "local user denied the session".to_string(),
                suggested_action: None,
            }),
            grant: None,
            media: None,
            media_profile: None,
            timestamp_ms: ISSUED_AT_MS + 200,
            expires_at_ms: ISSUED_AT_MS + 1_200,
            nonce: [3; 16],
        };
        let signed = SignedLanSessionBootstrap::sign(&target, payload.clone())
            .expect("target signs stable denial");
        signed
            .verify_for_request(
                ISSUED_AT_MS + 200,
                &request,
                target.public_key(),
                request.payload.target_key_epoch,
            )
            .expect("signed denial verifies without media or grant");

        let mut conflicting_legacy_message = payload.clone();
        conflicting_legacy_message.message = Some("try legacy auto-accept".to_string());
        assert_eq!(
            SignedLanSessionBootstrap::sign(&target, conflicting_legacy_message),
            Err(LanProtocolError::InvalidBootstrap),
            "RemoteFailure must be the sole authoritative denial channel"
        );

        let mut missing_failure = payload.clone();
        missing_failure.failure = None;
        assert_eq!(
            SignedLanSessionBootstrap::sign(&target, missing_failure),
            Err(LanProtocolError::InvalidBootstrap)
        );

        let mut denial_with_media = payload;
        denial_with_media.media = Some(LanMediaBootstrap {
            transport_kind: "quic".to_string(),
            quic: None,
        });
        assert_eq!(
            SignedLanSessionBootstrap::sign(&target, denial_with_media),
            Err(LanProtocolError::InvalidBootstrap),
            "a denial must not carry any transport bootstrap"
        );

        let mut accepted_without_grant = signed.payload.clone();
        let cert_der = vec![1, 2, 3];
        accepted_without_grant.accepted = true;
        accepted_without_grant.failure = None;
        accepted_without_grant.media = Some(LanMediaBootstrap {
            transport_kind: "quic".to_string(),
            quic: Some(LanQuicBootstrap {
                listen_addr: "127.0.0.1:21116".to_string(),
                server_name: "localhost".to_string(),
                certificate_fingerprint_sha256: certificate_fingerprint_sha256(&cert_der),
                cert_der,
            }),
        });
        assert_eq!(
            SignedLanSessionBootstrap::sign(&target, accepted_without_grant),
            Err(LanProtocolError::InvalidBootstrap),
            "an accepted bootstrap must carry its signed grant"
        );

        let wire = serde_json::to_value(signed).expect("denial wire");
        let mut missing_suggested_action = wire.clone();
        missing_suggested_action
            .get_mut("payload")
            .and_then(|payload| payload.get_mut("failure"))
            .and_then(serde_json::Value::as_object_mut)
            .expect("failure object")
            .remove("suggested_action");
        assert!(
            serde_json::from_value::<SignedLanSessionBootstrap>(missing_suggested_action).is_err(),
            "stable optional failure fields remain required on the signed wire"
        );

        let mut unknown_failure_field = wire;
        unknown_failure_field
            .get_mut("payload")
            .and_then(|payload| payload.get_mut("failure"))
            .and_then(serde_json::Value::as_object_mut)
            .expect("failure object")
            .insert("fallback_to_legacy".to_string(), serde_json::json!(true));
        assert!(
            serde_json::from_value::<SignedLanSessionBootstrap>(unknown_failure_field).is_err()
        );
    }

    #[test]
    fn signed_protocol_rejects_invalid_namespace_and_zero_epoch() {
        let signer = identity();

        assert_eq!(
            SignedLanAnnouncement::sign(
                &signer,
                0,
                announcement(),
                discovery_endpoint(),
                ISSUED_AT_MS + 1_000,
                [1; 16],
            ),
            Err(LanProtocolError::InvalidKeyEpoch)
        );

        let mut wrong_namespace = announcement();
        wrong_namespace.app_id = "other-product".to_string();
        assert_eq!(
            SignedLanAnnouncement::sign(
                &signer,
                1,
                wrong_namespace,
                discovery_endpoint(),
                ISSUED_AT_MS + 1_000,
                [2; 16],
            ),
            Err(LanProtocolError::InvalidNamespace)
        );
    }

    #[test]
    fn signed_announcement_enforces_maximum_lifetime_and_clock_skew() {
        let signer = identity();

        assert_eq!(
            SignedLanAnnouncement::sign(
                &signer,
                1,
                announcement(),
                discovery_endpoint(),
                ISSUED_AT_MS + 15_001,
                [3; 16],
            ),
            Err(LanProtocolError::InvalidLifetime)
        );

        let signed = SignedLanAnnouncement::sign(
            &signer,
            1,
            announcement(),
            discovery_endpoint(),
            ISSUED_AT_MS + 15_000,
            [4; 16],
        )
        .expect("maximum valid lifetime");
        signed
            .verify(ISSUED_AT_MS + 17_000)
            .expect("two seconds of clock skew are accepted");
        assert_eq!(
            signed.verify(ISSUED_AT_MS + 17_001),
            Err(LanProtocolError::Expired)
        );
    }

    #[test]
    fn signed_announcement_wire_requires_every_authenticated_field() {
        let signer = identity();
        let signed = SignedLanAnnouncement::sign(
            &signer,
            1,
            announcement(),
            discovery_endpoint(),
            ISSUED_AT_MS + 1_000,
            [5; 16],
        )
        .expect("signed announcement");
        let wire = serde_json::to_value(signed).expect("wire value");
        let announcement = wire
            .get("payload")
            .and_then(|payload| payload.get("announcement"))
            .and_then(serde_json::Value::as_object)
            .expect("announcement object");
        let authenticated_fields = [
            "magic",
            "app_id",
            "instance_id",
            "device_id",
            "device_name",
            "device_type",
            "protocol_version",
            "discovery_port",
            "transports",
            "service_build_id",
            "media_protocol_version",
            "media_capabilities",
            "mac_address",
            "timestamp_ms",
        ];
        for field in authenticated_fields {
            assert!(
                announcement.contains_key(field),
                "signed wire omitted authenticated field {field}"
            );
            let mut missing = wire.clone();
            missing
                .get_mut("payload")
                .and_then(|payload| payload.get_mut("announcement"))
                .and_then(serde_json::Value::as_object_mut)
                .expect("announcement object")
                .remove(field);
            assert!(
                serde_json::from_value::<SignedLanAnnouncement>(missing).is_err(),
                "signed wire accepted missing authenticated field {field}"
            );
        }

        let mut missing_endpoint = wire.clone();
        missing_endpoint
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .expect("payload object")
            .remove("discovery_endpoint");
        assert!(serde_json::from_value::<SignedLanAnnouncement>(missing_endpoint).is_err());

        let mut unknown = wire;
        unknown
            .get_mut("payload")
            .and_then(|payload| payload.get_mut("announcement"))
            .and_then(serde_json::Value::as_object_mut)
            .expect("announcement object")
            .insert("future_route".to_string(), serde_json::json!("attacker"));
        assert!(serde_json::from_value::<SignedLanAnnouncement>(unknown).is_err());
    }

    #[test]
    fn legacy_ack_accepts_missing_fingerprint_but_signed_bootstrap_does_not() {
        let legacy = serde_json::json!({
            "type": "remote_session_ack",
            "magic": super::super::discovery_identity::DISCOVERY_MAGIC,
            "app_id": super::super::discovery_identity::DISCOVERY_APP_ID,
            "instance_id": "legacy-target",
            "session_id": "legacy-session",
            "accepted": true,
            "message": null,
            "media": {
                "transport_kind": "quic",
                "quic": {
                    "listen_addr": "127.0.0.1:21116",
                    "server_name": "localhost",
                    "cert_der": [1, 2, 3]
                }
            },
            "media_profile": null,
            "timestamp_ms": ISSUED_AT_MS
        });
        let parsed = serde_json::from_value::<LanDiscoveryPacket>(legacy)
            .expect("legacy ACK remains parseable");
        let LanDiscoveryPacket::RemoteSessionAck {
            media: Some(media), ..
        } = parsed
        else {
            panic!("expected legacy ACK media");
        };
        let legacy_quic = media.quic.expect("legacy QUIC");
        assert_eq!(legacy_quic.listen_addr, "127.0.0.1:21116");
        assert_eq!(legacy_quic.server_name, "localhost");
        assert_eq!(legacy_quic.cert_der, vec![1, 2, 3]);

        let signed_payload = serde_json::json!({
            "magic": super::super::discovery_identity::DISCOVERY_MAGIC,
            "app_id": super::super::discovery_identity::DISCOVERY_APP_ID,
            "protocol_version": SIGNED_LAN_PROTOCOL_VERSION,
            "instance_id": "signed-target",
            "session_id": "signed-session",
            "controller_key_id": "controller-key",
            "controller_key_epoch": 1,
            "target_key_id": "target-key",
            "target_key_epoch": 1,
            "request_nonce": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            "accepted": true,
            "message": null,
            "media": {
                "transport_kind": "quic",
                "quic": {
                    "listen_addr": "127.0.0.1:21116",
                    "server_name": "localhost",
                    "cert_der": [1, 2, 3]
                }
            },
            "media_profile": null,
            "timestamp_ms": ISSUED_AT_MS,
            "expires_at_ms": ISSUED_AT_MS + 1_000,
            "nonce": [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2]
        });
        assert!(serde_json::from_value::<LanSessionBootstrap>(signed_payload).is_err());
    }
}
