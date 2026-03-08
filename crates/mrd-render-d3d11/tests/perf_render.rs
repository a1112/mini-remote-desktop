#![cfg(windows)]

use std::{fs, path::Path, time::Instant};

use mrd_observability::{ComponentKind, ComponentResult};
use mrd_render::{RenderFrame, RenderPixelFormat, RenderTarget, RendererFactory};
use mrd_render_d3d11::D3d11RendererFactory;

#[test]
#[ignore]
fn perf_d3d11_render_reports_latency_distribution() {
    let sample_count = std::env::var("MRD_COMPONENT_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let case_name =
        std::env::var("MRD_COMPONENT_CASE_NAME").unwrap_or_else(|_| "render.d3d11".into());
    let width = 1280usize;
    let height = 720usize;
    let frame_bytes = width * height * 3;
    let factory = D3d11RendererFactory;
    let mut renderer = factory.create().expect("create d3d11 renderer");
    renderer
        .attach_target(RenderTarget::WindowHandle(0))
        .expect("attach render target");
    let frame = RenderFrame {
        width,
        height,
        pixel_format: RenderPixelFormat::Rgb24,
        data: synthetic_rgb24_frame(width, height),
    };

    let mut latencies_ms = Vec::with_capacity(sample_count as usize);
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let started_at = Instant::now();

    for _ in 0..sample_count {
        let iter_started_at = Instant::now();
        match renderer.upload_frame(frame.clone()) {
            Ok(()) => {
                latencies_ms.push(iter_started_at.elapsed().as_secs_f64() * 1000.0);
                success_count += 1;
            }
            Err(_) => {
                failure_count += 1;
            }
        }
    }

    let result = ComponentResult::new(
        ComponentKind::Render,
        "d3d11",
        case_name,
        started_at.elapsed().as_secs_f64(),
        success_count,
        failure_count,
        &latencies_ms,
        Some(width as u32),
        Some(height as u32),
        Some(frame_bytes),
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
            serde_json::to_string_pretty(&result).expect("serialize render perf result"),
        )
        .expect("write render perf result");
    }

    assert!(result.sample_count > 0);
    assert!(result.latency_ms.p50_ms.is_some());
    assert!(result.latency_ms.p95_ms.is_some());
    assert!(result.latency_ms.p99_ms.is_some());
    assert!(result.success_ratio.is_some());
}

fn synthetic_rgb24_frame(width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0_u8; width * height * 3];
    for (index, chunk) in data.chunks_exact_mut(3).enumerate() {
        chunk[0] = (index % 255) as u8;
        chunk[1] = ((index / 2) % 255) as u8;
        chunk[2] = ((index / 3) % 255) as u8;
    }
    data
}
