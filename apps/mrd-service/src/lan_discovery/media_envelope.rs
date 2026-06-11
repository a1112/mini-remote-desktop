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
pub(super) const LAN_MEDIA_CODEC_AV1: u8 = 3;

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
    let profile_metadata = profile_metadata_bytes(&envelope.profile, envelope.codec)?;
    let profile_metadata_len = u16::try_from(profile_metadata.len())
        .context("LAN media v2 envelope profile metadata exceeds u16 length")?;
    let mut frame = Vec::with_capacity(
        LAN_MEDIA_ENVELOPE_HEADER_BYTES + profile_metadata.len() + envelope.payload.len(),
    );
    frame.extend_from_slice(LAN_MEDIA_ENVELOPE_MAGIC);
    frame.push(envelope.payload_type);
    frame.push(envelope.codec);
    frame.extend_from_slice(&profile_metadata_len.to_le_bytes());
    frame.extend_from_slice(&envelope.sequence.to_le_bytes());
    frame.extend_from_slice(&envelope.timestamp_us.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.width.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.height.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.fps.to_le_bytes());
    frame.extend_from_slice(&envelope.profile.bitrate_mbps.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&profile_metadata);
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
    let profile_metadata_len = u16::from_le_bytes(frame[10..12].try_into().unwrap()) as usize;
    let sequence = u64::from_le_bytes(frame[12..20].try_into().unwrap());
    let timestamp_us = u64::from_le_bytes(frame[20..28].try_into().unwrap());
    let width = u32::from_le_bytes(frame[28..32].try_into().unwrap());
    let height = u32::from_le_bytes(frame[32..36].try_into().unwrap());
    let fps = u32::from_le_bytes(frame[36..40].try_into().unwrap());
    let bitrate_mbps = u32::from_le_bytes(frame[40..44].try_into().unwrap());
    let payload_len = u32::from_le_bytes(frame[44..48].try_into().unwrap()) as usize;
    let Some(metadata_end) = LAN_MEDIA_ENVELOPE_HEADER_BYTES.checked_add(profile_metadata_len)
    else {
        anyhow::bail!("LAN media v2 envelope profile metadata length overflow");
    };
    let Some(expected_len) = metadata_end.checked_add(payload_len) else {
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
    let base_profile = lan_media_profile_from_envelope(width, height, fps, bitrate_mbps, codec);
    let profile = if profile_metadata_len == 0 {
        base_profile
    } else {
        decode_profile_metadata(
            &frame[LAN_MEDIA_ENVELOPE_HEADER_BYTES..metadata_end],
            base_profile,
        )?
    };
    Ok(LanMediaEnvelope {
        payload_type,
        codec,
        sequence,
        timestamp_us,
        profile,
        payload: frame[metadata_end..].to_vec(),
    })
}

fn profile_metadata_bytes(profile: &MediaProfile, codec: u8) -> Result<Vec<u8>> {
    let envelope_default = lan_media_profile_from_envelope(
        profile.width,
        profile.height,
        profile.fps,
        profile.bitrate_mbps,
        codec,
    );
    if &envelope_default == profile {
        Ok(Vec::new())
    } else {
        serde_json::to_vec(profile).context("failed to encode LAN media v2 profile metadata")
    }
}

fn decode_profile_metadata(metadata: &[u8], base_profile: MediaProfile) -> Result<MediaProfile> {
    let profile: MediaProfile = serde_json::from_slice(metadata)
        .context("failed to decode LAN media v2 profile metadata")?;
    if profile.width != base_profile.width
        || profile.height != base_profile.height
        || profile.fps != base_profile.fps
        || profile.bitrate_mbps != base_profile.bitrate_mbps
        || !profile.codec.eq_ignore_ascii_case(&base_profile.codec)
    {
        anyhow::bail!("LAN media v2 profile metadata does not match envelope header");
    }
    Ok(profile)
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
        LAN_MEDIA_CODEC_AV1 => "av1",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn av1_envelope_round_trips_codec_profile_name() {
        let encoded = encode_lan_media_envelope(LanMediaEnvelope {
            payload_type: LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
            codec: LAN_MEDIA_CODEC_AV1,
            sequence: 7,
            timestamp_us: 99,
            profile: MediaProfile {
                width: 1920,
                height: 1080,
                fps: 144,
                bitrate_mbps: 24,
                codec: "av1".to_string(),
                ..MediaProfile::default()
            },
            payload: vec![1, 2, 3],
        })
        .expect("encode envelope");

        let decoded = decode_lan_media_envelope(&encoded).expect("decode envelope");

        assert_eq!(decoded.codec, LAN_MEDIA_CODEC_AV1);
        assert_eq!(decoded.profile.codec, "av1");
        assert_eq!(lan_media_codec_name(LAN_MEDIA_CODEC_AV1), "av1");
    }

    #[test]
    fn envelope_round_trips_extended_media_profile_fields() {
        let profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            codec_profile: Some("main10".to_string()),
            bit_depth: Some(10),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("p010".to_string()),
            hdr_enabled: Some(true),
            color_mode: Some("grayscale".to_string()),
            color_pipeline: Some("hdr_main10".to_string()),
        };

        let encoded = encode_lan_media_envelope(LanMediaEnvelope {
            payload_type: LAN_MEDIA_PAYLOAD_ACCESS_UNIT,
            codec: LAN_MEDIA_CODEC_HEVC,
            sequence: 11,
            timestamp_us: 22,
            profile: profile.clone(),
            payload: b"main10-hevc".to_vec(),
        })
        .expect("encode envelope");

        let decoded = decode_lan_media_envelope(&encoded).expect("decode envelope");

        assert_eq!(decoded.profile, profile);
        assert_eq!(decoded.payload, b"main10-hevc");
    }
}
