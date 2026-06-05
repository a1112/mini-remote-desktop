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
