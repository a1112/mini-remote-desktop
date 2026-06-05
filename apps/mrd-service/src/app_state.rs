#![allow(dead_code)]

// mrd-service application state
//
// This module defines the shared state owned by mrd-service.
// After the hard-cut migration, this becomes the single source
// of truth for all session orchestration, transport runtime,
// and media control.

use crate::control_input::ControlInputRegistry;
use mrd_application::ports::SessionSnapshot;
#[cfg(target_os = "macos")]
use mrd_ipc::render_proxy::{
    decode_ack, encode_frame_header, RenderProxyFrameHeader, RenderProxyPixelFormat,
};
use mrd_ipc::{
    AttachedRenderSurface, AuditEvent, AuditLogQuery, CapabilitySnapshot, MediaAdaptationSnapshot,
    MediaPipelineSnapshot, MediaProfile, MediaSenderTransportSnapshot, MediaStageMetrics,
    MediaTestImpairmentSnapshot, PairedDeviceIdentity,
};
use mrd_proto::{DeviceId, SessionId};
#[cfg(windows)]
use mrd_render::RendererFactory;
#[cfg(any(windows, target_os = "macos"))]
use mrd_render::{BoxedRenderer, RenderFrame, RenderTarget, RendererSnapshot};
#[cfg(windows)]
use mrd_render_d3d11::D3d11RendererFactory;
#[cfg(target_os = "macos")]
use nix::sys::socket::{setsockopt, sockopt};
use std::collections::{HashMap, VecDeque};
#[cfg(target_os = "macos")]
use std::io::{self, IoSlice, Read, Write};
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::Arc;
#[cfg(any(windows, target_os = "macos"))]
use std::sync::Mutex as StdMutex;
#[cfg(any(windows, target_os = "macos"))]
use std::time::Duration;
#[cfg(any(windows, target_os = "macos"))]
use tokio::time::Instant;
use tokio::{sync::Mutex, task::AbortHandle};

mod capture_source_registry;
mod display_mode_registry;
mod media_profile_registry;
pub use capture_source_registry::CaptureSourceRegistry;
pub use display_mode_registry::DisplayModeRegistry;
pub use media_profile_registry::MediaProfileRegistry;

const MEDIA_STAGE_SAMPLE_LIMIT: usize = 240;
const AUDIT_EVENT_LIMIT: usize = 1_000;
#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Session registry tracking all active sessions
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, SessionSnapshot>,
}

impl SessionRegistry {
    pub fn insert(&mut self, session_id: SessionId, snapshot: SessionSnapshot) {
        self.sessions.insert(session_id, snapshot);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<&SessionSnapshot> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &SessionId) -> Option<&mut SessionSnapshot> {
        self.sessions.get_mut(session_id)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<SessionSnapshot> {
        self.sessions.remove(session_id)
    }

    pub fn list_all(&self) -> Vec<SessionSnapshot> {
        self.sessions.values().cloned().collect()
    }
}

/// Probe telemetry accumulated from LAN data-plane probe frames.
#[derive(Debug, Default)]
pub struct ProbeRegistry {
    probes: HashMap<SessionId, SessionProbeStats>,
}

/// Peer media capabilities observed for each active session.
#[derive(Debug, Default)]
pub struct SessionPeerMediaCapabilityRegistry {
    capabilities: HashMap<SessionId, Vec<String>>,
}

impl SessionPeerMediaCapabilityRegistry {
    pub fn set(&mut self, session_id: SessionId, capabilities: Vec<String>) {
        self.capabilities.insert(session_id, capabilities);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.get(session_id).cloned()
    }

    pub fn supports(&self, session_id: &SessionId, capability: &str) -> bool {
        self.capabilities
            .get(session_id)
            .map(|capabilities| capabilities.iter().any(|value| value == capability))
            .unwrap_or(false)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.remove(session_id)
    }
}

/// Cached service-owned capability snapshot exposed to UI and session handlers.
#[derive(Debug)]
pub struct CapabilitySnapshotRegistry {
    snapshot: CapabilitySnapshot,
    refresh_in_progress: bool,
}

impl Default for CapabilitySnapshotRegistry {
    fn default() -> Self {
        Self {
            snapshot: crate::capabilities::local_capability_snapshot_static(),
            refresh_in_progress: false,
        }
    }
}

impl CapabilitySnapshotRegistry {
    pub fn snapshot(&self) -> CapabilitySnapshot {
        self.snapshot.clone()
    }

    pub fn replace(&mut self, snapshot: CapabilitySnapshot) {
        self.snapshot = snapshot;
        self.refresh_in_progress = false;
    }

    pub fn begin_refresh(&mut self) -> bool {
        if self.refresh_in_progress {
            return false;
        }
        self.refresh_in_progress = true;
        true
    }

