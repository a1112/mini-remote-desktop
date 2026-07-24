#[cfg(target_os = "macos")]
use std::{
    collections::{HashMap, VecDeque},
    ffi::c_int,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use bytes::{Bytes, BytesMut};
#[cfg(target_os = "macos")]
use mrd_ipc::render_proxy::{
    decode_frame_header, encode_ack, expected_payload_len, RenderProxyAck, RenderProxyPixelFormat,
    FRAME_HEADER_LEN,
};
#[cfg(target_os = "macos")]
use mrd_pipeline_core::{DecodedFrame, DecodedFrameData, VideoDecoder};
#[cfg(target_os = "macos")]
use mrd_render::{RenderFrame, RenderTarget, RendererInstance, RendererSnapshot};
#[cfg(target_os = "macos")]
use nix::sys::socket::{setsockopt, sockopt};
#[cfg(target_os = "macos")]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_DECODE_ENV: &str = "MRD_MACOS_RENDER_PROXY_DECODE";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV: &str = "MRD_MACOS_RENDER_PROXY_ASYNC_PRESENT";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_PRESENT_ENV: &str = "MRD_MACOS_RENDER_PROXY_PRESENT";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV: &str = "MRD_MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_ENV: &str = "MRD_MACOS_RENDER_PROXY_SLOW_PRESENT_RESET";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_THRESHOLD_MS_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_THRESHOLD_MS";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FRAMES_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FRAMES";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_COOLDOWN_MS_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_COOLDOWN_MS";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS";
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS_ENV: &str =
    "MRD_MACOS_RENDER_PROXY_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS";
#[cfg(target_os = "macos")]
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES: usize = 8 * 1024 * 1024;
#[cfg(target_os = "macos")]
const DEFAULT_SLOW_PRESENT_RESET_THRESHOLD_MS: f64 = 32.0;
#[cfg(target_os = "macos")]
const DEFAULT_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS: f64 = 16.0;
#[cfg(target_os = "macos")]
const DEFAULT_SLOW_PRESENT_RESET_FRAMES: u32 = 2;
#[cfg(target_os = "macos")]
const DEFAULT_SLOW_PRESENT_RESET_COOLDOWN_MS: u64 = 750;
#[cfg(target_os = "macos")]
const DEFAULT_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS: u32 = 2;
#[cfg(target_os = "macos")]
const DEFAULT_RENDER_PROXY_MAX_DRAWABLE_COUNT: u32 = 2;
#[cfg(target_os = "macos")]
const DEFAULT_RENDER_PROXY_QUEUE_CAPACITY: usize = 3;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: c_int) -> c_int;
}

#[derive(Default)]
pub struct RenderProxyRegistry {
    #[cfg(target_os = "macos")]
    surfaces: Mutex<HashMap<(String, String), RenderProxySurface>>,
}

#[cfg(target_os = "macos")]
struct RenderProxySurface {
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
    render_queue: RenderProxyRenderQueue,
}

