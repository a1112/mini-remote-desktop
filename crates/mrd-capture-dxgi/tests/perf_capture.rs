#![cfg(windows)]

use std::{fs, path::Path, time::Instant};

use mrd_capture_dxgi::DxgiDesktopCapture;
use mrd_observability::{ComponentKind, ComponentResult};
use mrd_pipeline_core::FrameCapture;

#[test]
#[ignore]
fn perf_dxgi_capture_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    let case_name =
        std::env::var("MRD_COMPONENT_CASE_NAME").unwrap_or_else(|_| "capture.dxgi".into());
    let mut capture = DxgiDesktopCapture::new_primary().expect("create primary capture");
    let width = capture.width() as u32;
    let height = capture.height() as u32;

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let mut frame_bytes = None;
    let started_at = Instant::now();

    for _ in 0..sample_count {
        let iter_started_at = Instant::now();
        match capture.capture_frame() {
            Ok(frame) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                frame_bytes = Some(frame.data.len());
                success_count += 1;
            }
            Err(_) => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Capture,
        "dxgi",
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        Some(width),
        Some(height),
        frame_bytes,
        None,
        None,
        None,
        None,
        None,
        None,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize capture perf result"),
        )
        .expect("write capture perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.success_ratio.unwrap_or_default() > 0.0);
}
