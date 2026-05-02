use mrd_encode_nvenc::{NvencH264Encoder, NvencHevcEncoder};
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

#[test]
fn nvenc_encoder_is_probeable_or_emits_h264_access_unit() {
    let Ok(mut encoder) = NvencH264Encoder::new(16, 16, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        16,
        16,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x7f; 16 * 16 * 4],
    );
    let access_units = encoder.encode(&frame).expect("nvenc encode frame");

    assert!(!access_units.is_empty());
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert!(!access_units[0].bytes.is_empty());
}

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

#[test]
fn nvenc_hevc_encoder_prefers_d3d11_shared_bgra_input() {
    assert_eq!(
        NvencHevcEncoder::preferred_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
    );
}

#[test]
fn nvenc_hevc_main10_encoder_prefers_d3d11_shared_bgra_input() {
    assert_eq!(
        NvencHevcEncoder::preferred_main10_input_memory_kind(),
        mrd_pipeline_core::FrameMemoryKind::D3D11SharedBgra
    );
}

#[test]
fn nvenc_hevc_encoder_emits_hevc_access_unit_when_available() {
    let Ok(mut encoder) = NvencHevcEncoder::new_main(16, 16, 30) else {
        return;
    };

    let frame = CapturedFrame::from_cpu(
        16,
        16,
        FramePixelFormat::Bgra32,
        33_000,
        vec![0x7f; 16 * 16 * 4],
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

fn extract_sps_profile_idc(access_unit: &[u8]) -> Option<u8> {
    let mut offset = 0usize;
    while offset + 6 <= access_unit.len() {
        if access_unit[offset..].starts_with(&[0, 0, 0, 1]) {
            let nal_type = access_unit[offset + 4] & 0x1f;
            if nal_type == 7 {
                return access_unit.get(offset + 5).copied();
            }
            offset += 4;
        } else {
            offset += 1;
        }
    }
    None
}
