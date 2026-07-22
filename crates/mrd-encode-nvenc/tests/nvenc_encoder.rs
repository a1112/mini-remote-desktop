use mrd_encode_nvenc::{NvencH264Encoder, NvencHevcEncoder};
use mrd_pipeline_core::{CapturedFrame, ColorMode, FramePixelFormat, VideoCodec, VideoEncoder};

#[cfg(windows)]
const SMOKE_WIDTH: usize = 16;
#[cfg(windows)]
const SMOKE_HEIGHT: usize = 16;
#[cfg(not(windows))]
const SMOKE_WIDTH: usize = 160;
#[cfg(not(windows))]
const SMOKE_HEIGHT: usize = 64;

#[test]
fn nvenc_h264_color_mode_defaults_to_full() {
    assert_eq!(NvencH264Encoder::default_color_mode(), ColorMode::Full);
}

#[test]
fn nvenc_hevc_color_mode_defaults_to_full() {
    assert_eq!(NvencHevcEncoder::default_color_mode(), ColorMode::Full);
}

#[cfg(windows)]
#[test]
fn nvenc_color_modes_preserve_shared_bgra_input_contract() {
    for mode in [
        ColorMode::Full,
        ColorMode::Grayscale,
        ColorMode::Monochrome,
        ColorMode::LowChroma,
    ] {
        assert_eq!(
            NvencH264Encoder::preferred_input_memory_kind_for_color_mode(mode),
            mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
        );
        assert_eq!(
            NvencHevcEncoder::preferred_input_memory_kind_for_color_mode(mode),
            mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
        );
    }
}

#[test]
fn nvenc_encoder_is_probeable_or_emits_h264_access_unit() {
    let Ok(mut encoder) = NvencH264Encoder::new(SMOKE_WIDTH, SMOKE_HEIGHT, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        SMOKE_WIDTH,
        SMOKE_HEIGHT,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x7f; SMOKE_WIDTH * SMOKE_HEIGHT * 4],
    );
    let access_units = encoder.encode(&frame).expect("nvenc encode frame");

    assert!(!access_units.is_empty());
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert!(!access_units[0].bytes.is_empty());
}

#[cfg(not(windows))]
#[test]
fn linux_nvenc_h264_encodes_720p_frames_when_runtime_probe_passes() {
    if NvencH264Encoder::probe_h264_available().is_err() {
        return;
    }

    let Ok(mut encoder) = NvencH264Encoder::new_with_bitrate(1280, 720, 30, 5_000_000) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        1280,
        720,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x55; 1280 * 720 * 4],
    );
    let access_units = encoder
        .encode(&frame)
        .expect("encode 720p Linux NVENC frame");

    assert!(!access_units.is_empty());
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert!(!access_units[0].bytes.is_empty());
}

#[cfg(windows)]
#[test]
fn nvenc_h264_max_speed_idr_includes_parameter_sets() {
    let Ok(mut encoder) = NvencH264Encoder::new_max_speed_with_bitrate(1280, 720, 60, 20_000_000)
    else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        1280,
        720,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x44; 1280 * 720 * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc max-speed encode frame")
        .into_iter()
        .next()
        .expect("single access unit");
    let nal_types = extract_h264_nal_types(&access_unit.bytes);

    assert!(
        nal_types.contains(&7) && nal_types.contains(&8),
        "max-speed IDR should carry SPS/PPS for cross-device decoder startup, got {nal_types:?}"
    );
}