    pub fn finish_refresh(&mut self, snapshot: Option<CapabilitySnapshot>) {
        if let Some(snapshot) = snapshot {
            self.snapshot = snapshot;
        }
        self.refresh_in_progress = false;
    }
}

/// Runtime receiver media pipeline state keyed by session.
#[derive(Debug, Default)]
pub struct MediaPipelineRegistry {
    pipelines: HashMap<SessionId, MediaPipelineState>,
}

#[derive(Debug, Clone, Default)]
struct MediaPipelineState {
    attached_surfaces: HashMap<String, AttachedRenderSurface>,
    active_encoder: Option<String>,
    active_decoder: Option<String>,
    active_renderer: Option<String>,
    active_codec: Option<String>,
    active_codec_profile: Option<String>,
    active_bit_depth: Option<u8>,
    active_chroma_subsampling: Option<String>,
    active_pixel_format: Option<String>,
    active_hdr_enabled: Option<bool>,
    active_width: Option<u32>,
    active_height: Option<u32>,
    active_fps: Option<u32>,
    active_bitrate_mbps: Option<u32>,
    codec_fallback_reason: Option<String>,
    queue_depth: u32,
    dropped_frames: u64,
    render_presented_frames: u64,
    render_queue_replacements: u64,
    render_stale_frame_drops: u64,
    render_lock_drops: u64,
    render_present_skips: u64,
    render_pacing_target_fps: Option<u32>,
    render_queue_policy: Option<String>,
    swap_chain_max_frame_latency: Option<u32>,
    swap_chain_allow_tearing: Option<bool>,
    swap_chain_waitable_object: Option<bool>,
    swap_chain_present_mode: Option<String>,
    display_refresh_hz: Option<u32>,
    render_thread_priority: Option<String>,
    render_waitable_timeouts: u64,
    stage_samples: HashMap<String, VecDeque<f64>>,
    stage_summaries: HashMap<String, MediaStageMetrics>,
    test_impairment: Option<MediaTestImpairmentSnapshot>,
    sender_transport: MediaSenderTransportSnapshot,
    adaptation: Option<MediaAdaptationSnapshot>,
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Debug, PartialEq, Eq)]
pub enum MediaRenderQueueEnqueue {
    Start(MediaRenderFrame),
    Queued { replaced: bool, depth: usize },
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRenderFrame {
    Decoded(RenderFrame),
    #[cfg(target_os = "macos")]
    H264AccessUnit {
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    },
    #[cfg(target_os = "macos")]
    HevcAccessUnit {
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    },
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
struct MediaRenderQueueState {
    running: bool,
    pending: VecDeque<MediaRenderFrame>,
    last_enqueue_at: Option<Instant>,
    last_present_at: Option<Instant>,
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
pub struct MediaRenderQueueRegistry {
    queues: HashMap<SessionId, MediaRenderQueueState>,
}

#[cfg(any(windows, target_os = "macos"))]
impl MediaRenderQueueRegistry {
    pub fn enqueue_latest(
        &mut self,
        session_id: SessionId,
        frame: MediaRenderFrame,
    ) -> MediaRenderQueueEnqueue {
        self.enqueue_bounded(session_id, frame, 1)
    }

    pub fn enqueue_bounded(
        &mut self,
        session_id: SessionId,
        frame: MediaRenderFrame,
        max_pending_frames: usize,
    ) -> MediaRenderQueueEnqueue {
        let state = self.queues.entry(session_id).or_default();
        if !state.running {
            state.running = true;
            return MediaRenderQueueEnqueue::Start(frame);
        }

        let max_pending_frames = max_pending_frames.max(1);
        let replaced = if state.pending.len() >= max_pending_frames {
            state.pending.pop_front();
            true
        } else {
            false
        };
        state.pending.push_back(frame);
        MediaRenderQueueEnqueue::Queued {
            replaced,
            depth: state.pending.len(),
        }
    }

    pub fn take_next_or_finish(&mut self, session_id: &SessionId) -> Option<MediaRenderFrame> {
        let state = self.queues.get_mut(session_id)?;

        if let Some(frame) = state.pending.pop_front() {
            return Some(frame);
        }

        state.running = false;
        None
    }

    pub fn take_latest_or_finish(
        &mut self,
        session_id: &SessionId,
    ) -> (Option<MediaRenderFrame>, usize) {
        let Some(state) = self.queues.get_mut(session_id) else {
            return (None, 0);
        };

        let Some(frame) = state.pending.pop_back() else {
            state.running = false;
            return (None, 0);
        };
        let dropped = state.pending.len();
        state.pending.clear();
        (Some(frame), dropped)
    }

    pub fn pending_depth(&self, session_id: &SessionId) -> usize {
        self.queues
            .get(session_id)
            .map_or(0, |state| state.pending.len())
    }

    pub fn pacing_delay(&self, session_id: &SessionId, fps: u32, now: Instant) -> Duration {
        let Some(last_present_at) = self
            .queues
            .get(session_id)
            .and_then(|state| state.last_present_at)
        else {
            return Duration::ZERO;
        };
        let Some(frame_interval) = render_frame_interval(fps) else {
            return Duration::ZERO;
        };
        let elapsed = now
            .checked_duration_since(last_present_at)
            .unwrap_or(Duration::ZERO);
        frame_interval.saturating_sub(elapsed)
    }

    pub fn record_enqueued(&mut self, session_id: &SessionId, at: Instant) -> Option<Duration> {
        let state = self.queues.entry(session_id.clone()).or_default();
        let gap = state
            .last_enqueue_at
            .and_then(|last| at.checked_duration_since(last));
        state.last_enqueue_at = Some(at);
        gap
    }

    pub fn record_presented(&mut self, session_id: &SessionId, at: Instant) -> Option<Duration> {
        let state = self.queues.entry(session_id.clone()).or_default();
        let gap = state
            .last_present_at
            .and_then(|last| at.checked_duration_since(last));
        state.last_present_at = Some(at);
        gap
    }

    pub fn remove(&mut self, session_id: &SessionId) {
        self.queues.remove(session_id);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn render_frame_interval(fps: u32) -> Option<Duration> {
    if fps == 0 {
        return None;
    }

    Some(Duration::from_secs_f64(1.0 / f64::from(fps)))
}

impl MediaPipelineRegistry {
    pub fn attach_surface(&mut self, session_id: SessionId, surface: AttachedRenderSurface) {
        let session_id_label = session_id.0.clone();
        let surface_id = surface.surface_id.clone();
        let backend = surface.backend.clone();
        let state = self.pipelines.entry(session_id).or_default();
        if state.active_renderer.is_none() {
            state.active_renderer = Some(surface.backend.clone());
        }
        let replaced = state
            .attached_surfaces
            .insert(surface.surface_id.clone(), surface)
            .is_some();
        tracing::info!(
            session_id = %session_id_label,
            surface_id = %surface_id,
            backend = %backend,
            replaced,
            attached_surface_count = state.attached_surfaces.len(),
            "render-surface pipeline attach"
        );
    }

    pub fn detach_surface(&mut self, session_id: &SessionId, surface_id: &str) -> bool {
        let Some(state) = self.pipelines.get_mut(session_id) else {
            tracing::info!(
                session_id = %session_id.0,
                surface_id = %surface_id,
                "render-surface pipeline detach skipped: missing pipeline"
            );
            return false;
        };
        let removed = state.attached_surfaces.remove(surface_id).is_some();
        if state.attached_surfaces.is_empty() {
            state.active_renderer = None;
        }
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface_id,
            removed,
            attached_surface_count = state.attached_surfaces.len(),
            "render-surface pipeline detach"
        );
        removed
    }

    pub fn set_active_decoder(&mut self, session_id: SessionId, decoder: impl Into<String>) {
        self.pipelines.entry(session_id).or_default().active_decoder = Some(decoder.into());
    }

    pub fn set_active_encoder(&mut self, session_id: SessionId, encoder: impl Into<String>) {
        self.pipelines.entry(session_id).or_default().active_encoder = Some(encoder.into());
    }

    pub fn set_active_media_profile(&mut self, session_id: SessionId, profile: &MediaProfile) {
        let state = self.pipelines.entry(session_id).or_default();
        state.active_codec = Some(profile.codec.clone());
        state.active_codec_profile = profile.codec_profile.clone();
        state.active_bit_depth = profile.bit_depth;
        state.active_chroma_subsampling = profile.chroma_subsampling.clone();
        state.active_pixel_format = profile.pixel_format.clone();
        state.active_hdr_enabled = profile.hdr_enabled;
        state.active_width = Some(profile.width);
        state.active_height = Some(profile.height);
        state.active_fps = Some(profile.fps);
        state.active_bitrate_mbps = Some(profile.bitrate_mbps);
    }

    pub fn record_active_media_sample(
        &mut self,
        session_id: SessionId,
        profile: &MediaProfile,
        width: u32,
        height: u32,
        pixel_format: impl Into<String>,
    ) {
        self.set_active_media_profile(session_id.clone(), profile);
        let state = self.pipelines.entry(session_id).or_default();
        state.active_width = Some(width);
        state.active_height = Some(height);
        state.active_pixel_format = Some(pixel_format.into());
    }

    pub fn set_codec_fallback_reason(&mut self, session_id: SessionId, reason: Option<String>) {
        self.pipelines
            .entry(session_id)
            .or_default()
            .codec_fallback_reason = reason;
    }

    pub fn record_queue_depth(&mut self, session_id: SessionId, queue_depth: u32) {
        self.pipelines.entry(session_id).or_default().queue_depth = queue_depth;
    }

    pub fn increment_dropped_frames(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn increment_render_presented_frames(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_presented_frames = state.render_presented_frames.saturating_add(count);
    }

    pub fn increment_render_queue_replacements(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_queue_replacements = state.render_queue_replacements.saturating_add(count);
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn record_render_queue_replacements(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_queue_replacements = state.render_queue_replacements.saturating_add(count);
    }

    pub fn increment_render_stale_frame_drops(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_stale_frame_drops = state.render_stale_frame_drops.saturating_add(count);
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn increment_render_lock_drops(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_lock_drops = state.render_lock_drops.saturating_add(count);
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn increment_render_present_skips(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_present_skips = state.render_present_skips.saturating_add(count);
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn set_render_pacing_target_fps(&mut self, session_id: SessionId, fps: Option<u32>) {
        self.pipelines
            .entry(session_id)
            .or_default()
            .render_pacing_target_fps = fps;
    }

    pub fn set_render_queue_policy(
        &mut self,
        session_id: SessionId,
        policy: Option<impl Into<String>>,
    ) {
        self.pipelines
            .entry(session_id)
            .or_default()
            .render_queue_policy = policy.map(Into::into);
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn record_renderer_snapshot(&mut self, session_id: SessionId, snapshot: &RendererSnapshot) {
        let state = self.pipelines.entry(session_id).or_default();
        state.swap_chain_max_frame_latency = snapshot.swap_chain_max_frame_latency;
        state.swap_chain_allow_tearing = snapshot.swap_chain_allow_tearing;
        state.swap_chain_waitable_object = snapshot.swap_chain_waitable_object;
        state.swap_chain_present_mode = snapshot.swap_chain_present_mode.clone();
        state.display_refresh_hz = snapshot.display_refresh_hz;
        state.render_thread_priority = snapshot.render_thread_priority.clone();
    }

    pub fn increment_render_waitable_timeouts(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.render_waitable_timeouts = state.render_waitable_timeouts.saturating_add(count);
    }

    pub fn record_stage_duration_ms(
        &mut self,
        session_id: SessionId,
        stage: impl Into<String>,
        duration_ms: f64,
    ) {
        if !duration_ms.is_finite() || duration_ms < 0.0 {
            return;
        }
        let samples = self
            .pipelines
            .entry(session_id)
            .or_default()
            .stage_samples
            .entry(stage.into())
            .or_default();
        samples.push_back(duration_ms);
        while samples.len() > MEDIA_STAGE_SAMPLE_LIMIT {
            samples.pop_front();
        }
    }

    pub fn set_stage_metrics(
        &mut self,
        session_id: SessionId,
        metrics: impl IntoIterator<Item = MediaStageMetrics>,
    ) {
        let state = self.pipelines.entry(session_id).or_default();
        for metric in metrics {
            state.stage_summaries.insert(metric.stage.clone(), metric);
        }
    }

    pub fn set_test_impairment(
        &mut self,
        session_id: SessionId,
        impairment: Option<MediaTestImpairmentSnapshot>,
    ) {
        self.pipelines
            .entry(session_id)
            .or_default()
            .test_impairment = impairment;
    }

    pub fn set_sender_transport(
        &mut self,
        session_id: SessionId,
        transport: MediaSenderTransportSnapshot,
    ) {
        self.pipelines
            .entry(session_id)
            .or_default()
            .sender_transport = transport;
    }

    pub fn set_adaptation(
        &mut self,
        session_id: SessionId,
        adaptation: Option<MediaAdaptationSnapshot>,
    ) {
        self.pipelines.entry(session_id).or_default().adaptation = adaptation;
    }

    pub fn adaptation(&self, session_id: &SessionId) -> Option<MediaAdaptationSnapshot> {
        self.pipelines
            .get(session_id)
            .and_then(|state| state.adaptation.clone())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> MediaPipelineSnapshot {
        let state = self.pipelines.get(session_id);
        let stage_metrics = state.map(media_pipeline_stage_metrics).unwrap_or_default();
        MediaPipelineSnapshot {
            session_id: session_id.clone(),
            attached_surfaces: state
                .map(|state| state.attached_surfaces.values().cloned().collect())
                .unwrap_or_default(),
            active_encoder: state.and_then(|state| state.active_encoder.clone()),
            active_decoder: state.and_then(|state| state.active_decoder.clone()),
            active_renderer: state.and_then(|state| state.active_renderer.clone()),
            active_codec: state.and_then(|state| state.active_codec.clone()),
            active_codec_profile: state.and_then(|state| state.active_codec_profile.clone()),
            active_bit_depth: state.and_then(|state| state.active_bit_depth),
            active_chroma_subsampling: state
                .and_then(|state| state.active_chroma_subsampling.clone()),
            active_pixel_format: state.and_then(|state| state.active_pixel_format.clone()),
            active_hdr_enabled: state.and_then(|state| state.active_hdr_enabled),
            active_width: state.and_then(|state| state.active_width),
            active_height: state.and_then(|state| state.active_height),
            active_fps: state.and_then(|state| state.active_fps),
            active_bitrate_mbps: state.and_then(|state| state.active_bitrate_mbps),
            codec_fallback_reason: state.and_then(|state| state.codec_fallback_reason.clone()),
            queue_depth: state.map_or(0, |state| state.queue_depth),
            dropped_frames: state.map_or(0, |state| state.dropped_frames),
            render_presented_frames: state.map_or(0, |state| state.render_presented_frames),
            render_queue_replacements: state.map_or(0, |state| state.render_queue_replacements),
            render_stale_frame_drops: state.map_or(0, |state| state.render_stale_frame_drops),
            render_lock_drops: state.map_or(0, |state| state.render_lock_drops),
            render_present_skips: state.map_or(0, |state| state.render_present_skips),
            render_pacing_target_fps: state.and_then(|state| state.render_pacing_target_fps),
            render_queue_policy: state.and_then(|state| state.render_queue_policy.clone()),
            swap_chain_max_frame_latency: state
                .and_then(|state| state.swap_chain_max_frame_latency),
            swap_chain_allow_tearing: state.and_then(|state| state.swap_chain_allow_tearing),
            swap_chain_waitable_object: state.and_then(|state| state.swap_chain_waitable_object),
            swap_chain_present_mode: state.and_then(|state| state.swap_chain_present_mode.clone()),
            display_refresh_hz: state.and_then(|state| state.display_refresh_hz),
            render_thread_priority: state.and_then(|state| state.render_thread_priority.clone()),
            render_waitable_timeouts: state.map_or(0, |state| state.render_waitable_timeouts),
            stage_metrics,
            test_impairment: state.and_then(|state| state.test_impairment.clone()),
            sender_transport: state
                .map(|state| state.sender_transport.clone())
                .unwrap_or_default(),
            adaptation: state.and_then(|state| state.adaptation.clone()),
        }
    }

    pub fn remove(&mut self, session_id: &SessionId) {
        self.pipelines.remove(session_id);
    }
}

/// Native renderer instances owned by mrd-service for receiver sessions.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) type SharedSurfaceRenderer = Arc<StdMutex<BoxedRenderer>>;

/// Native renderer instances owned by mrd-service for receiver sessions.
#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
pub struct MediaSurfaceRendererRegistry {
    renderers: HashMap<(SessionId, String), SharedSurfaceRenderer>,
}

#[cfg(any(windows, target_os = "macos"))]
impl MediaSurfaceRendererRegistry {
    pub fn attach_surface(
        &mut self,
        session_id: &SessionId,
        surface: &AttachedRenderSurface,
    ) -> Result<(), String> {
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface.surface_id,
            backend = %surface.backend,
            window_handle = ?surface.window_handle,
            "render-surface renderer attach requested"
        );
        if !surface_backend_matches_platform(&surface.backend) {
            tracing::info!(
                session_id = %session_id.0,
                surface_id = %surface.surface_id,
                backend = %surface.backend,
                "render-surface renderer attach skipped: backend does not match this platform"
            );
            return Ok(());
        }
        let key = (session_id.clone(), surface.surface_id.clone());
        if self.renderers.contains_key(&key) {
            tracing::info!(
                session_id = %session_id.0,
                surface_id = %surface.surface_id,
                renderer_count = self.session_surface_count(session_id),
                "render-surface renderer attach reused existing renderer"
            );
            return Ok(());
        }
        let window_handle = surface.window_handle.ok_or_else(|| {
            format!(
                "render surface {} is missing native handle",
                surface.surface_id
            )
        })?;
        let mut renderer = create_platform_surface_renderer(surface)?;
        renderer
            .attach_target(RenderTarget::WindowHandle(window_handle as isize))
            .map_err(|error| {
                format!("attach {} renderer target failed: {error}", surface.backend)
            })?;
        self.renderers
            .insert(key, Arc::new(StdMutex::new(renderer)));
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface.surface_id,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer attach completed"
        );
        Ok(())
    }

    pub fn detach_surface(&mut self, session_id: &SessionId, surface_id: &str) {
        let removed = self
            .renderers
            .remove(&(session_id.clone(), surface_id.to_string()))
            .is_some();
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface_id,
            removed,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer detach"
        );
    }

    pub fn detach_session(&mut self, session_id: &SessionId) {
        let before = self.renderers.len();
        self.renderers
            .retain(|(renderer_session_id, _), _| renderer_session_id != session_id);
        let removed = before.saturating_sub(self.renderers.len());
        tracing::info!(
            session_id = %session_id.0,
            removed,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer detach session"
        );
    }

    pub fn renderers_for_session(&self, session_id: &SessionId) -> Vec<SharedSurfaceRenderer> {
        self.renderers
            .iter()
            .filter(|((renderer_session_id, _), _)| renderer_session_id == session_id)
            .map(|(_, renderer)| renderer.clone())
            .collect()
    }

    pub fn render_frame(
        &self,
        session_id: &SessionId,
        frame: &RenderFrame,
    ) -> Result<usize, String> {
        let mut rendered = 0;
        for renderer in self.renderers_for_session(session_id) {
            renderer
                .lock()
                .map_err(|_| "native surface renderer lock was poisoned".to_string())?
                .upload_frame(frame.clone())
                .map_err(|error| {
                    format!("upload frame to native surface renderer failed: {error}")
                })?;
            rendered += 1;
        }
        Ok(rendered)
    }

    pub fn session_surface_count(&self, session_id: &SessionId) -> usize {
        self.renderers
            .keys()
            .filter(|(renderer_session_id, _)| renderer_session_id == session_id)
            .count()
    }

    #[cfg(test)]
    pub fn insert_renderer_for_test(
        &mut self,
        session_id: &SessionId,
        surface_id: impl Into<String>,
        renderer: BoxedRenderer,
    ) {
        self.renderers.insert(
            (session_id.clone(), surface_id.into()),
            Arc::new(StdMutex::new(renderer)),
        );
    }
}

#[cfg(windows)]
fn surface_backend_matches_platform(backend: &str) -> bool {
    backend == "d3d11"
}

#[cfg(windows)]
fn create_platform_surface_renderer(
    surface: &AttachedRenderSurface,
) -> Result<BoxedRenderer, String> {
    if surface.backend != "d3d11" {
        return Err(format!(
            "unsupported Windows native render backend: {}",
            surface.backend
        ));
    }
    D3d11RendererFactory
        .create()
        .map_err(|error| format!("create D3D11 renderer failed: {error}"))
}

#[cfg(target_os = "macos")]
fn surface_backend_matches_platform(backend: &str) -> bool {
    matches!(backend, "macos" | "metal")
}

#[cfg(target_os = "macos")]
fn create_platform_surface_renderer(
    surface: &AttachedRenderSurface,
) -> Result<BoxedRenderer, String> {
    if !surface_backend_matches_platform(&surface.backend) {
        return Err(format!(
            "unsupported macOS native render backend: {}",
            surface.backend
        ));
    }
    let endpoint = surface.render_proxy_endpoint.clone().ok_or_else(|| {
        format!(
            "macOS render surface {} is missing render proxy endpoint",
            surface.surface_id
        )
    })?;
    Ok(Box::new(MacosRenderProxyRenderer::new(endpoint)))
}

#[cfg(target_os = "macos")]
struct MacosRenderProxyRenderer {
    endpoint: String,
    stream: Option<StdUnixStream>,
    snapshot: RendererSnapshot,
    sequence: u64,
}

#[cfg(target_os = "macos")]
impl MacosRenderProxyRenderer {
    fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            stream: None,
            snapshot: RendererSnapshot {
                attached_to_target: false,
                uploaded_frame_count: 0,
                presented_frame_count: 0,
                present_skipped_count: 0,
                render_queue_replacements: Some(0),
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: Some("render_proxy".to_string()),
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            },
            sequence: 0,
        }
    }

    fn ensure_stream(&mut self) -> Result<&mut StdUnixStream, RenderError> {
        if self.stream.is_none() {
            let stream = StdUnixStream::connect(&self.endpoint).map_err(|error| {
                RenderError::Message(format!("connect macOS render proxy failed: {error}"))
            })?;
            let timeout = Some(Duration::from_millis(250));
            let _ = stream.set_read_timeout(timeout);
            let _ = stream.set_write_timeout(timeout);
            configure_macos_render_proxy_socket(&stream);
            self.stream = Some(stream);
        }
        self.stream
            .as_mut()
            .ok_or_else(|| RenderError::Message("macOS render proxy stream missing".to_string()))
    }

    fn send_payload(
        &mut self,
        header: &RenderProxyFrameHeader,
        payload: &[u8],
    ) -> Result<mrd_ipc::render_proxy::RenderProxyAck, RenderError> {
        let header_bytes = encode_frame_header(header);
        let mut ack_bytes = [0_u8; mrd_ipc::render_proxy::ACK_LEN];
        let stream = self.ensure_stream()?;
        write_all_vectored_pair(stream, &header_bytes, payload).map_err(|error| {
            RenderError::Message(format!("write macOS render proxy payload failed: {error}"))
        })?;
        stream.read_exact(&mut ack_bytes).map_err(|error| {
            RenderError::Message(format!("read macOS render proxy ack failed: {error}"))
        })?;
        decode_ack(&ack_bytes).map_err(RenderError::Message)
    }

    fn send_payload_with_reconnect(
        &mut self,
        header: &RenderProxyFrameHeader,
        payload: &[u8],
    ) -> Result<mrd_ipc::render_proxy::RenderProxyAck, RenderError> {
        match self.send_payload(header, payload) {
            Ok(ack) => Ok(ack),
            Err(first_error) => {
                self.stream = None;
                self.send_payload(header, payload).map_err(|second_error| {
                    RenderError::Message(format!(
                        "{first_error}; retry after reconnect failed: {second_error}"
                    ))
                })
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn configure_macos_render_proxy_socket(stream: &StdUnixStream) {
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
fn write_all_vectored_pair<W: Write>(
    writer: &mut W,
    mut first: &[u8],
    mut second: &[u8],
) -> io::Result<()> {
    while !first.is_empty() || !second.is_empty() {
        let slices = [IoSlice::new(first), IoSlice::new(second)];
        let written = writer.write_vectored(&slices)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write macOS render proxy payload",
            ));
        }
        if written < first.len() {
            first = &first[written..];
        } else {
            let second_written = written.saturating_sub(first.len()).min(second.len());
            first = &[];
            second = &second[second_written..];
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
impl mrd_render::RendererInstance for MacosRenderProxyRenderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError> {
        let RenderTarget::WindowHandle(window_handle) = target;
        if window_handle == 0 {
            return Err(RenderError::Message(
                "macOS render proxy requires a non-null surface handle".to_string(),
            ));
        }
        self.snapshot.attached_to_target = true;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        let width = u32::try_from(frame.width)
            .map_err(|_| RenderError::Message("render proxy frame width overflow".to_string()))?;
        let height = u32::try_from(frame.height)
            .map_err(|_| RenderError::Message("render proxy frame height overflow".to_string()))?;
        let (pixel_format, payload, row_pitch) = match frame.data {
            RenderFrameData::Rgb24(data) => (
                RenderProxyPixelFormat::Rgb24,
                bytes::Bytes::from(data),
                0_usize,
            ),
            RenderFrameData::Bgra32(data) => (
                RenderProxyPixelFormat::Bgra32,
                bytes::Bytes::from(data),
                0_usize,
            ),
            RenderFrameData::Nv12 { data, pitch } => (
                RenderProxyPixelFormat::Nv12,
                bytes::Bytes::from(data),
                pitch,
            ),
            RenderFrameData::Nv12Bytes { data, pitch } => {
                (RenderProxyPixelFormat::Nv12, data, pitch)
            }
            #[cfg(windows)]
            RenderFrameData::D3D11SharedBgra { .. }
            | RenderFrameData::D3D11SharedNv12 { .. }
            | RenderFrameData::D3D11SharedP010 { .. } => {
                return Err(RenderError::Message(
                    "macOS render proxy does not accept D3D11 shared textures".to_string(),
                ))
            }
        };
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy frame payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format,
            width,
            height,
            sequence: self.sequence,
            timestamp_us: 0,
            payload_len,
            row_pitch: u32::try_from(row_pitch)
                .map_err(|_| RenderError::Message("render proxy row pitch overflow".to_string()))?,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, payload.as_ref())?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = frame.width;
        self.snapshot.last_height = frame.height;
        self.snapshot.last_pixel_format = Some(frame.pixel_format);
        Ok(())
    }

    fn upload_h264_access_unit(
        &mut self,
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    ) -> Result<(), RenderError> {
        let width = u32::try_from(width)
            .map_err(|_| RenderError::Message("render proxy H.264 width overflow".to_string()))?;
        let height = u32::try_from(height)
            .map_err(|_| RenderError::Message("render proxy H.264 height overflow".to_string()))?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy H.264 payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::H264,
            width,
            height,
            sequence: self.sequence,
            timestamp_us,
            payload_len,
            row_pitch: 0,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, &payload)?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = width as usize;
        self.snapshot.last_height = height as usize;
        Ok(())
    }

    fn upload_hevc_access_unit(
        &mut self,
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    ) -> Result<(), RenderError> {
        let width = u32::try_from(width)
            .map_err(|_| RenderError::Message("render proxy HEVC width overflow".to_string()))?;
        let height = u32::try_from(height)
            .map_err(|_| RenderError::Message("render proxy HEVC height overflow".to_string()))?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy HEVC payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::Hevc,
            width,
            height,
            sequence: self.sequence,
            timestamp_us,
            payload_len,
            row_pitch: 0,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, &payload)?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = width as usize;
        self.snapshot.last_height = height as usize;
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        self.snapshot.clone()
    }
}

#[cfg(target_os = "macos")]
fn finite_positive_duration_ms(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[cfg(target_os = "macos")]
fn record_render_proxy_present_config(
    snapshot: &mut RendererSnapshot,
    ack: &mrd_ipc::render_proxy::RenderProxyAck,
) {
    if let Some(max_drawable_count) = ack.max_drawable_count {
        snapshot.swap_chain_max_frame_latency = Some(max_drawable_count);
    }
    if let Some(display_sync_enabled) = ack.display_sync_enabled {
        snapshot.swap_chain_allow_tearing = Some(!display_sync_enabled);
        snapshot.swap_chain_present_mode =
            Some(render_proxy_present_mode(display_sync_enabled).to_string());
    }
}

#[cfg(target_os = "macos")]
fn render_proxy_present_mode(display_sync_enabled: bool) -> &'static str {
    if display_sync_enabled {
        "render_proxy_metal_display_sync"
    } else {
        "render_proxy_metal_immediate"
    }
}

fn media_pipeline_stage_metrics(state: &MediaPipelineState) -> Vec<MediaStageMetrics> {
    let mut metrics = state.stage_summaries.clone();
    for (stage, samples) in &state.stage_samples {
        metrics.insert(
            stage.clone(),
            MediaStageMetrics {
                stage: stage.clone(),
                p50_ms: percentile(samples, 0.50),
                p95_ms: percentile(samples, 0.95),
            },
        );
    }

    let mut metrics = metrics.into_values().collect::<Vec<_>>();
    metrics.sort_by(|left, right| left.stage.cmp(&right.stage));
    metrics
}

fn percentile(samples: &VecDeque<f64>, quantile: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.iter().copied().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let last = sorted.len().saturating_sub(1);
    let index = ((last as f64) * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted.get(index).copied()
}

/// Runtime media tasks keyed by session.
#[derive(Default)]
pub struct MediaTaskRegistry {
    tasks: HashMap<SessionId, Vec<AbortHandle>>,
}

impl MediaTaskRegistry {
    pub fn register(&mut self, session_id: SessionId, abort_handle: AbortHandle) {
        self.tasks.entry(session_id).or_default().push(abort_handle);
    }

    pub fn abort_session(&mut self, session_id: &SessionId) -> usize {
        let handles = self.tasks.remove(session_id).unwrap_or_default();
        let count = handles.len();
        for handle in handles {
            handle.abort();
        }
        count
    }

    pub fn active_count(&self, session_id: &SessionId) -> usize {
        self.tasks.get(session_id).map_or(0, Vec::len)
    }
}

#[derive(Debug, Clone, Default)]
struct SessionProbeStats {
    frames_received: u64,
    frames_decoded: u64,
    frames_dropped: u64,
    sequence_gap_drops: u64,
    decode_error_drops: u64,
    transient_drops: u64,
    bytes_received: u64,
    first_seen_ms: Option<u64>,
    last_seen_ms: Option<u64>,
    media_probe_valid: bool,
    media_probe_format: Option<String>,
    media_probe_width: Option<u32>,
    media_probe_height: Option<u32>,
    media_probe_target_fps: Option<u32>,
    media_probe_target_bitrate_mbps: Option<u32>,
    media_probe_payload_bytes: Option<u32>,
    last_media_sequence: Option<u64>,
    last_media_timestamp_us: Option<u64>,
    last_media_payload_hash: Option<String>,
    latest_frame: Option<DecodedPreviewFrame>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct DecodedPreviewFrame {
    width: u32,
    height: u32,
    pixel_format: String,
}

#[derive(Debug, Clone)]
pub struct MediaProbeFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub payload_bytes: u32,
    pub format: String,
    pub payload_hash: String,
}

#[derive(Debug, Clone)]
pub struct DecodedVideoFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub encoded_bytes: u32,
    pub format: String,
    pub pixel_format: String,
    pub payload_hash: String,
    pub preview_width: Option<u32>,
    pub preview_height: Option<u32>,
    pub rgb24: Option<Vec<u8>>,
}

impl ProbeRegistry {
    pub fn record_probe_frame(&mut self, session_id: &SessionId, bytes_received: u64, now_ms: u64) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = None;
    }

    pub fn record_media_probe_frame(
        &mut self,
        session_id: &SessionId,
        frame: MediaProbeFrameStats,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        if let Some(last_sequence) = stats.last_media_sequence {
            if frame.sequence > last_sequence.saturating_add(1) {
                let missing = frame.sequence.saturating_sub(last_sequence + 1);
                stats.frames_dropped = stats.frames_dropped.saturating_add(missing);
                stats.sequence_gap_drops = stats.sequence_gap_drops.saturating_add(missing);
            }
        }

        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(frame.bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.media_probe_valid = true;
        stats.media_probe_format = Some(frame.format);
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.payload_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        stats.last_error = None;
    }

    pub fn record_decoded_video_frame(
        &mut self,
        session_id: &SessionId,
        frame: DecodedVideoFrameStats,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        if let Some(last_sequence) = stats.last_media_sequence {
            if frame.sequence > last_sequence.saturating_add(1) {
                let missing = frame.sequence.saturating_sub(last_sequence + 1);
                stats.frames_dropped = stats.frames_dropped.saturating_add(missing);
                stats.sequence_gap_drops = stats.sequence_gap_drops.saturating_add(missing);
            }
        }

        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(frame.bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.media_probe_valid = true;
        stats.media_probe_format = Some(frame.format);
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.encoded_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        stats.latest_frame = Some(DecodedPreviewFrame {
            width: frame.preview_width.unwrap_or(frame.width),
            height: frame.preview_height.unwrap_or(frame.height),
            pixel_format: frame.pixel_format,
        });
        stats.last_error = None;
    }

    pub fn record_probe_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
        error: impl Into<String>,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.decode_error_drops = stats.decode_error_drops.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = Some(error.into());
    }

    pub fn record_transient_frame_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.transient_drops = stats.transient_drops.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
    }

    pub fn snapshot(&self, session_id: &SessionId) -> mrd_ipc::ProbeSnapshot {
        let Some(stats) = self.probes.get(session_id) else {
            return mrd_ipc::ProbeSnapshot {
                session_id: session_id.clone(),
                frames_received: 0,
                frames_decoded: 0,
                frames_dropped: 0,
                sequence_gap_drops: 0,
                decode_error_drops: 0,
                transient_drops: 0,
                current_fps: None,
                bitrate_mbps: None,
                media_probe_valid: false,
                media_probe_format: None,
                media_probe_width: None,
                media_probe_height: None,
                media_probe_target_fps: None,
                media_probe_target_bitrate_mbps: None,
                media_probe_payload_bytes: None,
                last_media_sequence: None,
                last_media_timestamp_us: None,
                last_media_payload_hash: None,
                latest_frame_data_url: None,
                latest_frame_width: None,
                latest_frame_height: None,
                latest_frame_pixel_format: None,
                last_error: None,
            };
        };

        let elapsed_ms = match (stats.first_seen_ms, stats.last_seen_ms) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        };
        let current_fps = if elapsed_ms > 0 {
            Some((stats.frames_decoded as f32 * 1000.0) / elapsed_ms as f32)
        } else {
            Some(0.0)
        };
        let bitrate_mbps = if elapsed_ms > 0 {
            Some((stats.bytes_received as f32 * 8.0) / elapsed_ms as f32 / 1000.0)
        } else {
            Some(0.0)
        };

        mrd_ipc::ProbeSnapshot {
            session_id: session_id.clone(),
            frames_received: stats.frames_received,
            frames_decoded: stats.frames_decoded,
            frames_dropped: stats.frames_dropped,
            sequence_gap_drops: stats.sequence_gap_drops,
            decode_error_drops: stats.decode_error_drops,
            transient_drops: stats.transient_drops,
            current_fps,
            bitrate_mbps,
            media_probe_valid: stats.media_probe_valid,
            media_probe_format: stats.media_probe_format.clone(),
            media_probe_width: stats.media_probe_width,
            media_probe_height: stats.media_probe_height,
            media_probe_target_fps: stats.media_probe_target_fps,
            media_probe_target_bitrate_mbps: stats.media_probe_target_bitrate_mbps,
            media_probe_payload_bytes: stats.media_probe_payload_bytes,
            last_media_sequence: stats.last_media_sequence,
            last_media_timestamp_us: stats.last_media_timestamp_us,
            last_media_payload_hash: stats.last_media_payload_hash.clone(),
            latest_frame_data_url: None,
            latest_frame_width: stats.latest_frame.as_ref().map(|frame| frame.width),
            latest_frame_height: stats.latest_frame.as_ref().map(|frame| frame.height),
            latest_frame_pixel_format: stats
                .latest_frame
                .as_ref()
                .map(|frame| frame.pixel_format.clone()),
            last_error: stats.last_error.clone(),
        }
    }
}

/// Shell state - tracks UI presence and service lifecycle
#[derive(Debug, Default)]
pub struct ShellState {
    /// UI process PID if attached
    pub ui_pid: Option<u32>,
    /// UI executable path for relaunch
    pub ui_executable_path: Option<String>,
    /// Tray availability (platform-dependent)
    pub tray_available: bool,
    /// Autostart enabled state (None if not supported)
    pub autostart_enabled: Option<bool>,
    /// Active session count (for tray display)
    pub active_session_count: usize,
    /// Last error message
    pub last_error: Option<String>,
}

/// Tray port - abstracts platform-specific tray implementation
pub type TrayPortRef = Arc<std::sync::Mutex<dyn crate::shell::TrayPort + Send + Sync>>;

/// Device registry
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    local_device: Option<(DeviceId, String)>, // (id, name)
}

/// In-memory paired device identity registry.
#[derive(Debug, Default)]
pub struct DeviceIdentityRegistry {
    paired_devices: HashMap<DeviceId, PairedDeviceIdentity>,
}

impl DeviceIdentityRegistry {
    pub fn upsert(
        &mut self,
        device_id: DeviceId,
        certificate_fingerprint: Option<String>,
        trust_status: impl Into<String>,
    ) {
        let display_name = device_id.0.clone();
        let existing = self.paired_devices.remove(&device_id);
        let certificate_fingerprint = certificate_fingerprint.or_else(|| {
            existing
                .as_ref()
                .and_then(|identity| identity.certificate_fingerprint.clone())
        });
        self.paired_devices.insert(
            device_id.clone(),
            PairedDeviceIdentity {
                display_name: existing
                    .as_ref()
                    .map(|identity| identity.display_name.clone())
                    .unwrap_or(display_name),
                device_id,
                certificate_fingerprint,
                trust_status: trust_status.into(),
                last_seen_ms: Some(now_unix_ms()),
            },
        );
    }

    pub fn revoke(&mut self, device_id: &DeviceId) {
        if let Some(identity) = self.paired_devices.get_mut(device_id) {
            identity.trust_status = "revoked".to_string();
            identity.last_seen_ms = Some(now_unix_ms());
        } else {
            self.upsert(device_id.clone(), None, "revoked");
        }
    }

    pub fn list(&self) -> Vec<PairedDeviceIdentity> {
        let mut identities = self.paired_devices.values().cloned().collect::<Vec<_>>();
        identities.sort_by(|a, b| a.device_id.0.cmp(&b.device_id.0));
        identities
    }
}

/// In-memory service audit event registry.
#[derive(Debug)]
pub struct AuditLogRegistry {
    next_id: u64,
    events: VecDeque<AuditEvent>,
    max_events: usize,
}

impl Default for AuditLogRegistry {
    fn default() -> Self {
        Self {
            next_id: 1,
            events: VecDeque::new(),
            max_events: AUDIT_EVENT_LIMIT,
        }
    }
}

impl AuditLogRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) -> AuditEvent {
        let event = AuditEvent {
            id: self.next_id,
            timestamp_ms: now_unix_ms(),
            action: action.into(),
            outcome: outcome.into(),
            session_id,
            actor_device_id,
            peer_device_id,
            transport_kind,
            reason,
            details,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.events.push_back(event.clone());
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
        event
    }

    pub fn query(&self, query: &AuditLogQuery) -> Vec<AuditEvent> {
        let mut events = self
            .events
            .iter()
            .filter(|event| {
                query
                    .session_id
                    .as_ref()
                    .is_none_or(|session_id| event.session_id.as_ref() == Some(session_id))
            })
            .filter(|event| {
                query
                    .action
                    .as_ref()
                    .is_none_or(|action| event.action == *action)
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(limit) = query.limit {
            let limit = limit as usize;
            if events.len() > limit {
                events = events.split_off(events.len() - limit);
            }
        }
        events
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

impl DeviceRegistry {
    pub fn register(&mut self, device_id: DeviceId, device_name: String) {
        self.local_device = Some((device_id, device_name));
    }

    pub fn register_if_unregistered(
        &mut self,
        device_id: DeviceId,
        device_name: String,
    ) -> Option<(DeviceId, String)> {
        if self.local_device.is_none() {
            self.register(device_id, device_name);
        }
        self.local_device.clone()
    }

    pub fn get_local_device(&self) -> Option<&(DeviceId, String)> {
        self.local_device.as_ref()
    }

    pub fn is_registered(&self) -> bool {
        self.local_device.is_some()
    }
}

pub fn default_lan_device_identity() -> (DeviceId, String) {
    lan_device_identity_from(
        std::env::var("MRD_LAN_DEVICE_ID").ok(),
        std::env::var("MRD_LAN_DEVICE_NAME").ok(),
        default_hostname(),
    )
}

fn default_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
}

fn lan_device_identity_from(
    configured_id: Option<String>,
    configured_name: Option<String>,
    hostname: Option<String>,
) -> (DeviceId, String) {
    let device_name = configured_name
        .and_then(non_empty_trimmed)
        .or_else(|| hostname.clone().and_then(non_empty_trimmed))
        .unwrap_or_else(|| "Rdesk LAN Device".to_string());
    let device_id = configured_id
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| build_lan_device_id(hostname.as_deref().unwrap_or(&device_name)));
    (DeviceId(device_id), device_name)
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_lan_device_id(seed: &str) -> String {
    let mut sanitized: String = seed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if sanitized.len() > 16 {
        sanitized = sanitized[sanitized.len() - 16..].to_string();
    }
    if sanitized.is_empty() {
        sanitized = "local".to_string();
    }
    format!("lan-{sanitized}")
}

/// Application state for mrd-service
///
/// This is the shared state that will be injected into IPC handlers.
/// After migration, it will own:
/// - RealtimeRuntime / signaling client
/// - WebrtcHost / WebrtcSessionCoordinator
/// - QuicHost / QuicSessionCoordinator
/// - Media senders/receivers
/// - Probe/telemetry state
/// - Shell/UI lifecycle state
/// - Tray port (Phase 4)
pub struct AppState {
    /// Session registry - single source of truth for all sessions
    pub sessions: Arc<Mutex<SessionRegistry>>,
    /// Device registry
    pub devices: Arc<Mutex<DeviceRegistry>>,
    /// Service-owned security and operations audit events.
    pub audit_log: Arc<Mutex<AuditLogRegistry>>,
    /// Service-owned device pairing and identity state.
    pub device_identities: Arc<Mutex<DeviceIdentityRegistry>>,
    /// Shell state - UI presence and service lifecycle
    pub shell: Arc<Mutex<ShellState>>,
    /// Tray port (Phase 4)
    pub tray: TrayPortRef,
    /// Peer-to-peer LAN discovery state.
    pub lan_discovery: Arc<crate::lan_discovery::LanDiscoveryState>,
    /// LAN probe telemetry keyed by session.
    pub probes: Arc<Mutex<ProbeRegistry>>,
    /// Negotiated media profile keyed by session.
    pub media_profiles: Arc<Mutex<MediaProfileRegistry>>,
    /// Selected capture source keyed by session.
    pub capture_sources: Arc<Mutex<CaptureSourceRegistry>>,
    /// Temporary display mode state keyed by session.
    pub display_modes: Arc<Mutex<DisplayModeRegistry>>,
    /// Peer media capabilities keyed by session.
    pub peer_media_capabilities: Arc<Mutex<SessionPeerMediaCapabilityRegistry>>,
    /// Cached local capability facts refreshed outside request handling.
    pub capability_snapshot: Arc<Mutex<CapabilitySnapshotRegistry>>,
    /// Service-owned keyboard and mouse injection state.
    pub control_input: Arc<Mutex<ControlInputRegistry>>,
    /// Receiver pipeline state keyed by session.
    pub media_pipelines: Arc<Mutex<MediaPipelineRegistry>>,
    /// Native renderer instances keyed by receiver session/surface.
    #[cfg(any(windows, target_os = "macos"))]
    pub media_surface_renderers: Arc<Mutex<MediaSurfaceRendererRegistry>>,
    /// Drop-oldest receiver render queues keyed by session.
    #[cfg(any(windows, target_os = "macos"))]
    pub media_render_queues: Arc<Mutex<MediaRenderQueueRegistry>>,
    /// Abort handles for active media tasks keyed by session.
    pub media_tasks: Arc<Mutex<MediaTaskRegistry>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_tray(Arc::new(std::sync::Mutex::new(
            crate::shell::NoOpTray::new(),
        )))
    }

    pub fn with_tray(tray: TrayPortRef) -> Self {
        Self::with_tray_and_lan_discovery_config(
            tray,
            crate::lan_discovery::LanDiscoveryConfig::default(),
        )
    }

    pub fn with_tray_and_lan_discovery_config(
        tray: TrayPortRef,
        lan_discovery_config: crate::lan_discovery::LanDiscoveryConfig,
    ) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionRegistry::default())),
            devices: Arc::new(Mutex::new(DeviceRegistry::default())),
            audit_log: Arc::new(Mutex::new(AuditLogRegistry::default())),
            device_identities: Arc::new(Mutex::new(DeviceIdentityRegistry::default())),
            shell: Arc::new(Mutex::new(ShellState::default())),
            tray,
            lan_discovery: Arc::new(crate::lan_discovery::LanDiscoveryState::new(
                lan_discovery_config,
            )),
            probes: Arc::new(Mutex::new(ProbeRegistry::default())),
            media_profiles: Arc::new(Mutex::new(MediaProfileRegistry::default())),
            capture_sources: Arc::new(Mutex::new(CaptureSourceRegistry::default())),
            display_modes: Arc::new(Mutex::new(DisplayModeRegistry::default())),
            peer_media_capabilities: Arc::new(Mutex::new(
                SessionPeerMediaCapabilityRegistry::default(),
            )),
            capability_snapshot: Arc::new(Mutex::new(CapabilitySnapshotRegistry::default())),
            control_input: Arc::new(Mutex::new(ControlInputRegistry::default())),
            media_pipelines: Arc::new(Mutex::new(MediaPipelineRegistry::default())),
            #[cfg(any(windows, target_os = "macos"))]
            media_surface_renderers: Arc::new(Mutex::new(MediaSurfaceRendererRegistry::default())),
            #[cfg(any(windows, target_os = "macos"))]
            media_render_queues: Arc::new(Mutex::new(MediaRenderQueueRegistry::default())),
            media_tasks: Arc::new(Mutex::new(MediaTaskRegistry::default())),
        }
    }

    /// Get a clone of the sessions Arc for injection into handlers
    pub fn sessions(&self) -> Arc<Mutex<SessionRegistry>> {
        self.sessions.clone()
    }

    /// Get a clone of the devices Arc for injection into handlers
    pub fn devices(&self) -> Arc<Mutex<DeviceRegistry>> {
        self.devices.clone()
    }

    /// Get a clone of the service audit log registry.
    pub fn audit_log(&self) -> Arc<Mutex<AuditLogRegistry>> {
        self.audit_log.clone()
    }

    /// Get a clone of the device identity registry.
    pub fn device_identities(&self) -> Arc<Mutex<DeviceIdentityRegistry>> {
        self.device_identities.clone()
    }

    /// Get a clone of the shell Arc for injection into handlers
    pub fn shell(&self) -> Arc<Mutex<ShellState>> {
        self.shell.clone()
    }

    /// Get a clone of the tray Arc for injection into handlers
    pub fn tray(&self) -> TrayPortRef {
        self.tray.clone()
    }

    /// Get a clone of the LAN discovery state.
    pub fn lan_discovery(&self) -> Arc<crate::lan_discovery::LanDiscoveryState> {
        self.lan_discovery.clone()
    }

    /// Get a clone of the probe telemetry registry.
    pub fn probes(&self) -> Arc<Mutex<ProbeRegistry>> {
        self.probes.clone()
    }

    /// Get a clone of the media profile registry.
    pub fn media_profiles(&self) -> Arc<Mutex<MediaProfileRegistry>> {
        self.media_profiles.clone()
    }

    /// Get a clone of the capture source registry.
    pub fn capture_sources(&self) -> Arc<Mutex<CaptureSourceRegistry>> {
        self.capture_sources.clone()
    }

    /// Get a clone of the display mode registry.
    pub fn display_modes(&self) -> Arc<Mutex<DisplayModeRegistry>> {
        self.display_modes.clone()
    }

    /// Get a clone of the peer media capability registry.
    pub fn peer_media_capabilities(&self) -> Arc<Mutex<SessionPeerMediaCapabilityRegistry>> {
        self.peer_media_capabilities.clone()
    }

    /// Get a clone of the local capability snapshot registry.
    pub fn capability_snapshot(&self) -> Arc<Mutex<CapabilitySnapshotRegistry>> {
        self.capability_snapshot.clone()
    }

    /// Get a clone of the service-owned control input registry.
    pub fn control_input(&self) -> Arc<Mutex<ControlInputRegistry>> {
        self.control_input.clone()
    }

    /// Return the currently cached local capability snapshot without running runtime probes.
    pub async fn cached_capability_snapshot(&self) -> CapabilitySnapshot {
        let mut snapshot = self.capability_snapshot.lock().await.snapshot();
        let input_injector_available = self.control_input.lock().await.is_available();
        crate::capabilities::apply_control_input_capability_status(
            &mut snapshot,
            input_injector_available,
        );
        snapshot
    }

    /// Refresh the local capability snapshot on a blocking worker without delaying IPC handlers.
    pub fn refresh_capability_snapshot_in_background(self: &Arc<Self>) {
        let app_state = Arc::clone(self);
        tokio::spawn(async move {
            let should_refresh = {
                let mut registry = app_state.capability_snapshot.lock().await;
                registry.begin_refresh()
            };
            if !should_refresh {
                return;
            }

            let snapshot =
                tokio::task::spawn_blocking(crate::capabilities::local_capability_snapshot)
                    .await
                    .map_err(|error| {
                        tracing::warn!("capability snapshot refresh task failed: {}", error);
                        error
                    })
                    .ok();
            app_state
                .capability_snapshot
                .lock()
                .await
                .finish_refresh(snapshot);
        });
    }

    #[cfg(test)]
    pub async fn replace_capability_snapshot_for_test(&self, snapshot: CapabilitySnapshot) {
        self.capability_snapshot.lock().await.replace(snapshot);
    }

    #[cfg(test)]
    pub async fn replace_control_input_for_test<I>(&self, injector: I)
    where
        I: mrd_input::InputInjector + 'static,
    {
        *self.control_input.lock().await = ControlInputRegistry::with_injector(injector);
    }

    /// Get a clone of the receiver media pipeline registry.
    pub fn media_pipelines(&self) -> Arc<Mutex<MediaPipelineRegistry>> {
        self.media_pipelines.clone()
    }

    /// Get a clone of the native receiver renderer registry.
    #[cfg(any(windows, target_os = "macos"))]
    pub fn media_surface_renderers(&self) -> Arc<Mutex<MediaSurfaceRendererRegistry>> {
        self.media_surface_renderers.clone()
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn media_render_queues(&self) -> Arc<Mutex<MediaRenderQueueRegistry>> {
        self.media_render_queues.clone()
    }

    /// Get a clone of the media task registry.
    pub fn media_tasks(&self) -> Arc<Mutex<MediaTaskRegistry>> {
        self.media_tasks.clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_application::ports::SessionLifecycleState;

    #[test]
    fn session_registry_tracks_sessions() {
        let mut registry = SessionRegistry::default();

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Created,
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        registry.insert(session_id.clone(), snapshot);

        let retrieved = registry.get(&session_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().transport, "quic");
    }

    #[test]
    fn device_registry_tracks_local_device() {
        let mut registry = DeviceRegistry::default();

        let device_id = DeviceId("test-device".to_string());
        registry.register(device_id.clone(), "Test Device".to_string());

        assert!(registry.is_registered());

        let retrieved = registry.get_local_device();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().0, device_id);
    }

    #[test]
    fn device_registry_keeps_explicit_registration() {
        let mut registry = DeviceRegistry::default();
        registry.register(
            DeviceId("explicit-device".to_string()),
            "Explicit Device".to_string(),
        );

        let registered = registry
            .register_if_unregistered(
                DeviceId("fallback-device".to_string()),
                "Fallback Device".to_string(),
            )
            .expect("registered device");

        assert_eq!(registered.0, DeviceId("explicit-device".to_string()));
        assert_eq!(registered.1, "Explicit Device");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_render_proxy_vectored_write_preserves_frame_bytes() {
        let (mut writer, mut reader) = StdUnixStream::pair().expect("create stream pair");
        let header = b"MRDR-header";
        let payload = b"nv12-payload";

        write_all_vectored_pair(&mut writer, header, payload).expect("write frame bytes");

        let mut frame = vec![0_u8; header.len() + payload.len()];
        reader.read_exact(&mut frame).expect("read frame bytes");

        let mut expected = Vec::from(header.as_slice());
        expected.extend_from_slice(payload);
        assert_eq!(frame, expected);
    }

    #[test]
    fn default_lan_identity_uses_configured_id_and_name() {
        let (device_id, device_name) = lan_device_identity_from(
            Some(" lan-MOCK7EBPZ3RC ".to_string()),
            Some(" Target PC ".to_string()),
            Some("ignored-host".to_string()),
        );

        assert_eq!(device_id, DeviceId("lan-MOCK7EBPZ3RC".to_string()));
        assert_eq!(device_name, "Target PC");
    }

    #[test]
    fn default_lan_identity_falls_back_to_hostname() {
        let (device_id, device_name) =
            lan_device_identity_from(None, None, Some("DESKTOP-ABC/123".to_string()));

        assert_eq!(device_id, DeviceId("lan-DESKTOPABC123".to_string()));
        assert_eq!(device_name, "DESKTOP-ABC/123");
    }

    #[test]
    fn probe_registry_tracks_received_probe_frames() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("probe-session".to_string());

        registry.record_probe_frame(&session_id, 1200, 1_000);
        registry.record_probe_frame(&session_id, 1200, 1_250);

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 2);
        assert_eq!(snapshot.frames_decoded, 2);
        assert!(snapshot.current_fps.unwrap_or_default() > 0.0);
        assert!(snapshot.bitrate_mbps.unwrap_or_default() > 0.0);
    }

    #[test]
    fn probe_registry_exposes_valid_media_probe_metadata() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("media-probe-session".to_string());

        registry.record_media_probe_frame(
            &session_id,
            MediaProbeFrameStats {
                bytes_received: 2400,
                sequence: 7,
                timestamp_us: 123_456,
                width: 32,
                height: 18,
                target_fps: 144,
                target_bitrate_mbps: 64,
                payload_bytes: 2400,
                format: "rgba8_test_pattern".to_string(),
                payload_hash: "fnv1a64:abc123".to_string(),
            },
            2_000,
        );

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert!(snapshot.media_probe_valid);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("rgba8_test_pattern")
        );
        assert_eq!(snapshot.media_probe_width, Some(32));
        assert_eq!(snapshot.media_probe_height, Some(18));
        assert_eq!(snapshot.media_probe_target_fps, Some(144));
        assert_eq!(snapshot.media_probe_target_bitrate_mbps, Some(64));
        assert_eq!(snapshot.media_probe_payload_bytes, Some(2400));
        assert_eq!(snapshot.last_media_sequence, Some(7));
        assert_eq!(snapshot.last_media_timestamp_us, Some(123_456));
        assert_eq!(
            snapshot.last_media_payload_hash.as_deref(),
            Some("fnv1a64:abc123")
        );
    }

    #[test]
    fn probe_registry_exposes_latest_decoded_video_metadata_without_image() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("decoded-video-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 4096,
                sequence: 11,
                timestamp_us: 987_654,
                width: 2,
                height: 2,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 1024,
                format: "h264_desktop_frame".to_string(),
                pixel_format: "rgb24".to_string(),
                payload_hash: "fnv1a64:preview".to_string(),
                preview_width: Some(2),
                preview_height: Some(2),
                rgb24: Some(vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
            },
            3_000,
        );

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("h264_desktop_frame")
        );
        assert_eq!(snapshot.latest_frame_width, Some(2));
        assert_eq!(snapshot.latest_frame_height, Some(2));
        assert_eq!(snapshot.latest_frame_pixel_format.as_deref(), Some("rgb24"));
        assert!(snapshot.latest_frame_data_url.is_none());
    }

    #[test]
    fn probe_registry_counts_decoded_video_without_preview_copy() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("decoded-video-metadata-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 12,
                timestamp_us: 1_111_111,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 20,
                encoded_bytes: 2048,
                format: "hevc_desktop_frame".to_string(),
                pixel_format: "cpu_nv12".to_string(),
                payload_hash: "fnv1a64:encoded".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            4_000,
        );

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(snapshot.media_probe_width, Some(1920));
        assert_eq!(snapshot.media_probe_height, Some(1080));
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("hevc_desktop_frame")
        );
        assert_eq!(
            snapshot.last_media_payload_hash.as_deref(),
            Some("fnv1a64:encoded")
        );
        assert!(snapshot.latest_frame_data_url.is_none());
    }

    #[test]
    fn probe_registry_counts_transient_drop_without_latching_error() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("transient-drop-session".to_string());

        registry.record_transient_frame_drop(&session_id, 512, 1_000);

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 0);
        assert_eq!(snapshot.frames_dropped, 1);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn probe_registry_breaks_down_drop_causes() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("drop-breakdown-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 10,
                timestamp_us: 100_000,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 2048,
                format: "hevc_desktop_frame".to_string(),
                pixel_format: "d3d11_shared_nv12".to_string(),
                payload_hash: "fnv1a64:first".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            1_000,
        );
        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 13,
                timestamp_us: 120_000,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 2048,
                format: "hevc_desktop_frame".to_string(),
                pixel_format: "d3d11_shared_nv12".to_string(),
                payload_hash: "fnv1a64:gap".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            1_020,
        );
        registry.record_probe_drop(&session_id, 512, 1_030, "decode failed");
        registry.record_transient_frame_drop(&session_id, 256, 1_040);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_dropped, 4);
        assert_eq!(snapshot.sequence_gap_drops, 2);
        assert_eq!(snapshot.decode_error_drops, 1);
        assert_eq!(snapshot.transient_drops, 1);
    }

    #[test]
    fn media_pipeline_registry_exposes_stage_metrics() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("metrics-session".to_string());

        registry.record_stage_duration_ms(session_id.clone(), "sender.capture", 1.0);
        registry.record_stage_duration_ms(session_id.clone(), "sender.capture", 3.0);
        registry.set_stage_metrics(
            session_id.clone(),
            [MediaStageMetrics {
                stage: "sender.encode".to_string(),
                p50_ms: Some(2.5),
                p95_ms: Some(4.5),
            }],
        );

        let snapshot = registry.snapshot(&session_id);

        assert!(snapshot.stage_metrics.iter().any(|metric| {
            metric.stage == "sender.capture"
                && metric.p50_ms == Some(3.0)
                && metric.p95_ms == Some(3.0)
        }));
        assert!(snapshot.stage_metrics.iter().any(|metric| {
            metric.stage == "sender.encode"
                && metric.p50_ms == Some(2.5)
                && metric.p95_ms == Some(4.5)
        }));
    }

    #[test]
    fn media_pipeline_registry_separates_render_drop_counters() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-drops-session".to_string());

        registry.increment_render_queue_replacements(session_id.clone(), 3);
        registry.increment_render_stale_frame_drops(session_id.clone(), 5);
        registry.increment_render_lock_drops(session_id.clone(), 2);
        registry.increment_render_present_skips(session_id.clone(), 4);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.dropped_frames, 14);
        assert_eq!(snapshot.render_queue_replacements, 3);
        assert_eq!(snapshot.render_stale_frame_drops, 5);
        assert_eq!(snapshot.render_lock_drops, 2);
        assert_eq!(snapshot.render_present_skips, 4);
    }

    #[test]
    fn media_pipeline_registry_can_record_queue_replacement_without_double_counting_drop() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-replacement-only-session".to_string());

        registry.record_render_queue_replacements(session_id.clone(), 3);
        registry.increment_render_stale_frame_drops(session_id.clone(), 3);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.dropped_frames, 3);
        assert_eq!(snapshot.render_queue_replacements, 3);
        assert_eq!(snapshot.render_stale_frame_drops, 3);
    }

    #[test]
    fn media_pipeline_registry_exposes_render_queue_policy() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-policy-session".to_string());

        registry.set_render_queue_policy(session_id.clone(), Some("latest"));

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.render_queue_policy.as_deref(), Some("latest"));
    }

    #[cfg(windows)]
    #[test]
    fn media_pipeline_registry_exposes_renderer_swapchain_pacing_metadata() {
        use mrd_render::RendererSnapshot;

        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-swapchain-session".to_string());

        registry.record_renderer_snapshot(
            session_id.clone(),
            &RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: 4,
                presented_frame_count: 4,
                present_skipped_count: 0,
                render_queue_replacements: None,
                last_present_status: Some("presented".to_string()),
                low_latency_frame_latency_target: Some(1),
                swap_chain_max_frame_latency: Some(1),
                swap_chain_allow_tearing: Some(true),
                swap_chain_waitable_object: Some(true),
                swap_chain_present_mode: Some("waitable".to_string()),
                display_refresh_hz: Some(144),
                render_thread_priority: Some("highest".to_string()),
                waitable_wait_count: Some(2),
                waitable_wait_total_ms: Some(1.25),
                waitable_timeout_count: Some(1),
                last_waitable_wait_ms: Some(0.75),
                last_render_prepare_wait_ms: Some(0.05),
                last_render_shared_resource_ms: Some(0.02),
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: Some(0.7),
                last_width: 2560,
                last_height: 1440,
                last_pixel_format: None,
            },
        );
        registry.increment_render_waitable_timeouts(session_id.clone(), 1);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.swap_chain_max_frame_latency, Some(1));
        assert_eq!(snapshot.swap_chain_allow_tearing, Some(true));
        assert_eq!(snapshot.swap_chain_waitable_object, Some(true));
        assert_eq!(
            snapshot.swap_chain_present_mode.as_deref(),
            Some("waitable")
        );
        assert_eq!(snapshot.display_refresh_hz, Some(144));
        assert_eq!(snapshot.render_thread_priority.as_deref(), Some("highest"));
        assert_eq!(snapshot.render_waitable_timeouts, 1);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_surface_renderer_registry_returns_shared_session_renderers() {
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingRenderer {
            uploads: Arc<AtomicUsize>,
        }

        impl RendererInstance for CountingRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                self.uploads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: self.uploads.load(Ordering::SeqCst) as u64,
                    presented_frame_count: self.uploads.load(Ordering::SeqCst) as u64,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: Some("presented".to_string()),
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 1,
                    last_height: 1,
                    last_pixel_format: None,
                }
            }
        }

        let mut registry = MediaSurfaceRendererRegistry::default();
        let session_a = SessionId("surface-session-a".to_string());
        let session_b = SessionId("surface-session-b".to_string());
        let uploads_a = Arc::new(AtomicUsize::new(0));
        let uploads_b = Arc::new(AtomicUsize::new(0));

        registry.insert_renderer_for_test(
            &session_a,
            "surface-a",
            Box::new(CountingRenderer {
                uploads: uploads_a.clone(),
            }),
        );
        registry.insert_renderer_for_test(
            &session_b,
            "surface-b",
            Box::new(CountingRenderer {
                uploads: uploads_b.clone(),
            }),
        );

        let session_a_renderers = registry.renderers_for_session(&session_a);
        assert_eq!(session_a_renderers.len(), 1);
        drop(registry);

        let frame = RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]);
        session_a_renderers[0]
            .lock()
            .expect("renderer lock")
            .upload_frame(frame)
            .expect("upload frame");

        assert_eq!(uploads_a.load(Ordering::SeqCst), 1);
        assert_eq!(uploads_b.load(Ordering::SeqCst), 0);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_surface_renderer_registry_reuses_existing_surface_on_duplicate_attach() {
        use mrd_ipc::AttachedRenderSurface;
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

        struct NoopRenderer;

        impl RendererInstance for NoopRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: 0,
                    presented_frame_count: 0,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: None,
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 1,
                    last_height: 1,
                    last_pixel_format: None,
                }
            }
        }

        let mut registry = MediaSurfaceRendererRegistry::default();
        let session_id = SessionId("surface-session".to_string());
        registry.insert_renderer_for_test(&session_id, "surface-1", Box::new(NoopRenderer));

        let result = registry.attach_surface(
            &session_id,
            &AttachedRenderSurface {
                surface_id: "surface-1".to_string(),
                backend: platform_surface_backend_for_test().to_string(),
                window_handle: Some(0),
                render_proxy_endpoint: None,
            },
        );

        assert!(result.is_ok());
        assert_eq!(registry.session_surface_count(&session_id), 1);
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn platform_surface_backend_for_test() -> &'static str {
        #[cfg(windows)]
        {
            "d3d11"
        }
        #[cfg(target_os = "macos")]
        {
            "macos"
        }
    }

    #[test]
    fn media_pipeline_registry_exposes_active_media_profile_sampling() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("active-profile-session".to_string());
        let profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
            ..MediaProfile::default()
        };

        registry.set_active_media_profile(session_id.clone(), &profile);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.active_codec.as_deref(), Some("hevc"));
        assert_eq!(snapshot.active_codec_profile.as_deref(), Some("main"));
        assert_eq!(snapshot.active_bit_depth, Some(8));
        assert_eq!(snapshot.active_chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(snapshot.active_pixel_format.as_deref(), Some("nv12"));
        assert_eq!(snapshot.active_hdr_enabled, Some(false));
        assert_eq!(snapshot.active_width, Some(2560));
        assert_eq!(snapshot.active_height, Some(1440));
        assert_eq!(snapshot.active_fps, Some(144));
        assert_eq!(snapshot.active_bitrate_mbps, Some(80));

        registry.record_active_media_sample(
            session_id.clone(),
            &profile,
            2560,
            1440,
            "d3d11_shared_nv12",
        );
        let snapshot = registry.snapshot(&session_id);
        assert_eq!(
            snapshot.active_pixel_format.as_deref(),
            Some("d3d11_shared_nv12")
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_keeps_latest_frame_while_worker_is_running() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));

        match registry.enqueue_latest(session_id.clone(), first.clone()) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        assert_eq!(
            registry.enqueue_latest(session_id.clone(), second),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 1
            }
        );
        assert_eq!(
            registry.enqueue_latest(session_id.clone(), third.clone()),
            MediaRenderQueueEnqueue::Queued {
                replaced: true,
                depth: 1
            }
        );

        assert_eq!(registry.take_next_or_finish(&session_id), Some(third));
        assert_eq!(registry.take_next_or_finish(&session_id), None);
        match registry.enqueue_latest(session_id.clone(), first.clone()) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker restart, got {other:?}"),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_can_hold_a_small_paced_backlog() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-paced-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
        let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));
        let fifth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![13, 14, 15]));

        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), second.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 1
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 1);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), third.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 2
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 2);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 3
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 3);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), fifth.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: true,
                depth: 3
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 3);

        assert_eq!(registry.take_next_or_finish(&session_id), Some(third));
        assert_eq!(registry.pending_depth(&session_id), 2);
        assert_eq!(registry.take_next_or_finish(&session_id), Some(fourth));
        assert_eq!(registry.pending_depth(&session_id), 1);
        assert_eq!(registry.take_next_or_finish(&session_id), Some(fifth));
        assert_eq!(registry.pending_depth(&session_id), 0);
        assert_eq!(registry.take_next_or_finish(&session_id), None);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_can_take_latest_and_drop_stale_backlog() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-latest-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
        let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));

        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        registry.enqueue_bounded(session_id.clone(), second, 3);
        registry.enqueue_bounded(session_id.clone(), third, 3);
        registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3);

        let (latest, dropped) = registry.take_latest_or_finish(&session_id);

        assert_eq!(latest, Some(fourth));
        assert_eq!(dropped, 2);
        assert_eq!(registry.pending_depth(&session_id), 0);
        assert_eq!(registry.take_latest_or_finish(&session_id), (None, 0));
        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker restart, got {other:?}"),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_paces_early_frames_to_target_fps() {
        use std::time::Duration;
        use tokio::time::Instant;

        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-pacing-session".to_string());
        let now = Instant::now();

        assert_eq!(registry.pacing_delay(&session_id, 165, now), Duration::ZERO);

        registry.record_presented(&session_id, now);
        let early = now + Duration::from_millis(2);
        let early_delay = registry.pacing_delay(&session_id, 165, early);
        assert!(
            early_delay >= Duration::from_millis(3),
            "expected pacing delay for early frame, got {early_delay:?}"
        );
        assert!(
            early_delay <= Duration::from_millis(5),
            "expected bounded pacing delay for early frame, got {early_delay:?}"
        );

        let late = now + Duration::from_millis(10);
        assert_eq!(
            registry.pacing_delay(&session_id, 165, late),
            Duration::ZERO
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_records_enqueue_and_present_gaps() {
        use std::time::Duration;
        use tokio::time::Instant;

        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-gap-session".to_string());
        let now = Instant::now();

        assert_eq!(registry.record_enqueued(&session_id, now), None);
        assert_eq!(
            registry.record_enqueued(&session_id, now + Duration::from_millis(7)),
            Some(Duration::from_millis(7))
        );

        assert_eq!(
            registry.record_presented(&session_id, now + Duration::from_millis(1)),
            None
        );
        assert_eq!(
            registry.record_presented(&session_id, now + Duration::from_millis(9)),
            Some(Duration::from_millis(8))
        );
    }

    #[test]
    fn media_profile_registry_tracks_negotiated_profile() {
        let mut registry = MediaProfileRegistry::default();
        let session_id = SessionId("profile-session".to_string());
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 64,
            codec: "h264".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };
        let negotiation = mrd_ipc::MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: None,
            selected_height: None,
            downgrade_reason: None,
        };

        registry.set(session_id.clone(), negotiation.clone());

        assert_eq!(registry.get(&session_id), Some(negotiation));
        assert!(registry.remove(&session_id).is_some());
        assert!(registry.get(&session_id).is_none());
    }

    #[test]
    fn capture_source_registry_tracks_selected_source() {
        let mut registry = CaptureSourceRegistry::default();
        let session_id = SessionId("capture-source-session".to_string());
        let source = mrd_ipc::CaptureSource {
            id: "windows:window:0x1234".to_string(),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: "Target App".to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: Some("Target App".to_string()),
            bundle_identifier: None,
            preview_data_url: Some("legacy-preview-token".to_string()),
            preview_width: Some(320),
            preview_height: Some(180),
        };
        let selection = mrd_ipc::CaptureSourceSelection {
            session_id: session_id.clone(),
            source: source.clone(),
            status: "selected".to_string(),
            reason: None,
        };

        registry.set(session_id.clone(), selection);

        assert_eq!(
            registry.get(&session_id).expect("selection").source.id,
            source.id
        );
        assert!(registry.remove(&session_id).is_some());
        assert!(registry.get(&session_id).is_none());
    }

    #[test]
    fn display_mode_registry_tracks_temporary_mode_for_restore() {
        let mut registry = DisplayModeRegistry::default();
        let session_id = SessionId("display-mode-session".to_string());
        let original = mrd_ipc::DisplayMode {
            id: "windows:display:0:2560x1600@60".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 2560,
            height: 1600,
            refresh_hz: 60,
            bit_depth: Some(32),
            is_current: true,
        };
        let requested = mrd_ipc::DisplayMode {
            id: "windows:display:0:1920x1080@60".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            bit_depth: Some(32),
            is_current: false,
        };

        let change = registry.record_change(
            session_id.clone(),
            requested.clone(),
            Some(original.clone()),
            requested.clone(),
            true,
        );

        assert_eq!(change.status, "changed");
        assert!(change.restore_required);
        assert_eq!(registry.restore_mode(&session_id), Some(original.clone()));

        let restored = registry.record_restore(session_id.clone(), requested, original.clone());
        assert_eq!(restored.status, "restored");
        assert!(!restored.restore_required);
        assert_eq!(restored.active, Some(original));
        assert!(registry.restore_mode(&session_id).is_none());
    }
}
