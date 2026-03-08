use std::{fs, path::Path, time::Instant};

use mrd_decode::create_decoder;
use mrd_observability::{ComponentKind, ComponentResult};
use openh264::{
    encoder::Encoder,
    formats::{RgbSliceU8, YUVBuffer},
};

#[test]
#[ignore]
fn perf_h264_software_decode_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| "decode.h264_software".into());
    let access_unit = encoded_access_unit();
    let mut decoder = create_decoder("h264_software").expect("create h264 decoder");

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let mut decoded_frame_bytes = None;
    let mut width = None;
    let mut height = None;
    let started_at = Instant::now();

    for _ in 0..sample_count {
        let iter_started_at = Instant::now();
        match decoder.push_access_unit(&access_unit) {
            Ok(()) => {
                let frames = decoder.drain_decoded_frames();
                if let Some(frame) = frames.first() {
                    decoded_frame_bytes = Some(frame.data.len());
                    width = Some(frame.width as u32);
                    height = Some(frame.height as u32);
                }
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                success_count += 1;
            }
            Err(_) => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Decode,
        "h264_software",
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        width,
        height,
        None,
        None,
        None,
        None,
        None,
        None,
        decoded_frame_bytes,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize decode perf result"),
        )
        .expect("write decode perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.decoded_frame_bytes.is_some());
}

fn encoded_access_unit() -> Vec<u8> {
    let mut rgb = Vec::with_capacity(16 * 16 * 3);
    for y in 0..16 {
        for x in 0..16 {
            rgb.push((x * 16) as u8);
            rgb.push((y * 16) as u8);
            rgb.push(96);
        }
    }
    let rgb_source = RgbSliceU8::new(&rgb, (16, 16));
    let yuv = YUVBuffer::from_rgb_source(rgb_source);
    let mut encoder = Encoder::new().expect("openh264 encoder");
    encoder.encode(&yuv).expect("encode access unit").to_vec()
}
