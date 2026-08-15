use anyhow::Result;
use mrd_ipc::MediaProfile;
use mrd_transport_quic_quinn::QuicMediaCodec;

use super::{
    normalize_lan_codec_name, LAN_MEDIA_CODEC_AV1, LAN_MEDIA_CODEC_H264, LAN_MEDIA_CODEC_HEVC,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanAccessUnitCodec {
    H264,
    Hevc,
    Av1,
}

impl LanAccessUnitCodec {
    pub(super) fn from_profile(profile: &MediaProfile) -> Self {
        match normalize_lan_codec_name(&profile.codec) {
            Some("hevc") => Self::Hevc,
            Some("av1") => Self::Av1,
            _ => Self::H264,
        }
    }

    pub(super) fn from_envelope_codec(codec: u8) -> Result<Self> {
        match codec {
            LAN_MEDIA_CODEC_H264 => Ok(Self::H264),
            LAN_MEDIA_CODEC_HEVC => Ok(Self::Hevc),
            LAN_MEDIA_CODEC_AV1 => Ok(Self::Av1),
            _ => anyhow::bail!("unsupported LAN media access unit codec: {codec}"),
        }
    }

    pub(super) fn quic_codec(self) -> QuicMediaCodec {
        match self {
            Self::H264 => QuicMediaCodec::H264,
            Self::Hevc => QuicMediaCodec::Hevc,
            Self::Av1 => QuicMediaCodec::Av1,
        }
    }

    pub(super) fn envelope_codec(self) -> u8 {
        match self {
            Self::H264 => LAN_MEDIA_CODEC_H264,
            Self::Hevc => LAN_MEDIA_CODEC_HEVC,
            Self::Av1 => LAN_MEDIA_CODEC_AV1,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }

    pub(super) fn display_name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Av1 => "AV1",
        }
    }
}

pub(super) fn h264_access_unit_is_keyframe(metadata_is_keyframe: bool, payload: &[u8]) -> bool {
    metadata_is_keyframe
        || h264_annexb_nal_types(payload)
            .into_iter()
            .any(|nal_type| nal_type == 5)
        || h264_avcc_nal_types(payload)
            .into_iter()
            .any(|nal_type| nal_type == 5)
}

pub(super) fn describe_lan_access_unit(codec: LanAccessUnitCodec, payload: &[u8]) -> String {
    match codec {
        LanAccessUnitCodec::H264 => describe_h264_access_unit(payload),
        LanAccessUnitCodec::Hevc | LanAccessUnitCodec::Av1 => describe_hevc_access_unit(payload),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_h264_idr_access_units_without_metadata() {
        let idr_annexb = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        let p_slice_annexb = [0, 0, 1, 0x41, 0x9a];
        let idr_avcc = [0, 0, 0, 3, 0x65, 0x88, 0x84];

        assert!(h264_access_unit_is_keyframe(false, &idr_annexb));
        assert!(h264_access_unit_is_keyframe(false, &idr_avcc));
        assert!(!h264_access_unit_is_keyframe(false, &p_slice_annexb));
        assert!(h264_access_unit_is_keyframe(true, &p_slice_annexb));
    }

    #[test]
    fn describes_h264_payload_nal_layouts() {
        let idr_annexb = [0, 0, 0, 1, 0x65, 0x88, 0x84];
        let description = describe_lan_access_unit(LanAccessUnitCodec::H264, &idr_annexb);

        assert!(description.contains("payload_bytes=7"));
        assert!(description.contains("prefix_hex=[00 00 00 01 65 88 84]"));
        assert!(description.contains("annexb_nals=[5]"));
        assert!(description.contains("avcc_nals=[]"));
    }

    #[test]
    fn describes_hevc_payload_without_h264_probe_fields() {
        let hevc_payload = [0, 0, 0, 1, 0x26, 0x01, 0xaa, 0xbb];
        let description = describe_lan_access_unit(LanAccessUnitCodec::Hevc, &hevc_payload);

        assert_eq!(
            description,
            "payload_bytes=8, prefix_hex=[00 00 00 01 26 01 aa bb]"
        );
    }

    #[test]
    fn maps_av1_profiles_to_av1_transport_codec() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 24,
            codec: "AV1".to_string(),
            ..MediaProfile::default()
        };

        let codec = LanAccessUnitCodec::from_profile(&profile);

        assert_eq!(codec, LanAccessUnitCodec::Av1);
        assert_eq!(codec.quic_codec(), QuicMediaCodec::Av1);
        assert_eq!(codec.name(), "av1");
        assert_eq!(codec.display_name(), "AV1");
    }
}
