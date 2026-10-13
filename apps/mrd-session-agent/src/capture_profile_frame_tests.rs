use mrd_agent_ipc::AgentCaptureProfile;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};

type PreparationUnderTest = super::ProfileFramePreparer;

fn profile(width: u32, height: u32) -> AgentCaptureProfile {
    AgentCaptureProfile {
        width,
        height,
        fps: 30,
        bitrate_bps: 600_000,
    }
}

fn synthetic(width: usize, height: usize, format: FramePixelFormat, seed: u8) -> CapturedFrame {
    let bytes = match format {
        FramePixelFormat::Rgb24 => 3,
        _ => 4,
    };
    let data = (0..width * height * bytes)
        .map(|index| {
            (index as u8)
                .wrapping_mul(73)
                .wrapping_add((index / 251) as u8)
                .wrapping_add(seed)
        })
        .collect();
    CapturedFrame::from_cpu(width, height, format, 10_000 + u64::from(seed), data)
}

#[test]
fn profile_frame_preparer_preserves_non_integer_scaling_and_every_pixel_format() {
    let mut preparer = PreparationUnderTest::default();
    for format in [
        FramePixelFormat::Bgra32,
        FramePixelFormat::Rgba32,
        FramePixelFormat::Rgb24,
    ] {
        for (source_width, source_height, width, height) in
            [(7, 5, 4, 2), (3, 5, 8, 6), (9, 3, 4, 8), (4, 2, 4, 2)]
        {
            let frame = synthetic(source_width, source_height, format, 17);
            let approved = profile(width, height);
            let expected = super::prepare_profile_frame(frame.clone(), approved).unwrap();
            let actual = preparer.prepare(&frame, approved).unwrap();
            assert_eq!(actual, &expected);
        }
    }
}

#[test]
fn profile_frame_preparer_reuses_output_for_changed_pixels_and_timestamp() {
    let mut preparer = PreparationUnderTest::default();
    let first = synthetic(7, 5, FramePixelFormat::Bgra32, 3);
    let approved = profile(4, 2);
    let prepared = preparer.prepare(&first, approved).unwrap();
    let pointer = prepared.data.as_ptr();
    let capacity = prepared.data.capacity();
    let second = synthetic(7, 5, FramePixelFormat::Bgra32, 29);
    let expected = super::prepare_profile_frame(second.clone(), approved).unwrap();
    let prepared = preparer.prepare(&second, approved).unwrap();
    assert_eq!(prepared, &expected);
    assert_eq!(
        prepared.data.as_ptr(),
        pointer,
        "output allocation must be reused"
    );
    assert_eq!(prepared.data.capacity(), capacity);
}

#[test]
fn profile_frame_preparer_same_size_returns_original_frame_without_copy() {
    let mut preparer = PreparationUnderTest::default();
    let frame = synthetic(4, 2, FramePixelFormat::Rgba32, 11);
    let actual = preparer.prepare(&frame, profile(4, 2)).unwrap();
    assert!(
        std::ptr::eq(actual, &frame),
        "same-size frame must be borrowed intact"
    );
}

#[test]
fn profile_frame_preparer_revalidates_shape_and_recovers_after_invalid_input() {
    let mut preparer = PreparationUnderTest::default();
    for (width, height, format, target_width, target_height) in [
        (7, 5, FramePixelFormat::Bgra32, 4, 2),
        (5, 7, FramePixelFormat::Rgba32, 4, 2),
        (9, 3, FramePixelFormat::Rgb24, 6, 4),
        (7, 5, FramePixelFormat::Bgra32, 4, 2),
    ] {
        let frame = synthetic(width, height, format, 31);
        let approved = profile(target_width, target_height);
        let expected = super::prepare_profile_frame(frame.clone(), approved).unwrap();
        assert_eq!(preparer.prepare(&frame, approved), Some(&expected));

        let mut invalid = frame.clone();
        invalid.data.pop();
        assert!(preparer.prepare(&invalid, approved).is_none());
        invalid.data.extend_from_slice(&[1, 2]);
        assert!(preparer.prepare(&invalid, approved).is_none());
        assert_eq!(preparer.prepare(&frame, approved), Some(&expected));
    }
}

#[test]
fn profile_frame_preparer_rejects_padding_zero_dimensions_and_overflow() {
    let mut preparer = PreparationUnderTest::default();
    let approved = profile(4, 2);
    // The capture adapter already repacks rows. A padded source is still invalid
    // at this boundary; reusing output must not weaken the exact packed length.
    let padded = CapturedFrame::from_cpu(4, 2, FramePixelFormat::Bgra32, 1, vec![0; 40]);
    let zero = CapturedFrame::from_cpu(0, 2, FramePixelFormat::Bgra32, 1, vec![]);
    let overflowing = CapturedFrame::from_cpu(usize::MAX, 2, FramePixelFormat::Bgra32, 1, vec![]);
    let unsupported = CapturedFrame::from_cpu(4, 2, FramePixelFormat::Nv12, 1, vec![0; 32]);
    for frame in [&padded, &zero, &overflowing, &unsupported] {
        assert!(preparer.prepare(frame, approved).is_none());
    }
    let valid = synthetic(7, 5, FramePixelFormat::Bgra32, 5);
    for invalid in [
        AgentCaptureProfile {
            width: 3,
            ..approved
        },
        AgentCaptureProfile { fps: 0, ..approved },
        AgentCaptureProfile {
            bitrate_bps: 0,
            ..approved
        },
    ] {
        assert!(preparer.prepare(&valid, invalid).is_none());
    }
    let expected = super::prepare_profile_frame(valid.clone(), approved).unwrap();
    assert_eq!(preparer.prepare(&valid, approved), Some(&expected));
}

