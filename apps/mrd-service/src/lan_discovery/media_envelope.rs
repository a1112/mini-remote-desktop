use super::apply_lan_media_profile_defaults;
use super::media_probe::fnv1a64;
use anyhow::{Context, Result};
use mrd_ipc::MediaProfile;

const LAN_MEDIA_ENVELOPE_MAGIC: &[u8; 8] = b"MRDMV2F1";
const LAN_MEDIA_ENVELOPE_HEADER_BYTES: usize = 48;
pub(super) const LAN_MEDIA_PAYLOAD_ACCESS_UNIT: u8 = 1;
#[cfg(test)]
pub(super) const LAN_MEDIA_PAYLOAD_H264_ACCESS_UNIT: u8 = LAN_MEDIA_PAYLOAD_ACCESS_UNIT;
pub(super) const LAN_MEDIA_PAYLOAD_PROBE_FRAME: u8 = 2;
pub(super) const LAN_MEDIA_CODEC_H264: u8 = 1;
pub(super) const LAN_MEDIA_CODEC_HEVC: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LanMediaEnvelope {
    pub(super) payload_type: u8,
    pub(super) codec: u8,
    pub(super) sequence: u64,
    pub(super) timestamp_us: u64,
    pub(super) profile: MediaProfile,
    pub(super) payload: Vec<u8>,
}

pub(super) fn encode_lan_media_envelope(envelope: LanMediaEnvelope) -> Result<Vec<u8>> {
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

pub(super) fn decode_lan_media_envelope(frame: &[u8]) -> Result<LanMediaEnvelope> {
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

pub(super) fn lan_media_codec_name(codec: u8) -> &'static str {
    match codec {
        LAN_MEDIA_CODEC_H264 => "h264",
        LAN_MEDIA_CODEC_HEVC => "hevc",
        _ => "unknown",
    }
}

pub(super) fn lan_media_profile_id(profile: &MediaProfile) -> u32 {
    let mut bytes = Vec::with_capacity(20 + profile.codec.len());
    bytes.extend_from_slice(&profile.width.to_le_bytes());
    bytes.extend_from_slice(&profile.height.to_le_bytes());
    bytes.extend_from_slice(&profile.fps.to_le_bytes());
    bytes.extend_from_slice(&profile.bitrate_mbps.to_le_bytes());
    bytes.extend_from_slice(profile.codec.as_bytes());
    bytes.push(0);
    if let Some(color_mode) = profile.color_mode.as_deref() {
        bytes.extend_from_slice(color_mode.as_bytes());
    }
    bytes.push(0);
    if let Some(color_pipeline) = profile.color_pipeline.as_deref() {
        bytes.extend_from_slice(color_pipeline.as_bytes());
    }
    fnv1a64(&bytes) as u32
}
