use super::discovery_identity::default_app_id;
use super::discovery_identity::{DISCOVERY_APP_ID, DISCOVERY_MAGIC};
use mrd_identity::{public_key_id, verify_context_bytes, DeviceIdentity};
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, ControlInputEvent, ControlInputLane, DisplayMode,
    DisplayModeChange, MediaProfile, MediaProfileNegotiation, RemoteDevicePowerAction,
};
use mrd_transport_quic_quinn::certificate_fingerprint_sha256;
use ring::digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, net::SocketAddr};

#[cfg(test)]
pub(super) const PROTOCOL_VERSION: u32 = 1;
pub const SIGNED_LAN_PROTOCOL_VERSION: u32 = 2;
pub(super) const DISCOVERY_PACKET_BUFFER_BYTES: usize = 65_535;
pub(super) const DISCOVERY_SAFE_UDP_PAYLOAD_BYTES: usize = 60_000;

const ANNOUNCEMENT_SIGNATURE_CONTEXT: &str = "MRD_LAN_ANNOUNCEMENT_V2";
const SESSION_REQUEST_SIGNATURE_CONTEXT: &str = "MRD_LAN_SESSION_REQUEST_V2";
const SESSION_BOOTSTRAP_SIGNATURE_CONTEXT: &str = "MRD_LAN_SESSION_BOOTSTRAP_V2";
const ANNOUNCEMENT_MAX_LIFETIME_MS: u64 = 15_000;
const SESSION_MAX_LIFETIME_MS: u64 = 5_000;
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
pub(super) const LAN_INPUT_CONTROL_TRANSPORT: &str = "input_control_v1";
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanAnnouncement {
    pub payload: SignedLanAnnouncementPayload,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    #[serde(deserialize_with = "deserialize_required_option")]
    pub source_discovery_port: Option<u16>,
    pub source_endpoint: SocketAddr,
    pub source_media_capabilities: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub requested_media_profile: Option<MediaProfile>,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedLanSessionRequest {
    pub payload: LanSessionRequest,
    pub capability_hash: [u8; 32],
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    pub media: Option<LanMediaBootstrap>,
    pub media_profile: Option<MediaProfileNegotiation>,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: [u8; 16],
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
    media: Option<AuthenticatedLanMediaBootstrap>,
    #[serde(deserialize_with = "deserialize_required_option")]
    media_profile: Option<MediaProfileNegotiation>,
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
            media: wire.media.map(Into::into),
            media_profile: wire.media_profile,
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

impl SignedLanSessionBootstrap {
    pub fn sign(
        identity: &DeviceIdentity,
        payload: LanSessionBootstrap,
    ) -> Result<Self, LanProtocolError> {
        validate_session_bootstrap(&payload)?;
        if payload.target_key_id != identity.key_id() {
            return Err(LanProtocolError::InvalidKeyBinding);
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
            if media.transport_kind != request.payload.transport_kind {
                return Err(LanProtocolError::PeerBindingMismatch);
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
        SESSION_MAX_LIFETIME_MS,
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
    validate_nonce(&payload.nonce)
}

fn validate_session_bootstrap(payload: &LanSessionBootstrap) -> Result<(), LanProtocolError> {
    validate_session_bootstrap_structure(payload)?;
    validate_lifetime(
        payload.timestamp_ms,
        payload.expires_at_ms,
        SESSION_MAX_LIFETIME_MS,
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
    match (payload.accepted, payload.media.as_ref()) {
        (true, Some(media)) => validate_media_bootstrap(media)?,
        (true, None) | (false, Some(_)) => return Err(LanProtocolError::InvalidBootstrap),
        (false, None) => {
            if payload.media_profile.is_some() {
                return Err(LanProtocolError::InvalidBootstrap);
            }
        }
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
struct SessionBootstrapCommitment<'a> {
    schema_version: u32,
    kind: &'static str,
    bootstrap: &'a LanSessionBootstrap,
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
    use mrd_identity::DeviceIdentity;
    use ring::rand::SystemRandom;

    const ISSUED_AT_MS: u64 = 10_000;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::generate(&SystemRandom::new()).expect("test identity")
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
