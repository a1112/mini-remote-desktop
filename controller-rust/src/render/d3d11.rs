use super::RendererConfig;
use crate::thread_tuning::{apply_current_thread_tuning, ThreadRole};
use crate::video::decoder::{DecodedFrame, DecodedFrameData};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use windows::core::PCSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR};

#[derive(Default)]
struct SharedFrame {
    latest: Option<DecodedFrame>,
    sequence: u64,
}

#[derive(Debug, Clone)]
pub struct OverlaySharedStats {
    pub selected_transport: String,
    pub media_path: String,
    pub decoder_backend: String,
    pub decoded_frames: u64,
    pub decode_fps: f64,
    pub avg_decode_ms: f64,
    pub p95_decode_ms: f64,
    pub jitter_ms: f64,
    pub e2e_avg_ms: f64,
    pub e2e_p50_ms: f64,
    pub e2e_p95_ms: f64,
    pub e2e_p99_ms: f64,
    pub decode_failures: u64,
    pub last_decode_error: String,
}

impl Default for OverlaySharedStats {
    fn default() -> Self {
        Self {
            selected_transport: "unknown".to_string(),
            media_path: "webrtc".to_string(),
            decoder_backend: "h264".to_string(),
            decoded_frames: 0,
            decode_fps: 0.0,
            avg_decode_ms: -1.0,
            p95_decode_ms: -1.0,
            jitter_ms: -1.0,
            e2e_avg_ms: -1.0,
            e2e_p50_ms: -1.0,
            e2e_p95_ms: -1.0,
            e2e_p99_ms: -1.0,
            decode_failures: 0,
            last_decode_error: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayPanel {
    Overview,
    Pipeline,
    Transport,
    Debug,
}

impl OverlayPanel {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Pipeline => "Pipeline",
            Self::Transport => "Transport",
            Self::Debug => "Debug",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct OverlayRenderMetrics {
    rendered_frames: u64,
    render_fps: f64,
    received_frames: u64,
    receive_fps: f64,
    present_avg_ms: f64,
    present_p50_ms: f64,
    present_p95_ms: f64,
    present_p99_ms: f64,
}

#[derive(Debug)]
struct OverlayUiState {
    collapsed: bool,
    panel: OverlayPanel,
    last_text: String,
    res_idx: usize,
    win_idx: usize,
    br_idx: usize,
    cap_idx: usize,
    enc_idx: usize,
    control_queue: Arc<Mutex<Vec<OverlaySwitchCommand>>>,
}

impl OverlayUiState {
    fn new(control_queue: Arc<Mutex<Vec<OverlaySwitchCommand>>>) -> Self {
        Self {
            collapsed: false,
            panel: OverlayPanel::Overview,
            last_text: String::new(),
            res_idx: 1,
            win_idx: 0,
            br_idx: 2,
            cap_idx: 0,
            enc_idx: 0,
            control_queue,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OverlaySwitchField {
    Resolution,
    CaptureWindow,
    Bitrate,
    CaptureBackend,
    Encoder,
}

#[derive(Debug, Clone)]
pub struct OverlaySwitchCommand {
    pub field: OverlaySwitchField,
    pub value: String,
}

const RES_PRESETS: [(&str, &str, &str); 5] = [
    ("native", "0x0", "RES:native"),
    ("1080p", "1920x1080", "RES:1080p"),
    ("2k", "2560x1440", "RES:2k"),
    ("4k", "3840x2160", "RES:4k"),
    ("720p", "1280x720", "RES:720p"),
];
const WIN_PRESETS: [(&str, &str); 2] = [("auto", "WIN:auto"), ("foreground", "WIN:fg")];
const BR_PRESETS: [(&str, &str); 5] = [
    ("8000", "BR:8M"),
    ("12000", "BR:12M"),
    ("20000", "BR:20M"),
    ("30000", "BR:30M"),
    ("50000", "BR:50M"),
];
const CAP_PRESETS: [(&str, &str); 3] = [("dxgi", "CAP:dxgi"), ("wgc", "CAP:wgc"), ("auto", "CAP:auto")];
const ENC_PRESETS: [(&str, &str); 3] = [("nvenc", "ENC:nvenc"), ("openh264", "ENC:openh264"), ("auto", "ENC:auto")];

const ID_BTN_COLLAPSE: i32 = 101;
const ID_BTN_COPY: i32 = 102;
const ID_BTN_OVERVIEW: i32 = 103;
const ID_BTN_PIPELINE: i32 = 104;
const ID_BTN_TRANSPORT: i32 = 105;
const ID_BTN_DEBUG: i32 = 106;
const ID_BTN_CLOSE: i32 = 107;
const ID_BTN_RESOLUTION: i32 = 108;
const ID_BTN_WINDOW: i32 = 109;
const ID_BTN_BITRATE: i32 = 110;
const ID_BTN_CAPTURE: i32 = 111;
const ID_BTN_ENCODER: i32 = 112;
const ID_EDIT_PANEL: i32 = 201;

pub struct D3D11Renderer {
    window: HWND,
    frame_count: Arc<AtomicU64>,
    video_frames_received: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    shared_frame: Arc<Mutex<SharedFrame>>,
    overlay_stats: Arc<Mutex<OverlaySharedStats>>,
    overlay_control_queue: Arc<Mutex<Vec<OverlaySwitchCommand>>>,
}

#[derive(Clone)]
pub struct D3D11FrameSink {
    video_frames_received: Arc<AtomicU64>,
    shared_frame: Arc<Mutex<SharedFrame>>,
}

impl D3D11FrameSink {
    pub fn submit(&self, frame: DecodedFrame) {
        self.video_frames_received.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut shared) = self.shared_frame.lock() {
            shared.sequence = shared.sequence.wrapping_add(1);
            shared.latest = Some(frame);
        }
    }
}

impl D3D11Renderer {
    pub fn new(config: RendererConfig) -> Result<Self> {
        let video_frames_received = Arc::new(AtomicU64::new(0));
        Self::new_with_stats(config, video_frames_received)
    }

    pub fn new_with_stats(
        config: RendererConfig,
        video_frames_received: Arc<AtomicU64>,
    ) -> Result<Self> {
        let frame_count = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let shared_frame = Arc::new(Mutex::new(SharedFrame::default()));
        let overlay_stats = Arc::new(Mutex::new(OverlaySharedStats::default()));
        let overlay_control_queue = Arc::new(Mutex::new(Vec::<OverlaySwitchCommand>::new()));
        let (window_tx, window_rx) = std::sync::mpsc::sync_channel::<std::result::Result<isize, String>>(1);

        let frame_count_clone = frame_count.clone();
        let video_frames_clone = video_frames_received.clone();
        let running_clone = running.clone();
        let shared_frame_clone = shared_frame.clone();
        let overlay_stats_clone = overlay_stats.clone();
        let control_queue_clone = overlay_control_queue.clone();
        let config_clone = config.clone();
        thread::spawn(move || {
            let (_thread_tuning, _thread_tuning_guard) = apply_current_thread_tuning(ThreadRole::Render);

            let window = match Self::create_window(
                config_clone.window_width,
                config_clone.window_height,
                control_queue_clone.clone(),
            ) {
                Ok(w) => {
                    let _ = window_tx.send(Ok(w.0 as isize));
                    w
                }
                Err(e) => {
                    let _ = window_tx.send(Err(e.to_string()));
                    running_clone.store(false, Ordering::Relaxed);
                    return;
                }
            };

            if let Err(e) = Self::render_loop(
                window,
                frame_count_clone,
                video_frames_clone,
                running_clone,
                shared_frame_clone,
                overlay_stats_clone,
                config_clone,
            ) {
                error!(error = %e, "render loop failed");
            }
        });

        let window = match window_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(raw)) => HWND(raw as *mut _),
            Ok(Err(e)) => return Err(anyhow::anyhow!("create window on render thread failed: {e}")),
            Err(e) => return Err(anyhow::anyhow!("wait render thread window handle failed: {e}")),
        };

        info!("D3D11 video renderer initialized");
        Ok(Self {
            window,
            frame_count,
            video_frames_received,
            running,
            shared_frame,
            overlay_stats,
            overlay_control_queue,
        })
    }

    pub fn submit_decoded_frame(&self, frame: DecodedFrame) {
        self.frame_sink().submit(frame);
    }

    pub fn frame_sink(&self) -> D3D11FrameSink {
        D3D11FrameSink {
            video_frames_received: self.video_frames_received.clone(),
            shared_frame: self.shared_frame.clone(),
        }
    }

    pub fn overlay_stats_handle(&self) -> Arc<Mutex<OverlaySharedStats>> {
        self.overlay_stats.clone()
    }

    pub fn drain_overlay_switch_commands(&self) -> Vec<OverlaySwitchCommand> {
        if let Ok(mut q) = self.overlay_control_queue.lock() {
            std::mem::take(&mut *q)
        } else {
            Vec::new()
        }
    }

    pub fn update_video_stats(&self, _frame: &super::super::webrtc::peer::VideoFrame) {
        self.video_frames_received.fetch_add(1, Ordering::Relaxed);
    }

    fn render_loop(
        window: HWND,
        frame_count: Arc<AtomicU64>,
        video_frames_received: Arc<AtomicU64>,
        running: Arc<AtomicBool>,
        shared_frame: Arc<Mutex<SharedFrame>>,
        overlay_stats: Arc<Mutex<OverlaySharedStats>>,
        config: RendererConfig,
    ) -> Result<()> {
        let mut msg = MSG::default();
        let started_at = Instant::now();
        let mut last_frame_sequence = 0u64;
        let mut present_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut shared_acquire_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut shared_copy_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut shared_release_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut shared_draw_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut last_present_stats = Instant::now();
        let mut last_shared_draw_stats = Instant::now();
        let mut ui_last_update = Instant::now();
        let mut last_rate_sample = Instant::now();
        let mut last_rendered_count = 0u64;
        let mut last_received_count = 0u64;
        let mut last_stale_dropped_count = 0u64;
        let mut cpu_upload_frames = 0u64;
        let mut gpu_external_frames = 0u64;
        let mut stale_dropped_frames = 0u64;
        let mut age_dropped_frames = 0u64;
        let mut metrics = OverlayRenderMetrics::default();
        let render_max_age_us = std::env::var("MRD_RENDER_MAX_AGE_MS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .map(|ms| (ms * 1000.0) as u64)
            .unwrap_or(0);

        let mut d3d = D3DContext::new(window, &config)?;
        let idle_wait_timeout_ms = idle_wait_timeout_ms(config.low_latency_mode);

        while running.load(Ordering::Relaxed) {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if !running.load(Ordering::Relaxed) {
                break;
            }

            let maybe_frame = {
                let mut guard = match shared_frame.try_lock() {
                    Ok(g) => g,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        unsafe {
                            let _ = MsgWaitForMultipleObjectsEx(
                                None,
                                idle_wait_timeout_ms,
                                QS_ALLINPUT,
                                MWMO_INPUTAVAILABLE,
                            );
                        }
                        continue;
                    }
                    Err(std::sync::TryLockError::Poisoned(_)) => {
                        warn!("frame mutex poisoned");
                        break;
                    }
                };
                if guard.sequence == last_frame_sequence {
                    None
                } else {
                    if last_frame_sequence != 0 {
                        let gap = guard.sequence.saturating_sub(last_frame_sequence.saturating_add(1));
                        stale_dropped_frames = stale_dropped_frames.saturating_add(gap);
                    }
                    last_frame_sequence = guard.sequence;
                    guard.latest.take()
                }
            };

            if let Some(frame) = maybe_frame {
                if render_max_age_us > 0 && frame.capture_start_unix_us != 0 {
                    if let Ok(elapsed) =
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    {
                        let now_us = elapsed.as_micros().min(u64::MAX as u128) as u64;
                        if now_us > frame.capture_start_unix_us
                            && now_us - frame.capture_start_unix_us > render_max_age_us
                        {
                            age_dropped_frames = age_dropped_frames.saturating_add(1);
                            continue;
                        }
                    }
                }
                match &frame.data {
                    DecodedFrameData::CpuNv12(_) => {
                        cpu_upload_frames = cpu_upload_frames.saturating_add(1);
                        d3d.upload_nv12(&frame)?;
                        d3d.draw_frame()?;
                    }
                    DecodedFrameData::D3d11Nv12 { texture, subresource } => {
                        gpu_external_frames = gpu_external_frames.saturating_add(1);
                        if let Err(e) =
                            d3d.draw_external_nv12(texture, *subresource, frame.width, frame.height)
                        {
                            warn!(error = %e, "draw_external_nv12 failed; trying CPU readback fallback");
                            cpu_upload_frames = cpu_upload_frames.saturating_add(1);
                            if let Err(fallback_err) = d3d
                                .draw_external_nv12_via_cpu(texture, *subresource, frame.width, frame.height)
                            {
                                warn!(error = %fallback_err, "draw_external_nv12 CPU fallback failed; dropping frame");
                                continue;
                            }
                        }
                    }
                    DecodedFrameData::D3d11SharedNv12 { shared_handle } => {
                        gpu_external_frames = gpu_external_frames.saturating_add(1);
                        match d3d.draw_shared_nv12(*shared_handle, frame.width, frame.height) {
                            Ok(timing) => {
                                if shared_acquire_samples_ms.len() >= 1024 {
                                    shared_acquire_samples_ms.pop_front();
                                    shared_copy_samples_ms.pop_front();
                                    shared_release_samples_ms.pop_front();
                                    shared_draw_samples_ms.pop_front();
                                }
                                shared_acquire_samples_ms.push_back(timing.acquire_ms);
                                shared_copy_samples_ms.push_back(timing.copy_ms);
                                shared_release_samples_ms.push_back(timing.release_ms);
                                shared_draw_samples_ms.push_back(timing.draw_ms);
                            }
                            Err(e) => {
                                warn!(error = %e, "draw_shared_nv12 failed; dropping frame");
                                continue;
                            }
                        }
                    }
                }
                if frame.capture_start_unix_us != 0 {
                    if let Ok(elapsed) =
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    {
                        let now_us = elapsed.as_micros().min(u64::MAX as u128) as u64;
                        if now_us >= frame.capture_start_unix_us {
                            let e2e_ms = (now_us - frame.capture_start_unix_us) as f64 / 1000.0;
                            if present_samples_ms.len() >= 1024 {
                                present_samples_ms.pop_front();
                            }
                            present_samples_ms.push_back(e2e_ms);
                        }
                    }
                }
                frame_count.fetch_add(1, Ordering::Relaxed);
            } else {
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(
                        None,
                        idle_wait_timeout_ms,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                }
                continue;
            }

            if last_present_stats.elapsed() >= Duration::from_secs(2) && !present_samples_ms.is_empty() {
                let mut sorted: Vec<f64> = present_samples_ms.iter().copied().collect();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let idx = |p: f64| -> usize {
                    ((sorted.len() as f64) * p)
                        .floor()
                        .min((sorted.len().saturating_sub(1)) as f64) as usize
                };
                let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
                metrics.present_avg_ms = avg;
                metrics.present_p50_ms = sorted[idx(0.50)];
                metrics.present_p95_ms = sorted[idx(0.95)];
                metrics.present_p99_ms = sorted[idx(0.99)];
                info!(
                    capture_to_present_avg_ms = format!("{:.3}", avg),
                    capture_to_present_p50_ms = format!("{:.3}", sorted[idx(0.50)]),
                    capture_to_present_p95_ms = format!("{:.3}", sorted[idx(0.95)]),
                    capture_to_present_p99_ms = format!("{:.3}", sorted[idx(0.99)]),
                    samples = sorted.len(),
                    "[PRESENT-STATS]"
                );
                last_present_stats = Instant::now();
            }
            if last_shared_draw_stats.elapsed() >= Duration::from_secs(2)
                && !shared_acquire_samples_ms.is_empty()
            {
                let summarize = |samples: &std::collections::VecDeque<f64>| -> (f64, f64) {
                    let mut sorted: Vec<f64> = samples.iter().copied().collect();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let p95_idx = (((sorted.len() as f64) * 0.95).floor() as usize)
                        .min(sorted.len().saturating_sub(1));
                    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
                    (avg, sorted[p95_idx])
                };
                let (acq_avg, acq_p95) = summarize(&shared_acquire_samples_ms);
                let (copy_avg, copy_p95) = summarize(&shared_copy_samples_ms);
                let (rel_avg, rel_p95) = summarize(&shared_release_samples_ms);
                let (draw_avg, draw_p95) = summarize(&shared_draw_samples_ms);
                info!(
                    acquire_avg_ms = format!("{:.3}", acq_avg),
                    acquire_p95_ms = format!("{:.3}", acq_p95),
                    copy_avg_ms = format!("{:.3}", copy_avg),
                    copy_p95_ms = format!("{:.3}", copy_p95),
                    release_avg_ms = format!("{:.3}", rel_avg),
                    release_p95_ms = format!("{:.3}", rel_p95),
                    draw_avg_ms = format!("{:.3}", draw_avg),
                    draw_p95_ms = format!("{:.3}", draw_p95),
                    samples = shared_acquire_samples_ms.len(),
                    "[SHARED-DRAW-STATS]"
                );
                last_shared_draw_stats = Instant::now();
            }

            let rendered = frame_count.load(Ordering::Relaxed);
            let recv = video_frames_received.load(Ordering::Relaxed);
            if last_rate_sample.elapsed() >= Duration::from_secs(1) {
                let dt = last_rate_sample.elapsed().as_secs_f64().max(0.001);
                let render_fps = (rendered.saturating_sub(last_rendered_count)) as f64 / dt;
                let recv_fps = (recv.saturating_sub(last_received_count)) as f64 / dt;
                metrics.rendered_frames = rendered;
                metrics.render_fps = render_fps;
                metrics.received_frames = recv;
                metrics.receive_fps = recv_fps;
                info!(
                    rendered_frames = rendered,
                    rendered_fps = format!("{:.2}", render_fps),
                    received_frames = recv,
                    received_fps = format!("{:.2}", recv_fps),
                    stale_dropped_total = stale_dropped_frames,
                    stale_dropped_per_sec = stale_dropped_frames.saturating_sub(last_stale_dropped_count),
                    age_dropped_total = age_dropped_frames,
                    gpu_external_frames,
                    cpu_upload_frames,
                    gpu_zero_copy_ratio = format!(
                        "{:.3}",
                        (gpu_external_frames as f64)
                            / ((gpu_external_frames + cpu_upload_frames).max(1) as f64)
                    ),
                    uptime_s = format!("{:.2}", started_at.elapsed().as_secs_f64()),
                    "renderer progress"
                );
                last_rate_sample = Instant::now();
                last_rendered_count = rendered;
                last_received_count = recv;
                last_stale_dropped_count = stale_dropped_frames;
            }
            if ui_last_update.elapsed() >= Duration::from_millis(250) {
                let shared = overlay_stats.lock().map(|v| v.clone()).unwrap_or_default();
                Self::update_overlay_panel(window, &metrics, &shared);
                ui_last_update = Instant::now();
            }
        }

        info!("render loop ended");
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COMMAND => {
                Self::on_overlay_command(window, wparam);
                LRESULT(0)
            }
            WM_SIZE => {
                Self::layout_overlay_controls(window);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => {
                // Borderless window move area: top toolbar.
                let x = (lparam.0 as i16) as i32;
                let y = ((lparam.0 >> 16) as i16) as i32;
                let mut pt = POINT { x, y };
                let _ = ScreenToClient(window, &mut pt);
                if pt.y >= 0 && pt.y <= 62 {
                    return LRESULT(HTCAPTION as isize);
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(window, &mut ps);
                let _ = EndPaint(window, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut OverlayUiState };
                if !ptr.is_null() {
                    unsafe {
                        let _ = SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                        drop(Box::from_raw(ptr));
                    }
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    fn create_window(
        width: u32,
        height: u32,
        control_queue: Arc<Mutex<Vec<OverlaySwitchCommand>>>,
    ) -> Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class_name = windows::core::w!("ControllerWindowD3D11");
            let wnd_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::window_proc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hbrBackground: HBRUSH::default(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hIcon: HICON::default(),
                lpszMenuName: PCWSTR::null(),
            };
            let atom = RegisterClassW(&wnd_class);
            if atom == 0 {
                let error = GetLastError();
                if error != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(anyhow::anyhow!("register class failed: {:?}", error));
                }
            }

            let borderless = std::env::var("MRD_BORDERLESS")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            let style = if borderless {
                WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | WS_SYSMENU | WS_MINIMIZEBOX
            } else {
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN
            };
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                windows::core::w!("Remote Desktop - D3D11"),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width as i32,
                height as i32,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("failed to create window")?;
            Self::init_overlay_controls(window, control_queue)?;
            Self::layout_overlay_controls(window);
            Ok(window)
        }
    }

    fn init_overlay_controls(
        window: HWND,
        control_queue: Arc<Mutex<Vec<OverlaySwitchCommand>>>,
    ) -> Result<()> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let _collapse = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("收起"),
                WS_CHILD | WS_VISIBLE,
                8,
                6,
                60,
                24,
                Some(window),
                Some(HMENU(ID_BTN_COLLAPSE as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _copy = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("复制"),
                WS_CHILD | WS_VISIBLE,
                72,
                6,
                60,
                24,
                Some(window),
                Some(HMENU(ID_BTN_COPY as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _overview = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("总览"),
                WS_CHILD | WS_VISIBLE,
                136,
                6,
                56,
                24,
                Some(window),
                Some(HMENU(ID_BTN_OVERVIEW as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _pipeline = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("链路"),
                WS_CHILD | WS_VISIBLE,
                196,
                6,
                56,
                24,
                Some(window),
                Some(HMENU(ID_BTN_PIPELINE as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _transport = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("传输"),
                WS_CHILD | WS_VISIBLE,
                256,
                6,
                56,
                24,
                Some(window),
                Some(HMENU(ID_BTN_TRANSPORT as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _debug = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("调试"),
                WS_CHILD | WS_VISIBLE,
                316,
                6,
                56,
                24,
                Some(window),
                Some(HMENU(ID_BTN_DEBUG as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _close = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("关闭"),
                WS_CHILD | WS_VISIBLE,
                376,
                6,
                56,
                24,
                Some(window),
                Some(HMENU(ID_BTN_CLOSE as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _res = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("RES:1080p"),
                WS_CHILD | WS_VISIBLE,
                8,
                34,
                86,
                24,
                Some(window),
                Some(HMENU(ID_BTN_RESOLUTION as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _win = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("WIN:auto"),
                WS_CHILD | WS_VISIBLE,
                98,
                34,
                86,
                24,
                Some(window),
                Some(HMENU(ID_BTN_WINDOW as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _br = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("BR:20M"),
                WS_CHILD | WS_VISIBLE,
                188,
                34,
                86,
                24,
                Some(window),
                Some(HMENU(ID_BTN_BITRATE as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _cap = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("CAP:dxgi"),
                WS_CHILD | WS_VISIBLE,
                278,
                34,
                86,
                24,
                Some(window),
                Some(HMENU(ID_BTN_CAPTURE as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _enc = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("BUTTON"),
                windows::core::w!("ENC:nvenc"),
                WS_CHILD | WS_VISIBLE,
                368,
                34,
                96,
                24,
                Some(window),
                Some(HMENU(ID_BTN_ENCODER as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let _panel = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("EDIT"),
                windows::core::w!(""),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_VSCROLL.0
                        | (ES_LEFT as u32)
                        | (ES_MULTILINE as u32)
                        | (ES_AUTOVSCROLL as u32)
                        | (ES_READONLY as u32),
                ),
                8,
                64,
                500,
                220,
                Some(window),
                Some(HMENU(ID_EDIT_PANEL as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )?;
            let boxed = Box::new(OverlayUiState::new(control_queue));
            let raw = Box::into_raw(boxed);
            let _ = SetWindowLongPtrW(window, GWLP_USERDATA, raw as isize);
        }
        Ok(())
    }

    fn ui_state(window: HWND) -> Option<&'static mut OverlayUiState> {
        unsafe {
            let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut OverlayUiState;
            if ptr.is_null() { None } else { Some(&mut *ptr) }
        }
    }

    fn on_overlay_command(window: HWND, wparam: WPARAM) {
        let id = (wparam.0 & 0xFFFF) as i32;
        if let Some(state) = Self::ui_state(window) {
            match id {
                ID_BTN_COLLAPSE => {
                    state.collapsed = !state.collapsed;
                    unsafe {
                        let label = if state.collapsed {
                            windows::core::w!("展开")
                        } else {
                            windows::core::w!("收起")
                        };
                        if let Ok(btn) = GetDlgItem(Some(window), ID_BTN_COLLAPSE) {
                            let _ = SetWindowTextW(btn, label);
                        }
                    }
                    Self::layout_overlay_controls(window);
                }
                ID_BTN_COPY => {
                    let _ = Self::copy_text_to_clipboard(window, &state.last_text);
                }
                ID_BTN_OVERVIEW => state.panel = OverlayPanel::Overview,
                ID_BTN_PIPELINE => state.panel = OverlayPanel::Pipeline,
                ID_BTN_TRANSPORT => state.panel = OverlayPanel::Transport,
                ID_BTN_DEBUG => state.panel = OverlayPanel::Debug,
                ID_BTN_CLOSE => unsafe {
                    let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                },
                ID_BTN_RESOLUTION => {
                    state.res_idx = (state.res_idx + 1) % RES_PRESETS.len();
                    let (value, label, btn) = (
                        RES_PRESETS[state.res_idx].1.to_string(),
                        RES_PRESETS[state.res_idx].2,
                        ID_BTN_RESOLUTION,
                    );
                    Self::set_button_text(window, btn, label);
                    if let Ok(mut q) = state.control_queue.lock() {
                        q.push(OverlaySwitchCommand { field: OverlaySwitchField::Resolution, value });
                    }
                }
                ID_BTN_WINDOW => {
                    state.win_idx = (state.win_idx + 1) % WIN_PRESETS.len();
                    let (value, label, btn) = (
                        WIN_PRESETS[state.win_idx].0.to_string(),
                        WIN_PRESETS[state.win_idx].1,
                        ID_BTN_WINDOW,
                    );
                    Self::set_button_text(window, btn, label);
                    if let Ok(mut q) = state.control_queue.lock() {
                        q.push(OverlaySwitchCommand { field: OverlaySwitchField::CaptureWindow, value });
                    }
                }
                ID_BTN_BITRATE => {
                    state.br_idx = (state.br_idx + 1) % BR_PRESETS.len();
                    let (value, label, btn) = (
                        BR_PRESETS[state.br_idx].0.to_string(),
                        BR_PRESETS[state.br_idx].1,
                        ID_BTN_BITRATE,
                    );
                    Self::set_button_text(window, btn, label);
                    if let Ok(mut q) = state.control_queue.lock() {
                        q.push(OverlaySwitchCommand { field: OverlaySwitchField::Bitrate, value });
                    }
                }
                ID_BTN_CAPTURE => {
                    state.cap_idx = (state.cap_idx + 1) % CAP_PRESETS.len();
                    let (value, label, btn) = (
                        CAP_PRESETS[state.cap_idx].0.to_string(),
                        CAP_PRESETS[state.cap_idx].1,
                        ID_BTN_CAPTURE,
                    );
                    Self::set_button_text(window, btn, label);
                    if let Ok(mut q) = state.control_queue.lock() {
                        q.push(OverlaySwitchCommand { field: OverlaySwitchField::CaptureBackend, value });
                    }
                }
                ID_BTN_ENCODER => {
                    state.enc_idx = (state.enc_idx + 1) % ENC_PRESETS.len();
                    let (value, label, btn) = (
                        ENC_PRESETS[state.enc_idx].0.to_string(),
                        ENC_PRESETS[state.enc_idx].1,
                        ID_BTN_ENCODER,
                    );
                    Self::set_button_text(window, btn, label);
                    if let Ok(mut q) = state.control_queue.lock() {
                        q.push(OverlaySwitchCommand { field: OverlaySwitchField::Encoder, value });
                    }
                }
                _ => {}
            }
        }
    }

    fn set_button_text(window: HWND, id: i32, label: &str) {
        let mut w: Vec<u16> = label.encode_utf16().collect();
        w.push(0);
        unsafe {
            if let Ok(btn) = GetDlgItem(Some(window), id) {
                let _ = SetWindowTextW(btn, PCWSTR(w.as_ptr()));
            }
        }
    }

    fn layout_overlay_controls(window: HWND) {
        unsafe {
            let mut rect = RECT::default();
            if GetClientRect(window, &mut rect).is_err() {
                return;
            }
            let width = (rect.right - rect.left).max(320);
            let Ok(panel) = GetDlgItem(Some(window), ID_EDIT_PANEL) else {
                return;
            };
            let mut panel_h = 220;
            if let Some(state) = Self::ui_state(window) {
                if state.collapsed {
                    panel_h = 0;
                    let _ = ShowWindow(panel, SW_HIDE);
                } else {
                    let _ = ShowWindow(panel, SW_SHOW);
                }
            }
            let _ = MoveWindow(panel, 8, 64, (width - 16).max(120), panel_h.max(0), true);
            for id in [
                ID_EDIT_PANEL,
                ID_BTN_COLLAPSE,
                ID_BTN_COPY,
                ID_BTN_OVERVIEW,
                ID_BTN_PIPELINE,
                ID_BTN_TRANSPORT,
                ID_BTN_DEBUG,
                ID_BTN_CLOSE,
                ID_BTN_RESOLUTION,
                ID_BTN_WINDOW,
                ID_BTN_BITRATE,
                ID_BTN_CAPTURE,
                ID_BTN_ENCODER,
            ] {
                if let Ok(ctrl) = GetDlgItem(Some(window), id) {
                    let _ = SetWindowPos(
                        ctrl,
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }

    fn copy_text_to_clipboard(window: HWND, text: &str) -> Result<()> {
        let _ = text;
        unsafe {
            let edit = GetDlgItem(Some(window), ID_EDIT_PANEL)
                .map_err(|e| anyhow::anyhow!("GetDlgItem failed: {e}"))?;
            // EM_SETSEL + WM_COPY
            let _ = SendMessageW(edit, 0x00B1, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SendMessageW(edit, WM_COPY, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        Ok(())
    }

    fn update_overlay_panel(
        window: HWND,
        metrics: &OverlayRenderMetrics,
        shared: &OverlaySharedStats,
    ) {
        let Some(state) = Self::ui_state(window) else {
            return;
        };
        let panel_text = Self::build_panel_text(state.panel, metrics, shared);
        if panel_text == state.last_text {
            return;
        }
        state.last_text = panel_text.clone();
        let wide: Vec<u16> = panel_text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            if let Ok(edit) = GetDlgItem(Some(window), ID_EDIT_PANEL) {
                let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
            }
        }
    }

    fn build_panel_text(
        panel: OverlayPanel,
        m: &OverlayRenderMetrics,
        s: &OverlaySharedStats,
    ) -> String {
        match panel {
            OverlayPanel::Overview => format!(
                "Panel: {}\r\n\
                 传输: selected={} / active={}\r\n\
                 渲染: {:.2} FPS (frames={})\r\n\
                 接收: {:.2} FPS (frames={})\r\n\
                 解码: {:.2} FPS (frames={})\r\n\
                 decode avg/p95: {:.3} / {:.3} ms\r\n\
                 jitter: {:.3} ms\r\n\
                 E2E avg/p50/p95/p99: {:.3} / {:.3} / {:.3} / {:.3} ms\r\n\
                 present avg/p50/p95/p99: {:.3} / {:.3} / {:.3} / {:.3} ms\r\n\
                 decode_failures: {}\r\n\
                 操作: 收起 | 复制 | 切换面板",
                panel.title(),
                s.selected_transport,
                s.media_path,
                m.render_fps,
                m.rendered_frames,
                m.receive_fps,
                m.received_frames,
                s.decode_fps,
                s.decoded_frames,
                s.avg_decode_ms,
                s.p95_decode_ms,
                s.jitter_ms,
                s.e2e_avg_ms,
                s.e2e_p50_ms,
                s.e2e_p95_ms,
                s.e2e_p99_ms,
                m.present_avg_ms,
                m.present_p50_ms,
                m.present_p95_ms,
                m.present_p99_ms,
                s.decode_failures,
            ),
            OverlayPanel::Pipeline => format!(
                "Panel: {}\r\n\
                 [Capture]\r\n\
                 来源帧(接收): {:.2} FPS, frames={}\r\n\
                 [Decode]\r\n\
                 backend={} fps={:.2} frames={}\r\n\
                 avg={:.3}ms p95={:.3}ms jitter={:.3}ms\r\n\
                 [Render]\r\n\
                 render_fps={:.2} rendered_frames={}\r\n\
                 present avg={:.3} p50={:.3} p95={:.3} p99={:.3} ms\r\n\
                 [End-to-End]\r\n\
                 avg={:.3} p50={:.3} p95={:.3} p99={:.3} ms",
                panel.title(),
                m.receive_fps,
                m.received_frames,
                s.decoder_backend,
                s.decode_fps,
                s.decoded_frames,
                s.avg_decode_ms,
                s.p95_decode_ms,
                s.jitter_ms,
                m.render_fps,
                m.rendered_frames,
                m.present_avg_ms,
                m.present_p50_ms,
                m.present_p95_ms,
                m.present_p99_ms,
                s.e2e_avg_ms,
                s.e2e_p50_ms,
                s.e2e_p95_ms,
                s.e2e_p99_ms,
            ),
            OverlayPanel::Transport => format!(
                "Panel: {}\r\n\
                 selected_transport={}\r\n\
                 active_media_path={}\r\n\
                 接收速率={:.2} FPS\r\n\
                 注: 发送端码率/AU/丢包在 agent 侧日志 [RTCP-PANEL]\r\n\
                 当前窗口可复制本面板全文。",
                panel.title(),
                s.selected_transport,
                s.media_path,
                m.receive_fps,
            ),
            OverlayPanel::Debug => format!(
                "Panel: {}\r\n\
                 decode_failures={}\r\n\
                 last_decode_error={}\r\n\
                 decoded_frames={}\r\n\
                 rendered_frames={}\r\n\
                 received_frames={}\r\n\
                 decoder_backend={}\r\n\
                 selected_transport={} active_path={}",
                panel.title(),
                s.decode_failures,
                if s.last_decode_error.is_empty() { "none" } else { &s.last_decode_error },
                s.decoded_frames,
                m.rendered_frames,
                m.received_frames,
                s.decoder_backend,
                s.selected_transport,
                s.media_path,
            ),
        }
    }

    pub fn window_handle(&self) -> HWND {
        self.window
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for D3D11Renderer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(50));
        unsafe {
            if !self.window.is_invalid() {
                let _ = PostMessageW(Some(self.window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

struct D3DContext {
    window: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    y_tex: Option<ID3D11Texture2D>,
    uv_tex: Option<ID3D11Texture2D>,
    y_srv: Option<ID3D11ShaderResourceView>,
    uv_srv: Option<ID3D11ShaderResourceView>,
    external_nv12_srv_cache:
        HashMap<(usize, u32), (ID3D11ShaderResourceView, ID3D11ShaderResourceView)>,
    shared_nv12_cache: HashMap<isize, SharedNv12View>,
    shared_copy_tex: Option<ID3D11Texture2D>,
    shared_copy_y_srv: Option<ID3D11ShaderResourceView>,
    shared_copy_uv_srv: Option<ID3D11ShaderResourceView>,
    shared_copy_w: u32,
    shared_copy_h: u32,
    external_readback_tex: Option<ID3D11Texture2D>,
    external_readback_w: u32,
    external_readback_h: u32,
    frame_width: u32,
    frame_height: u32,
    vsync: bool,
    allow_tearing: bool,
    low_latency_mode: bool,
    present_min_interval: Option<Duration>,
    last_present_at: Option<Instant>,
    present_spin_us: u64,
}

struct SharedNv12View {
    texture: ID3D11Texture2D,
    keyed_mutex: IDXGIKeyedMutex,
    y_srv: ID3D11ShaderResourceView,
    uv_srv: ID3D11ShaderResourceView,
}

#[derive(Default)]
struct SharedDrawTiming {
    acquire_ms: f64,
    copy_ms: f64,
    draw_ms: f64,
    release_ms: f64,
}

impl D3DContext {
    fn new(window: HWND, config: &RendererConfig) -> Result<Self> {
        unsafe {
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut chosen_level = D3D_FEATURE_LEVEL_11_0;

            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut chosen_level),
                Some(&mut context),
            )
            .context("D3D11CreateDevice failed")?;

            let device = device.context("missing D3D11 device")?;
            let context = context.context("missing D3D11 context")?;
            let dxgi_device: IDXGIDevice = device.cast().context("cast to IDXGIDevice failed")?;
            let adapter = dxgi_device.GetAdapter().context("GetAdapter failed")?;
            let factory: IDXGIFactory2 = adapter
                .GetParent()
                .context("GetParent IDXGIFactory2 failed")?;

            let mut allow_tearing = false;
            if let Ok(factory5) = factory.cast::<IDXGIFactory5>() {
                let mut tearing: u32 = 0;
                if factory5
                    .CheckFeatureSupport(
                        DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                        &mut tearing as *mut _ as *mut c_void,
                        std::mem::size_of::<u32>() as u32,
                    )
                    .is_ok()
                {
                    allow_tearing = tearing != 0;
                }
            }

            let mut sc_desc1 = DXGI_SWAP_CHAIN_DESC1::default();
            sc_desc1.Width = 0;
            sc_desc1.Height = 0;
            sc_desc1.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
            sc_desc1.Stereo = FALSE;
            sc_desc1.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
            sc_desc1.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
            sc_desc1.BufferCount = 2;
            sc_desc1.Scaling = DXGI_SCALING_STRETCH;
            sc_desc1.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
            sc_desc1.AlphaMode = DXGI_ALPHA_MODE_IGNORE;
            sc_desc1.Flags = if !config.vsync && allow_tearing {
                DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
            } else {
                0
            };

            let swap_chain = factory
                .CreateSwapChainForHwnd(
                    &device,
                    window,
                    &sc_desc1,
                    None,
                    None,
                )
                .context("CreateSwapChainForHwnd failed")?;
            let _ = factory.MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER);

            if let Ok(dxgi_device1) = device.cast::<IDXGIDevice1>() {
                let latency = config.max_frame_latency.clamp(1, 16);
                let _ignored = dxgi_device1.SetMaximumFrameLatency(latency);
            }

            let back_buffer: ID3D11Texture2D = swap_chain
                .GetBuffer(0)
                .context("swap chain GetBuffer failed")?;
            let mut rtv = None;
            device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut rtv))
                .context("CreateRenderTargetView failed")?;
            let rtv = rtv.context("missing render target view")?;

            let vs_src = b"
struct VSOut {
    float4 pos : SV_POSITION;
    float2 uv  : TEXCOORD0;
};
VSOut main(uint vid : SV_VertexID) {
    float2 p[3];
    p[0] = float2(-1.0, -1.0);
    p[1] = float2(-1.0,  3.0);
    p[2] = float2( 3.0, -1.0);
    VSOut o;
    o.pos = float4(p[vid], 0.0, 1.0);
    o.uv = float2((p[vid].x + 1.0) * 0.5, 1.0 - (p[vid].y + 1.0) * 0.5);
    return o;
}";

            let ps_src_limited_709 = b"
Texture2D texY  : register(t0);
Texture2D texUV : register(t1);
SamplerState samp : register(s0);
float4 main(float4 pos : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float y = texY.Sample(samp, uv).r;
    float2 uvv = texUV.Sample(samp, uv).rg;
    float c = max(0.0, y - (16.0 / 255.0)) * 1.16438356;
    float u = uvv.x - 0.5;
    float v = uvv.y - 0.5;
    float r = c + 1.79274107 * v;
    float g = c - 0.21324861 * u - 0.53290933 * v;
    float b = c + 2.11240179 * u;
    return float4(saturate(r), saturate(g), saturate(b), 1.0);
}";
            let ps_src_full_601 = b"
Texture2D texY  : register(t0);
Texture2D texUV : register(t1);
SamplerState samp : register(s0);
float4 main(float4 pos : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float y = texY.Sample(samp, uv).r;
    float2 uvv = texUV.Sample(samp, uv).rg;
    float u = uvv.x - 0.5;
    float v = uvv.y - 0.5;
    float r = y + 1.402 * v;
    float g = y - 0.344136 * u - 0.714136 * v;
    float b = y + 1.772 * u;
    return float4(saturate(r), saturate(g), saturate(b), 1.0);
}";
            let color_mode = std::env::var("MRD_COLOR_MODE")
                .unwrap_or_else(|_| "limited709".to_string())
                .to_ascii_lowercase();
            let ps_src: &[u8] = if color_mode == "full601" {
                &ps_src_full_601[..]
            } else {
                &ps_src_limited_709[..]
            };
            info!(%color_mode, "using yuv->rgb shader mode");

            let vs_blob = compile_hlsl(vs_src, b"main\0", b"vs_5_0\0")?;
            let ps_blob = compile_hlsl(ps_src, b"main\0", b"ps_5_0\0")?;

            let mut vs = None;
            let vs_bytes = std::slice::from_raw_parts(
                vs_blob.GetBufferPointer() as *const u8,
                vs_blob.GetBufferSize(),
            );
            device
                .CreateVertexShader(
                    vs_bytes,
                    None,
                    Some(&mut vs),
                )
                .context("CreateVertexShader failed")?;
            let vs = vs.context("missing vertex shader")?;
            let mut ps = None;
            let ps_bytes = std::slice::from_raw_parts(
                ps_blob.GetBufferPointer() as *const u8,
                ps_blob.GetBufferSize(),
            );
            device
                .CreatePixelShader(
                    ps_bytes,
                    None,
                    Some(&mut ps),
                )
                .context("CreatePixelShader failed")?;
            let ps = ps.context("missing pixel shader")?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0, 0.0, 0.0, 0.0],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut sampler = None;
            device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .context("CreateSamplerState failed")?;
            let sampler = sampler.context("missing sampler state")?;

            info!(
                vsync = config.vsync,
                allow_tearing,
                low_latency_mode = config.low_latency_mode,
                max_frame_latency = config.max_frame_latency,
                "D3D11 swapchain initialized"
            );

            Ok(Self {
                window,
                device,
                context,
                swap_chain,
                rtv,
                vs,
                ps,
                sampler,
                y_tex: None,
                uv_tex: None,
                y_srv: None,
                uv_srv: None,
                external_nv12_srv_cache: HashMap::new(),
                shared_nv12_cache: HashMap::new(),
                shared_copy_tex: None,
                shared_copy_y_srv: None,
                shared_copy_uv_srv: None,
                shared_copy_w: 0,
                shared_copy_h: 0,
                external_readback_tex: None,
                external_readback_w: 0,
                external_readback_h: 0,
                frame_width: 0,
                frame_height: 0,
                vsync: config.vsync,
                allow_tearing,
                low_latency_mode: config.low_latency_mode,
                present_min_interval: std::env::var("MRD_PRESENT_TARGET_FPS")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|fps| *fps > 0.0)
                    .map(|fps| Duration::from_secs_f64((1.0 / fps).max(0.000_5))),
                last_present_at: None,
                present_spin_us: std::env::var("MRD_PRESENT_SPIN_US")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(200),
            })
        }
    }

    fn ensure_textures(&mut self, width: u32, height: u32) -> Result<()> {
        if self.frame_width == width && self.frame_height == height && self.y_tex.is_some() {
            return Ok(());
        }

        unsafe {
            self.frame_width = width;
            self.frame_height = height;

            let y_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };
            let uv_desc = D3D11_TEXTURE2D_DESC {
                Width: width / 2,
                Height: height / 2,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };

            let mut y_tex = None;
            self.device
                .CreateTexture2D(&y_desc, None, Some(&mut y_tex))
                .context("CreateTexture2D Y failed")?;
            let y_tex = y_tex.context("missing y texture")?;

            let mut uv_tex = None;
            self.device
                .CreateTexture2D(&uv_desc, None, Some(&mut uv_tex))
                .context("CreateTexture2D UV failed")?;
            let uv_tex = uv_tex.context("missing uv texture")?;

            let mut y_srv = None;
            self.device
                .CreateShaderResourceView(&y_tex, None, Some(&mut y_srv))
                .context("CreateShaderResourceView Y failed")?;
            let y_srv = y_srv.context("missing y srv")?;

            let mut uv_srv = None;
            self.device
                .CreateShaderResourceView(&uv_tex, None, Some(&mut uv_srv))
                .context("CreateShaderResourceView UV failed")?;
            let uv_srv = uv_srv.context("missing uv srv")?;

            self.y_tex = Some(y_tex);
            self.uv_tex = Some(uv_tex);
            self.y_srv = Some(y_srv);
            self.uv_srv = Some(uv_srv);

            info!(width, height, "video texture resized");
        }
        Ok(())
    }

    fn upload_nv12(&mut self, frame: &DecodedFrame) -> Result<()> {
        self.ensure_textures(frame.width, frame.height)?;
        let y = frame
            .y_plane()
            .context("CPU NV12 frame expected for upload path")?;
        let uv = frame
            .uv_plane()
            .context("CPU NV12 frame expected for upload path")?;
        let width = frame.width as usize;
        let height = frame.height as usize;

        unsafe {
            let y_tex = self.y_tex.as_ref().context("missing y texture")?;
            let uv_tex = self.uv_tex.as_ref().context("missing uv texture")?;

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(y_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .context("Map Y failed")?;
            for row in 0..height {
                let src_off = row * width;
                let dst = (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(y[src_off..src_off + width].as_ptr(), dst, width);
            }
            self.context.Unmap(y_tex, 0);

            self.context
                .Map(uv_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .context("Map UV failed")?;
            let uv_h = height / 2;
            for row in 0..uv_h {
                let src_off = row * width;
                let dst = (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(uv[src_off..src_off + width].as_ptr(), dst, width);
            }
            self.context.Unmap(uv_tex, 0);
        }
        Ok(())
    }

    fn draw_external_nv12(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let (y_srv, uv_srv) = self.external_nv12_srvs(texture, subresource)?;
        self.draw_with_srvs(width, height, &y_srv, &uv_srv)
    }

    fn draw_external_nv12_via_cpu(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.ensure_external_readback_texture(width, height)?;
        let readback = self
            .external_readback_tex
            .as_ref()
            .context("missing external readback texture")?;
        unsafe {
            self.context
                .CopySubresourceRegion(readback, 0, 0, 0, 0, texture, subresource, None);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .context("Map external readback failed")?;

            let y_rows = height as usize;
            let uv_rows = (height / 2) as usize;
            let row_bytes = width as usize;
            let y_size = row_bytes * y_rows;
            let mut nv12 = vec![0u8; y_size + row_bytes * uv_rows];
            let src_y_base = mapped.pData as *const u8;
            for row in 0..y_rows {
                let src = src_y_base.add(row * mapped.RowPitch as usize);
                let dst = nv12.as_mut_ptr().add(row * row_bytes);
                std::ptr::copy_nonoverlapping(src, dst, row_bytes);
            }
            let src_uv_base = src_y_base.add(y_rows * mapped.RowPitch as usize);
            let dst_uv_base = nv12.as_mut_ptr().add(y_size);
            for row in 0..uv_rows {
                let src = src_uv_base.add(row * mapped.RowPitch as usize);
                let dst = dst_uv_base.add(row * row_bytes);
                std::ptr::copy_nonoverlapping(src, dst, row_bytes);
            }
            self.context.Unmap(readback, 0);

            let frame = DecodedFrame::from_cpu_nv12(
                Arc::new(nv12),
                width,
                height,
                0,
                0,
                0,
            );
            self.upload_nv12(&frame)?;
            self.draw_frame()?;
        }
        Ok(())
    }

    fn ensure_external_readback_texture(&mut self, width: u32, height: u32) -> Result<()> {
        if self.external_readback_tex.is_some()
            && self.external_readback_w == width
            && self.external_readback_h == height
        {
            return Ok(());
        }
        unsafe {
            self.external_readback_w = width;
            self.external_readback_h = height;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut tex = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("CreateTexture2D(external readback) failed")?;
            self.external_readback_tex = tex;
            self.external_nv12_srv_cache.clear();
        }
        Ok(())
    }

    fn draw_shared_nv12(
        &mut self,
        shared_handle: isize,
        width: u32,
        height: u32,
    ) -> Result<SharedDrawTiming> {
        static SHARED_DRAW_TRACE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let timeout_ms = std::env::var("MRD_D3D11_KEYED_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let trace_draw = std::env::var("MRD_SHARED_KEYED_TRACE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
            && SHARED_DRAW_TRACE.fetch_add(1, Ordering::Relaxed) < 32;
        let (source_tex, keyed_mutex) = {
            let view = self.ensure_shared_nv12_view(shared_handle)?;
            (view.texture.clone(), view.keyed_mutex.clone())
        };
        let mut timing = SharedDrawTiming::default();
        let t0 = Instant::now();
        unsafe {
            if trace_draw {
                info!("shared-keyed render before AcquireSync(1)");
            }
            if let Err(e) = keyed_mutex.AcquireSync(1, timeout_ms) {
                if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    anyhow::bail!("shared keyed mutex AcquireSync timeout");
                }
                return Err(anyhow::anyhow!("shared keyed mutex AcquireSync failed: {e}"));
            }
            if trace_draw {
                info!("shared-keyed render acquired");
            }
        }
        timing.acquire_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        self.ensure_shared_copy_texture(width, height)?;
        let copy_tex = self
            .shared_copy_tex
            .as_ref()
            .context("missing shared copy texture")?;
        unsafe {
            self.context.CopyResource(copy_tex, &source_tex);
            // Submit copy before releasing keyed mutex to avoid consumer/producer overlap artifacts.
            self.context.Flush();
        }
        timing.copy_ms = t1.elapsed().as_secs_f64() * 1000.0;

        let t2 = Instant::now();
        unsafe {
            if trace_draw {
                info!("shared-keyed render before ReleaseSync(0)");
            }
            keyed_mutex
                .ReleaseSync(0)
                .context("shared keyed mutex ReleaseSync failed")?;
            if trace_draw {
                info!("shared-keyed render released");
            }
        }
        timing.release_ms = t2.elapsed().as_secs_f64() * 1000.0;

        let y_srv = self
            .shared_copy_y_srv
            .as_ref()
            .context("missing shared copy y srv")?
            .clone();
        let uv_srv = self
            .shared_copy_uv_srv
            .as_ref()
            .context("missing shared copy uv srv")?
            .clone();
        let t3 = Instant::now();
        self.draw_with_srvs(width, height, &y_srv, &uv_srv)?;
        timing.draw_ms = t3.elapsed().as_secs_f64() * 1000.0;
        Ok(timing)
    }

    fn ensure_shared_copy_texture(&mut self, width: u32, height: u32) -> Result<()> {
        if self.shared_copy_tex.is_some() && self.shared_copy_w == width && self.shared_copy_h == height {
            return Ok(());
        }
        unsafe {
            self.shared_copy_w = width;
            self.shared_copy_h = height;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut tex = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("CreateTexture2D(shared copy) failed")?;
            let tex = tex.context("missing shared copy texture")?;
            let (y_srv, uv_srv) = self.create_external_nv12_srvs(&tex, 0)?;
            self.shared_copy_tex = Some(tex);
            self.shared_copy_y_srv = Some(y_srv);
            self.shared_copy_uv_srv = Some(uv_srv);
        }
        Ok(())
    }

    fn ensure_shared_nv12_view(&mut self, shared_handle: isize) -> Result<&SharedNv12View> {
        if !self.shared_nv12_cache.contains_key(&shared_handle) {
            let texture = unsafe { self.open_shared_texture(shared_handle)? };
            let keyed_mutex: IDXGIKeyedMutex = texture
                .cast()
                .context("cast shared texture to IDXGIKeyedMutex failed")?;
            let (y_srv, uv_srv) = self.create_external_nv12_srvs(&texture, 0)?;
            self.shared_nv12_cache.insert(
                shared_handle,
                SharedNv12View {
                    texture,
                    keyed_mutex,
                    y_srv,
                    uv_srv,
                },
            );
        }
        self.shared_nv12_cache
            .get(&shared_handle)
            .context("shared nv12 view cache miss")
    }

    unsafe fn open_shared_texture(&self, shared_handle: isize) -> Result<ID3D11Texture2D> {
        let mut texture: Option<ID3D11Texture2D> = None;
        self.device
            .OpenSharedResource(HANDLE(shared_handle as *mut c_void), &mut texture)
            .context("OpenSharedResource(ID3D11Texture2D) failed")?;
        texture.context("OpenSharedResource returned null texture")
    }

    fn create_external_nv12_srvs(
        &self,
        texture: &ID3D11Texture2D,
        subresource: u32,
    ) -> Result<(ID3D11ShaderResourceView, ID3D11ShaderResourceView)> {
        unsafe {
            let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut tex_desc);
            if tex_desc.Format != DXGI_FORMAT_NV12 {
                anyhow::bail!(
                    "external texture format is {:?}, expected NV12 (w={}, h={}, mips={}, array={})",
                    tex_desc.Format,
                    tex_desc.Width,
                    tex_desc.Height,
                    tex_desc.MipLevels,
                    tex_desc.ArraySize
                );
            }

            let device3: ID3D11Device3 = self
                .device
                .cast()
                .context("ID3D11Device3 required for NV12 plane SRV")?;

            let mip_levels = tex_desc.MipLevels.max(1);
            let mut array_slice = subresource / mip_levels;
            let mip_slice = subresource % mip_levels;
            if array_slice >= tex_desc.ArraySize.max(1) {
                warn!(
                    subresource,
                    mip_levels,
                    array_size = tex_desc.ArraySize,
                    "external subresource out of range; clamping to 0"
                );
                array_slice = 0;
            }

            let mut y_desc = D3D11_SHADER_RESOURCE_VIEW_DESC1::default();
            y_desc.Format = DXGI_FORMAT_R8_UNORM;
            let mut uv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC1::default();
            uv_desc.Format = DXGI_FORMAT_R8G8_UNORM;
            if tex_desc.ArraySize <= 1 {
                y_desc.ViewDimension =
                    windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2D;
                y_desc.Anonymous.Texture2D = D3D11_TEX2D_SRV1 {
                    MostDetailedMip: mip_slice,
                    MipLevels: 1,
                    PlaneSlice: 0,
                };
                uv_desc.ViewDimension =
                    windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2D;
                uv_desc.Anonymous.Texture2D = D3D11_TEX2D_SRV1 {
                    MostDetailedMip: mip_slice,
                    MipLevels: 1,
                    PlaneSlice: 1,
                };
            } else {
                y_desc.ViewDimension =
                    windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2DARRAY;
                y_desc.Anonymous.Texture2DArray = D3D11_TEX2D_ARRAY_SRV1 {
                    MostDetailedMip: mip_slice,
                    MipLevels: 1,
                    FirstArraySlice: array_slice,
                    ArraySize: 1,
                    PlaneSlice: 0,
                };
                uv_desc.ViewDimension =
                    windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2DARRAY;
                uv_desc.Anonymous.Texture2DArray = D3D11_TEX2D_ARRAY_SRV1 {
                    MostDetailedMip: mip_slice,
                    MipLevels: 1,
                    FirstArraySlice: array_slice,
                    ArraySize: 1,
                    PlaneSlice: 1,
                };
            }

            let mut y_srv1 = None;
            device3
                .CreateShaderResourceView1(texture, Some(&y_desc), Some(&mut y_srv1))
                .with_context(|| {
                    format!(
                        "CreateShaderResourceView1(Y) failed (subresource={}, w={}, h={}, mips={}, array={})",
                        subresource, tex_desc.Width, tex_desc.Height, tex_desc.MipLevels, tex_desc.ArraySize
                    )
                })?;
            let mut uv_srv1 = None;
            device3
                .CreateShaderResourceView1(texture, Some(&uv_desc), Some(&mut uv_srv1))
                .with_context(|| {
                    format!(
                        "CreateShaderResourceView1(UV) failed (subresource={}, w={}, h={}, mips={}, array={})",
                        subresource, tex_desc.Width, tex_desc.Height, tex_desc.MipLevels, tex_desc.ArraySize
                    )
                })?;

            let y_srv = y_srv1
                .context("missing Y SRV1")?
                .cast()
                .context("cast Y SRV1->SRV failed")?;
            let uv_srv = uv_srv1
                .context("missing UV SRV1")?
                .cast()
                .context("cast UV SRV1->SRV failed")?;
            Ok((y_srv, uv_srv))
        }
    }

    fn external_nv12_srvs(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
    ) -> Result<(ID3D11ShaderResourceView, ID3D11ShaderResourceView)> {
        let key = (texture.as_raw() as usize, subresource);
        if let Some((y_srv, uv_srv)) = self.external_nv12_srv_cache.get(&key) {
            return Ok((y_srv.clone(), uv_srv.clone()));
        }
        let (y_srv, uv_srv) = self.create_external_nv12_srvs(texture, subresource)?;
        if self.external_nv12_srv_cache.len() >= 64 {
            self.external_nv12_srv_cache.clear();
        }
        self.external_nv12_srv_cache
            .insert(key, (y_srv.clone(), uv_srv.clone()));
        Ok((y_srv, uv_srv))
    }

    fn draw_frame(&mut self) -> Result<()> {
        let y_srv = self.y_srv.clone().context("missing y srv")?;
        let uv_srv = self.uv_srv.clone().context("missing uv srv")?;
        self.draw_with_srvs(self.frame_width, self.frame_height, &y_srv, &uv_srv)
    }

    fn draw_with_srvs(
        &mut self,
        _width: u32,
        _height: u32,
        y_srv: &ID3D11ShaderResourceView,
        uv_srv: &ID3D11ShaderResourceView,
    ) -> Result<()> {
        if let Some(interval) = self.present_min_interval {
            if let Some(last) = self.last_present_at {
                let elapsed = last.elapsed();
                if elapsed < interval {
                    let remaining = interval - elapsed;
                    let spin_budget = Duration::from_micros(self.present_spin_us.min(2_000));
                    if remaining > spin_budget {
                        std::thread::sleep(remaining - spin_budget);
                    }
                    let spin_start = Instant::now();
                    while spin_start.elapsed() < spin_budget && last.elapsed() < interval {
                        std::hint::spin_loop();
                    }
                }
            }
        }
        unsafe {
            self.context.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);

            let mut client = RECT::default();
            let _ = GetClientRect(self.window, &mut client);
            let view_w = (client.right - client.left).max(1) as f32;
            let view_h = (client.bottom - client.top).max(1) as f32;
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: view_w,
                Height: view_h,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            self.context.RSSetViewports(Some(&[viewport]));
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY(
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST.0,
            ));
            self.context.VSSetShader(&self.vs, None);
            self.context.PSSetShader(&self.ps, None);

            self.context
                .PSSetShaderResources(0, Some(&[Some(y_srv.clone()), Some(uv_srv.clone())]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.Draw(3, 0);

            let (sync_interval, flags) =
                present_params(self.vsync, self.allow_tearing, self.low_latency_mode);
            let hr = self.swap_chain.Present(sync_interval, flags);
            if hr == DXGI_ERROR_WAS_STILL_DRAWING {
                return Ok(());
            }
            hr.ok().context("swapchain present failed")?;
        }
        self.last_present_at = Some(Instant::now());
        Ok(())
    }
}

fn present_params(vsync: bool, allow_tearing: bool, low_latency_mode: bool) -> (u32, DXGI_PRESENT) {
    let mut flags = DXGI_PRESENT(0);
    if vsync {
        return (1, flags);
    }
    if allow_tearing {
        flags = DXGI_PRESENT(flags.0 | DXGI_PRESENT_ALLOW_TEARING.0);
    }
    if low_latency_mode {
        flags = DXGI_PRESENT(flags.0 | DXGI_PRESENT_DO_NOT_WAIT.0);
    }
    (0, flags)
}

fn idle_wait_timeout_ms(low_latency_mode: bool) -> u32 {
    if low_latency_mode {
        0
    } else {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::{idle_wait_timeout_ms, present_params};
    use windows::Win32::Graphics::Dxgi::{
        DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_PRESENT_DO_NOT_WAIT,
    };

    #[test]
    fn present_params_with_vsync_disables_tearing_flag() {
        let (sync, flags) = present_params(true, true, true);
        assert_eq!(sync, 1);
        assert_eq!(flags, DXGI_PRESENT(0));
    }

    #[test]
    fn present_params_uses_tearing_when_allowed() {
        let (sync, flags) = present_params(false, true, false);
        assert_eq!(sync, 0);
        assert_eq!(flags, DXGI_PRESENT_ALLOW_TEARING);
    }

    #[test]
    fn present_params_without_tearing_falls_back_to_zero_flags() {
        let (sync, flags) = present_params(false, false, false);
        assert_eq!(sync, 0);
        assert_eq!(flags, DXGI_PRESENT(0));
    }

    #[test]
    fn present_params_adds_do_not_wait_in_low_latency_mode() {
        let (sync, flags) = present_params(false, true, true);
        assert_eq!(sync, 0);
        assert_eq!(
            flags,
            DXGI_PRESENT(DXGI_PRESENT_ALLOW_TEARING.0 | DXGI_PRESENT_DO_NOT_WAIT.0)
        );
    }

    #[test]
    fn idle_wait_timeout_is_zero_in_low_latency_mode() {
        assert_eq!(idle_wait_timeout_ms(true), 0);
    }

    #[test]
    fn idle_wait_timeout_is_five_ms_when_not_low_latency() {
        assert_eq!(idle_wait_timeout_ms(false), 5);
    }
}

fn compile_hlsl(src: &[u8], entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    unsafe {
        let mut blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;
        D3DCompile(
            src.as_ptr() as *const c_void,
            src.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut blob,
            Some(&mut err_blob),
        )
        .map_err(|e| {
            if let Some(err) = err_blob {
                let ptr = err.GetBufferPointer() as *const u8;
                let len = err.GetBufferSize();
                let msg = String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len));
                anyhow::anyhow!("D3DCompile failed: {} ({})", e, msg)
            } else {
                anyhow::anyhow!("D3DCompile failed: {}", e)
            }
        })?;
        blob.context("shader blob is empty")
    }
}