#[cfg(target_os = "macos")]
struct RenderProxyState {
    render_queue: RenderProxyRenderQueue,
    h264_pixel_buffer_decoder: Option<mrd_codec_videotoolbox::VideoToolboxH264PixelBufferDecoder>,
    hevc_pixel_buffer_decoder: Option<mrd_codec_videotoolbox::VideoToolboxHevcPixelBufferDecoder>,
    h264_decoder: Option<Box<dyn VideoDecoder>>,
    hevc_decoder: Option<Box<dyn VideoDecoder>>,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct RenderProxyRenderQueue {
    inner: Arc<RenderProxyRenderQueueInner>,
}

#[cfg(target_os = "macos")]
struct RenderProxyRenderQueueInner {
    renderer: Mutex<mrd_render_macos::MacosMetalRenderer>,
    state: Mutex<RenderProxyRenderQueueState>,
    slow_present_state: Mutex<RenderProxySlowPresentResetState>,
    ready: Condvar,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    async_present: bool,
    present_enabled: bool,
    target_window_handle: isize,
    slow_present_reset: RenderProxySlowPresentResetConfig,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct RenderProxyRenderQueueState {
    pending: VecDeque<RenderProxyQueuedFrame>,
    stats: RenderProxyRenderStats,
    shutdown: bool,
}

#[cfg(target_os = "macos")]
#[derive(Default, Copy, Clone)]
struct RenderProxyRenderStats {
    presented_frames: u64,
    present_skips: u64,
    queue_replacements: u64,
    draw_present_duration_ms: f64,
    next_drawable_duration_ms: f64,
    encode_commit_duration_ms: f64,
    max_drawable_count: Option<u32>,
    display_sync_enabled: Option<bool>,
}

#[cfg(target_os = "macos")]
#[derive(Copy, Clone)]
struct RenderProxySlowPresentResetConfig {
    enabled: bool,
    threshold_ms: f64,
    double_buffer_fallback_threshold_ms: f64,
    consecutive_frames: u32,
    cooldown: Duration,
    fallback_after_resets: u32,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct RenderProxySlowPresentResetState {
    consecutive_slow_frames: u32,
    last_reset_at: Option<Instant>,
    reset_count: u64,
    max_drawable_count_override: Option<u32>,
}

#[cfg(target_os = "macos")]
impl RenderProxyRenderStats {
    fn add(&mut self, other: Self) {
        self.presented_frames = self.presented_frames.saturating_add(other.presented_frames);
        self.present_skips = self.present_skips.saturating_add(other.present_skips);
        self.queue_replacements = self
            .queue_replacements
            .saturating_add(other.queue_replacements);
        self.draw_present_duration_ms += other.draw_present_duration_ms;
        self.next_drawable_duration_ms += other.next_drawable_duration_ms;
        self.encode_commit_duration_ms += other.encode_commit_duration_ms;
        if other.max_drawable_count.is_some() {
            self.max_drawable_count = other.max_drawable_count;
        }
        if other.display_sync_enabled.is_some() {
            self.display_sync_enabled = other.display_sync_enabled;
        }
    }
}

#[cfg(target_os = "macos")]
enum RenderProxyQueuedFrame {
    RenderFrame(RenderFrame),
    CvPixelBufferNv12(mrd_codec_videotoolbox::VideoToolboxPixelBufferFrame),
}

impl RenderProxyRegistry {
    #[cfg(target_os = "macos")]
    pub fn attach_surface(
        &self,
        session_id: &str,
        surface_id: &str,
        window_handle: isize,
    ) -> Result<Option<String>, String> {
        let key = (session_id.to_string(), surface_id.to_string());
        self.detach_surface(session_id, surface_id);

        let path = render_proxy_socket_path(session_id, surface_id);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)
            .map_err(|error| format!("bind macOS render proxy socket failed: {error}"))?;
        let mut renderer = mrd_render_macos::MacosMetalRenderer::new()
            .map_err(|error| format!("create macOS render proxy renderer failed: {error}"))?;
        renderer
            .attach_target_with_max_drawable_count(
                RenderTarget::WindowHandle(window_handle),
                macos_render_proxy_max_drawable_count(),
            )
            .map_err(|error| format!("attach macOS render proxy renderer failed: {error}"))?;
        let render_queue = RenderProxyRenderQueue::spawn(renderer, window_handle)
            .map_err(|error| format!("start macOS render proxy queue failed: {error}"))?;
        let state = Arc::new(Mutex::new(RenderProxyState {
            render_queue: render_queue.clone(),
            h264_pixel_buffer_decoder: None,
            hevc_pixel_buffer_decoder: None,
            h264_decoder: None,
            hevc_decoder: None,
        }));
        let task_path = path.clone();
        let task = tokio::spawn(async move {
            run_macos_render_proxy(listener, state).await;
            let _ = std::fs::remove_file(task_path);
        });
        self.surfaces
            .lock()
            .expect("lock render proxy registry")
            .insert(
                key,
                RenderProxySurface {
                    path: path.clone(),
                    task,
                    render_queue,
                },
            );
        Ok(Some(path.to_string_lossy().to_string()))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn attach_surface(
        &self,
        _session_id: &str,
        _surface_id: &str,
        _window_handle: isize,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    #[cfg(target_os = "macos")]
    pub fn detach_surface(&self, session_id: &str, surface_id: &str) {
        let removed = self
            .surfaces
            .lock()
            .expect("lock render proxy registry")
            .remove(&(session_id.to_string(), surface_id.to_string()));
        if let Some(surface) = removed {
            surface.task.abort();
            surface.render_queue.shutdown();
            let _ = std::fs::remove_file(surface.path);
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn detach_surface(&self, _session_id: &str, _surface_id: &str) {}
}

#[cfg(target_os = "macos")]
fn render_proxy_socket_path(session_id: &str, surface_id: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    surface_id.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!(
        "mrd-render-proxy-{}-{hash:016x}.sock",
        std::process::id()
    ))
}

#[cfg(target_os = "macos")]
impl RenderProxyRenderQueue {
    fn spawn(
        renderer: mrd_render_macos::MacosMetalRenderer,
        target_window_handle: isize,
    ) -> Result<Self, String> {
        let async_present = macos_render_proxy_async_present_enabled();
        let present_enabled = macos_render_proxy_present_enabled();
        let slow_present_reset = macos_render_proxy_slow_present_reset_config();
        let inner = Arc::new(RenderProxyRenderQueueInner {
            renderer: Mutex::new(renderer),
            state: Mutex::new(RenderProxyRenderQueueState::default()),
            slow_present_state: Mutex::new(RenderProxySlowPresentResetState::default()),
            ready: Condvar::new(),
            worker: Mutex::new(None),
            async_present,
            present_enabled,
            target_window_handle,
            slow_present_reset,
        });
        if async_present && present_enabled {
            let worker_inner = inner.clone();
            let handle = thread::Builder::new()
                .name("mrd-rdesk-render-proxy".to_string())
                .spawn(move || run_render_proxy_render_worker(worker_inner))
                .map_err(|error| format!("spawn macOS render proxy worker failed: {error}"))?;
            *inner
                .worker
                .lock()
                .map_err(|_| "macOS render proxy worker lock was poisoned".to_string())? =
                Some(handle);
        }
        Ok(Self { inner })
    }

    fn enqueue_latest(
        &self,
        frame: RenderProxyQueuedFrame,
    ) -> Result<RenderProxyRenderStats, String> {
        if !self.inner.present_enabled {
            let mut stats = self.take_stats()?;
            stats.present_skips = stats.present_skips.saturating_add(1);
            drop(frame);
            return Ok(stats);
        }
        if !self.inner.async_present {
            return self.render_immediately(frame);
        }
        self.enqueue_latest_async(frame)
    }

    fn enqueue_latest_async(
        &self,
        frame: RenderProxyQueuedFrame,
    ) -> Result<RenderProxyRenderStats, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "macOS render proxy queue lock was poisoned".to_string())?;
        if state.shutdown {
            return Err("macOS render proxy queue is shut down".to_string());
        }
        let replaced = push_render_proxy_frame_bounded(
            &mut state.pending,
            frame,
            DEFAULT_RENDER_PROXY_QUEUE_CAPACITY,
        );
        if replaced {
            state.stats.queue_replacements = state.stats.queue_replacements.saturating_add(1);
        }
        let stats = std::mem::take(&mut state.stats);
        self.inner.ready.notify_one();
        Ok(stats)
    }

    fn render_immediately(
        &self,
        frame: RenderProxyQueuedFrame,
    ) -> Result<RenderProxyRenderStats, String> {
        let mut stats = self.take_stats()?;
        let mut renderer = self
            .inner
            .renderer
            .lock()
            .map_err(|_| "macOS render proxy renderer lock was poisoned".to_string())?;
        let mut frame_stats = render_proxy_frame_once(&mut renderer, frame)?;
        maybe_reset_render_proxy_slow_present(&self.inner, &mut renderer, &mut frame_stats);
        stats.add(frame_stats);
        Ok(stats)
    }

    fn take_stats(&self) -> Result<RenderProxyRenderStats, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "macOS render proxy queue lock was poisoned".to_string())?;
        Ok(std::mem::take(&mut state.stats))
    }

