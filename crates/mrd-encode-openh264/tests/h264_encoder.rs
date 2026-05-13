use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

fn bgra_frame(timestamp_us: u64, value: u8) -> CapturedFrame {
    CapturedFrame::from_cpu(
        16,
        16,
        FramePixelFormat::Bgra32,
        timestamp_us,
        vec![value; 16 * 16 * 4],
    )
}

#[test]
fn openh264_encoder_emits_h264_access_unit_for_bgra_frame() {
    let mut encoder = OpenH264Encoder::new(16, 16, 30).expect("create encoder");
    let frame = bgra_frame(1234, 127);

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

#[test]
fn openh264_encoder_does_not_mark_every_access_unit_as_keyframe() {
    let mut encoder = OpenH264Encoder::new(16, 16, 30).expect("create encoder");

    let first = encoder
        .encode(&bgra_frame(0, 64))
        .expect("encode first frame");
    let mut followup_keyframes = 0;
    for index in 1..8 {
        let units = encoder
            .encode(&bgra_frame(index * 33_333, 64 + index as u8))
            .expect("encode followup frame");
        if units[0].is_keyframe {
            followup_keyframes += 1;
        }
    }

    assert!(first[0].is_keyframe);
    assert!(
        followup_keyframes < 7,
        "short GOP followup frames should not all be marked as keyframes"
    );
}

#[test]
fn openh264_encoder_forces_recovery_keyframe_after_one_second() {
    let mut encoder = OpenH264Encoder::new(16, 16, 144).expect("create encoder");

    let _ = encoder
        .encode(&bgra_frame(0, 64))
        .expect("encode first frame");
    let recovery = encoder
        .encode(&bgra_frame(1_100_000, 96))
        .expect("encode recovery frame");

    assert!(recovery[0].is_keyframe);
}

#[test]
fn openh264_encoder_does_not_emit_empty_access_units_with_bitrate_control() {
    let width = 2560;
    let height = 1440;
    let mut encoder =
        OpenH264Encoder::new_with_bitrate(width, height, 60, 5_000_000).expect("create encoder");

    for index in 0..8 {
        let frame = CapturedFrame::from_cpu(
            width,
            height,
            FramePixelFormat::Bgra32,
            index * 16_666,
            vec![index as u8; width * height * 4],
        );
        let access_units = encoder.encode(&frame).expect("encode frame");

        assert!(
            access_units.iter().all(|unit| !unit.bytes.is_empty()),
            "bitrate-controlled OpenH264 output must not pass empty access units downstream"
        );
    }
}

#[test]
fn openh264_encoder_rejects_odd_dimensions_without_panicking() {
    let error = match OpenH264Encoder::new(17, 15, 30) {
        Ok(_) => panic!("odd dimensions should fail"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("openh264 requires even frame dimensions"));
}
