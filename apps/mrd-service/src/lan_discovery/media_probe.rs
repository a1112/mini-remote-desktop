use super::normalize_lan_codec_name;
#[cfg(test)]
use super::LanAccessUnitCodec;
use crate::app_state::MediaProbeFrameStats;
use anyhow::Result;
use mrd_ipc::MediaProfile;

const LAN_MEDIA_PROBE_MAGIC: &[u8; 8] = b"MRDMPF01";
const LAN_MEDIA_PROBE_HEADER_BYTES: usize = 56;
const LAN_MEDIA_PROBE_H264_FORMAT: &str = "compressed_h264_test_pattern";
const LAN_MEDIA_PROBE_HEVC_FORMAT: &str = "compressed_hevc_test_pattern";
const LAN_MEDIA_PROBE_H264_FORMAT_CODE: u32 = 2;
const LAN_MEDIA_PROBE_HEVC_FORMAT_CODE: u32 = 3;

#[cfg(test)]
pub(super) fn build_media_probe_frame(
    sequence: u64,
    timestamp_us: u64,
    profile: &MediaProfile,
) -> Vec<u8> {
    let media_payload = build_probe_compressed_pattern(sequence, profile);
    let payload_hash = fnv1a64(&media_payload);
    let format_code = media_probe_format_code_for_profile(profile);
    let mut frame = Vec::with_capacity(LAN_MEDIA_PROBE_HEADER_BYTES + media_payload.len());
    frame.extend_from_slice(LAN_MEDIA_PROBE_MAGIC);
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(&timestamp_us.to_le_bytes());
    frame.extend_from_slice(&profile.width.to_le_bytes());
    frame.extend_from_slice(&profile.height.to_le_bytes());
    frame.extend_from_slice(&format_code.to_le_bytes());
    frame.extend_from_slice(&(media_payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload_hash.to_le_bytes());
    frame.extend_from_slice(&profile.fps.to_le_bytes());
    frame.extend_from_slice(&profile.bitrate_mbps.to_le_bytes());
    frame.extend_from_slice(&media_payload);
    frame
}

pub(super) fn decode_media_probe_frame(frame: &[u8]) -> Result<MediaProbeFrameStats> {
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
    let format = media_probe_format(format_code)?;

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
        format: format.to_string(),
        payload_hash: format!("fnv1a64:{actual_hash:016x}"),
    })
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

#[cfg(test)]
pub(super) fn media_payload_bytes(profile: &MediaProfile) -> usize {
    ((profile.bitrate_mbps as usize * 1_000_000 / 8) / profile.fps.max(1) as usize).max(1)
}

#[cfg(test)]
fn media_probe_format_code_for_profile(profile: &MediaProfile) -> u32 {
    if LanAccessUnitCodec::from_profile(profile) == LanAccessUnitCodec::Hevc {
        LAN_MEDIA_PROBE_HEVC_FORMAT_CODE
    } else {
        LAN_MEDIA_PROBE_H264_FORMAT_CODE
    }
}

fn media_probe_format(format_code: u32) -> Result<&'static str> {
    match format_code {
        LAN_MEDIA_PROBE_H264_FORMAT_CODE => Ok(LAN_MEDIA_PROBE_H264_FORMAT),
        LAN_MEDIA_PROBE_HEVC_FORMAT_CODE => Ok(LAN_MEDIA_PROBE_HEVC_FORMAT),
        _ => anyhow::bail!("unsupported media probe format code: {format_code}"),
    }
}

pub(super) fn decoded_video_probe_format(codec: &str) -> String {
    if normalize_lan_codec_name(codec) == Some("hevc") {
        "hevc_desktop_frame".to_string()
    } else if codec.trim().eq_ignore_ascii_case("av1") {
        "av1_desktop_frame".to_string()
    } else {
        "h264_desktop_frame".to_string()
    }
}

const FNV1A64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(super) fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_extend(FNV1A64_OFFSET_BASIS, bytes)
}

fn fnv1a64_extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}

pub(super) fn fnv1a64_media_metadata(
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload_len: usize,
) -> u64 {
    let mut hash = FNV1A64_OFFSET_BASIS;
    hash = fnv1a64_extend(hash, &profile.width.to_le_bytes());
    hash = fnv1a64_extend(hash, &profile.height.to_le_bytes());
    hash = fnv1a64_extend(hash, &profile.fps.to_le_bytes());
    hash = fnv1a64_extend(hash, &profile.bitrate_mbps.to_le_bytes());
    hash = fnv1a64_extend(hash, profile.codec.as_bytes());
    hash = fnv1a64_extend(hash, &[0]);
    if let Some(color_mode) = profile.color_mode.as_deref() {
        hash = fnv1a64_extend(hash, color_mode.as_bytes());
    }
    hash = fnv1a64_extend(hash, &[0]);
    if let Some(color_pipeline) = profile.color_pipeline.as_deref() {
        hash = fnv1a64_extend(hash, color_pipeline.as_bytes());
    }
    hash = fnv1a64_extend(hash, &sequence.to_le_bytes());
    hash = fnv1a64_extend(hash, &timestamp_us.to_le_bytes());
    hash = fnv1a64_extend(hash, &(encoded_payload_len as u64).to_le_bytes());
    hash
}
