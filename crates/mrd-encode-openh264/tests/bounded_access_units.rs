use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoEncoder};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;
const ACCESS_UNIT_LIMIT: usize = 128 * 1024;

// Deterministic synthetic desktop content; never captures a local desktop or
// records encoded bytes. The moving panel exercises P frames between IDRs.
fn synthetic_desktop(frame_index: usize) -> CapturedFrame {
    let mut pixels = vec![0; WIDTH * HEIGHT * 4];
    let panel_x = 160 + frame_index * 6;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let grid = (x / 24 + y / 20) % 2;
            let moving_panel = (panel_x..panel_x + 480).contains(&x) && (240..600).contains(&y);
            let color = if moving_panel {
                [48, 128, 208, 255]
            } else if y < 72 || y > HEIGHT - 48 {
                [32, 32, 32, 255]
            } else if grid == 0 {
                [72, 84, 96, 255]
            } else {
                [88, 100, 112, 255]
            };
            pixels[(y * WIDTH + x) * 4..(y * WIDTH + x + 1) * 4].copy_from_slice(&color);
        }
    }
    CapturedFrame::from_cpu(
        WIDTH,
        HEIGHT,
        FramePixelFormat::Bgra32,
        frame_index as u64 * 33_333,
        pixels,
    )
}

#[test]
fn hd30_bounded_software_decodes_first_p_frames_and_periodic_idr() {
    let mut encoder = OpenH264Encoder::new_with_bitrate_and_access_unit_limit(
        WIDTH,
        HEIGHT,
        30,
        10_000_000,
        ACCESS_UNIT_LIMIT,
    )
    .expect("create budgeted software encoder");
    let mut decoder = Decoder::new().expect("create software decoder");
    let mut decoded_frames = 0;
    let mut keyframes = 0;
    let mut maximum_bytes = 0;

    for frame_index in 0..=30 {
        let units = encoder
            .encode(&synthetic_desktop(frame_index))
            .expect("encode synthetic desktop");
        if frame_index == 0 || frame_index == 30 {
            assert_eq!(units.len(), 1, "one complete recovery access unit");
            assert!(units[0].is_keyframe, "recovery frame is a complete IDR");
        }
        for unit in units {
            assert!(!unit.bytes.is_empty());
            assert!(unit.bytes.len() <= ACCESS_UNIT_LIMIT);
            assert_eq!(unit.timestamp_us, frame_index as u64 * 33_333);
            maximum_bytes = maximum_bytes.max(unit.bytes.len());
            keyframes += usize::from(unit.is_keyframe);
            let decoded = decoder
                .decode(&unit.bytes)
                .expect("decode the entire normalized access unit")
                .expect("decode one complete frame");
            assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
            decoded_frames += 1;
        }
    }

    assert!(decoded_frames > 2, "dependent P frames are also decoded");
    assert_eq!(keyframes, 2);
    eprintln!(
        "software_budget_characterization decoded_frames={decoded_frames} keyframes={keyframes} max_access_unit_bytes={maximum_bytes} width={WIDTH} height={HEIGHT}"
    );
}

#[test]
fn rejected_complete_access_unit_cannot_be_followed_by_dependent_p_frames() {
    let mut encoder = OpenH264Encoder::new_with_bitrate_and_access_unit_limit(
        WIDTH,
        HEIGHT,
        30,
        10_000_000,
        8 * 1024,
    )
    .expect("create deliberately small-budget encoder");
    let mut frame = synthetic_desktop(0);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            // Deliberately difficult non-private picture, illustrating that
            // the rate-control target cannot guarantee an arbitrary AU size.
            let cell = (x / 3) as u32 ^ ((y / 3) as u32).wrapping_mul(0x9e37_79b9);
            let value = cell.wrapping_mul(0x85eb_ca6b).rotate_left(13) as u8;
            frame.data[(y * WIDTH + x) * 4..(y * WIDTH + x + 1) * 4]
                .copy_from_slice(&[value, value, value, 255]);
        }
    }
    let first_error = encoder
        .encode(&frame)
        .expect_err("reject oversized entire AU");
    assert_eq!(
        first_error.to_string(),
        "openh264 encoded access unit exceeds budget"
    );
    frame.timestamp_us = 33_333;
    let next_error = encoder
        .encode(&frame)
        .expect_err("never emit a P frame referencing the rejected AU");
    assert_eq!(
        next_error.to_string(),
        "openh264 access unit budget already rejected"
    );
}