#[test]
fn profile_frame_preparer_actual_large_display_is_byte_exact() {
    let frame = synthetic(2560, 1440, FramePixelFormat::Bgra32, 19);
    let approved = profile(1920, 1080);
    let expected = super::prepare_profile_frame(frame.clone(), approved).unwrap();
    let mut preparer = PreparationUnderTest::default();
    let actual = preparer.prepare(&frame, approved).unwrap();
    assert_eq!(actual, &expected);
    assert_eq!(actual.data.len(), 8_294_400);
}

// This baseline changes only input ownership, keeping the original validation,
// fresh output allocation and mapping loop. Holding the synthetic source in
// both paths avoids timing input cloning/deallocation as a scaler difference.
fn borrowed_original(
    frame: &CapturedFrame,
    approved: AgentCaptureProfile,
) -> Option<CapturedFrame> {
    if !approved.is_valid() || frame.width == 0 || frame.height == 0 {
        return None;
    }
    let pixel_bytes = match frame.pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
        FramePixelFormat::Nv12 => return None,
    };
    let source_len = frame
        .width
        .checked_mul(frame.height)?
        .checked_mul(pixel_bytes)?;
    if frame.data.len() != source_len {
        return None;
    }
    let width = approved.width as usize;
    let height = approved.height as usize;
    if (frame.width, frame.height) == (width, height) {
        return Some(frame.clone());
    }
    let mut data = vec![0; width.checked_mul(height)?.checked_mul(pixel_bytes)?];
    for y in 0..height {
        let source_y = y * frame.height / height;
        for x in 0..width {
            let source_x = x * frame.width / width;
            let source = (source_y * frame.width + source_x) * pixel_bytes;
            let target = (y * width + x) * pixel_bytes;
            data[target..target + pixel_bytes]
                .copy_from_slice(&frame.data[source..source + pixel_bytes]);
        }
    }
    Some(CapturedFrame::from_cpu(
        width,
        height,
        frame.pixel_format,
        frame.timestamp_us,
        data,
    ))
}

#[test]
#[ignore = "finite release-only synthetic scaler measurement; does not capture a desktop"]
fn profile_frame_scaler_offline_benchmark() {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };

    assert!(
        !cfg!(debug_assertions),
        "run this benchmark in release mode"
    );
    let frame = synthetic(2560, 1440, FramePixelFormat::Bgra32, 19);
    let approved = profile(1920, 1080);
    let original = super::prepare_profile_frame(frame.clone(), approved).unwrap();
    let baseline = borrowed_original(&frame, approved).unwrap();
    assert_eq!(baseline, original);
    let mut preparer = PreparationUnderTest::default();
    assert_eq!(preparer.prepare(&frame, approved), Some(&original));
    drop(original);
    drop(baseline);
    for _ in 0..20 {
        black_box(borrowed_original(black_box(&frame), approved).unwrap());
        black_box(preparer.prepare(black_box(&frame), approved).unwrap());
    }

    let mut old_samples = Vec::with_capacity(180);
    let mut new_samples = Vec::with_capacity(180);
    let whole_run = Instant::now();
    for _ in 0..3 {
        for optimized in [false, true, true, false] {
            for _ in 0..30 {
                assert!(
                    whole_run.elapsed() < Duration::from_secs(10),
                    "finite benchmark budget exceeded"
                );
                let start = Instant::now();
                if optimized {
                    black_box(preparer.prepare(black_box(&frame), approved).unwrap());
                } else {
                    let output = borrowed_original(black_box(&frame), approved).unwrap();
                    black_box(&output);
                    drop(output);
                }
                let elapsed = start.elapsed().as_nanos();
                if optimized {
                    new_samples.push(elapsed);
                } else {
                    old_samples.push(elapsed);
                }
            }
        }
    }
    old_samples.sort_unstable();
    new_samples.sort_unstable();
    assert_eq!((old_samples.len(), new_samples.len()), (180, 180));
    let percentile = |samples: &[u128], percent: usize| {
        samples[(samples.len() * percent).div_ceil(100) - 1] as f64 / 1000.0
    };
    let old_median_us = percentile(&old_samples, 50);
    let new_median_us = percentile(&new_samples, 50);
    eprintln!(
        "profile_frame_scaler_metadata source_width=2560 source_height=1440 width=1920 height=1080 pixel_format=bgra32 source_bytes={} output_bytes=8294400 samples_per_variant=180 warmup_per_variant=20 old_median_us={:.3} old_p95_us={:.3} new_median_us={:.3} new_p95_us={:.3} median_speedup={:.3} measurement_elapsed_ms={}",
        frame.data.len(), old_median_us, percentile(&old_samples, 95),
        new_median_us, percentile(&new_samples, 95), old_median_us / new_median_us,
        whole_run.elapsed().as_millis(),
    );
    let original = super::prepare_profile_frame(frame.clone(), approved).unwrap();
    assert_eq!(preparer.prepare(&frame, approved), Some(&original));
}