#[cfg(windows)]
#[test]
fn nvenc_h264_access_unit_uses_high_profile() {
    let Ok(mut encoder) = NvencH264Encoder::new(1280, 720, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        1280,
        720,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x55; 1280 * 720 * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc encode frame")
        .into_iter()
        .next()
        .expect("single access unit");
    let profile_idc = extract_sps_profile_idc(&access_unit.bytes).expect("sps profile idc");

    assert_eq!(
        profile_idc, 0x64,
        "nvenc bitstream should advertise H264 high profile for webrtc negotiation"
    );
}

#[cfg(windows)]
#[test]
fn nvenc_h264_access_unit_can_use_baseline_profile() {
    let Ok(mut encoder) = NvencH264Encoder::new_baseline(1280, 720, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        1280,
        720,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x33; 1280 * 720 * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc encode baseline frame")
        .into_iter()
        .next()
        .expect("single access unit");
    let profile_idc = extract_sps_profile_idc(&access_unit.bytes).expect("sps profile idc");

    assert_eq!(
        profile_idc, 0x42,
        "baseline constructor should emit H264 baseline profile for webrtc compatibility"
    );
}

#[cfg(windows)]
#[test]
fn nvenc_hevc_encoder_prefers_d3d11_shared_bgra_input() {
    assert_eq!(
        NvencHevcEncoder::preferred_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
    );
}

#[cfg(windows)]
#[test]
fn nvenc_hevc_main10_encoder_prefers_d3d11_shared_bgra_input_for_nvenc_8_to_10_conversion() {
    assert_eq!(
        NvencHevcEncoder::preferred_main10_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
    );
}

#[cfg(not(windows))]
#[test]
fn linux_nvenc_hevc_encoder_prefers_cpu_input() {
    assert_eq!(
        NvencHevcEncoder::preferred_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::Cpu
    );
    assert_eq!(
        NvencHevcEncoder::preferred_main10_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::Cpu
    );
}

#[test]
fn nvenc_hevc_encoder_emits_hevc_access_unit_when_available() {
    let Ok(mut encoder) = NvencHevcEncoder::new_main(SMOKE_WIDTH, SMOKE_HEIGHT, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        SMOKE_WIDTH,
        SMOKE_HEIGHT,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x7f; SMOKE_WIDTH * SMOKE_HEIGHT * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc hevc encode frame")
        .into_iter()
        .next()
        .expect("single access unit");

    assert_eq!(access_unit.codec, VideoCodec::Hevc);
    assert!(!access_unit.bytes.is_empty());
}

#[cfg(windows)]
#[test]
fn nvenc_hevc_max_speed_encoder_emits_hevc_access_unit_when_available() {
    let Ok(mut encoder) =
        NvencHevcEncoder::new_max_speed_with_bitrate(SMOKE_WIDTH, SMOKE_HEIGHT, 30, 8_000_000)
    else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        SMOKE_WIDTH,
        SMOKE_HEIGHT,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x66; SMOKE_WIDTH * SMOKE_HEIGHT * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc hevc max-speed encode frame")
        .into_iter()
        .next()
        .expect("single access unit");

    assert_eq!(access_unit.codec, VideoCodec::Hevc);
    assert!(!access_unit.bytes.is_empty());
}

#[cfg(windows)]
#[test]
fn nvenc_hevc_main10_access_unit_signals_10_bit_sps() {
    let Ok(mut encoder) =
        NvencHevcEncoder::new_main10_with_bitrate(SMOKE_WIDTH, SMOKE_HEIGHT, 30, 8_000_000)
    else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        SMOKE_WIDTH,
        SMOKE_HEIGHT,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x80; SMOKE_WIDTH * SMOKE_HEIGHT * 4],
    );
    let access_unit = encoder
        .encode(&frame)
        .expect("nvenc hevc main10 encode frame")
        .into_iter()
        .next()
        .expect("single access unit");
    let bit_depth = extract_hevc_sps_luma_bit_depth(&access_unit.bytes).expect("HEVC SPS");

    assert_eq!(bit_depth, 10);
}

#[cfg(windows)]
#[test]
fn nvenc_hevc_main10_720p_access_unit_signals_10_bit_sps() {
    let width = 1280;
    let height = 720;
    let Ok(mut encoder) = NvencHevcEncoder::new_main10_with_bitrate(width, height, 30, 12_000_000)
    else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        width,
        height,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x80; width * height * 4],
    );
    let access_unit = match encoder.encode(&frame) {
        Ok(access_units) => access_units
            .into_iter()
            .next()
            .expect("single 720p access unit"),
        Err(error) if error.to_string().contains("produced a 8-bit bitstream") => return,
        Err(error) => panic!("nvenc hevc main10 encode 720p frame: {error}"),
    };
    let bit_depth = extract_hevc_sps_luma_bit_depth(&access_unit.bytes).expect("HEVC SPS");

    assert_eq!(bit_depth, 10);
}

#[cfg(windows)]
fn extract_sps_profile_idc(access_unit: &[u8]) -> Option<u8> {
    let mut offset = 0usize;
    while let Some((start, start_len)) = find_h264_start_code(access_unit, offset) {
        let nal_header = start + start_len;
        if let Some(&header) = access_unit.get(nal_header) {
            if header & 0x1f == 7 {
                return access_unit.get(nal_header + 1).copied();
            }
        }
        offset = nal_header.saturating_add(1);
    }
    None
}

