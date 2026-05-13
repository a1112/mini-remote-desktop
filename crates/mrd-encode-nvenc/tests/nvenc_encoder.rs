use mrd_encode_nvenc::{NvencH264Encoder, NvencHevcEncoder};
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

#[cfg(windows)]
const SMOKE_WIDTH: usize = 16;
#[cfg(windows)]
const SMOKE_HEIGHT: usize = 16;
#[cfg(not(windows))]
const SMOKE_WIDTH: usize = 160;
#[cfg(not(windows))]
const SMOKE_HEIGHT: usize = 64;

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
fn nvenc_hevc_main10_encoder_prefers_d3d11_shared_bgra_input() {
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
