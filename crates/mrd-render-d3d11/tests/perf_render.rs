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
    let frame = RenderFrame::from_rgb24(width, height, synthetic_rgb24_frame(width, height));

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

#[test]
#[ignore]
fn perf_d3d11_render_bgra32_vs_rgb24() {
    let sample_count = 500;
    let width = 1920usize;
    let height = 1080usize;

    let factory = D3d11RendererFactory;
    let mut renderer = factory.create().expect("create d3d11 renderer");
    renderer
        .attach_target(RenderTarget::WindowHandle(0))
        .expect("attach render target");

    println!("Performance Test: {width}x{height}, {sample_count} samples\n");

    // Test RGB24
    let rgb_frame = RenderFrame::from_rgb24(width, height, synthetic_rgb24_frame(width, height));

    let mut rgb_latencies = Vec::with_capacity(sample_count);
    let rgb_started = Instant::now();

    for _ in 0..sample_count {
        let start = Instant::now();
        let _ = renderer.upload_frame(rgb_frame.clone());
        rgb_latencies.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let rgb_total = rgb_started.elapsed();

    // Test BGRA32
    let bgra_frame =
        RenderFrame::from_bgra32(width, height, synthetic_bgra32_frame(width, height));

    let mut bgra_latencies = Vec::with_capacity(sample_count);
    let bgra_started = Instant::now();

    for _ in 0..sample_count {
        let start = Instant::now();
        let _ = renderer.upload_frame(bgra_frame.clone());
        bgra_latencies.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let bgra_total = bgra_started.elapsed();

    // Calculate statistics
    let rgb_latencies_sorted = {
        let mut v = rgb_latencies.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };
    let bgra_latencies_sorted = {
        let mut v = bgra_latencies.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };

    let rgb_p50 = rgb_latencies_sorted[sample_count / 2];
    let rgb_p95 = rgb_latencies_sorted[(sample_count * 95) / 100];
    let rgb_p99 = rgb_latencies_sorted[(sample_count * 99) / 100];
    let rgb_avg: f64 = rgb_latencies.iter().sum::<f64>() / sample_count as f64;

    let bgra_p50 = bgra_latencies_sorted[sample_count / 2];
    let bgra_p95 = bgra_latencies_sorted[(sample_count * 95) / 100];
    let bgra_p99 = bgra_latencies_sorted[(sample_count * 99) / 100];
    let bgra_avg: f64 = bgra_latencies.iter().sum::<f64>() / sample_count as f64;

    println!("RGB24 Results:");
    println!("  Total:  {:.2}s ({:.2} FPS)", rgb_total.as_secs_f64(), sample_count as f64 / rgb_total.as_secs_f64());
    println!("  Avg:    {:.3}ms", rgb_avg);
    println!("  P50:    {:.3}ms", rgb_p50);
    println!("  P95:    {:.3}ms", rgb_p95);
    println!("  P99:    {:.3}ms", rgb_p99);

    println!("\nBGRA32 Results:");
    println!("  Total:  {:.2}s ({:.2} FPS)", bgra_total.as_secs_f64(), sample_count as f64 / bgra_total.as_secs_f64());
    println!("  Avg:    {:.3}ms", bgra_avg);
    println!("  P50:    {:.3}ms", bgra_p50);
    println!("  P95:    {:.3}ms", bgra_p95);
    println!("  P99:    {:.3}ms", bgra_p99);

    println!("\nImprovement (BGRA32 vs RGB24):");
    println!("  Avg:  {:.2}%", ((rgb_avg - bgra_avg) / rgb_avg) * 100.0);
    println!("  P50:  {:.2}%", ((rgb_p50 - bgra_p50) / rgb_p50) * 100.0);
    println!("  P95:  {:.2}%", ((rgb_p95 - bgra_p95) / rgb_p95) * 100.0);
    println!("  P99:  {:.2}%", ((rgb_p99 - bgra_p99) / rgb_p99) * 100.0);
    println!("  FPS:  +{:.1}%", (sample_count as f64 / bgra_total.as_secs_f64() / (sample_count as f64 / rgb_total.as_secs_f64()) - 1.0) * 100.0);
}

#[test]
#[ignore]
fn perf_d3d11_render_rgb24_optimized() {
    let sample_count = 500;
    let width = 1920usize;
    let height = 1080usize;

    let factory = D3d11RendererFactory;
    let mut renderer = factory.create().expect("create d3d11 renderer");
    renderer
        .attach_target(RenderTarget::WindowHandle(0))
        .expect("attach render target");

    let rgb_frame = RenderFrame::from_rgb24(width, height, synthetic_rgb24_frame(width, height));

    let mut latencies = Vec::with_capacity(sample_count);
    let started = Instant::now();

    for _ in 0..sample_count {
        let start = Instant::now();
        let _ = renderer.upload_frame(rgb_frame.clone());
        latencies.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let total = started.elapsed();

    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let avg = latencies.iter().sum::<f64>() / sample_count as f64;
    let p50 = sorted[sample_count / 2];
    let p95 = sorted[(sample_count * 95) / 100];
    let p99 = sorted[(sample_count * 99) / 100];

    println!("RGB24 Optimized (SIMD) Performance: {width}x{height}, {sample_count} samples");
    println!("  Total:  {:.2}s ({:.2} FPS)", total.as_secs_f64(), sample_count as f64 / total.as_secs_f64());
    println!("  Avg:    {:.3}ms", avg);
    println!("  P50:    {:.3}ms", p50);
    println!("  P95:    {:.3}ms", p95);
    println!("  P99:    {:.3}ms", p99);
}

#[test]
#[ignore]
fn perf_rgb24_to_bgra_conversion() {
    let sample_count = 1000;
    let width = 1920usize;
    let height = 1080usize;
    let pixels = width * height;

    let src: Vec<u8> = (0..pixels * 3).map(|i| (i % 256) as u8).collect();

    println!("RGB24->BGRA Conversion Performance Test: {width}x{height}, {sample_count} iterations\n");

    // Test scalar version
    let mut scalar_dst = vec![0_u8; pixels * 4];
    let scalar_started = Instant::now();

    for _ in 0..sample_count {
        for (src_idx, dst_idx) in (0..pixels).map(|i| (i * 3, i * 4)) {
            scalar_dst[dst_idx] = src[src_idx + 2];
            scalar_dst[dst_idx + 1] = src[src_idx + 1];
            scalar_dst[dst_idx + 2] = src[src_idx];
            scalar_dst[dst_idx + 3] = 255;
        }
    }

    let scalar_total = scalar_started.elapsed();

    // Test SIMD version
    let mut simd_dst = vec![0_u8; pixels * 4];
    let simd_started = Instant::now();

    for _ in 0..sample_count {
        mrd_render_d3d11::simd::rgb24_to_bgra(&src, &mut simd_dst, width, height);
    }

    let simd_total = simd_started.elapsed();

    let scalar_ms = scalar_total.as_secs_f64() * 1000.0;
    let simd_ms = simd_total.as_secs_f64() * 1000.0;

    println!("Scalar version:");
    println!("  Total:  {:.3}s", scalar_total.as_secs_f64());
    println!("  Per iteration: {:.3}ms", scalar_ms / sample_count as f64);

    println!("\nSIMD version:");
    println!("  Total:  {:.3}s", simd_total.as_secs_f64());
    println!("  Per iteration: {:.3}ms", simd_ms / sample_count as f64);

    println!("\nImprovement:");
    println!("  Speedup: {:.2}x", scalar_ms / simd_ms);
    println!("  Time saved: {:.3}ms per frame", (scalar_ms - simd_ms) / sample_count as f64);
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

fn synthetic_bgra32_frame(width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0_u8; width * height * 4];
    for (index, chunk) in data.chunks_exact_mut(4).enumerate() {
        chunk[0] = ((index / 3) % 255) as u8; // B
        chunk[1] = ((index / 2) % 255) as u8; // G
        chunk[2] = (index % 255) as u8;       // R
        chunk[3] = 255;                       // A
    }
    data
}