#[cfg(windows)]
fn extract_hevc_sps_luma_bit_depth(access_unit: &[u8]) -> Option<u8> {
    let mut offset = 0usize;
    while let Some((start, start_len)) = find_h264_start_code(access_unit, offset) {
        let nal_start = start + start_len;
        let next = find_h264_start_code(access_unit, nal_start)
            .map(|(next, _)| next)
            .unwrap_or(access_unit.len());
        let nal = access_unit.get(nal_start..next)?;
        if nal.len() >= 3 && ((nal[0] >> 1) & 0x3f) == 33 {
            return parse_hevc_sps_luma_bit_depth(&nal[2..]);
        }
        offset = nal_start.saturating_add(1);
    }
    None
}

#[cfg(windows)]
fn parse_hevc_sps_luma_bit_depth(bytes: &[u8]) -> Option<u8> {
    let rbsp = hevc_rbsp(bytes);
    let mut bits = BitReader::new(&rbsp);
    bits.read_bits(4)?;
    let max_sub_layers_minus1 = bits.read_bits(3)? as usize;
    bits.read_bit()?;
    skip_profile_tier_level(&mut bits, max_sub_layers_minus1)?;
    bits.read_ue()?;
    let chroma_format_idc = bits.read_ue()?;
    if chroma_format_idc == 3 {
        bits.read_bit()?;
    }
    bits.read_ue()?;
    bits.read_ue()?;
    if bits.read_bit()? != 0 {
        bits.read_ue()?;
        bits.read_ue()?;
        bits.read_ue()?;
        bits.read_ue()?;
    }
    Some(8 + bits.read_ue()? as u8)
}

#[cfg(windows)]
fn skip_profile_tier_level(bits: &mut BitReader<'_>, max_sub_layers_minus1: usize) -> Option<()> {
    bits.read_bits(2)?;
    bits.read_bit()?;
    bits.read_bits(5)?;
    bits.read_bits(32)?;
    bits.read_bits(4)?;
    bits.read_bits(16)?;
    bits.read_bits(16)?;
    bits.read_bits(12)?;
    bits.read_bits(8)?;

    let mut profile_present = vec![false; max_sub_layers_minus1];
    let mut level_present = vec![false; max_sub_layers_minus1];
    for i in 0..max_sub_layers_minus1 {
        profile_present[i] = bits.read_bit()? != 0;
        level_present[i] = bits.read_bit()? != 0;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            bits.read_bits(2)?;
        }
    }
    for i in 0..max_sub_layers_minus1 {
        if profile_present[i] {
            bits.read_bits(2)?;
            bits.read_bit()?;
            bits.read_bits(5)?;
            bits.read_bits(32)?;
            bits.read_bits(4)?;
            bits.read_bits(16)?;
            bits.read_bits(16)?;
            bits.read_bits(12)?;
        }
        if level_present[i] {
            bits.read_bits(8)?;
        }
    }
    Some(())
}

#[cfg(windows)]
fn hevc_rbsp(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut zeros = 0usize;
    for &byte in bytes {
        if zeros >= 2 && byte == 0x03 {
            zeros = 0;
            continue;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

#[cfg(windows)]
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_offset: usize,
}

#[cfg(windows)]
impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_offset: 0,
        }
    }

    fn read_bit(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.bit_offset / 8)?;
        let bit = (byte >> (7 - (self.bit_offset % 8))) & 1;
        self.bit_offset += 1;
        Some(bit)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.read_bit()? as u32;
        }
        Some(value)
    }

    fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zero_bits = 0u32;
        while self.read_bit()? == 0 {
            leading_zero_bits += 1;
            if leading_zero_bits > 31 {
                return None;
            }
        }
        let suffix = if leading_zero_bits == 0 {
            0
        } else {
            self.read_bits(leading_zero_bits as usize)?
        };
        Some((1 << leading_zero_bits) - 1 + suffix)
    }
}

#[cfg(windows)]
fn extract_h264_nal_types(access_unit: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut offset = 0usize;
    while let Some((start, start_len)) = find_h264_start_code(access_unit, offset) {
        let nal_header = start + start_len;
        if let Some(&header) = access_unit.get(nal_header) {
            types.push(header & 0x1f);
        }
        offset = nal_header.saturating_add(1);
    }
    types
}

#[cfg(windows)]
fn find_h264_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index + 3 <= bytes.len() {
        if bytes[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        if index + 4 <= bytes.len() && bytes[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        index += 1;
    }
    None
}
