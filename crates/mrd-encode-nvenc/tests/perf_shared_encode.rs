//! Synthetic shared-GPU-source throughput probe, not a desktop freshness or
//! end-to-end latency benchmark. Every frame is a different uniform clear color;
//! this deliberately excludes CPU frame allocation/upload and is easy to encode.
#![cfg(windows)]

use std::{fs, time::Instant};

use mrd_encode_nvenc::NvencH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FrameMemoryKind, VideoCodec, VideoEncoder};
use windows::{
    core::Interface,
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
                ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_RESOURCE_MISC_SHARED, D3D11_SDK_VERSION,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                IDXGIResource,
            },
        },
    },
};

const WIDTH: usize = 2560;
const HEIGHT: usize = 1440;
const FPS: u32 = 144;
const BITRATE: u32 = 80_000_000;
const WARMUP: u64 = 32;
const SOURCE_SLOTS: usize = 3;

#[test]
#[ignore = "requires real NVIDIA NVENC and D3D11; run alone with --release --nocapture"]
fn perf_shared_h264_2k144_reports_latency_and_throughput() {
    let samples = std::env::var("MRD_COMPONENT_SAMPLES")
        .map(|value| {
            value
                .parse::<u64>()
                .expect("MRD_COMPONENT_SAMPLES must be an integer")
        })
        .unwrap_or(432);
    assert!(
        (144..=100_000).contains(&samples),
        "use at least 144 samples"
    );

    let (device, context) = create_device();
    let sources: Vec<_> = (0..SOURCE_SLOTS).map(|_| create_source(&device)).collect();
    let mut encoder = NvencH264Encoder::new_max_speed_with_bitrate(WIDTH, HEIGHT, FPS, BITRATE)
        .expect("strict probe requires a working P1 ultra-low-latency H264 NVENC encoder");
    assert_eq!(
        encoder.input_memory_kind(),
        FrameMemoryKind::D3D11SharedBgra
    );

    let mut latencies_ms = Vec::with_capacity(samples as usize);
    let mut output_count = 0_u64;
    let mut measured_output_count = 0_u64;
    let mut measured_output_bytes = 0_u64;
    let mut measured_keyframes = 0_u64;
    let mut measured_started = None;
    let mut tail_encode_ms = 0.0;

    // H264 currently keeps one output pending. One extra input retires the last
    // measured output; it is included in wall time but excluded from call stats.
    for index in 0..WARMUP + samples + 1 {
        if index == WARMUP {
            measured_started = Some(Instant::now());
        }
        let source = &sources[index as usize % SOURCE_SLOTS];
        let color = [
            (index % 251) as f32 / 250.0,
            ((index * 37) % 251) as f32 / 250.0,
            ((index * 101) % 251) as f32 / 250.0,
            1.0,
        ];
        unsafe {
            context.ClearRenderTargetView(&source.view, &color);
            context.Flush();
        }
        let frame = CapturedFrame::from_d3d11_shared_bgra(
            WIDTH,
            HEIGHT,
            timestamp_us(index),
            source.handle,
            (WIDTH * 4) as u32,
        );
        assert!(
            frame.data.is_empty(),
            "shared probe must not allocate CPU pixels"
        );
        let call_started = Instant::now();
        let outputs = encoder
            .encode(&frame)
            .expect("strict shared H264 encode must succeed");
        let call_ms = call_started.elapsed().as_secs_f64() * 1000.0;
        if (WARMUP..WARMUP + samples).contains(&index) {
            latencies_ms.push(call_ms);
        } else if index == WARMUP + samples {
            tail_encode_ms = call_ms;
        }
        for output in outputs {
            assert_eq!(output.codec, VideoCodec::H264);
            assert!(
                !output.bytes.is_empty(),
                "NVENC output must contain a bitstream"
            );
            assert_eq!(
                output.timestamp_us,
                timestamp_us(output_count),
                "output order/timestamp"
            );
            if (WARMUP..WARMUP + samples).contains(&output_count) {
                measured_output_count += 1;
                measured_output_bytes += output.bytes.len() as u64;
                measured_keyframes += u64::from(output.is_keyframe);
            }
            output_count += 1;
        }
    }
    let elapsed_seconds = measured_started
        .expect("measurement started")
        .elapsed()
        .as_secs_f64();
    assert_eq!(latencies_ms.len(), samples as usize);
    assert_eq!(
        output_count,
        WARMUP + samples,
        "only the final extra input may remain pending"
    );
    assert_eq!(
        measured_output_count, samples,
        "every measured input must produce output"
    );

    let teardown_started = Instant::now();
    drop(encoder); // Keep all source COM owners alive until pending work drains.
    let teardown_ms = teardown_started.elapsed().as_secs_f64() * 1000.0;
    latencies_ms.sort_by(f64::total_cmp);
    let p50 = percentile(&latencies_ms, 0.50);
    let p95 = percentile(&latencies_ms, 0.95);
    let throughput = measured_output_count as f64 / elapsed_seconds;
    let budget_ms = 1000.0 / f64::from(FPS);
    let report = serde_json::json!({
        "case": "encode.nvenc.shared.h264.2k144.synthetic_gpu",
        "source": "three shared BGRA textures; changing uniform GPU RTV clears",
        "scope": "unpaced component throughput; not desktop unique FPS, image quality, or end-to-end latency",
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "width": WIDTH, "height": HEIGHT, "fps_configured": FPS, "bitrate_bps": BITRATE,
        "preset": "P1", "tuning": "UltraLowLatency", "source_slots": SOURCE_SLOTS,
        "warmup_inputs": WARMUP, "measured_inputs": samples, "extra_tail_inputs": 1,
        "total_outputs": output_count, "measured_outputs": measured_output_count,
        "measured_output_bytes": measured_output_bytes, "measured_keyframes": measured_keyframes,
        "measured_loop_seconds_including_tail": elapsed_seconds,
        "completed_outputs_per_second": throughput,
        "encode_call_p50_ms": p50, "encode_call_p95_ms": p95,
        "encode_call_max_ms": latencies_ms.last().copied().unwrap(),
        "tail_encode_call_ms": tail_encode_ms, "teardown_ms": teardown_ms,
        "frame_budget_ms": budget_ms,
        "component_throughput_at_least_144": throughput >= f64::from(FPS),
        "encode_call_p95_within_frame_budget": p95 <= budget_ms,
    });
    let json = serde_json::to_string_pretty(&report).expect("serialize component result");
    println!("{json}");
    if let Ok(path) = std::env::var("MRD_COMPONENT_RESULT_PATH") {
        fs::write(path, &json).expect("write component result");
    }
}

fn timestamp_us(index: u64) -> u64 {
    index * 1_000_000 / u64::from(FPS)
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    sorted[(quantile * sorted.len() as f64).ceil().max(1.0) as usize - 1]
}

struct SharedSource {
    _texture: ID3D11Texture2D,
    view: ID3D11RenderTargetView,
    handle: isize,
}

fn create_device() -> (ID3D11Device, ID3D11DeviceContext) {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .expect("hardware D3D11 source device must initialize");
    (
        device.expect("D3D11 device"),
        context.expect("D3D11 context"),
    )
}

fn create_source(device: &ID3D11Device) -> SharedSource {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: WIDTH as u32,
        Height: HEIGHT as u32,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .expect("create shared GPU source texture");
    let texture = texture.expect("shared source texture");
    let mut view = None;
    unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut view)) }
        .expect("create source render target view");
    let dxgi: IDXGIResource = texture.cast().expect("shared DXGI resource");
    let handle = unsafe { dxgi.GetSharedHandle() }.expect("shared source handle");
    SharedSource {
        _texture: texture,
        view: view.expect("source RTV"),
        handle: handle.0 as isize,
    }
}