    fn shutdown(&self) {
        {
            let Ok(mut state) = self.inner.state.lock() else {
                return;
            };
            state.shutdown = true;
            state.pending.clear();
            self.inner.ready.notify_all();
        }
        if let Ok(mut worker) = self.inner.worker.lock() {
            if let Some(handle) = worker.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn run_render_proxy_render_worker(inner: Arc<RenderProxyRenderQueueInner>) {
    configure_render_proxy_render_thread();
    while let Some(frame) = take_next_render_proxy_frame(&inner) {
        let stats = inner
            .renderer
            .lock()
            .map_err(|_| "macOS render proxy renderer lock was poisoned".to_string())
            .and_then(|mut renderer| {
                let mut stats = render_proxy_frame_once(&mut renderer, frame)?;
                maybe_reset_render_proxy_slow_present(&inner, &mut renderer, &mut stats);
                Ok(stats)
            });
        match stats {
            Ok(stats) => record_render_proxy_stats(&inner, stats),
            Err(error) => {
                tracing::warn!(%error, "macOS render proxy worker failed to present frame")
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn configure_render_proxy_render_thread() {
    let status = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
    if status != 0 {
        tracing::debug!(status, "failed to raise macOS render proxy worker QoS");
    }
}

#[cfg(target_os = "macos")]
fn take_next_render_proxy_frame(
    inner: &RenderProxyRenderQueueInner,
) -> Option<RenderProxyQueuedFrame> {
    let mut state = inner.state.lock().ok()?;
    loop {
        if state.shutdown {
            return None;
        }
        if let Some(frame) = state.pending.pop_front() {
            return Some(frame);
        }
        state = inner.ready.wait(state).ok()?;
    }
}

#[cfg(target_os = "macos")]
fn push_render_proxy_frame_bounded<T>(
    pending: &mut VecDeque<T>,
    frame: T,
    capacity: usize,
) -> bool {
    let capacity = capacity.max(1);
    let replaced = if pending.len() >= capacity {
        pending.pop_front();
        true
    } else {
        false
    };
    pending.push_back(frame);
    replaced
}

#[cfg(target_os = "macos")]
fn render_proxy_frame_once(
    renderer: &mut mrd_render_macos::MacosMetalRenderer,
    frame: RenderProxyQueuedFrame,
) -> Result<RenderProxyRenderStats, String> {
    let draw_started = std::time::Instant::now();
    let before = renderer.snapshot();
    match frame {
        RenderProxyQueuedFrame::RenderFrame(frame) => renderer
            .upload_frame(frame)
            .map_err(|error| format!("macOS render proxy upload failed: {error}"))?,
        RenderProxyQueuedFrame::CvPixelBufferNv12(frame) => unsafe {
            renderer
                .upload_cv_pixel_buffer_nv12(
                    frame.width(),
                    frame.height(),
                    frame.pixel_buffer_ptr(),
                )
                .map_err(|error| {
                    format!("macOS render proxy CVPixelBuffer upload failed: {error}")
                })?;
        },
    }
    let after = renderer.snapshot();
    Ok(RenderProxyRenderStats {
        presented_frames: renderer_snapshot_presented_delta(&before, &after),
        present_skips: after
            .present_skipped_count
            .saturating_sub(before.present_skipped_count),
        queue_replacements: 0,
        draw_present_duration_ms: draw_started.elapsed().as_secs_f64() * 1000.0,
        next_drawable_duration_ms: after.last_render_wait_for_drawable_ms.unwrap_or(0.0),
        encode_commit_duration_ms: after.last_render_encode_commit_ms.unwrap_or(0.0),
        max_drawable_count: after.swap_chain_max_frame_latency,
        display_sync_enabled: after
            .swap_chain_allow_tearing
            .map(|allow_tearing| !allow_tearing),
    })
}

#[cfg(target_os = "macos")]
fn maybe_reset_render_proxy_slow_present(
    inner: &RenderProxyRenderQueueInner,
    renderer: &mut mrd_render_macos::MacosMetalRenderer,
    stats: &mut RenderProxyRenderStats,
) {
    let config = inner.slow_present_reset;
    if !config.enabled {
        return;
    }

    let double_buffer_slow_present = render_proxy_double_buffer_fallback_due(stats, config);
    let slow_present = double_buffer_slow_present || render_proxy_slow_present_due(stats, config);
    let Ok(mut state) = inner.slow_present_state.lock() else {
        return;
    };
    if !slow_present {
        state.consecutive_slow_frames = 0;
        return;
    }

    state.consecutive_slow_frames = state.consecutive_slow_frames.saturating_add(1);
    if state.consecutive_slow_frames < config.consecutive_frames {
        return;
    }

    let now = Instant::now();
    if state
        .last_reset_at
        .is_some_and(|last_reset_at| now.duration_since(last_reset_at) < config.cooldown)
    {
        return;
    }

    state.consecutive_slow_frames = 0;
    state.last_reset_at = Some(now);
    state.reset_count = state.reset_count.saturating_add(1);
    let reset_count = state.reset_count;
    let fallback_to_triple_buffer = stats.max_drawable_count == Some(2)
        && (double_buffer_slow_present || reset_count >= u64::from(config.fallback_after_resets));
    if fallback_to_triple_buffer {
        state.max_drawable_count_override = Some(3);
    }
    let max_drawable_count_override = state.max_drawable_count_override;
    drop(state);

    let recreated_renderer = max_drawable_count_override.is_some();
    let reset_result = match max_drawable_count_override {
        Some(max_drawable_count) => {
            let mut replacement = match mrd_render_macos::MacosMetalRenderer::new() {
                Ok(replacement) => replacement,
                Err(error) => {
                    return log_render_proxy_slow_present_reset_error(
                        error,
                        stats.draw_present_duration_ms,
                        config.threshold_ms,
                    );
                }
            };
            match replacement.attach_target_with_max_drawable_count(
                RenderTarget::WindowHandle(inner.target_window_handle),
                max_drawable_count,
            ) {
                Ok(()) => {
                    *renderer = replacement;
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        None => renderer.attach_target(RenderTarget::WindowHandle(inner.target_window_handle)),
    };

    match reset_result {
        Ok(()) => {
            let snapshot = renderer.snapshot();
            stats.max_drawable_count = snapshot.swap_chain_max_frame_latency;
            stats.display_sync_enabled = snapshot
                .swap_chain_allow_tearing
                .map(|allow_tearing| !allow_tearing);
            eprintln!(
                "macOS render proxy reset Metal layer after slow present reset_count={reset_count} draw_present_ms={:.3} next_drawable_ms={:.3} threshold_ms={:.3} double_buffer_threshold_ms={:.3} max_drawable_count={:?} fallback_to_triple_buffer={fallback_to_triple_buffer} recreated_renderer={recreated_renderer}",
                stats.draw_present_duration_ms,
                stats.next_drawable_duration_ms,
                config.threshold_ms,
                config.double_buffer_fallback_threshold_ms,
                stats.max_drawable_count
            );
            tracing::warn!(
                reset_count,
                draw_present_ms = stats.draw_present_duration_ms,
                next_drawable_ms = stats.next_drawable_duration_ms,
                threshold_ms = config.threshold_ms,
                double_buffer_threshold_ms = config.double_buffer_fallback_threshold_ms,
                max_drawable_count = ?stats.max_drawable_count,
                fallback_to_triple_buffer,
                recreated_renderer,
                "macOS render proxy reset Metal layer after slow present"
            );
        }
        Err(error) => {
            log_render_proxy_slow_present_reset_error(
                error,
                stats.draw_present_duration_ms,
                config.threshold_ms,
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn render_proxy_slow_present_due(
    stats: &RenderProxyRenderStats,
    config: RenderProxySlowPresentResetConfig,
) -> bool {
    stats.draw_present_duration_ms.is_finite()
        && stats.draw_present_duration_ms >= config.threshold_ms
}

#[cfg(target_os = "macos")]
fn render_proxy_double_buffer_fallback_due(
    stats: &RenderProxyRenderStats,
    config: RenderProxySlowPresentResetConfig,
) -> bool {
    stats.max_drawable_count == Some(2)
        && render_proxy_present_wait_sample_ms(stats)
            .is_some_and(|sample_ms| sample_ms >= config.double_buffer_fallback_threshold_ms)
}

#[cfg(target_os = "macos")]
fn render_proxy_present_wait_sample_ms(stats: &RenderProxyRenderStats) -> Option<f64> {
    [
        stats.draw_present_duration_ms,
        stats.next_drawable_duration_ms,
    ]
    .into_iter()
    .filter(|value| value.is_finite() && *value >= 0.0)
    .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

#[cfg(target_os = "macos")]
fn log_render_proxy_slow_present_reset_error(
    error: mrd_render::RenderError,
    draw_present_duration_ms: f64,
    threshold_ms: f64,
) {
    eprintln!(
        "macOS render proxy failed to reset Metal layer after slow present error={error} draw_present_ms={:.3} threshold_ms={:.3}",
        draw_present_duration_ms, threshold_ms
    );
    tracing::warn!(
        %error,
        draw_present_ms = draw_present_duration_ms,
        threshold_ms,
        "macOS render proxy failed to reset Metal layer after slow present"
    );
}

#[cfg(target_os = "macos")]
fn record_render_proxy_stats(inner: &RenderProxyRenderQueueInner, stats: RenderProxyRenderStats) {
    let Ok(mut state) = inner.state.lock() else {
        return;
    };
    state.stats.presented_frames = state
        .stats
        .presented_frames
        .saturating_add(stats.presented_frames);
    state.stats.present_skips = state
        .stats
        .present_skips
        .saturating_add(stats.present_skips);
    state.stats.draw_present_duration_ms += stats.draw_present_duration_ms;
    if stats.max_drawable_count.is_some() {
        state.stats.max_drawable_count = stats.max_drawable_count;
    }
    if stats.display_sync_enabled.is_some() {
        state.stats.display_sync_enabled = stats.display_sync_enabled;
    }
    state.stats.next_drawable_duration_ms += stats.next_drawable_duration_ms;
    state.stats.encode_commit_duration_ms += stats.encode_commit_duration_ms;
}

#[cfg(target_os = "macos")]
async fn run_macos_render_proxy(listener: UnixListener, state: Arc<Mutex<RenderProxyState>>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        configure_macos_render_proxy_socket(&stream);
        handle_macos_render_proxy_connection(stream, state.clone()).await;
    }
}

#[cfg(target_os = "macos")]
async fn handle_macos_render_proxy_connection(
    mut stream: UnixStream,
    state: Arc<Mutex<RenderProxyState>>,
) {
    loop {
        let mut header_bytes = [0_u8; FRAME_HEADER_LEN];
        if stream.read_exact(&mut header_bytes).await.is_err() {
            break;
        }
        let header = match decode_frame_header(&header_bytes) {
            Ok(header) => header,
            Err(_) => break,
        };
        let expected_len = match header.pixel_format {
            RenderProxyPixelFormat::H264 | RenderProxyPixelFormat::Hevc => {
                header.payload_len as usize
            }
            _ => {
                let Some(expected_len) = expected_payload_len(
                    header.pixel_format,
                    header.width,
                    header.height,
                    header.row_pitch,
                ) else {
                    break;
                };
                expected_len
            }
        };
        if expected_len != header.payload_len as usize || expected_len > 64 * 1024 * 1024 {
            break;
        }
        let ack = match header.pixel_format {
            RenderProxyPixelFormat::H264 => {
                let Ok(payload) = read_render_proxy_payload_vec(&mut stream, expected_len).await
                else {
                    break;
                };
                upload_render_proxy_h264_access_unit(&state, payload)
                    .unwrap_or_else(|error| render_proxy_upload_error_ack(&header, error))
            }
            RenderProxyPixelFormat::Hevc => {
                let Ok(payload) = read_render_proxy_payload_vec(&mut stream, expected_len).await
                else {
                    break;
                };
                upload_render_proxy_hevc_access_unit(&state, payload)
                    .unwrap_or_else(|error| render_proxy_upload_error_ack(&header, error))
            }
            RenderProxyPixelFormat::Rgb24 | RenderProxyPixelFormat::Bgra32 => {
                let Ok(payload) = read_render_proxy_payload_vec(&mut stream, expected_len).await
                else {
                    break;
                };
                let frame = match header.pixel_format {
                    RenderProxyPixelFormat::Rgb24 => RenderFrame::from_rgb24(
                        header.width as usize,
                        header.height as usize,
                        payload,
                    ),
                    RenderProxyPixelFormat::Bgra32 => RenderFrame::from_bgra32(
                        header.width as usize,
                        header.height as usize,
                        payload,
                    ),
                    RenderProxyPixelFormat::Nv12
                    | RenderProxyPixelFormat::H264
                    | RenderProxyPixelFormat::Hevc => {
                        unreachable!("handled above")
                    }
                };
                upload_render_proxy_frame(&state, frame)
                    .unwrap_or_else(|error| render_proxy_upload_error_ack(&header, error))
            }
            RenderProxyPixelFormat::Nv12 => {
                let Ok(payload) = read_render_proxy_payload_bytes(&mut stream, expected_len).await
                else {
                    break;
                };
                let frame = RenderFrame::from_nv12_bytes(
                    header.width as usize,
                    header.height as usize,
                    payload,
                    header.row_pitch.max(header.width) as usize,
                );
                upload_render_proxy_frame(&state, frame)
                    .unwrap_or_else(|error| render_proxy_upload_error_ack(&header, error))
            }
        };
        if stream.write_all(&encode_ack(&ack)).await.is_err() {
            break;
        }
    }
}

#[cfg(target_os = "macos")]
async fn read_render_proxy_payload_vec(
    stream: &mut UnixStream,
    expected_len: usize,
) -> std::io::Result<Vec<u8>> {
    let mut payload = vec![0_u8; expected_len];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

#[cfg(target_os = "macos")]
async fn read_render_proxy_payload_bytes(
    stream: &mut UnixStream,
    expected_len: usize,
) -> std::io::Result<Bytes> {
    let mut payload = BytesMut::with_capacity(expected_len);
    while payload.len() < expected_len {
        let remaining = expected_len - payload.len();
        let read = (&mut *stream)
            .take(remaining as u64)
            .read_buf(&mut payload)
            .await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "render proxy payload ended early",
            ));
        }
    }
    Ok(payload.freeze())
}

#[cfg(target_os = "macos")]
fn configure_macos_render_proxy_socket(stream: &UnixStream) {
    if let Err(error) = setsockopt(
        stream,
        sockopt::SndBuf,
        &MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES,
    ) {
        tracing::debug!(
            %error,
            "failed to set macOS render proxy socket send buffer"
        );
    }
    if let Err(error) = setsockopt(
        stream,
        sockopt::RcvBuf,
        &MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES,
    ) {
        tracing::debug!(
            %error,
            "failed to set macOS render proxy socket receive buffer"
        );
    }
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_frame(
    state: &Arc<Mutex<RenderProxyState>>,
    frame: RenderFrame,
) -> Result<RenderProxyAck, String> {
    let started = std::time::Instant::now();
    let state = state
        .lock()
        .map_err(|_| "macOS render proxy state lock was poisoned".to_string())?;
    let render_stats = state
        .render_queue
        .enqueue_latest(RenderProxyQueuedFrame::RenderFrame(frame))?;
    let mut render_stats = render_stats;
    flush_render_proxy_queue_stats(&state.render_queue, &mut render_stats)?;
    Ok(RenderProxyAck {
        presented_frames: render_stats.presented_frames,
        present_skips: render_stats.present_skips,
        queue_replacements: render_stats.queue_replacements,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms: 0.0,
        draw_present_duration_ms: render_stats.draw_present_duration_ms,
        next_drawable_duration_ms: render_stats.next_drawable_duration_ms,
        encode_commit_duration_ms: render_stats.encode_commit_duration_ms,
        max_drawable_count: render_stats.max_drawable_count,
        display_sync_enabled: render_stats.display_sync_enabled,
    })
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_h264_access_unit(
    state: &Arc<Mutex<RenderProxyState>>,
    payload: Vec<u8>,
) -> Result<RenderProxyAck, String> {
    let started = std::time::Instant::now();
    if !macos_render_proxy_decode_enabled() {
        return Ok(render_proxy_decode_disabled_ack(started));
    }
    let mut state = state
        .lock()
        .map_err(|_| "macOS render proxy state lock was poisoned".to_string())?;
    if macos_render_proxy_cv_pixel_buffer_decode_enabled() {
        match upload_render_proxy_h264_pixel_buffer_locked(&mut state, &payload, started) {
            Ok(ack) => return Ok(ack),
            Err(_) => {
                state.h264_pixel_buffer_decoder = None;
            }
        }
    }
    upload_render_proxy_h264_cpu_locked(&mut state, &payload, started)
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_h264_pixel_buffer_locked(
    state: &mut RenderProxyState,
    payload: &[u8],
    started: std::time::Instant,
) -> Result<RenderProxyAck, String> {
    if state.h264_pixel_buffer_decoder.is_none() {
        let decoder =
            mrd_codec_videotoolbox::VideoToolboxH264PixelBufferDecoder::new().map_err(|error| {
                format!("create VideoToolbox CVPixelBuffer H.264 decoder failed: {error}")
            })?;
        state.h264_pixel_buffer_decoder = Some(decoder);
    }
    let decode_started = std::time::Instant::now();
    let decoded_frames = {
        let decoder = state
            .h264_pixel_buffer_decoder
            .as_mut()
            .ok_or_else(|| "VideoToolbox CVPixelBuffer H.264 decoder missing".to_string())?;
        decoder
            .push_access_unit(payload)
            .map_err(|error| format!("VideoToolbox CVPixelBuffer H.264 decode failed: {error}"))?;
        decoder.drain_decoded_frames()
    };
    let decode_duration_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let mut render_stats = RenderProxyRenderStats::default();
    for decoded_frame in decoded_frames {
        let stats = state
            .render_queue
            .enqueue_latest(RenderProxyQueuedFrame::CvPixelBufferNv12(decoded_frame))?;
        render_stats.add(stats);
    }
    flush_render_proxy_queue_stats(&state.render_queue, &mut render_stats)?;
    Ok(RenderProxyAck {
        presented_frames: render_stats.presented_frames,
        present_skips: render_stats.present_skips,
        queue_replacements: render_stats.queue_replacements,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms,
        draw_present_duration_ms: render_stats.draw_present_duration_ms,
        next_drawable_duration_ms: render_stats.next_drawable_duration_ms,
        encode_commit_duration_ms: render_stats.encode_commit_duration_ms,
        max_drawable_count: render_stats.max_drawable_count,
        display_sync_enabled: render_stats.display_sync_enabled,
    })
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_hevc_access_unit(
    state: &Arc<Mutex<RenderProxyState>>,
    payload: Vec<u8>,
) -> Result<RenderProxyAck, String> {
    let started = std::time::Instant::now();
    if !macos_render_proxy_decode_enabled() {
        return Ok(render_proxy_decode_disabled_ack(started));
    }
    let mut state = state
        .lock()
        .map_err(|_| "macOS render proxy state lock was poisoned".to_string())?;
    if macos_render_proxy_cv_pixel_buffer_decode_enabled() {
        match upload_render_proxy_hevc_pixel_buffer_locked(&mut state, &payload, started) {
            Ok(ack) => return Ok(ack),
            Err(_) => {
                state.hevc_pixel_buffer_decoder = None;
            }
        }
    }
    upload_render_proxy_hevc_cpu_locked(&mut state, &payload, started)
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_hevc_pixel_buffer_locked(
    state: &mut RenderProxyState,
    payload: &[u8],
    started: std::time::Instant,
) -> Result<RenderProxyAck, String> {
    if state.hevc_pixel_buffer_decoder.is_none() {
        let decoder =
            mrd_codec_videotoolbox::VideoToolboxHevcPixelBufferDecoder::new().map_err(|error| {
                format!("create VideoToolbox CVPixelBuffer HEVC decoder failed: {error}")
            })?;
        state.hevc_pixel_buffer_decoder = Some(decoder);
    }
    let decode_started = std::time::Instant::now();
    let decoded_frames = {
        let decoder = state
            .hevc_pixel_buffer_decoder
            .as_mut()
            .ok_or_else(|| "VideoToolbox CVPixelBuffer HEVC decoder missing".to_string())?;
        decoder
            .push_access_unit(&payload)
            .map_err(|error| format!("VideoToolbox CVPixelBuffer HEVC decode failed: {error}"))?;
        decoder.drain_decoded_frames()
    };
    let decode_duration_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let mut render_stats = RenderProxyRenderStats::default();
    for decoded_frame in decoded_frames {
        let stats = state
            .render_queue
            .enqueue_latest(RenderProxyQueuedFrame::CvPixelBufferNv12(decoded_frame))?;
        render_stats.add(stats);
    }
    flush_render_proxy_queue_stats(&state.render_queue, &mut render_stats)?;
    Ok(RenderProxyAck {
        presented_frames: render_stats.presented_frames,
        present_skips: render_stats.present_skips,
        queue_replacements: render_stats.queue_replacements,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms,
        draw_present_duration_ms: render_stats.draw_present_duration_ms,
        next_drawable_duration_ms: render_stats.next_drawable_duration_ms,
        encode_commit_duration_ms: render_stats.encode_commit_duration_ms,
        max_drawable_count: render_stats.max_drawable_count,
        display_sync_enabled: render_stats.display_sync_enabled,
    })
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_hevc_cpu_locked(
    state: &mut RenderProxyState,
    payload: &[u8],
    started: std::time::Instant,
) -> Result<RenderProxyAck, String> {
    if state.hevc_decoder.is_none() {
        let decoder = mrd_codec_videotoolbox::VideoToolboxHevcDecoder::new()
            .map_err(|error| format!("create VideoToolbox HEVC decoder failed: {error}"))?;
        state.hevc_decoder = Some(Box::new(decoder));
    }
    let decoder = state
        .hevc_decoder
        .as_mut()
        .ok_or_else(|| "VideoToolbox HEVC decoder missing".to_string())?;
    let decode_started = std::time::Instant::now();
    decoder
        .push_access_unit(payload)
        .map_err(|error| format!("VideoToolbox HEVC decode failed: {error}"))?;
    let decoded_frames = decoder.drain_decoded_frames();
    let decode_duration_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let mut render_stats = RenderProxyRenderStats::default();
    for decoded_frame in decoded_frames {
        let frame = decoded_frame_to_render_frame(decoded_frame)?;
        let stats = state
            .render_queue
            .enqueue_latest(RenderProxyQueuedFrame::RenderFrame(frame))?;
        render_stats.add(stats);
    }
    flush_render_proxy_queue_stats(&state.render_queue, &mut render_stats)?;
    Ok(RenderProxyAck {
        presented_frames: render_stats.presented_frames,
        present_skips: render_stats.present_skips,
        queue_replacements: render_stats.queue_replacements,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms,
        draw_present_duration_ms: render_stats.draw_present_duration_ms,
        next_drawable_duration_ms: render_stats.next_drawable_duration_ms,
        encode_commit_duration_ms: render_stats.encode_commit_duration_ms,
        max_drawable_count: render_stats.max_drawable_count,
        display_sync_enabled: render_stats.display_sync_enabled,
    })
}

#[cfg(target_os = "macos")]
fn upload_render_proxy_h264_cpu_locked(
    state: &mut RenderProxyState,
    payload: &[u8],
    started: std::time::Instant,
) -> Result<RenderProxyAck, String> {
    if state.h264_decoder.is_none() {
        let decoder = mrd_codec_videotoolbox::VideoToolboxH264Decoder::new()
            .map_err(|error| format!("create VideoToolbox H.264 decoder failed: {error}"))?;
        state.h264_decoder = Some(Box::new(decoder));
    }
    let decoder = state
        .h264_decoder
        .as_mut()
        .ok_or_else(|| "VideoToolbox H.264 decoder missing".to_string())?;
    let decode_started = std::time::Instant::now();
    decoder
        .push_access_unit(&payload)
        .map_err(|error| format!("VideoToolbox H.264 decode failed: {error}"))?;
    let decoded_frames = decoder.drain_decoded_frames();
    let decode_duration_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let mut render_stats = RenderProxyRenderStats::default();
    for decoded_frame in decoded_frames {
        let frame = decoded_frame_to_render_frame(decoded_frame)?;
        let stats = state
            .render_queue
            .enqueue_latest(RenderProxyQueuedFrame::RenderFrame(frame))?;
        render_stats.add(stats);
    }
    flush_render_proxy_queue_stats(&state.render_queue, &mut render_stats)?;
    Ok(RenderProxyAck {
        presented_frames: render_stats.presented_frames,
        present_skips: render_stats.present_skips,
        queue_replacements: render_stats.queue_replacements,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms,
        draw_present_duration_ms: render_stats.draw_present_duration_ms,
        next_drawable_duration_ms: render_stats.next_drawable_duration_ms,
        encode_commit_duration_ms: render_stats.encode_commit_duration_ms,
        max_drawable_count: render_stats.max_drawable_count,
        display_sync_enabled: render_stats.display_sync_enabled,
    })
}

#[cfg(target_os = "macos")]
fn flush_render_proxy_queue_stats(
    render_queue: &RenderProxyRenderQueue,
    render_stats: &mut RenderProxyRenderStats,
) -> Result<(), String> {
    render_stats.add(render_queue.take_stats()?);
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_cv_pixel_buffer_decode_enabled() -> bool {
    match std::env::var(MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
    {
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => false,
        _ => true,
    }
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_decode_enabled() -> bool {
    macos_env_bool(MACOS_RENDER_PROXY_DECODE_ENV, true)
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_async_present_enabled() -> bool {
    match std::env::var(MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
    {
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => false,
        _ => true,
    }
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_present_enabled() -> bool {
    macos_env_bool(MACOS_RENDER_PROXY_PRESENT_ENV, true)
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_max_drawable_count() -> u32 {
    macos_render_proxy_max_drawable_count_from_env_value(
        std::env::var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV).ok(),
    )
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_max_drawable_count_from_env_value(value: Option<String>) -> u32 {
    match value.as_deref().map(str::trim) {
        Some("2") => 2,
        Some("3") => 3,
        _ => DEFAULT_RENDER_PROXY_MAX_DRAWABLE_COUNT,
    }
}

#[cfg(target_os = "macos")]
fn macos_render_proxy_slow_present_reset_config() -> RenderProxySlowPresentResetConfig {
    RenderProxySlowPresentResetConfig {
        enabled: macos_env_bool(MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_ENV, true),
        threshold_ms: macos_env_f64(
            MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_THRESHOLD_MS_ENV,
            DEFAULT_SLOW_PRESENT_RESET_THRESHOLD_MS,
        )
        .max(1.0),
        double_buffer_fallback_threshold_ms: macos_env_f64(
            MACOS_RENDER_PROXY_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS_ENV,
            DEFAULT_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS,
        )
        .max(1.0),
        consecutive_frames: macos_env_u32(
            MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FRAMES_ENV,
            DEFAULT_SLOW_PRESENT_RESET_FRAMES,
        )
        .max(1),
        cooldown: Duration::from_millis(
            u64::from(macos_env_u32(
                MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_COOLDOWN_MS_ENV,
                DEFAULT_SLOW_PRESENT_RESET_COOLDOWN_MS as u32,
            ))
            .max(1),
        ),
        fallback_after_resets: macos_env_u32(
            MACOS_RENDER_PROXY_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS_ENV,
            DEFAULT_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS,
        )
        .max(1),
    }
}

#[cfg(target_os = "macos")]
fn macos_env_bool(name: &str, default_value: bool) -> bool {
    match std::env::var(name)
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
    {
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on") => true,
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => false,
        _ => default_value,
    }
}

#[cfg(target_os = "macos")]
fn macos_env_f64(name: &str, default_value: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default_value)
}

#[cfg(target_os = "macos")]
fn macos_env_u32(name: &str, default_value: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_value)
}

#[cfg(target_os = "macos")]
fn render_proxy_upload_error_ack(
    header: &mrd_ipc::render_proxy::RenderProxyFrameHeader,
    error: String,
) -> RenderProxyAck {
    tracing::warn!(
        pixel_format = ?header.pixel_format,
        width = header.width,
        height = header.height,
        sequence = header.sequence,
        payload_len = header.payload_len,
        %error,
        "macOS render proxy upload failed"
    );
    empty_render_proxy_ack()
}

#[cfg(target_os = "macos")]
fn empty_render_proxy_ack() -> RenderProxyAck {
    RenderProxyAck {
        presented_frames: 0,
        present_skips: 0,
        queue_replacements: 0,
        upload_duration_ms: 0.0,
        decode_duration_ms: 0.0,
        draw_present_duration_ms: 0.0,
        next_drawable_duration_ms: 0.0,
        encode_commit_duration_ms: 0.0,
        max_drawable_count: None,
        display_sync_enabled: None,
    }
}

#[cfg(target_os = "macos")]
fn render_proxy_decode_disabled_ack(started: std::time::Instant) -> RenderProxyAck {
    RenderProxyAck {
        presented_frames: 0,
        present_skips: 1,
        queue_replacements: 0,
        upload_duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        decode_duration_ms: 0.0,
        draw_present_duration_ms: 0.0,
        next_drawable_duration_ms: 0.0,
        encode_commit_duration_ms: 0.0,
        max_drawable_count: None,
        display_sync_enabled: None,
    }
}

#[cfg(target_os = "macos")]
fn decoded_frame_to_render_frame(frame: DecodedFrame) -> Result<RenderFrame, String> {
    match frame.data {
        DecodedFrameData::CpuRgb24(data) => {
            Ok(RenderFrame::from_rgb24(frame.width, frame.height, data))
        }
        DecodedFrameData::CpuBgra32(data) => {
            Ok(RenderFrame::from_bgra32(frame.width, frame.height, data))
        }
        DecodedFrameData::CpuNv12 { data, pitch } => Ok(RenderFrame::from_nv12(
            frame.width,
            frame.height,
            data,
            pitch,
        )),
        DecodedFrameData::CpuI420 { .. } => {
            Err("VideoToolbox proxy decode returned unsupported I420 frame".to_string())
        }
        DecodedFrameData::CpuP010 { .. } => {
            Err("VideoToolbox proxy decode returned unsupported P010 frame".to_string())
        }
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 { .. } | DecodedFrameData::D3D11SharedP010 { .. } => {
            Err("macOS render proxy cannot render D3D11 shared decoded frames".to_string())
        }
    }
}

#[cfg(target_os = "macos")]
fn renderer_snapshot_presented_delta(before: &RendererSnapshot, after: &RendererSnapshot) -> u64 {
    let presented = after
        .presented_frame_count
        .saturating_sub(before.presented_frame_count);
    if presented > 0 {
        return presented;
    }
    let uploaded = after
        .uploaded_frame_count
        .saturating_sub(before.uploaded_frame_count);
    let skipped = after
        .present_skipped_count
        .saturating_sub(before.present_skipped_count);
    if uploaded > 0 && skipped == 0 && after.last_present_status.is_none() {
        uploaded
    } else {
        0
    }
}

#[cfg(target_os = "macos")]
impl Drop for RenderProxyRegistry {
    fn drop(&mut self) {
        for (_, surface) in self
            .surfaces
            .get_mut()
            .expect("lock render proxy registry during drop")
            .drain()
        {
            surface.task.abort();
            surface.render_queue.shutdown();
            let _ = std::fs::remove_file(surface.path);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn bounded_render_queue_preserves_short_decode_bursts() {
        let mut pending = VecDeque::new();

        assert!(!push_render_proxy_frame_bounded(&mut pending, 1, 3));
        assert!(!push_render_proxy_frame_bounded(&mut pending, 2, 3));
        assert!(!push_render_proxy_frame_bounded(&mut pending, 3, 3));
        assert_eq!(pending.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn bounded_render_queue_drops_oldest_frame_when_saturated() {
        let mut pending = VecDeque::from([1, 2, 3]);

        assert!(push_render_proxy_frame_bounded(&mut pending, 4, 3));
        assert_eq!(pending.into_iter().collect::<Vec<_>>(), vec![2, 3, 4]);
    }

    fn test_slow_present_config() -> RenderProxySlowPresentResetConfig {
        RenderProxySlowPresentResetConfig {
            enabled: true,
            threshold_ms: DEFAULT_SLOW_PRESENT_RESET_THRESHOLD_MS,
            double_buffer_fallback_threshold_ms: DEFAULT_DOUBLE_BUFFER_FALLBACK_THRESHOLD_MS,
            consecutive_frames: DEFAULT_SLOW_PRESENT_RESET_FRAMES,
            cooldown: Duration::from_millis(DEFAULT_SLOW_PRESENT_RESET_COOLDOWN_MS),
            fallback_after_resets: DEFAULT_SLOW_PRESENT_RESET_FALLBACK_AFTER_RESETS,
        }
    }

    #[test]
    fn double_buffer_fallback_uses_next_drawable_wait_sample() {
        let config = test_slow_present_config();
        let stats = RenderProxyRenderStats {
            draw_present_duration_ms: 17.0,
            next_drawable_duration_ms: 16.5,
            max_drawable_count: Some(2),
            ..RenderProxyRenderStats::default()
        };

        assert!(render_proxy_double_buffer_fallback_due(&stats, config));
        assert!(!render_proxy_slow_present_due(&stats, config));
    }

    #[test]
    fn triple_buffer_uses_regular_slow_present_threshold() {
        let config = test_slow_present_config();
        let stats = RenderProxyRenderStats {
            draw_present_duration_ms: 17.0,
            next_drawable_duration_ms: 16.5,
            max_drawable_count: Some(3),
            ..RenderProxyRenderStats::default()
        };

        assert!(!render_proxy_double_buffer_fallback_due(&stats, config));
        assert!(!render_proxy_slow_present_due(&stats, config));
    }

    #[test]
    fn async_present_env_defaults_on_and_accepts_falsey_override() {
        std::env::remove_var(MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV);
        assert!(macos_render_proxy_async_present_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV, "false");
        assert!(!macos_render_proxy_async_present_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV, "true");
        assert!(macos_render_proxy_async_present_enabled());
        std::env::remove_var(MACOS_RENDER_PROXY_ASYNC_PRESENT_ENV);
    }

    #[test]
    fn present_env_defaults_on_and_accepts_falsey_override() {
        std::env::remove_var(MACOS_RENDER_PROXY_PRESENT_ENV);
        assert!(macos_render_proxy_present_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_PRESENT_ENV, "off");
        assert!(!macos_render_proxy_present_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_PRESENT_ENV, "1");
        assert!(macos_render_proxy_present_enabled());
        std::env::remove_var(MACOS_RENDER_PROXY_PRESENT_ENV);
    }

    #[test]
    fn max_drawable_count_env_defaults_to_two_and_accepts_three() {
        std::env::remove_var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV);
        assert_eq!(macos_render_proxy_max_drawable_count(), 2);

        std::env::set_var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV, "3");
        assert_eq!(macos_render_proxy_max_drawable_count(), 3);

        std::env::set_var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV, "2");
        assert_eq!(macos_render_proxy_max_drawable_count(), 2);

        std::env::set_var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV, "4");
        assert_eq!(macos_render_proxy_max_drawable_count(), 2);
        std::env::remove_var(MACOS_RENDER_PROXY_MAX_DRAWABLE_COUNT_ENV);
    }

    #[test]
    fn decode_env_defaults_on_and_accepts_falsey_override() {
        std::env::remove_var(MACOS_RENDER_PROXY_DECODE_ENV);
        assert!(macos_render_proxy_decode_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_DECODE_ENV, "no");
        assert!(!macos_render_proxy_decode_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_DECODE_ENV, "true");
        assert!(macos_render_proxy_decode_enabled());
        std::env::remove_var(MACOS_RENDER_PROXY_DECODE_ENV);
    }

    #[test]
    fn cv_pixel_buffer_decode_env_defaults_on_and_accepts_falsey_override() {
        std::env::remove_var(MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV);
        assert!(macos_render_proxy_cv_pixel_buffer_decode_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV, "off");
        assert!(!macos_render_proxy_cv_pixel_buffer_decode_enabled());

        std::env::set_var(MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV, "1");
        assert!(macos_render_proxy_cv_pixel_buffer_decode_enabled());
        std::env::remove_var(MACOS_RENDER_PROXY_CVPIXELBUFFER_DECODE_ENV);
    }

    #[tokio::test]
    async fn payload_bytes_reader_does_not_consume_trailing_frame_bytes() {
        let (mut writer, mut reader) = UnixStream::pair().expect("create unix stream pair");
        let payload = [1_u8, 2, 3, 4, 5];
        let trailing = [9_u8, 8, 7, 6];

        writer.write_all(&payload).await.expect("write payload");
        writer
            .write_all(&trailing)
            .await
            .expect("write trailing bytes");

        let read_payload = read_render_proxy_payload_bytes(&mut reader, payload.len())
            .await
            .expect("read payload");
        assert_eq!(read_payload.as_ref(), payload);

        let mut read_trailing = [0_u8; 4];
        reader
            .read_exact(&mut read_trailing)
            .await
            .expect("read trailing bytes");
        assert_eq!(read_trailing, trailing);
    }
}
