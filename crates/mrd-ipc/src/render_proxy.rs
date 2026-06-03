#![allow(missing_docs)]

const FRAME_MAGIC: &[u8; 4] = b"MRDR";
const ACK_MAGIC: &[u8; 4] = b"MRDA";
pub const FRAME_HEADER_LEN: usize = 40;
pub const ACK_LEN: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderProxyPixelFormat {
    Rgb24,
    Bgra32,
    Nv12,
    H264,
    Hevc,
}

impl RenderProxyPixelFormat {
    pub fn code(self) -> u8 {
        match self {
            Self::Rgb24 => 1,
            Self::Bgra32 => 2,
            Self::Nv12 => 3,
            Self::H264 => 4,
            Self::Hevc => 5,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Rgb24),
            2 => Some(Self::Bgra32),
            3 => Some(Self::Nv12),
            4 => Some(Self::H264),
            5 => Some(Self::Hevc),
            _ => None,
        }
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb24 => 3,
            Self::Bgra32 => 4,
            Self::Nv12 | Self::H264 | Self::Hevc => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderProxyFrameHeader {
    pub pixel_format: RenderProxyPixelFormat,
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub payload_len: u32,
    pub row_pitch: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderProxyAck {
    pub presented_frames: u64,
    pub present_skips: u64,
    pub queue_replacements: u64,
    pub upload_duration_ms: f64,
    pub decode_duration_ms: f64,
    pub draw_present_duration_ms: f64,
    pub max_drawable_count: Option<u32>,
    pub display_sync_enabled: Option<bool>,
    pub next_drawable_duration_ms: f64,
    pub encode_commit_duration_ms: f64,
}

pub fn encode_frame_header(header: &RenderProxyFrameHeader) -> [u8; FRAME_HEADER_LEN] {
    let mut bytes = [0_u8; FRAME_HEADER_LEN];
    bytes[0..4].copy_from_slice(FRAME_MAGIC);
    bytes[4] = 1;
    bytes[5] = header.pixel_format.code();
    bytes[8..12].copy_from_slice(&header.width.to_le_bytes());
    bytes[12..16].copy_from_slice(&header.height.to_le_bytes());
    bytes[16..24].copy_from_slice(&header.sequence.to_le_bytes());
    bytes[24..32].copy_from_slice(&header.timestamp_us.to_le_bytes());
    bytes[32..36].copy_from_slice(&header.payload_len.to_le_bytes());
    bytes[36..40].copy_from_slice(&header.row_pitch.to_le_bytes());
    bytes
}

pub fn decode_frame_header(
    bytes: &[u8; FRAME_HEADER_LEN],
) -> Result<RenderProxyFrameHeader, String> {
    if &bytes[0..4] != FRAME_MAGIC {
        return Err("render proxy frame header has invalid magic".to_string());
    }
    if bytes[4] != 1 {
        return Err(format!(
            "render proxy frame header has unsupported version {}",
            bytes[4]
        ));
    }
    let pixel_format = RenderProxyPixelFormat::from_code(bytes[5]).ok_or_else(|| {
        format!(
            "render proxy frame header has unsupported pixel format {}",
            bytes[5]
        )
    })?;
    let width = u32::from_le_bytes(bytes[8..12].try_into().expect("width bytes"));
    let height = u32::from_le_bytes(bytes[12..16].try_into().expect("height bytes"));
    let sequence = u64::from_le_bytes(bytes[16..24].try_into().expect("sequence bytes"));
    let timestamp_us = u64::from_le_bytes(bytes[24..32].try_into().expect("timestamp bytes"));
    let payload_len = u32::from_le_bytes(bytes[32..36].try_into().expect("payload len bytes"));
    let row_pitch = u32::from_le_bytes(bytes[36..40].try_into().expect("row pitch bytes"));
    Ok(RenderProxyFrameHeader {
        pixel_format,
        width,
        height,
        sequence,
        timestamp_us,
        payload_len,
        row_pitch,
    })
}

pub fn encode_ack(ack: &RenderProxyAck) -> [u8; ACK_LEN] {
    let mut bytes = [0_u8; ACK_LEN];
    bytes[0..4].copy_from_slice(ACK_MAGIC);
    bytes[4] = 5;
    bytes[8..16].copy_from_slice(&ack.presented_frames.to_le_bytes());
    bytes[16..24].copy_from_slice(&ack.present_skips.to_le_bytes());
    bytes[24..32].copy_from_slice(&ack.queue_replacements.to_le_bytes());
    bytes[32..40].copy_from_slice(&ack.upload_duration_ms.to_le_bytes());
    bytes[40..48].copy_from_slice(&ack.decode_duration_ms.to_le_bytes());
    bytes[48..56].copy_from_slice(&ack.draw_present_duration_ms.to_le_bytes());
    bytes[56..60].copy_from_slice(&ack.max_drawable_count.unwrap_or(0).to_le_bytes());
    let display_sync_state = match ack.display_sync_enabled {
        None => 0_u32,
        Some(false) => 1,
        Some(true) => 2,
    };
    bytes[60..64].copy_from_slice(&display_sync_state.to_le_bytes());
    bytes[64..72].copy_from_slice(&ack.next_drawable_duration_ms.to_le_bytes());
    bytes[72..80].copy_from_slice(&ack.encode_commit_duration_ms.to_le_bytes());
    bytes
}

pub fn decode_ack(bytes: &[u8; ACK_LEN]) -> Result<RenderProxyAck, String> {
    if &bytes[0..4] != ACK_MAGIC {
        return Err("render proxy ack has invalid magic".to_string());
    }
    if bytes[4] != 5 {
        return Err(format!(
            "render proxy ack has unsupported version {}",
            bytes[4]
        ));
    }
    let max_drawable_count =
        u32::from_le_bytes(bytes[56..60].try_into().expect("max drawable bytes"));
    let display_sync_state =
        u32::from_le_bytes(bytes[60..64].try_into().expect("display sync bytes"));
    Ok(RenderProxyAck {
        presented_frames: u64::from_le_bytes(bytes[8..16].try_into().expect("present bytes")),
        present_skips: u64::from_le_bytes(bytes[16..24].try_into().expect("skip bytes")),
        queue_replacements: u64::from_le_bytes(
            bytes[24..32].try_into().expect("replacement bytes"),
        ),
        upload_duration_ms: f64::from_le_bytes(bytes[32..40].try_into().expect("upload bytes")),
        decode_duration_ms: f64::from_le_bytes(bytes[40..48].try_into().expect("decode bytes")),
        draw_present_duration_ms: f64::from_le_bytes(
            bytes[48..56].try_into().expect("draw present bytes"),
        ),
        max_drawable_count: (max_drawable_count > 0).then_some(max_drawable_count),
        display_sync_enabled: match display_sync_state {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        },
        next_drawable_duration_ms: f64::from_le_bytes(
            bytes[64..72].try_into().expect("next drawable bytes"),
        ),
        encode_commit_duration_ms: f64::from_le_bytes(
            bytes[72..80].try_into().expect("encode commit bytes"),
        ),
    })
}

pub fn expected_payload_len(
    pixel_format: RenderProxyPixelFormat,
    width: u32,
    height: u32,
    row_pitch: u32,
) -> Option<usize> {
    if pixel_format == RenderProxyPixelFormat::Nv12 {
        let pitch = row_pitch.max(width) as usize;
        return pitch
            .checked_mul(height as usize)?
            .checked_add(pitch.checked_mul((height as usize).div_ceil(2))?);
    }
    if matches!(
        pixel_format,
        RenderProxyPixelFormat::H264 | RenderProxyPixelFormat::Hevc
    ) {
        return None;
    }

    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(pixel_format.bytes_per_pixel())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_proxy_frame_header_round_trips() {
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::Bgra32,
            width: 1280,
            height: 720,
            sequence: 42,
            timestamp_us: 123,
            payload_len: 1280 * 720 * 4,
            row_pitch: 0,
        };

        let encoded = encode_frame_header(&header);

        assert_eq!(decode_frame_header(&encoded).unwrap(), header);
    }

    #[test]
    fn render_proxy_ack_round_trips() {
        let ack = RenderProxyAck {
            presented_frames: 1,
            present_skips: 0,
            queue_replacements: 3,
            upload_duration_ms: 0.42,
            decode_duration_ms: 0.11,
            draw_present_duration_ms: 0.31,
            max_drawable_count: Some(2),
            display_sync_enabled: Some(false),
            next_drawable_duration_ms: 0.21,
            encode_commit_duration_ms: 0.12,
        };

        let encoded = encode_ack(&ack);
        let decoded = decode_ack(&encoded).unwrap();

        assert_eq!(decoded.presented_frames, 1);
        assert_eq!(decoded.present_skips, 0);
        assert_eq!(decoded.queue_replacements, 3);
        assert_eq!(decoded.upload_duration_ms, 0.42);
        assert_eq!(decoded.decode_duration_ms, 0.11);
        assert_eq!(decoded.draw_present_duration_ms, 0.31);
        assert_eq!(decoded.max_drawable_count, Some(2));
        assert_eq!(decoded.display_sync_enabled, Some(false));
        assert_eq!(decoded.next_drawable_duration_ms, 0.21);
        assert_eq!(decoded.encode_commit_duration_ms, 0.12);
    }

    #[test]
    fn render_proxy_h264_header_round_trips_without_fixed_payload_size() {
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::H264,
            width: 1920,
            height: 1080,
            sequence: 7,
            timestamp_us: 123_456,
            payload_len: 4096,
            row_pitch: 0,
        };

        let encoded = encode_frame_header(&header);
        let decoded = decode_frame_header(&encoded).unwrap();

        assert_eq!(decoded, header);
        assert_eq!(
            expected_payload_len(RenderProxyPixelFormat::H264, 1920, 1080, 0),
            None
        );
    }

    #[test]
    fn render_proxy_hevc_header_round_trips_without_fixed_payload_size() {
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::Hevc,
            width: 2560,
            height: 1440,
            sequence: 8,
            timestamp_us: 456_789,
            payload_len: 8192,
            row_pitch: 0,
        };

        let encoded = encode_frame_header(&header);
        let decoded = decode_frame_header(&encoded).unwrap();

        assert_eq!(decoded, header);
        assert_eq!(
            expected_payload_len(RenderProxyPixelFormat::Hevc, 2560, 1440, 0),
            None
        );
    }
}
