#![cfg_attr(target_os = "macos", allow(deprecated, unexpected_cfgs))]

#[cfg(target_os = "macos")]
use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoEncoder};
#[cfg(target_os = "macos")]
use mrd_render::{RenderFrame, RenderTarget, RendererInstance, RendererSnapshot};
#[cfg(target_os = "macos")]
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::{
    env, fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
#[path = "../render_proxy.rs"]
mod render_proxy;

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macos_metal_present_probe is only available on macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn run() -> Result<()> {
    let config = ProbeConfig::from_args(env::args().skip(1).collect())?;
    if config.serve_render_proxy {
        return run_render_proxy_server(&config);
    }

    let window = MacosProbeWindow::new(
        config.width,
        config.height,
        config.show_window,
        config.child_view,
        config.activate_app,
        config.borderless_window,
        config.fullscreen_window,
    )?;
    window.pump_events()?;

    let mut renderer = mrd_render_macos::MacosMetalRenderer::new()
        .context("create macOS Metal probe renderer failed")?;
    renderer
        .attach_target(RenderTarget::WindowHandle(window.ns_view_value()))
        .context("attach macOS Metal probe renderer failed")?;

    let template = build_probe_frame(config.width, config.height);
    let samples = if config.render_thread {
        anyhow::ensure!(
            !config.pump_events,
            "--render-thread currently requires --no-pump-events"
        );
        let thread_config = config.clone();
        thread::spawn(move || collect_probe_samples(&thread_config, &template, &mut renderer, None))
            .join()
            .map_err(|_| anyhow::anyhow!("macOS Metal render probe thread panicked"))??
    } else {
        collect_probe_samples(&config, &template, &mut renderer, Some(&window))?
    };

    if config.pump_events {
        window.pump_events()?;
    }

    let measured_total_ms: f64 = samples.upload_present_ms.iter().sum();
    let measured_presented_frames = samples
        .last_snapshot
        .presented_frame_count
        .saturating_sub(samples.measured_start_presented_frames);
    let measured_uploaded_frames = samples
        .last_snapshot
        .uploaded_frame_count
        .saturating_sub(samples.measured_start_uploaded_frames);
    let report = ProbeReport {
        frames_requested: config.frames,
        warmup_frames: config.warmup_frames,
        measured_frames: samples.upload_present_ms.len(),
        width: config.width,
        height: config.height,
        codec: config.codec.as_str(),
        fps: config.fps,
        bitrate_mbps: config.bitrate_mbps,
        show_window: config.show_window,
        child_view: config.child_view,
        activate_app: config.activate_app,
        borderless_window: config.borderless_window,
        fullscreen_window: config.fullscreen_window,
        pump_events: config.pump_events,
        render_thread: config.render_thread,
        env: ProbeEnv {
            metal_display_sync: env::var("MRD_MACOS_METAL_DISPLAY_SYNC").ok(),
            metal_max_drawable_count: env::var("MRD_MACOS_METAL_MAX_DRAWABLE_COUNT").ok(),
            metal_nv12_buffer_upload: env::var("MRD_MACOS_METAL_NV12_BUFFER_UPLOAD").ok(),
            metal_invalidate_view_on_geometry_sync: env::var(
                "MRD_MACOS_METAL_INVALIDATE_VIEW_ON_GEOMETRY_SYNC",
            )
            .ok(),
            metal_geometry_sync_interval_ms: env::var("MRD_MACOS_METAL_GEOMETRY_SYNC_INTERVAL_MS")
                .ok(),
        },
        renderer: RendererProbeSnapshot {
            attached_to_target: samples.last_snapshot.attached_to_target,
            uploaded_frame_count: samples.last_snapshot.uploaded_frame_count,
            presented_frame_count: samples.last_snapshot.presented_frame_count,
            present_skipped_count: samples.last_snapshot.present_skipped_count,
            last_present_status: samples.last_snapshot.last_present_status,
            swap_chain_present_mode: samples.last_snapshot.swap_chain_present_mode,
            swap_chain_max_frame_latency: samples.last_snapshot.swap_chain_max_frame_latency,
            swap_chain_allow_tearing: samples.last_snapshot.swap_chain_allow_tearing,
            last_width: samples.last_snapshot.last_width,
            last_height: samples.last_snapshot.last_height,
        },
        encoded_access_units: samples.encoded_access_units,
        encoded_keyframes: samples.encoded_keyframes,
        encoded_bytes: samples.encoded_bytes,
        measured_uploaded_frames,
        measured_presented_frames,
        measured_upload_present_fps: fps(samples.upload_present_ms.len(), measured_total_ms),
        measured_presented_fps: fps(measured_presented_frames as usize, measured_total_ms),
        clone_ms: SampleSummary::from_samples(&samples.clone_ms),
        upload_present_ms: SampleSummary::from_samples(&samples.upload_present_ms),
        next_drawable_ms: SampleSummary::from_samples(&samples.next_drawable_ms),
        encode_commit_ms: SampleSummary::from_samples(&samples.encode_commit_ms),
        draw_present_ms: SampleSummary::from_samples(&samples.draw_present_ms),
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_render_proxy_server(config: &ProbeConfig) -> Result<()> {
    let session_id = config
        .session_id
        .as_deref()
        .context("--serve-render-proxy requires --session-id")?;
    let surface_id = config
        .surface_id
        .as_deref()
        .context("--serve-render-proxy requires --surface-id")?;
    let window = MacosProbeWindow::new(
        config.width,
        config.height,
        config.show_window,
        config.child_view,
        config.activate_app,
        config.borderless_window,
        config.fullscreen_window,
    )?;
    window.pump_events()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create macOS render proxy runtime failed")?;
    let _runtime_guard = runtime.enter();
    let registry = render_proxy::RenderProxyRegistry::default();
    let render_proxy_endpoint = registry
        .attach_surface(session_id, surface_id, window.ns_view_value())
        .map_err(anyhow::Error::msg)?
        .context("macOS render proxy did not return an endpoint")?;
    let report = RenderProxyReadyReport {
        session_id,
        surface_id,
        backend: "macos",
        window_handle: window.ns_view_value() as i64,
        render_proxy_endpoint: &render_proxy_endpoint,
        width: config.width,
        height: config.height,
    };
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(ready_path) = config.ready_path.as_ref() {
        fs::write(ready_path, format!("{report_json}\n")).with_context(|| {
            format!(
                "write render proxy ready report failed: {}",
                ready_path.display()
            )
        })?;
    }
    println!("{report_json}");

    let deadline = Instant::now() + Duration::from_secs(config.hold_secs);
    while Instant::now() < deadline {
        if config.pump_events {
            window.pump_events()?;
        }
        thread::sleep(Duration::from_millis(16));
    }
    registry.detach_surface(session_id, surface_id);
    Ok(())
}

#[cfg(target_os = "macos")]
fn collect_probe_samples(
    config: &ProbeConfig,
    template: &RenderFrame,
    renderer: &mut mrd_render_macos::MacosMetalRenderer,
    window: Option<&MacosProbeWindow>,
) -> Result<ProbeRunSamples> {
    let total_frames = config.frames + config.warmup_frames;
    let mut compressed_encoder = create_compressed_probe_encoder(config)?;
    let mut samples = ProbeRunSamples {
        clone_ms: Vec::with_capacity(config.frames),
        upload_present_ms: Vec::with_capacity(config.frames),
        next_drawable_ms: Vec::with_capacity(config.frames),
        encode_commit_ms: Vec::with_capacity(config.frames),
        draw_present_ms: Vec::with_capacity(config.frames),
        last_snapshot: renderer.snapshot(),
        measured_start_uploaded_frames: 0,
        measured_start_presented_frames: 0,
        encoded_access_units: 0,
        encoded_keyframes: 0,
        encoded_bytes: 0,
    };

    for frame_index in 0..total_frames {
        if config.pump_events {
            if let Some(window) = window {
                window.pump_events()?;
            }
        }
        if frame_index == config.warmup_frames {
            let snapshot = renderer.snapshot();
            samples.measured_start_uploaded_frames = snapshot.uploaded_frame_count;
            samples.measured_start_presented_frames = snapshot.presented_frame_count;
        }

        let clone_started = Instant::now();
        let frame = if compressed_encoder.is_none() {
            Some(template.clone())
        } else {
            None
        };
        let captured_frame = if compressed_encoder.is_some() {
            Some(build_probe_captured_frame(
                config.width,
                config.height,
                frame_timestamp_us(frame_index, config.fps),
                frame_index as u8,
            ))
        } else {
            None
        };
        let frame_clone_ms = elapsed_ms(clone_started.elapsed());

        let upload_started = Instant::now();
        if let Some(encoder) = compressed_encoder.as_mut() {
            let units = encoder
                .encode(captured_frame.as_ref().expect("compressed frame"))
                .with_context(|| format!("encode probe frame {frame_index} failed"))?;
            for unit in units {
                samples.encoded_access_units = samples.encoded_access_units.saturating_add(1);
                samples.encoded_bytes = samples
                    .encoded_bytes
                    .saturating_add(u64::try_from(unit.bytes.len()).unwrap_or(u64::MAX));
                if unit.is_keyframe {
                    samples.encoded_keyframes = samples.encoded_keyframes.saturating_add(1);
                }
                match config.codec {
                    ProbeCodec::Bgra => unreachable!("BGRA probe does not use compressed encoder"),
                    ProbeCodec::H264 => renderer
                        .upload_h264_access_unit(
                            config.width,
                            config.height,
                            unit.timestamp_us,
                            bytes::Bytes::from(unit.bytes),
                        )
                        .with_context(|| {
                            format!("present probe H.264 access unit {frame_index} failed")
                        })?,
                    ProbeCodec::Hevc => renderer
                        .upload_hevc_access_unit(
                            config.width,
                            config.height,
                            unit.timestamp_us,
                            bytes::Bytes::from(unit.bytes),
                        )
                        .with_context(|| {
                            format!("present probe HEVC access unit {frame_index} failed")
                        })?,
                }
            }
        } else {
            renderer
                .upload_frame(frame.expect("BGRA probe frame"))
                .with_context(|| format!("present probe frame {frame_index} failed"))?;
        }
        let frame_upload_present_ms = elapsed_ms(upload_started.elapsed());
        samples.last_snapshot = renderer.snapshot();

        if frame_index >= config.warmup_frames {
            samples.clone_ms.push(frame_clone_ms);
            samples.upload_present_ms.push(frame_upload_present_ms);
            if let Some(value) = samples.last_snapshot.last_render_wait_for_drawable_ms {
                samples.next_drawable_ms.push(value);
            }
            if let Some(value) = samples.last_snapshot.last_render_encode_commit_ms {
                samples.encode_commit_ms.push(value);
            }
            if let Some(value) = samples.last_snapshot.last_render_draw_present_ms {
                samples.draw_present_ms.push(value);
            }
        }
    }

    Ok(samples)
}

#[cfg(target_os = "macos")]
struct ProbeRunSamples {
    clone_ms: Vec<f64>,
    upload_present_ms: Vec<f64>,
    next_drawable_ms: Vec<f64>,
    encode_commit_ms: Vec<f64>,
    draw_present_ms: Vec<f64>,
    last_snapshot: RendererSnapshot,
    measured_start_uploaded_frames: u64,
    measured_start_presented_frames: u64,
    encoded_access_units: u64,
    encoded_keyframes: u64,
    encoded_bytes: u64,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct ProbeConfig {
    frames: usize,
    warmup_frames: usize,
    width: usize,
    height: usize,
    codec: ProbeCodec,
    fps: u32,
    bitrate_mbps: u32,
    show_window: bool,
    child_view: bool,
    activate_app: bool,
    borderless_window: bool,
    fullscreen_window: bool,
    pump_events: bool,
    render_thread: bool,
    serve_render_proxy: bool,
    ready_path: Option<PathBuf>,
    session_id: Option<String>,
    surface_id: Option<String>,
    hold_secs: u64,
}

#[cfg(target_os = "macos")]
impl ProbeConfig {
    fn from_args(args: Vec<String>) -> Result<Self> {
        let mut config = Self {
            frames: 300,
            warmup_frames: 30,
            width: 1920,
            height: 1080,
            codec: ProbeCodec::Bgra,
            fps: 60,
            bitrate_mbps: 12,
            show_window: true,
            child_view: false,
            activate_app: false,
            borderless_window: false,
            fullscreen_window: false,
            pump_events: true,
            render_thread: false,
            serve_render_proxy: false,
            ready_path: None,
            session_id: None,
            surface_id: None,
            hold_secs: 600,
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--frames" => {
                    index += 1;
                    config.frames = parse_usize_arg(&args, index, "--frames")?;
                }
                "--warmup" => {
                    index += 1;
                    config.warmup_frames = parse_usize_arg(&args, index, "--warmup")?;
                }
                "--width" => {
                    index += 1;
                    config.width = parse_usize_arg(&args, index, "--width")?;
                }
                "--height" => {
                    index += 1;
                    config.height = parse_usize_arg(&args, index, "--height")?;
                }
                "--codec" => {
                    index += 1;
                    config.codec = ProbeCodec::parse(
                        args.get(index)
                            .with_context(|| "--codec requires a value".to_string())?,
                    )?;
                }
                "--fps" => {
                    index += 1;
                    config.fps = parse_u32_arg(&args, index, "--fps")?;
                }
                "--bitrate-mbps" => {
                    index += 1;
                    config.bitrate_mbps = parse_u32_arg(&args, index, "--bitrate-mbps")?;
                }
                "--show" => config.show_window = true,
                "--hide" => config.show_window = false,
                "--child-view" => config.child_view = true,
                "--content-view" => config.child_view = false,
                "--activate-app" => config.activate_app = true,
                "--no-activate-app" => config.activate_app = false,
                "--borderless" => config.borderless_window = true,
                "--titled" => config.borderless_window = false,
                "--fullscreen-window" => config.fullscreen_window = true,
                "--windowed" => config.fullscreen_window = false,
                "--pump-events" => config.pump_events = true,
                "--no-pump-events" => config.pump_events = false,
                "--render-thread" => config.render_thread = true,
                "--main-thread" => config.render_thread = false,
                "--serve-render-proxy" => config.serve_render_proxy = true,
                "--ready-path" => {
                    index += 1;
                    config.ready_path =
                        Some(PathBuf::from(args.get(index).with_context(|| {
                            "--ready-path requires a value".to_string()
                        })?));
                }
                "--session-id" => {
                    index += 1;
                    config.session_id = Some(
                        args.get(index)
                            .with_context(|| "--session-id requires a value".to_string())?
                            .to_string(),
                    );
                }
                "--surface-id" => {
                    index += 1;
                    config.surface_id = Some(
                        args.get(index)
                            .with_context(|| "--surface-id requires a value".to_string())?
                            .to_string(),
                    );
                }
                "--hold-secs" => {
                    index += 1;
                    config.hold_secs = parse_u64_arg(&args, index, "--hold-secs")?;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                value => anyhow::bail!("unknown argument: {value}"),
            }
            index += 1;
        }
        anyhow::ensure!(config.frames > 0, "--frames must be greater than 0");
        anyhow::ensure!(config.width > 0, "--width must be greater than 0");
        anyhow::ensure!(config.height > 0, "--height must be greater than 0");
        anyhow::ensure!(config.width % 2 == 0, "--width must be even");
        anyhow::ensure!(config.height % 2 == 0, "--height must be even");
        anyhow::ensure!(config.fps > 0, "--fps must be greater than 0");
        anyhow::ensure!(
            config.bitrate_mbps > 0,
            "--bitrate-mbps must be greater than 0"
        );
        if config.serve_render_proxy {
            anyhow::ensure!(
                config
                    .session_id
                    .as_deref()
                    .map_or(false, |value| !value.is_empty()),
                "--serve-render-proxy requires --session-id"
            );
            anyhow::ensure!(
                config
                    .surface_id
                    .as_deref()
                    .map_or(false, |value| !value.is_empty()),
                "--serve-render-proxy requires --surface-id"
            );
            anyhow::ensure!(config.hold_secs > 0, "--hold-secs must be greater than 0");
        }
        Ok(config)
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeCodec {
    Bgra,
    H264,
    Hevc,
}

#[cfg(target_os = "macos")]
impl ProbeCodec {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bgra" | "raw" | "raw_bgra" => Ok(Self::Bgra),
            "h264" | "h.264" | "avc" => Ok(Self::H264),
            "hevc" | "h265" | "h.265" => Ok(Self::Hevc),
            _ => anyhow::bail!("unsupported --codec value: {value}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Bgra => "bgra",
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_usize_arg(args: &[String], index: usize, name: &str) -> Result<usize> {
    let value = args
        .get(index)
        .with_context(|| format!("{name} requires a value"))?;
    value
        .parse::<usize>()
        .with_context(|| format!("invalid {name} value: {value}"))
}

#[cfg(target_os = "macos")]
fn parse_u32_arg(args: &[String], index: usize, name: &str) -> Result<u32> {
    let value = args
        .get(index)
        .with_context(|| format!("{name} requires a value"))?;
    value
        .parse::<u32>()
        .with_context(|| format!("invalid {name} value: {value}"))
}

#[cfg(target_os = "macos")]
fn parse_u64_arg(args: &[String], index: usize, name: &str) -> Result<u64> {
    let value = args
        .get(index)
        .with_context(|| format!("{name} requires a value"))?;
    value
        .parse::<u64>()
        .with_context(|| format!("invalid {name} value: {value}"))
}

#[cfg(target_os = "macos")]
fn print_help() {
    println!(
        "Usage: macos_metal_present_probe [--frames N] [--warmup N] [--width N] [--height N] [--codec bgra|h264|hevc] [--fps N] [--bitrate-mbps N] [--show|--hide] [--content-view|--child-view] [--activate-app|--no-activate-app] [--borderless|--titled] [--fullscreen-window|--windowed] [--pump-events|--no-pump-events] [--main-thread|--render-thread] [--serve-render-proxy --session-id ID --surface-id ID --ready-path PATH --hold-secs N]"
    );
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct ProbeReport {
    frames_requested: usize,
    warmup_frames: usize,
    measured_frames: usize,
    width: usize,
    height: usize,
    codec: &'static str,
    fps: u32,
    bitrate_mbps: u32,
    show_window: bool,
    child_view: bool,
    activate_app: bool,
    borderless_window: bool,
    fullscreen_window: bool,
    pump_events: bool,
    render_thread: bool,
    env: ProbeEnv,
    renderer: RendererProbeSnapshot,
    encoded_access_units: u64,
    encoded_keyframes: u64,
    encoded_bytes: u64,
    measured_uploaded_frames: u64,
    measured_presented_frames: u64,
    measured_upload_present_fps: Option<f64>,
    measured_presented_fps: Option<f64>,
    clone_ms: SampleSummary,
    upload_present_ms: SampleSummary,
    next_drawable_ms: SampleSummary,
    encode_commit_ms: SampleSummary,
    draw_present_ms: SampleSummary,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct RenderProxyReadyReport<'a> {
    session_id: &'a str,
    surface_id: &'a str,
    backend: &'a str,
    window_handle: i64,
    render_proxy_endpoint: &'a str,
    width: usize,
    height: usize,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct ProbeEnv {
    metal_display_sync: Option<String>,
    metal_max_drawable_count: Option<String>,
    metal_nv12_buffer_upload: Option<String>,
    metal_invalidate_view_on_geometry_sync: Option<String>,
    metal_geometry_sync_interval_ms: Option<String>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct RendererProbeSnapshot {
    attached_to_target: bool,
    uploaded_frame_count: u64,
    presented_frame_count: u64,
    present_skipped_count: u64,
    last_present_status: Option<String>,
    swap_chain_present_mode: Option<String>,
    swap_chain_max_frame_latency: Option<u32>,
    swap_chain_allow_tearing: Option<bool>,
    last_width: usize,
    last_height: usize,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct SampleSummary {
    count: usize,
    avg: Option<f64>,
    p50: Option<f64>,
    p95: Option<f64>,
    max: Option<f64>,
}

#[cfg(target_os = "macos")]
impl SampleSummary {
    fn from_samples(samples: &[f64]) -> Self {
        let mut values = samples
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.total_cmp(right));
        if values.is_empty() {
            return Self {
                count: 0,
                avg: None,
                p50: None,
                p95: None,
                max: None,
            };
        }

        let sum: f64 = values.iter().sum();
        Self {
            count: values.len(),
            avg: Some(round_ms(sum / values.len() as f64)),
            p50: Some(round_ms(percentile(&values, 0.50))),
            p95: Some(round_ms(percentile(&values, 0.95))),
            max: values.last().copied().map(round_ms),
        }
    }
}

#[cfg(target_os = "macos")]
fn percentile(sorted_samples: &[f64], quantile: f64) -> f64 {
    let index = ((sorted_samples.len() - 1) as f64 * quantile).ceil() as usize;
    sorted_samples[index.min(sorted_samples.len() - 1)]
}

#[cfg(target_os = "macos")]
fn fps(frames: usize, total_ms: f64) -> Option<f64> {
    if frames == 0 || !total_ms.is_finite() || total_ms <= 0.0 {
        return None;
    }
    Some(round_ms(frames as f64 * 1000.0 / total_ms))
}

#[cfg(target_os = "macos")]
fn elapsed_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(target_os = "macos")]
fn round_ms(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(target_os = "macos")]
fn create_compressed_probe_encoder(config: &ProbeConfig) -> Result<Option<Box<dyn VideoEncoder>>> {
    let bitrate = config.bitrate_mbps.saturating_mul(1_000_000).max(1);
    match config.codec {
        ProbeCodec::Bgra => Ok(None),
        ProbeCodec::H264 => Ok(Some(Box::new(
            mrd_codec_videotoolbox::VideoToolboxH264Encoder::new_with_bitrate(
                config.width,
                config.height,
                config.fps,
                bitrate,
            )
            .context("create VideoToolbox H.264 probe encoder failed")?,
        ))),
        ProbeCodec::Hevc => Ok(Some(Box::new(
            mrd_codec_videotoolbox::VideoToolboxHevcEncoder::new_with_bitrate(
                config.width,
                config.height,
                config.fps,
                bitrate,
            )
            .context("create VideoToolbox HEVC probe encoder failed")?,
        ))),
    }
}

#[cfg(target_os = "macos")]
fn frame_timestamp_us(frame_index: usize, fps: u32) -> u64 {
    let fps = u64::from(fps.max(1));
    (frame_index as u64).saturating_mul(1_000_000) / fps
}

#[cfg(target_os = "macos")]
fn build_probe_captured_frame(
    width: usize,
    height: usize,
    timestamp_us: u64,
    frame_index: u8,
) -> CapturedFrame {
    CapturedFrame::from_cpu(
        width,
        height,
        FramePixelFormat::Bgra32,
        timestamp_us,
        build_probe_bgra(width, height, frame_index),
    )
}

#[cfg(target_os = "macos")]
fn build_probe_frame(width: usize, height: usize) -> RenderFrame {
    RenderFrame::from_bgra32(width, height, build_probe_bgra(width, height, 0))
}

#[cfg(target_os = "macos")]
fn build_probe_bgra(width: usize, height: usize, frame_index: u8) -> Vec<u8> {
    let mut bgra = vec![0_u8; width * height * 4];
    let motion = usize::from(frame_index) % 96;
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let band = (((x + motion) / 48) + ((y + motion) / 48)) % 3;
            let edge = x < 16 || y < 16 || x + 16 >= width || y + 16 >= height;
            let (r, g, b) = if edge {
                (255, 255, 255)
            } else if band == 0 {
                (24, 190, 255)
            } else if band == 1 {
                (255, 82, 120)
            } else {
                (42, 230, 150)
            };
            bgra[offset] = b;
            bgra[offset + 1] = g;
            bgra[offset + 2] = r;
            bgra[offset + 3] = 255;
        }
    }
    bgra
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
struct MacosProbeWindow {
    ns_window: isize,
    ns_view: isize,
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
impl MacosProbeWindow {
    fn new(
        width: usize,
        height: usize,
        show_window: bool,
        child_view: bool,
        activate_app: bool,
        borderless_window: bool,
        fullscreen_window: bool,
    ) -> Result<Self> {
        run_on_macos_main_thread(move || unsafe {
            use cocoa::{
                appkit::{
                    NSApp, NSApplication, NSBackingStoreBuffered, NSView, NSWindow,
                    NSWindowStyleMask,
                },
                base::{id, nil, NO, YES},
                foundation::{NSPoint, NSRect, NSSize, NSString},
            };
            use objc::{msg_send, sel, sel_impl};

            let clamped_width = width.clamp(320, 3840) as f64;
            let clamped_height = height.clamp(240, 2160) as f64;
            let frame = if fullscreen_window {
                main_screen_frame().unwrap_or_else(|| {
                    NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        NSSize::new(clamped_width, clamped_height),
                    )
                })
            } else {
                NSRect::new(
                    NSPoint::new(80.0, 80.0),
                    NSSize::new(clamped_width, clamped_height),
                )
            };
            let style = if borderless_window || fullscreen_window {
                NSWindowStyleMask::NSBorderlessWindowMask
            } else {
                NSWindowStyleMask::NSTitledWindowMask
                    | NSWindowStyleMask::NSClosableWindowMask
                    | NSWindowStyleMask::NSResizableWindowMask
            };
            let window: id = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                frame,
                style,
                NSBackingStoreBuffered,
                NO,
            );
            if window == nil {
                anyhow::bail!("create macOS Metal probe window failed");
            }

            let content_frame = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
            let view: id = NSView::alloc(nil).initWithFrame_(content_frame);
            if view == nil {
                let _: () = msg_send![window, release];
                anyhow::bail!("create macOS Metal probe NSView failed");
            }

            let _: () = msg_send![window, setReleasedWhenClosed: NO];
            view.setWantsLayer(YES);
            if child_view {
                let content_view: id = msg_send![window, contentView];
                if content_view == nil {
                    let _: () = msg_send![view, release];
                    let _: () = msg_send![window, release];
                    anyhow::bail!("macOS Metal probe window has no contentView");
                }
                let _: () = msg_send![content_view, addSubview: view];
            } else {
                window.setContentView_(view);
            }
            let title = NSString::alloc(nil).init_str("MRD Metal Present Probe");
            window.setTitle_(title);
            let _: () = msg_send![title, release];
            if !fullscreen_window {
                window.center();
            }
            if show_window {
                if activate_app {
                    let app = NSApp();
                    app.activateIgnoringOtherApps_(YES);
                }
                window.makeKeyAndOrderFront_(nil);
            } else {
                let _: () = msg_send![window, orderOut: nil];
            }

            Ok(Self {
                ns_window: window as isize,
                ns_view: view as isize,
            })
        })
    }

    fn ns_view_value(&self) -> isize {
        self.ns_view
    }

    fn pump_events(&self) -> Result<()> {
        let ns_window = self.ns_window;
        run_on_macos_main_thread(move || unsafe {
            use cocoa::{
                appkit::{NSApp, NSApplication},
                base::{id, nil, YES},
                foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSUInteger},
            };
            use objc::{msg_send, sel, sel_impl};

            let app = NSApp();
            let pool = NSAutoreleasePool::new(nil);
            loop {
                let event: id = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                    usize::MAX as NSUInteger,
                    nil,
                    NSDefaultRunLoopMode,
                    YES,
                );
                if event == nil {
                    break;
                }
                app.sendEvent_(event);
            }
            let _: () = msg_send![app, updateWindows];
            let window = ns_window as id;
            if window != nil {
                let _: () = msg_send![window, displayIfNeeded];
            }
            pool.drain();
            Ok(())
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
unsafe fn main_screen_frame() -> Option<cocoa::foundation::NSRect> {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};

    let screen: id = msg_send![class!(NSScreen), mainScreen];
    if screen == nil {
        return None;
    }
    Some(msg_send![screen, frame])
}

#[cfg(target_os = "macos")]
impl Drop for MacosProbeWindow {
    fn drop(&mut self) {
        let ns_window = self.ns_window;
        let ns_view = self.ns_view;
        let _ = run_on_macos_main_thread(move || unsafe {
            use cocoa::base::{id, nil};
            use objc::{msg_send, sel, sel_impl};

            let window = ns_window as id;
            let view = ns_view as id;
            if window != nil {
                let _: () = msg_send![window, orderOut: nil];
                let _: () = msg_send![window, close];
                let _: () = msg_send![window, release];
            }
            if view != nil {
                let _: () = msg_send![view, release];
            }
            Ok(())
        });
    }
}

#[cfg(target_os = "macos")]
fn run_on_macos_main_thread<T, F>(f: F) -> Result<T>
where
    T: Send,
    F: FnOnce() -> Result<T> + Send,
{
    if unsafe { pthread_main_np() } != 0 {
        return f();
    }

    let mut result = None;
    dispatch2::DispatchQueue::main().exec_sync(|| {
        result = Some(f());
    });
    result.unwrap_or_else(|| anyhow::bail!("macOS main-thread task did not return"))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_main_np() -> std::ffi::c_int;
}
