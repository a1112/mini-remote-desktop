#![cfg(windows)]

use std::{fs, path::Path, time::Instant};

use mrd_capture_winrt::WinrtCapture;
use mrd_observability::{ComponentKind, ComponentResult};
use mrd_pipeline_core::{FrameCapture, FrameMemoryKind};

#[test]
#[ignore]
fn perf_winrt_monitor_capture_reports_latency_distribution() {
    let mut capture = WinrtCapture::from_monitor_index(0).expect("create primary WinRT capture");
    capture.start().expect("start WinRT monitor capture");
    run_capture_perf("winrt_monitor", capture, false);
}

#[test]
#[ignore]
fn perf_winrt_monitor_shared_texture_capture_reports_latency_distribution() {
    let mut capture = WinrtCapture::from_monitor_index_shared_texture(0)
        .expect("create primary WinRT shared capture");
    capture.start().expect("start WinRT shared monitor capture");
    run_capture_perf("winrt_monitor_shared", capture, true);
}

#[test]
#[ignore]
fn perf_winrt_window_capture_reports_latency_distribution() {
    let hwnd = std::env::var("MRD_CAPTURE_WINDOW_HWND")
        .ok()
        .and_then(|value| parse_hwnd(&value).ok());

    let Some(hwnd) = hwnd else {
        println!("MRD_CAPTURE_WINDOW_HWND is not set; skipping window capture perf sample");
        return;
    };

    let mut capture =
        WinrtCapture::from_window_handle(hwnd).expect("create selected WinRT window capture");
    capture.start().expect("start WinRT window capture");
    run_capture_perf("winrt_window", capture, false);
}

#[test]
#[ignore]
fn perf_winrt_window_shared_texture_capture_reports_latency_distribution() {
    let hwnd = std::env::var("MRD_CAPTURE_WINDOW_HWND")
        .ok()
        .and_then(|value| parse_hwnd(&value).ok());

    let Some(hwnd) = hwnd else {
        println!("MRD_CAPTURE_WINDOW_HWND is not set; skipping window shared capture perf sample");
        return;
    };

    let mut capture = WinrtCapture::from_window_handle_shared_texture(hwnd)
        .expect("create selected WinRT shared window capture");
    capture.start().expect("start WinRT shared window capture");
    run_capture_perf("winrt_window_shared", capture, true);
}

fn run_capture_perf(default_backend: &str, mut capture: impl FrameCapture, expect_zero_copy: bool) {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);
    let backend = std::env::var("MRD_COMPONENT_BACKEND").unwrap_or_else(|_| default_backend.into());
    let case_name = std::env::var("MRD_COMPONENT_CASE_NAME")
        .unwrap_or_else(|_| format!("capture.{default_backend}"));

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let mut frame_bytes = None;
    let mut width = None;
    let mut height = None;
    let mut zero_copy_count = 0_u64;
    let started_at = Instant::now();

    for _ in 0..sample_count {
        let iter_started_at = Instant::now();
        match capture.capture_frame() {
            Ok(frame) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                frame_bytes = Some(frame.data.len());
                width = Some(frame.width as u32);
                height = Some(frame.height as u32);
                if capture.output_memory_kind() == FrameMemoryKind::D3D11SharedBgra
                    && frame.d3d11_shared_bgra().is_some()
                {
                    zero_copy_count += 1;
                }
                success_count += 1;
            }
            Err(error) => {
                eprintln!("WinRT capture sample failed: {error}");
                failure_count += 1;
            }
        }
    }

    let zero_copy_hit_ratio = if success_count > 0 && zero_copy_count > 0 {
        Some(zero_copy_count as f64 / success_count as f64)
    } else {
        None
    };

    let result = ComponentResult::new(
        ComponentKind::Capture,
        backend.clone(),
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        width,
        height,
        frame_bytes,
        zero_copy_hit_ratio,
        None,
        None,
        None,
        None,
        None,
    );

    if let Ok(result_path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(
            Path::new(&result_path),
            serde_json::to_string_pretty(&result).expect("serialize WinRT capture perf result"),
        )
        .expect("write WinRT capture perf result");
    }

    println!(
        "capture backend={backend} samples={} success={} failures={} fps={:.2} p50={:?}ms p95={:?}ms zero_copy={:?}",
        result.sample_count,
        result.success_count,
        result.failure_count,
        result.throughput_fps,
        result.latency_ms.p50_ms,
        result.latency_ms.p95_ms,
        result.zero_copy_hit_ratio
    );

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.success_ratio.unwrap_or_default() > 0.0);
    if expect_zero_copy {
        assert!(result.zero_copy_hit_ratio.unwrap_or_default() >= 0.99);
    }
}

fn parse_hwnd(input: &str) -> Result<isize, std::num::ParseIntError> {
    let trimmed = input.trim().rsplit(':').next().unwrap_or(input).trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        isize::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<isize>()
    }
}
