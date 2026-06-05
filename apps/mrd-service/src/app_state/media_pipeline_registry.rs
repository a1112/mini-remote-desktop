use mrd_ipc::{
    AttachedRenderSurface, MediaAdaptationSnapshot, MediaPipelineSnapshot, MediaProfile,
    MediaSenderTransportSnapshot, MediaStageMetrics, MediaTestImpairmentSnapshot,
};
use mrd_proto::SessionId;
#[cfg(any(windows, target_os = "macos"))]
use mrd_render::RendererSnapshot;
use std::collections::{HashMap, VecDeque};

const MEDIA_STAGE_SAMPLE_LIMIT: usize = 240;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;

    #[test]
    fn stage_metrics_keep_sliding_window_and_sorted_snapshot() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("pipeline-stage-window-session".to_string());

        for index in 0..260 {
            registry.record_stage_duration_ms(session_id.clone(), "sender.capture", index as f64);
        }
        registry.record_stage_duration_ms(session_id.clone(), "receiver.decode", 2.0);

        let snapshot = registry.snapshot(&session_id);
        let stages = snapshot
            .stage_metrics
            .iter()
            .map(|metric| metric.stage.as_str())
            .collect::<Vec<_>>();

        assert_eq!(stages, vec!["receiver.decode", "sender.capture"]);
        let capture = snapshot
            .stage_metrics
            .iter()
            .find(|metric| metric.stage == "sender.capture")
            .expect("sender capture metrics");
        assert_eq!(capture.p50_ms, Some(140.0));
        assert_eq!(capture.p95_ms, Some(247.0));
    }
}
