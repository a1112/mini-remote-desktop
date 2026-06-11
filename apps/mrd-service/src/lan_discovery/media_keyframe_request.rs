use super::media_envelope::lan_media_profile_id;
use super::now_us;
use anyhow::{Context, Result};
use bytes::Bytes;
use mrd_ipc::MediaProfile;
use mrd_transport_quic_quinn::{
    fragment_media_payload_v3, is_quic_media_v3_datagram, QuicMediaCodec, QuicMediaFragment,
    QuicMediaPayloadType, QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};

const LAN_MEDIA_CONTROL_REQUEST_KEYFRAME: &[u8] = b"request_keyframe";

pub(super) fn encode_lan_keyframe_request_datagram(
    profile: &MediaProfile,
    sequence: u32,
    max_datagram_size: usize,
) -> Result<Bytes> {
    let fragments = fragment_media_payload_v3(
        QuicMediaPayloadType::Control,
        QuicMediaCodec::None,
        lan_media_profile_id(profile),
        sequence,
        now_us(),
        false,
        LAN_MEDIA_CONTROL_REQUEST_KEYFRAME,
        max_datagram_size.max(QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN + 1),
    )
    .context("failed to encode LAN keyframe request control datagram")?;
    fragments
        .into_iter()
        .next()
        .context("LAN keyframe request control datagram encoder produced no fragments")
}

pub(super) fn decode_lan_keyframe_request_datagram(datagram: &[u8]) -> Result<bool> {
    if !is_quic_media_v3_datagram(datagram) {
        return Ok(false);
    }
    let fragment = QuicMediaFragment::decode(datagram)
        .context("failed to decode LAN media control datagram")?;
    Ok(fragment.payload_type == QuicMediaPayloadType::Control
        && fragment.codec == QuicMediaCodec::None
        && fragment.fragment_index == 0
        && fragment.fragment_count == 1
        && fragment.payload.as_ref() == LAN_MEDIA_CONTROL_REQUEST_KEYFRAME)
}
