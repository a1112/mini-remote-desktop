use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

#[test]
fn openh264_encoder_emits_h264_access_unit_for_bgra_frame() {
    let mut encoder = OpenH264Encoder::new(16, 16, 30).expect("create encoder");
    let frame = CapturedFrame {
        width: 16,
        height: 16,
        pixel_format: FramePixelFormat::Bgra32,
        timestamp_us: 1234,
        data: vec![127; 16 * 16 * 4],
    };

    let access_units = encoder.encode(&frame).expect("encode frame");

    assert!(!access_units.is_empty());
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert_eq!(access_units[0].timestamp_us, 1234);
    assert!(!access_units[0].bytes.is_empty());
    assert!(
        access_units[0]
            .bytes
            .windows(4)
            .any(|window| window == [0, 0, 0, 1]),
        "encoder output should be normalized to Annex-B for RTP packetization"
    );
}
