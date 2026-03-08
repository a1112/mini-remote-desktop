use mrd_encode_nvenc::NvencH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

#[test]
fn nvenc_encoder_is_probeable_or_emits_h264_access_unit() {
    let Ok(mut encoder) = NvencH264Encoder::new(16, 16, 30) else {
        return;
    };

    let frame = CapturedFrame {
        width: 16,
        height: 16,
        pixel_format: FramePixelFormat::Bgra32,
        timestamp_us: 33_000,
        data: vec![0x7f; 16 * 16 * 4],
    };
    let access_units = encoder.encode(&frame).expect("nvenc encode frame");

    assert!(!access_units.is_empty());
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert!(!access_units[0].bytes.is_empty());
}
