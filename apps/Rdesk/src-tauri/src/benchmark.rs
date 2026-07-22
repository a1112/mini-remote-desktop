use std::{
    fs,
    path::{Path, PathBuf},
};

use mrd_decode_nvdec::{probe_runtime as probe_nvdec_runtime, NvdecCapabilityProbe};
use mrd_ipc::{
    ExperienceEndpointSide, ExperienceFpsWindow, ExperienceFreezeMetrics, ExperienceProbeSnapshot,
    ExperienceResourceSample,
};
use mrd_observability::{PipelineProbeSnapshot, StageId};
use mrd_pipeline_core::{ColorMode, ColorPipeline};
use serde::{Deserialize, Serialize};

const MAX_EXPERIENCE_FPS_WINDOWS: usize = 3_600;
const MAX_EXPERIENCE_FRAME_INTERVALS: usize = 4_096;
const MAX_EXPERIENCE_RESOURCE_SAMPLES: usize = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BenchmarkManifest {
    pub run_id: String,
    pub scenario: String,
    pub transport: String,
    pub capture_backend: String,
    pub encode_backend: String,
    pub decode_backend: String,
    pub renderer_backend: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_secs: u64,
    pub git_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkSummary {
    pub run_id: String,
    pub scenario: String,
    pub transport: String,
    pub capture_backend: String,
    pub encode_backend: String,
    pub decode_backend: String,
    pub renderer_backend: String,
    pub width: u32,
    pub height: u32,
    pub fps_target: u32,
    pub duration_secs: u64,
    pub session_established: bool,
    pub first_frame_seen: bool,
    pub first_frame_time_ms: Option<f64>,
    pub probe_complete: bool,
    pub fps_observed: f64,
    pub bitrate_kbps: f64,
    #[serde(default)]
    pub target_bitrate_kbps: Option<f64>,
    #[serde(default)]
    pub encoded_fps: Option<f64>,
    #[serde(default)]
    pub decoded_fps: Option<f64>,
    #[serde(default)]
    pub zero_copy_enabled: Option<bool>,
    #[serde(default)]
    pub total_bitstream_bytes: Option<u64>,
    pub keyframes: u64,
    pub dropped_frames: u64,
    pub quic_receiver_completed_frames: Option<u64>,
    pub quic_receiver_expired_frames: Option<u64>,
    pub quic_receiver_evicted_frames: Option<u64>,
    pub quic_receiver_duplicate_fragments: Option<u64>,
    pub quic_receiver_rejected_fragments: Option<u64>,
    pub quic_receiver_pending_frames: Option<u64>,
    pub quic_receiver_reassembly_drops: Option<u64>,
    pub zero_write_access_unit_count: u64,
    pub warning_count: u64,
    pub error_count: u64,
    pub restart_count: u64,
    pub encode_total_p95_ms: Option<f64>,
    pub send_write_p95_ms: Option<f64>,
    pub decode_total_p95_ms: Option<f64>,
    pub frame_sink_ingest_p95_ms: Option<f64>,
    pub render_upload_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_submit_wait_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_execute_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_prepare_wait_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_shared_resource_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_draw_present_p95_ms: Option<f64>,
    pub render_present_p95_ms: Option<f64>,
    #[serde(default)]
    pub render_submitted_frames: Option<u64>,
    #[serde(default)]
    pub render_uploaded_frames: Option<u64>,
    #[serde(default)]
    pub render_presented_frames: Option<u64>,
    #[serde(default)]
    pub render_present_skipped_frames: Option<u64>,
    #[serde(default)]
    pub render_queue_replacements: Option<u64>,
    #[serde(default)]
    pub render_stale_frame_drops: Option<u64>,
    #[serde(default)]
    pub swap_chain_max_frame_latency: Option<u32>,
    #[serde(default)]
    pub swap_chain_allow_tearing: Option<bool>,
    #[serde(default)]
    pub swap_chain_waitable_object: Option<bool>,
    #[serde(default)]
    pub swap_chain_present_mode: Option<String>,
    #[serde(default)]
    pub display_refresh_hz: Option<u32>,
    #[serde(default)]
    pub render_thread_priority: Option<String>,
    #[serde(default)]
    pub render_pixel_format: Option<String>,
    #[serde(default)]
    pub color_mode: Option<String>,
    #[serde(default)]
    pub color_pipeline: Option<String>,
    #[serde(default)]
    pub nvdec_shared_copy_attempts: Option<u64>,
    #[serde(default)]
    pub nvdec_shared_copy_successes: Option<u64>,
    #[serde(default)]
    pub nvdec_shared_copy_failures: Option<u64>,
    #[serde(default)]
    pub nvdec_shared_copy_last_stage: Option<String>,
    #[serde(default)]
    pub nvdec_shared_copy_last_api: Option<String>,
    #[serde(default)]
    pub nvdec_shared_copy_last_error: Option<String>,
    pub nvdec_runtime_summary: String,
    pub nvdec_h264_capability: String,
    pub nvdec_hevc_capability: String,
    pub nvdec_hevc_main10_capability: String,
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub run_skipped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<ExperienceProbeSnapshot>,
    pub run_passed: bool,
}

fn experience_from_present_intervals(
    first_visible_frame_ms: Option<f64>,
    duration_secs: u64,
    target_fps: u32,
    frame_intervals_ms: &[f64],
) -> ExperienceProbeSnapshot {
    let duration_ms = duration_secs.saturating_mul(1_000) as f64;
    let window_limit = usize::try_from(duration_secs)
        .unwrap_or(usize::MAX)
        .min(MAX_EXPERIENCE_FPS_WINDOWS);
    let mut window_counts = vec![0_u32; window_limit];
    let bounded_intervals = frame_intervals_ms
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .take(MAX_EXPERIENCE_FRAME_INTERVALS)
        .collect::<Vec<_>>();
    if let Some(mut presented_ms) =
        first_visible_frame_ms.filter(|value| value.is_finite() && *value >= 0.0)
    {
        if presented_ms < duration_ms {
            if let Some(count) = window_counts.get_mut((presented_ms / 1_000.0) as usize) {
                *count = count.saturating_add(1);
            }
        }
        for interval_ms in bounded_intervals.iter().copied() {
            presented_ms += interval_ms;
            if presented_ms >= duration_ms {
                break;
            }
            if let Some(count) = window_counts.get_mut((presented_ms / 1_000.0) as usize) {
                *count = count.saturating_add(1);
            }
        }
    }

    let expected_ms = 1_000.0 / f64::from(target_fps.max(1));
    let mut prior_total_ms = 0.0;
    let mut prior_count = 0_u64;
    let mut stall_count = 0_u64;
    let mut total_stall_duration_ms = 0.0;
    let mut freeze_count = 0_u64;
    let mut total_freeze_duration_ms = 0.0;
    for interval_ms in bounded_intervals.iter().copied() {
        if interval_ms > (expected_ms * 3.0).max(100.0) {
            stall_count = stall_count.saturating_add(1);
            total_stall_duration_ms += interval_ms;
        }
        let prior_mean_ms = if prior_count == 0 {
            expected_ms
        } else {
            prior_total_ms / prior_count as f64
        };
        if interval_ms > (prior_mean_ms * 3.0).max(150.0) {
            freeze_count = freeze_count.saturating_add(1);
            total_freeze_duration_ms += interval_ms;
        }
        prior_total_ms += interval_ms;
        prior_count = prior_count.saturating_add(1);
    }

    ExperienceProbeSnapshot {
        first_visible_frame_ms,
        fps_windows: window_counts
            .into_iter()
            .enumerate()
            .map(|(window, frame_count)| ExperienceFpsWindow {
                start_monotonic_ms: window as f64 * 1_000.0,
                duration_ms: 1_000.0,
                frame_count,
                fps: f64::from(frame_count),
            })
            .collect(),
        frame_intervals_ms: bounded_intervals,
        stall_count,
        total_stall_duration_ms,
        freeze_metrics: ExperienceFreezeMetrics {
            freeze_count,
            total_freeze_duration_ms,
        },
        input_probes: Vec::new(),
        resource_samples: Vec::new(),
        adaptation_transitions: Vec::new(),
    }
}

fn controller_resource_sample(
    snapshot: &crate::resource_monitor::SystemResourceSnapshot,
    monotonic_ms: f64,
) -> Option<ExperienceResourceSample> {
    if !snapshot.target_found {
        return None;
    }
    let sample = ExperienceResourceSample {
        side: ExperienceEndpointSide::Controller,
        monotonic_ms,
        cpu_usage_percent: f64::from(snapshot.cpu_usage_percent).clamp(0.0, 100.0),
        rss_mb: snapshot.memory_used_mb as f64,
        gpu_usage_percent: (snapshot.gpu_usage_metrics_scope == "process")
            .then_some(snapshot.gpu_usage_percent.map(f64::from))
            .flatten(),
        vram_used_mb: (snapshot.gpu_memory_metrics_scope == "process")
            .then_some(snapshot.gpu_memory_used_mb.map(|value| value as f64))
            .flatten(),
    };
    sample.is_finite().then_some(sample)
}

impl BenchmarkSummary {
    fn nvdec_capability_summary() -> (String, String, String, String) {
        let runtime = probe_nvdec_runtime();
        let capability_text = |codec: &str, bit_depth_minus8: u8| {
            runtime
                .capability_probes
                .iter()
                .find(|probe| probe.codec == codec && probe.bit_depth_minus8 == bit_depth_minus8)
                .map(render_nvdec_capability)
                .unwrap_or_else(|| {
                    format!(
                        "{codec} {}-bit capability probe unavailable",
                        bit_depth_minus8 + 8
                    )
                })
        };

        (
            runtime.summary,
            capability_text("h264", 0),
            capability_text("hevc", 0),
            capability_text("hevc", 2),
        )
    }

    fn counter(probe: &PipelineProbeSnapshot, name: &str) -> Option<u64> {
        probe
            .counters
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| *value)
    }

    fn renderer_enabled(renderer_backend: &str) -> bool {
        !renderer_backend.eq_ignore_ascii_case("none")
    }

    fn nonzero_counter(value: Option<u64>) -> Option<u64> {
        value.filter(|value| *value > 0)
    }

    fn render_health_failure_reason(
        renderer_backend: &str,
        render_submitted_frames: Option<u64>,
        render_uploaded_frames: Option<u64>,
        render_presented_frames: Option<u64>,
    ) -> Option<String> {
        if !Self::renderer_enabled(renderer_backend) {
            return None;
        }

        if let Some(submitted_frames) = render_submitted_frames {
            if submitted_frames >= 30 {
                let uploaded_frames = render_uploaded_frames.unwrap_or(0);
                let minimum_uploaded = (submitted_frames / 10).max(3);
                if uploaded_frames < minimum_uploaded {
                    return Some(format!(
                        "render upload starvation: uploaded {uploaded_frames} of {submitted_frames} submitted render frames, minimum {minimum_uploaded}"
                    ));
                }
            }
        }

        let uploaded_frames = match render_uploaded_frames {
            Some(value) if value >= 30 => value,
            _ => return None,
        };
        let presented_frames = render_presented_frames.unwrap_or(0);
        let minimum_presented = (uploaded_frames / 10).max(3);
        if presented_frames < minimum_presented {
            return Some(format!(
                "render present collapse: presented {presented_frames} of {uploaded_frames} uploaded render frames, minimum {minimum_presented}"
            ));
        }

        None
    }

    pub fn from_probe(
        manifest: &BenchmarkManifest,
        probe: &PipelineProbeSnapshot,
        session_established: bool,
        first_frame_seen: bool,
        first_frame_time_ms: f64,
    ) -> Self {
        let (
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
        ) = Self::nvdec_capability_summary();
        let stage_p95 = |stage: StageId| {
            probe
                .stages
                .iter()
                .find(|(candidate, _)| *candidate == stage)
                .and_then(|(_, stats)| stats.p95_ms)
        };
        let probe_complete = [
            StageId::EncodeTotal,
            StageId::SendWrite,
            StageId::DecodeTotal,
            StageId::FrameSinkIngest,
        ]
        .iter()
        .all(|stage| {
            probe
                .stages
                .iter()
                .any(|(candidate, stats)| candidate == stage && stats.count > 0)
        });
        let render_submitted_frames = Self::counter(probe, "render_submitted_frames");
        let render_uploaded_frames = Self::counter(probe, "render_uploaded_frames");
        let render_presented_frames = Self::counter(probe, "render_presented_frames");
        let render_present_skipped_frames = Self::counter(probe, "render_present_skipped_frames");
        let render_queue_replacements = Self::counter(probe, "render_queue_replacements");
        let render_stale_frame_drops = Self::counter(probe, "render_stale_frame_drops");
        let failure_reason = Self::render_health_failure_reason(
            &manifest.renderer_backend,
            Self::nonzero_counter(render_submitted_frames),
            Self::nonzero_counter(render_uploaded_frames),
            render_presented_frames,
        );
        let run_passed =
            session_established && first_frame_seen && probe_complete && failure_reason.is_none();

        Self {
            run_id: manifest.run_id.clone(),
            scenario: manifest.scenario.clone(),
            transport: manifest.transport.clone(),
            capture_backend: manifest.capture_backend.clone(),
            encode_backend: manifest.encode_backend.clone(),
            decode_backend: manifest.decode_backend.clone(),
            renderer_backend: manifest.renderer_backend.clone(),
            width: manifest.width,
            height: manifest.height,
            fps_target: manifest.fps,
            duration_secs: manifest.duration_secs,
            session_established,
            first_frame_seen,
            first_frame_time_ms: Some(first_frame_time_ms),
            probe_complete,
            fps_observed: probe.fps,
            bitrate_kbps: probe.bitrate_kbps,
            target_bitrate_kbps: None,
            encoded_fps: None,
            decoded_fps: None,
            zero_copy_enabled: None,
            total_bitstream_bytes: None,
            keyframes: probe.keyframes,
            dropped_frames: probe.dropped_frames,
            quic_receiver_completed_frames: Self::counter(probe, "quic_receiver_completed_frames"),
            quic_receiver_expired_frames: Self::counter(probe, "quic_receiver_expired_frames"),
            quic_receiver_evicted_frames: Self::counter(probe, "quic_receiver_evicted_frames"),
            quic_receiver_duplicate_fragments: Self::counter(
                probe,
                "quic_receiver_duplicate_fragments",
            ),
            quic_receiver_rejected_fragments: Self::counter(
                probe,
                "quic_receiver_rejected_fragments",
            ),
            quic_receiver_pending_frames: Self::counter(probe, "quic_receiver_pending_frames"),
            quic_receiver_reassembly_drops: Self::counter(probe, "quic_receiver_reassembly_drops"),
            zero_write_access_unit_count: 0,
            warning_count: 0,
            error_count: 0,
            restart_count: 0,
            encode_total_p95_ms: stage_p95(StageId::EncodeTotal),
            send_write_p95_ms: stage_p95(StageId::SendWrite),
            decode_total_p95_ms: stage_p95(StageId::DecodeTotal),
            frame_sink_ingest_p95_ms: stage_p95(StageId::FrameSinkIngest),
            render_upload_p95_ms: stage_p95(StageId::RenderUpload),
            render_submit_wait_p95_ms: stage_p95(StageId::RenderSubmitWait),
            render_execute_p95_ms: stage_p95(StageId::RenderExecute),
            render_prepare_wait_p95_ms: stage_p95(StageId::RenderPrepareWait),
            render_shared_resource_p95_ms: stage_p95(StageId::RenderSharedResource),
            render_draw_present_p95_ms: stage_p95(StageId::RenderDrawPresent),
            render_present_p95_ms: stage_p95(StageId::RenderPresent),
            render_submitted_frames,
            render_uploaded_frames,
            render_presented_frames,
            render_present_skipped_frames,
            render_queue_replacements,
            render_stale_frame_drops,
            swap_chain_max_frame_latency: None,
            swap_chain_allow_tearing: None,
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            render_pixel_format: None,
            color_mode: Some(ColorMode::Full.as_str().to_string()),
            color_pipeline: Some(ColorPipeline::Sdr8.as_str().to_string()),
            nvdec_shared_copy_attempts: Self::counter(probe, "nvdec_shared_copy_attempts"),
            nvdec_shared_copy_successes: Self::counter(probe, "nvdec_shared_copy_successes"),
            nvdec_shared_copy_failures: Self::counter(probe, "nvdec_shared_copy_failures"),
            nvdec_shared_copy_last_stage: None,
            nvdec_shared_copy_last_api: None,
            nvdec_shared_copy_last_error: None,
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
            failure_reason,
            run_skipped: false,
            experience: None,
            run_passed,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_transport_probes(
        manifest: &BenchmarkManifest,
        sender_probe: &PipelineProbeSnapshot,
        receiver_probe: &PipelineProbeSnapshot,
        session_established: bool,
        first_frame_seen: bool,
        first_frame_time_ms: f64,
        zero_write_access_unit_count: u64,
        observed_decoded_frames: Option<u64>,
    ) -> Self {
        let (
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
        ) = Self::nvdec_capability_summary();
        let find_stage = |probe: &PipelineProbeSnapshot, stage: StageId| {
            probe
                .stages
                .iter()
                .find(|(candidate, _)| *candidate == stage)
                .and_then(|(_, stats)| stats.p95_ms)
        };
        let probe_complete = [
            find_stage(sender_probe, StageId::EncodeTotal),
            find_stage(sender_probe, StageId::SendWrite),
            find_stage(receiver_probe, StageId::DecodeTotal),
            find_stage(receiver_probe, StageId::FrameSinkIngest),
        ]
        .iter()
        .all(Option::is_some);
        let fps_observed = observed_decoded_frames
            .filter(|_| manifest.duration_secs > 0)
            .map(|frames| frames as f64 / manifest.duration_secs as f64)
            .unwrap_or(receiver_probe.fps);
        let render_submitted_frames = Self::counter(receiver_probe, "render_submitted_frames");
        let render_uploaded_frames = Self::counter(receiver_probe, "render_uploaded_frames");
        let render_presented_frames = Self::counter(receiver_probe, "render_presented_frames");
        let render_present_skipped_frames =
            Self::counter(receiver_probe, "render_present_skipped_frames");
        let render_queue_replacements = Self::counter(receiver_probe, "render_queue_replacements");
        let render_stale_frame_drops = Self::counter(receiver_probe, "render_stale_frame_drops");
        let failure_reason = Self::render_health_failure_reason(
            &manifest.renderer_backend,
            Self::nonzero_counter(render_submitted_frames),
            Self::nonzero_counter(render_uploaded_frames),
            render_presented_frames,
        );
        let run_passed =
            session_established && first_frame_seen && probe_complete && failure_reason.is_none();

        Self {
            run_id: manifest.run_id.clone(),
            scenario: manifest.scenario.clone(),
            transport: manifest.transport.clone(),
            capture_backend: manifest.capture_backend.clone(),
            encode_backend: manifest.encode_backend.clone(),
            decode_backend: manifest.decode_backend.clone(),
            renderer_backend: manifest.renderer_backend.clone(),
            width: manifest.width,
            height: manifest.height,
            fps_target: manifest.fps,
            duration_secs: manifest.duration_secs,
            session_established,
            first_frame_seen,
            first_frame_time_ms: Some(first_frame_time_ms),
            probe_complete,
            fps_observed,
            bitrate_kbps: sender_probe.bitrate_kbps,
            target_bitrate_kbps: None,
            encoded_fps: None,
            decoded_fps: None,
            zero_copy_enabled: None,
            total_bitstream_bytes: None,
            keyframes: sender_probe.keyframes.max(receiver_probe.keyframes),
            dropped_frames: sender_probe
                .dropped_frames
                .max(receiver_probe.dropped_frames),
            quic_receiver_completed_frames: Self::counter(
                receiver_probe,
                "quic_receiver_completed_frames",
            ),
            quic_receiver_expired_frames: Self::counter(
                receiver_probe,
                "quic_receiver_expired_frames",
            ),
            quic_receiver_evicted_frames: Self::counter(
                receiver_probe,
                "quic_receiver_evicted_frames",
            ),
            quic_receiver_duplicate_fragments: Self::counter(
                receiver_probe,
                "quic_receiver_duplicate_fragments",
            ),
            quic_receiver_rejected_fragments: Self::counter(
                receiver_probe,
                "quic_receiver_rejected_fragments",
            ),
            quic_receiver_pending_frames: Self::counter(
                receiver_probe,
                "quic_receiver_pending_frames",
            ),
            quic_receiver_reassembly_drops: Self::counter(
                receiver_probe,
                "quic_receiver_reassembly_drops",
            ),
            zero_write_access_unit_count,
            warning_count: 0,
            error_count: 0,
            restart_count: 0,
            encode_total_p95_ms: find_stage(sender_probe, StageId::EncodeTotal),
            send_write_p95_ms: find_stage(sender_probe, StageId::SendWrite),
            decode_total_p95_ms: find_stage(receiver_probe, StageId::DecodeTotal),
            frame_sink_ingest_p95_ms: find_stage(receiver_probe, StageId::FrameSinkIngest),
            render_upload_p95_ms: find_stage(receiver_probe, StageId::RenderUpload),
            render_submit_wait_p95_ms: find_stage(receiver_probe, StageId::RenderSubmitWait),
            render_execute_p95_ms: find_stage(receiver_probe, StageId::RenderExecute),
            render_prepare_wait_p95_ms: find_stage(receiver_probe, StageId::RenderPrepareWait),
            render_shared_resource_p95_ms: find_stage(
                receiver_probe,
                StageId::RenderSharedResource,
            ),
            render_draw_present_p95_ms: find_stage(receiver_probe, StageId::RenderDrawPresent),
            render_present_p95_ms: find_stage(receiver_probe, StageId::RenderPresent),
            render_submitted_frames,
            render_uploaded_frames,
            render_presented_frames,
            render_present_skipped_frames,
            render_queue_replacements,
            render_stale_frame_drops,
            swap_chain_max_frame_latency: None,
            swap_chain_allow_tearing: None,
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            render_pixel_format: None,
            color_mode: Some(ColorMode::Full.as_str().to_string()),
            color_pipeline: Some(ColorPipeline::Sdr8.as_str().to_string()),
            nvdec_shared_copy_attempts: Self::counter(receiver_probe, "nvdec_shared_copy_attempts"),
            nvdec_shared_copy_successes: Self::counter(
                receiver_probe,
                "nvdec_shared_copy_successes",
            ),
            nvdec_shared_copy_failures: Self::counter(receiver_probe, "nvdec_shared_copy_failures"),
            nvdec_shared_copy_last_stage: None,
            nvdec_shared_copy_last_api: None,
            nvdec_shared_copy_last_error: None,
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
            failure_reason,
            run_skipped: false,
            experience: None,
            run_passed,
        }
    }

    pub fn csv_header() -> Vec<&'static str> {
        vec![
            "run_id",
            "scenario",
            "transport",
            "capture_backend",
            "encode_backend",
            "decode_backend",
            "renderer_backend",
            "width",
            "height",
            "fps_target",
            "duration_secs",
            "session_established",
            "first_frame_seen",
            "first_frame_time_ms",
            "probe_complete",
            "fps_observed",
            "bitrate_kbps",
            "target_bitrate_kbps",
            "encoded_fps",
            "decoded_fps",
            "zero_copy_enabled",
            "total_bitstream_bytes",
            "keyframes",
            "dropped_frames",
            "quic_receiver_completed_frames",
            "quic_receiver_expired_frames",
            "quic_receiver_evicted_frames",
            "quic_receiver_duplicate_fragments",
            "quic_receiver_rejected_fragments",
            "quic_receiver_pending_frames",
            "quic_receiver_reassembly_drops",
            "zero_write_access_unit_count",
            "warning_count",
            "error_count",
            "restart_count",
            "encode_total_p95_ms",
            "send_write_p95_ms",
            "decode_total_p95_ms",
            "frame_sink_ingest_p95_ms",
            "render_upload_p95_ms",
            "render_submit_wait_p95_ms",
            "render_execute_p95_ms",
            "render_prepare_wait_p95_ms",
            "render_shared_resource_p95_ms",
            "render_draw_present_p95_ms",
            "render_present_p95_ms",
            "render_submitted_frames",
            "render_uploaded_frames",
            "render_presented_frames",
            "render_present_skipped_frames",
            "render_queue_replacements",
            "render_stale_frame_drops",
            "swap_chain_max_frame_latency",
            "swap_chain_allow_tearing",
            "swap_chain_waitable_object",
            "swap_chain_present_mode",
            "display_refresh_hz",
            "render_thread_priority",
            "render_pixel_format",
            "color_mode",
            "color_pipeline",
            "nvdec_shared_copy_attempts",
            "nvdec_shared_copy_successes",
            "nvdec_shared_copy_failures",
            "nvdec_shared_copy_last_stage",
            "nvdec_shared_copy_last_api",
            "nvdec_shared_copy_last_error",
            "nvdec_runtime_summary",
            "nvdec_h264_capability",
            "nvdec_hevc_capability",
            "nvdec_hevc_main10_capability",
            "failure_reason",
            "run_skipped",
            "run_passed",
        ]
    }

    pub fn csv_row(&self) -> Vec<String> {
        vec![
            self.run_id.clone(),
            self.scenario.clone(),
            self.transport.clone(),
            self.capture_backend.clone(),
            self.encode_backend.clone(),
            self.decode_backend.clone(),
            self.renderer_backend.clone(),
            self.width.to_string(),
            self.height.to_string(),
            self.fps_target.to_string(),
            self.duration_secs.to_string(),
            self.session_established.to_string(),
            self.first_frame_seen.to_string(),
            option_f64(self.first_frame_time_ms),
            self.probe_complete.to_string(),
            self.fps_observed.to_string(),
            self.bitrate_kbps.to_string(),
            option_f64(self.target_bitrate_kbps),
            option_f64(self.encoded_fps),
            option_f64(self.decoded_fps),
            option_bool(self.zero_copy_enabled),
            option_u64(self.total_bitstream_bytes),
            self.keyframes.to_string(),
            self.dropped_frames.to_string(),
            option_u64(self.quic_receiver_completed_frames),
            option_u64(self.quic_receiver_expired_frames),
            option_u64(self.quic_receiver_evicted_frames),
            option_u64(self.quic_receiver_duplicate_fragments),
            option_u64(self.quic_receiver_rejected_fragments),
            option_u64(self.quic_receiver_pending_frames),
            option_u64(self.quic_receiver_reassembly_drops),
            self.zero_write_access_unit_count.to_string(),
            self.warning_count.to_string(),
            self.error_count.to_string(),
            self.restart_count.to_string(),
            option_f64(self.encode_total_p95_ms),
            option_f64(self.send_write_p95_ms),
            option_f64(self.decode_total_p95_ms),
            option_f64(self.frame_sink_ingest_p95_ms),
            option_f64(self.render_upload_p95_ms),
            option_f64(self.render_submit_wait_p95_ms),
            option_f64(self.render_execute_p95_ms),
            option_f64(self.render_prepare_wait_p95_ms),
            option_f64(self.render_shared_resource_p95_ms),
            option_f64(self.render_draw_present_p95_ms),
            option_f64(self.render_present_p95_ms),
            option_u64(self.render_submitted_frames),
            option_u64(self.render_uploaded_frames),
            option_u64(self.render_presented_frames),
            option_u64(self.render_present_skipped_frames),
            option_u64(self.render_queue_replacements),
            option_u64(self.render_stale_frame_drops),
            option_u32(self.swap_chain_max_frame_latency),
            option_bool(self.swap_chain_allow_tearing),
            option_bool(self.swap_chain_waitable_object),
            self.swap_chain_present_mode.clone().unwrap_or_default(),
            option_u32(self.display_refresh_hz),
            self.render_thread_priority.clone().unwrap_or_default(),
            self.render_pixel_format.clone().unwrap_or_default(),
            self.color_mode.clone().unwrap_or_default(),
            self.color_pipeline.clone().unwrap_or_default(),
            option_u64(self.nvdec_shared_copy_attempts),
            option_u64(self.nvdec_shared_copy_successes),
            option_u64(self.nvdec_shared_copy_failures),
            self.nvdec_shared_copy_last_stage
                .clone()
                .unwrap_or_default(),
            self.nvdec_shared_copy_last_api.clone().unwrap_or_default(),
            csv_escape_text(
                self.nvdec_shared_copy_last_error
                    .as_deref()
                    .unwrap_or_default(),
            ),
            self.nvdec_runtime_summary.clone(),
            self.nvdec_h264_capability.clone(),
            self.nvdec_hevc_capability.clone(),
            self.nvdec_hevc_main10_capability.clone(),
            csv_escape_text(self.failure_reason.as_deref().unwrap_or_default()),
            self.run_skipped.to_string(),
            self.run_passed.to_string(),
        ]
    }
}

fn render_nvdec_capability(probe: &NvdecCapabilityProbe) -> String {
    format!(
        "runtime_supported={} ({}) ; wired_supported={} ({})",
        probe.runtime_supported, probe.runtime_reason, probe.wired_supported, probe.wired_reason
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkPaths {
    pub run_dir: PathBuf,
    pub manifest_json: PathBuf,
    pub summary_json: PathBuf,
    pub summary_csv: PathBuf,
    pub report_md: PathBuf,
    pub logs_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub reports_dir: PathBuf,
    pub host_stdout: PathBuf,
    pub host_stderr: PathBuf,
    pub signaling_stdout: PathBuf,
    pub signaling_stderr: PathBuf,
}

impl BenchmarkPaths {
    pub fn new(repo_root: &Path, date: String, profile: String, run_id: String) -> Self {
        let run_dir = repo_root
            .join("artifacts")
            .join("benchmarks")
            .join(date)
            .join(profile)
            .join(run_id);
        let logs_dir = run_dir.join("logs");
        let sessions_dir = run_dir.join("sessions");
        let reports_dir = run_dir.join("reports");
        Self {
            manifest_json: run_dir.join("manifest.json"),
            summary_json: run_dir.join("summary.json"),
            summary_csv: run_dir.join("summary.csv"),
            report_md: reports_dir.join("markdown-report.md"),
            host_stdout: logs_dir.join("host.stdout.log"),
            host_stderr: logs_dir.join("host.stderr.log"),
            signaling_stdout: logs_dir.join("signaling.stdout.log"),
            signaling_stderr: logs_dir.join("signaling.stderr.log"),
            run_dir,
            logs_dir,
            sessions_dir,
            reports_dir,
        }
    }

    pub fn probe_json(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{session_id}.probe.json"))
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [
            &self.run_dir,
            &self.logs_dir,
            &self.sessions_dir,
            &self.reports_dir,
        ] {
            fs::create_dir_all(dir)
                .map_err(|error| format!("create benchmark artifact dir failed: {error}"))?;
        }
        Ok(())
    }
}

pub fn write_benchmark_artifacts(
    paths: &BenchmarkPaths,
    manifest: &BenchmarkManifest,
    summary: &BenchmarkSummary,
    session_id: &str,
    probe: &PipelineProbeSnapshot,
) -> Result<(), String> {
    paths.ensure_dirs()?;
    fs::write(
        &paths.manifest_json,
        serde_json::to_string_pretty(manifest)
            .map_err(|error| format!("serialize benchmark manifest failed: {error}"))?,
    )
    .map_err(|error| format!("write benchmark manifest failed: {error}"))?;
    fs::write(
        &paths.summary_json,
        serde_json::to_string_pretty(summary)
            .map_err(|error| format!("serialize benchmark summary failed: {error}"))?,
    )
    .map_err(|error| format!("write benchmark summary failed: {error}"))?;
    let csv = format!(
        "{}\n{}\n",
        BenchmarkSummary::csv_header().join(","),
        summary.csv_row().join(",")
    );
    fs::write(&paths.summary_csv, csv)
        .map_err(|error| format!("write benchmark csv failed: {error}"))?;
    fs::write(
        paths.probe_json(session_id),
        serde_json::to_string_pretty(probe)
            .map_err(|error| format!("serialize benchmark probe failed: {error}"))?,
    )
    .map_err(|error| format!("write benchmark probe failed: {error}"))?;
    fs::write(
        &paths.report_md,
        render_markdown_report(manifest, summary, session_id),
    )
    .map_err(|error| format!("write benchmark markdown report failed: {error}"))?;
    Ok(())
}

fn option_f64(value: Option<f64>) -> String {
    value.map(|item| item.to_string()).unwrap_or_default()
}

fn csv_escape_text(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace(',', ";")
}

fn option_u64(value: Option<u64>) -> String {
    value.map(|item| item.to_string()).unwrap_or_default()
}

fn option_u32(value: Option<u32>) -> String {
    value.map(|item| item.to_string()).unwrap_or_default()
}

fn option_bool(value: Option<bool>) -> String {
    value.map(|item| item.to_string()).unwrap_or_default()
}

fn render_markdown_report(
    manifest: &BenchmarkManifest,
    summary: &BenchmarkSummary,
    session_id: &str,
) -> String {
    format!(
        "# Transport Benchmark Report\n\n\
Run: `{run_id}`  \n\
Scenario: `{scenario}`  \n\
Transport: `{transport}`  \n\
Commit: `{commit}`  \n\
Resolution: `{width}x{height}@{fps}`  \n\
Duration: `{duration}s`\n\n\
## Result\n\n\
- Status: `{status}`\n\
- Session established: `{session_established}`\n\
- First frame seen: `{first_frame_seen}`\n\
- First frame time ms: `{first_frame_ms}`\n\
- Probe complete: `{probe_complete}`\n\
- Failure reason: `{failure_reason}`\n\
\n## Metrics\n\n\
| Metric | Value |\n\
| --- | --- |\n\
| fps_observed | {fps_observed} |\n\
| bitrate_kbps | {bitrate_kbps} |\n\
| encode_total_p95_ms | {encode_p95} |\n\
| send_write_p95_ms | {send_p95} |\n\
| decode_total_p95_ms | {decode_p95} |\n\
| frame_sink_ingest_p95_ms | {frame_sink_p95} |\n\
| render_upload_p95_ms | {render_p95} |\n\
| render_submit_wait_p95_ms | {render_submit_wait_p95} |\n\
| render_execute_p95_ms | {render_execute_p95} |\n\
| render_prepare_wait_p95_ms | {render_prepare_wait_p95} |\n\
| render_shared_resource_p95_ms | {render_shared_resource_p95} |\n\
| render_draw_present_p95_ms | {render_draw_present_p95} |\n\
| render_present_p95_ms | {present_p95} |\n\
| swap_chain_max_frame_latency | {swap_chain_max_frame_latency} |\n\
| swap_chain_allow_tearing | {swap_chain_allow_tearing} |\n\
| swap_chain_waitable_object | {swap_chain_waitable} |\n\
| swap_chain_present_mode | {swap_chain_present_mode} |\n\
| display_refresh_hz | {display_refresh_hz} |\n\
| render_thread_priority | {render_thread_priority} |\n\
| render_pixel_format | {render_pixel_format} |\n\
| color_mode | {color_mode} |\n\
| color_pipeline | {color_pipeline} |\n\
| nvdec_shared_copy_attempts | {nvdec_shared_copy_attempts} |\n\
| nvdec_shared_copy_successes | {nvdec_shared_copy_successes} |\n\
| nvdec_shared_copy_failures | {nvdec_shared_copy_failures} |\n\
| nvdec_shared_copy_last_stage | {nvdec_shared_copy_last_stage} |\n\
| nvdec_shared_copy_last_api | {nvdec_shared_copy_last_api} |\n\
| nvdec_shared_copy_last_error | {nvdec_shared_copy_last_error} |\n\
| keyframes | {keyframes} |\n\
| dropped_frames | {dropped_frames} |\n\
| quic_receiver_completed_frames | {quic_completed} |\n\
| quic_receiver_expired_frames | {quic_expired} |\n\
| quic_receiver_evicted_frames | {quic_evicted} |\n\
| quic_receiver_duplicate_fragments | {quic_duplicate} |\n\
| quic_receiver_rejected_fragments | {quic_rejected} |\n\
| quic_receiver_pending_frames | {quic_pending} |\n\
| quic_receiver_reassembly_drops | {quic_drops} |\n\
| warning_count | {warning_count} |\n\
| error_count | {error_count} |\n\
\n## NVDEC Capability\n\n\
- nvdec_runtime_summary: `{nvdec_runtime_summary}`\n\
- nvdec_h264: `{nvdec_h264_capability}`\n\
- nvdec_hevc: `{nvdec_hevc_capability}`\n\
- nvdec_hevc_main10: `{nvdec_hevc_main10_capability}`\n\
\n## Paths\n\n\
- Probe: `sessions/{session_id}.probe.json`\n\
- Summary: `summary.json`\n\
- CSV: `summary.csv`\n\
- Logs: `logs/`\n",
        run_id = manifest.run_id,
        scenario = manifest.scenario,
        transport = manifest.transport,
        commit = manifest.git_commit,
        width = manifest.width,
        height = manifest.height,
        fps = manifest.fps,
        duration = manifest.duration_secs,
        status = if summary.run_passed { "PASS" } else { "FAIL" },
        session_established = summary.session_established,
        first_frame_seen = summary.first_frame_seen,
        first_frame_ms = option_f64(summary.first_frame_time_ms),
        probe_complete = summary.probe_complete,
        failure_reason = summary.failure_reason.as_deref().unwrap_or_default(),
        fps_observed = summary.fps_observed,
        bitrate_kbps = summary.bitrate_kbps,
        encode_p95 = option_f64(summary.encode_total_p95_ms),
        send_p95 = option_f64(summary.send_write_p95_ms),
        decode_p95 = option_f64(summary.decode_total_p95_ms),
        frame_sink_p95 = option_f64(summary.frame_sink_ingest_p95_ms),
        render_p95 = option_f64(summary.render_upload_p95_ms),
        render_submit_wait_p95 = option_f64(summary.render_submit_wait_p95_ms),
        render_execute_p95 = option_f64(summary.render_execute_p95_ms),
        render_prepare_wait_p95 = option_f64(summary.render_prepare_wait_p95_ms),
        render_shared_resource_p95 = option_f64(summary.render_shared_resource_p95_ms),
        render_draw_present_p95 = option_f64(summary.render_draw_present_p95_ms),
        present_p95 = option_f64(summary.render_present_p95_ms),
        swap_chain_max_frame_latency = option_u32(summary.swap_chain_max_frame_latency),
        swap_chain_allow_tearing = option_bool(summary.swap_chain_allow_tearing),
        swap_chain_waitable = option_bool(summary.swap_chain_waitable_object),
        swap_chain_present_mode = summary
            .swap_chain_present_mode
            .as_deref()
            .unwrap_or_default(),
        display_refresh_hz = option_u32(summary.display_refresh_hz),
        render_thread_priority = summary
            .render_thread_priority
            .as_deref()
            .unwrap_or_default(),
        render_pixel_format = summary.render_pixel_format.as_deref().unwrap_or_default(),
        color_mode = summary.color_mode.as_deref().unwrap_or_default(),
        color_pipeline = summary.color_pipeline.as_deref().unwrap_or_default(),
        nvdec_shared_copy_attempts = option_u64(summary.nvdec_shared_copy_attempts),
        nvdec_shared_copy_successes = option_u64(summary.nvdec_shared_copy_successes),
        nvdec_shared_copy_failures = option_u64(summary.nvdec_shared_copy_failures),
        nvdec_shared_copy_last_stage = summary
            .nvdec_shared_copy_last_stage
            .as_deref()
            .unwrap_or_default(),
        nvdec_shared_copy_last_api = summary
            .nvdec_shared_copy_last_api
            .as_deref()
            .unwrap_or_default(),
        nvdec_shared_copy_last_error = summary
            .nvdec_shared_copy_last_error
            .as_deref()
            .unwrap_or_default(),
        keyframes = summary.keyframes,
        dropped_frames = summary.dropped_frames,
        quic_completed = option_u64(summary.quic_receiver_completed_frames),
        quic_expired = option_u64(summary.quic_receiver_expired_frames),
        quic_evicted = option_u64(summary.quic_receiver_evicted_frames),
        quic_duplicate = option_u64(summary.quic_receiver_duplicate_fragments),
        quic_rejected = option_u64(summary.quic_receiver_rejected_fragments),
        quic_pending = option_u64(summary.quic_receiver_pending_frames),
        quic_drops = option_u64(summary.quic_receiver_reassembly_drops),
        warning_count = summary.warning_count,
        error_count = summary.error_count,
        nvdec_runtime_summary = summary.nvdec_runtime_summary,
        nvdec_h264_capability = summary.nvdec_h264_capability,
        nvdec_hevc_capability = summary.nvdec_hevc_capability,
        nvdec_hevc_main10_capability = summary.nvdec_hevc_main10_capability,
        session_id = session_id,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        thread,
        time::{Duration, Instant},
    };

    #[cfg(any(windows, target_os = "linux"))]
    use mrd_encode_nvenc_av1::NvencAv1Encoder;
    use mrd_observability::{PipelineProbeSnapshot, StageId, StageStatsSnapshot};
    use mrd_pipeline_core::{ColorMode, ColorPipeline};
    use mrd_proto::SessionId;

    use crate::test_harness::{
        CaptureType, DecoderType, EncoderType, RendererType, TestChain, TestConfig, TestHarness,
        TransportKind,
    };

    use super::{
        controller_resource_sample, experience_from_present_intervals, BenchmarkManifest,
        BenchmarkPaths, BenchmarkSummary, MAX_EXPERIENCE_RESOURCE_SAMPLES,
    };

    #[test]
    fn experience_artifact_uses_present_intervals_for_windows_stalls_and_freezes() {
        let snapshot = experience_from_present_intervals(
            Some(100.0),
            2,
            60,
            &[16.0, 16.0, 200.0, 800.0, 16.0],
        );

        assert_eq!(snapshot.first_visible_frame_ms, Some(100.0));
        assert_eq!(snapshot.frame_intervals_ms.len(), 5);
        assert_eq!(snapshot.fps_windows.len(), 2);
        assert_eq!(
            snapshot
                .fps_windows
                .iter()
                .map(|window| window.frame_count)
                .sum::<u32>(),
            6
        );
        assert_eq!(snapshot.stall_count, 2);
        assert_eq!(snapshot.freeze_metrics.freeze_count, 2);
    }

    #[tokio::test]
    async fn benchmark_run_writes_requested_artifacts() {
        let Some(run_id) = env_string("MRD_BENCH_RUN_ID") else {
            return;
        };
        let repo_root = env_string("MRD_BENCH_ARTIFACT_ROOT").unwrap_or_else(|| ".".to_string());
        let profile =
            env_string("MRD_BENCH_PROFILE").unwrap_or_else(|| "transport-webrtc-baseline".into());
        let date = env_string("MRD_BENCH_DATE").unwrap_or_else(|| "manual".into());
        let transport = env_string("MRD_BENCH_TRANSPORT").unwrap_or_else(|| "webrtc".into());
        let encode_backend =
            env_string("MRD_BENCH_ENCODE_BACKEND").unwrap_or_else(|| "openh264".into());
        let decode_backend =
            env_string("MRD_BENCH_DECODE_BACKEND").unwrap_or_else(|| "h264_software".into());
        let capture_backend =
            env_string("MRD_BENCH_CAPTURE_BACKEND").unwrap_or_else(|| "dxgi".into());
        let renderer_backend =
            env_string("MRD_BENCH_RENDERER_BACKEND").unwrap_or_else(|| "d3d11".into());
        let width = env_u32("MRD_BENCH_WIDTH", 1280);
        let height = env_u32("MRD_BENCH_HEIGHT", 720);
        let fps = env_u32("MRD_BENCH_FPS", 30);
        let duration_secs = env_u64("MRD_BENCH_DURATION_SECS", 20);
        let session_id = SessionId(format!("session-{run_id}"));
        let manifest = BenchmarkManifest {
            run_id: run_id.clone(),
            scenario: env_string("MRD_BENCH_SCENARIO").unwrap_or_else(|| "quick.transport".into()),
            transport: transport.clone(),
            capture_backend,
            encode_backend: encode_backend.clone(),
            decode_backend: decode_backend.clone(),
            renderer_backend,
            width,
            height,
            fps,
            duration_secs,
            git_commit: env_string("MRD_BENCH_GIT_COMMIT").unwrap_or_else(|| "unknown".into()),
        };
        let paths = BenchmarkPaths::new(Path::new(&repo_root), date, profile, run_id);

        if transport == "quic_quinn" {
            let outcome = crate::quic_transport_harness::run_quic_benchmark_pipeline(
                session_id.clone(),
                width as usize,
                height as usize,
                fps,
                duration_secs,
                &encode_backend,
                &decode_backend,
            )
            .await
            .expect("run quic benchmark pipeline");
            let summary = BenchmarkSummary::from_transport_probes(
                &manifest,
                &outcome.sender_probe,
                &outcome.receiver_probe,
                true,
                outcome.sink_snapshot.frame_count > 0,
                outcome.first_frame_time_ms,
                0,
                Some(outcome.sink_snapshot.frame_count),
            );
            super::write_benchmark_artifacts(
                &paths,
                &manifest,
                &summary,
                &session_id.0,
                &outcome.receiver_probe,
            )
            .expect("write quic benchmark artifacts");
        } else {
            let (summary, probe) = run_harness_benchmark(&manifest, &session_id);
            super::write_benchmark_artifacts(&paths, &manifest, &summary, &session_id.0, &probe)
                .expect("write webrtc benchmark artifacts");
        }

        assert!(paths.manifest_json.exists());
        assert!(paths.summary_json.exists());
        assert!(paths.summary_csv.exists());
    }

    fn run_harness_benchmark(
        manifest: &BenchmarkManifest,
        session_id: &SessionId,
    ) -> (BenchmarkSummary, PipelineProbeSnapshot) {
        if let Some(reason) = unsupported_encoder_backend_reason(&manifest.encode_backend) {
            return unsupported_benchmark_result(manifest, session_id, reason);
        }

        let mut harness = TestHarness::new().expect("create benchmark harness");
        harness.set_chain(TestChain::Custom {
            capture: parse_capture_backend(&manifest.capture_backend),
            encoder: parse_encoder_backend(&manifest.encode_backend),
            decoder: parse_decoder_backend(&manifest.decode_backend),
        });
        harness.set_config(TestConfig {
            resolution: Some((manifest.width as usize, manifest.height as usize)),
            fps: Some(manifest.fps),
            bitrate: std::env::var("MRD_BENCH_BITRATE_BPS")
                .ok()
                .and_then(|value| value.parse::<u32>().ok()),
            color_mode: benchmark_color_mode_from_env(),
            color_pipeline: benchmark_color_pipeline_from_env(),
            source_id: env_string("MRD_BENCH_SOURCE_ID"),
            display_id: env_string("MRD_BENCH_DISPLAY_ID"),
            renderer: Some(parse_renderer_backend(&manifest.renderer_backend)),
            transport: Some(parse_transport_backend(&manifest.transport)),
            zero_copy: Some(benchmark_zero_copy_enabled(manifest)),
            pace_to_fps: Some(env_bool("MRD_BENCH_PACE_TO_FPS", false)),
            visual_preview: Some(false),
            ..Default::default()
        });

        let started = Instant::now();
        harness.start().expect("start benchmark harness");
        let first_frame_time_ms = wait_for_first_presented_frame(&harness, Duration::from_secs(8));
        let sample_deadline = Instant::now() + Duration::from_secs(manifest.duration_secs);
        let mut resource_monitor = crate::resource_monitor::ResourceMonitor::new();
        let mut resource_samples = Vec::new();
        while Instant::now() < sample_deadline {
            let snapshot = resource_monitor
                .snapshot_for_process(Some(std::process::id()), "Rdesk benchmark controller");
            if let Some(sample) =
                controller_resource_sample(&snapshot, started.elapsed().as_secs_f64() * 1_000.0)
            {
                if resource_samples.len() == MAX_EXPERIENCE_RESOURCE_SAMPLES {
                    resource_samples.remove(0);
                }
                resource_samples.push(sample);
            }
            let remaining = sample_deadline.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                thread::sleep(remaining.min(Duration::from_secs(1)));
            }
        }
        harness.stop_and_wait().expect("stop benchmark harness");
        let metrics = harness.get_metrics();
        let elapsed_secs = started.elapsed().as_secs_f64().max(0.001);
        let bitrate_kbps = (metrics.total_bitstream_bytes as f64 * 8.0) / elapsed_secs / 1000.0;
        let first_frame_seen = first_frame_time_ms.is_some();
        let probe = probe_from_metrics(manifest, session_id, &metrics, bitrate_kbps);
        let render_probe_complete = manifest.renderer_backend == "none"
            || (metrics.render_latency_p95_ms > 0.0 && metrics.render_present_gap_p95_ms > 0.0);
        let probe_complete = metrics.encoded_units > 0
            && metrics.decoded_frames > 0
            && metrics.encode_latency_p95_ms > 0.0
            && metrics.decode_latency_p95_ms > 0.0
            && render_probe_complete;
        let failure_reason = harness_failure_reason(
            &metrics,
            &manifest.renderer_backend,
            first_frame_time_ms,
            probe_complete,
        );
        let run_passed = first_frame_seen
            && probe_complete
            && metrics.encode_failures == 0
            && metrics.decode_failures == 0
            && failure_reason.is_none();

        let mut experience = experience_from_present_intervals(
            first_frame_time_ms,
            manifest.duration_secs,
            manifest.fps,
            &metrics.render_present_intervals_ms,
        );
        experience.resource_samples = resource_samples;
        let summary = BenchmarkSummary {
            run_id: manifest.run_id.clone(),
            scenario: manifest.scenario.clone(),
            transport: manifest.transport.clone(),
            capture_backend: manifest.capture_backend.clone(),
            encode_backend: manifest.encode_backend.clone(),
            decode_backend: manifest.decode_backend.clone(),
            renderer_backend: manifest.renderer_backend.clone(),
            width: manifest.width,
            height: manifest.height,
            fps_target: manifest.fps,
            duration_secs: manifest.duration_secs,
            session_established: metrics.error_message.is_none(),
            first_frame_seen,
            first_frame_time_ms,
            probe_complete,
            fps_observed: observed_fps_for_summary(manifest, &metrics),
            bitrate_kbps,
            target_bitrate_kbps: configured_target_bitrate_kbps(),
            encoded_fps: nonzero_option(metrics.encoded_fps),
            decoded_fps: nonzero_option(metrics.decoded_fps),
            zero_copy_enabled: Some(benchmark_zero_copy_enabled(manifest)),
            total_bitstream_bytes: Some(metrics.total_bitstream_bytes as u64),
            keyframes: 0,
            dropped_frames: metrics.dropped_frames as u64,
            quic_receiver_completed_frames: None,
            quic_receiver_expired_frames: None,
            quic_receiver_evicted_frames: None,
            quic_receiver_duplicate_fragments: None,
            quic_receiver_rejected_fragments: None,
            quic_receiver_pending_frames: None,
            quic_receiver_reassembly_drops: None,
            zero_write_access_unit_count: 0,
            warning_count: 0,
            error_count: u64::from(metrics.error_message.is_some()),
            restart_count: 0,
            encode_total_p95_ms: nonzero_option(metrics.encode_latency_p95_ms),
            send_write_p95_ms: nonzero_option(metrics.transport_latency_p95_ms),
            decode_total_p95_ms: nonzero_option(metrics.decode_latency_p95_ms),
            frame_sink_ingest_p95_ms: nonzero_option(metrics.interactive_latency_p95_ms),
            render_upload_p95_ms: nonzero_option(metrics.render_latency_p95_ms),
            render_submit_wait_p95_ms: nonzero_option(metrics.render_submit_wait_latency_p95_ms),
            render_execute_p95_ms: nonzero_option(metrics.render_execute_latency_p95_ms),
            render_prepare_wait_p95_ms: nonzero_option(metrics.render_prepare_wait_latency_p95_ms),
            render_shared_resource_p95_ms: nonzero_option(
                metrics.render_shared_resource_latency_p95_ms,
            ),
            render_draw_present_p95_ms: nonzero_option(metrics.render_draw_present_latency_p95_ms),
            render_present_p95_ms: nonzero_option(metrics.render_present_gap_p95_ms),
            render_submitted_frames: Some(metrics.render_submitted_frames),
            render_uploaded_frames: Some(metrics.render_uploaded_frames),
            render_presented_frames: Some(metrics.render_presented_frames),
            render_present_skipped_frames: Some(metrics.render_present_skipped_frames),
            render_queue_replacements: Some(metrics.render_queue_replacements),
            render_stale_frame_drops: Some(metrics.render_stale_frame_drops),
            swap_chain_max_frame_latency: metrics.swap_chain_max_frame_latency,
            swap_chain_allow_tearing: metrics.swap_chain_allow_tearing,
            swap_chain_waitable_object: metrics.swap_chain_waitable_object,
            swap_chain_present_mode: metrics.swap_chain_present_mode.clone(),
            display_refresh_hz: metrics.display_refresh_hz,
            render_thread_priority: metrics.render_thread_priority.clone(),
            render_pixel_format: metrics.render_pixel_format.clone(),
            color_mode: metrics
                .color_mode
                .clone()
                .or_else(|| Some(ColorMode::Full.as_str().to_string())),
            color_pipeline: metrics
                .color_pipeline
                .clone()
                .or_else(|| Some(ColorPipeline::Sdr8.as_str().to_string())),
            nvdec_shared_copy_attempts: nonzero_u64_option(metrics.nvdec_shared_copy_attempts),
            nvdec_shared_copy_successes: nonzero_u64_option(metrics.nvdec_shared_copy_successes),
            nvdec_shared_copy_failures: nonzero_u64_option(metrics.nvdec_shared_copy_failures),
            nvdec_shared_copy_last_stage: metrics.nvdec_shared_copy_last_stage.clone(),
            nvdec_shared_copy_last_api: metrics.nvdec_shared_copy_last_api.clone(),
            nvdec_shared_copy_last_error: metrics.nvdec_shared_copy_last_error.clone(),
            nvdec_runtime_summary: String::new(),
            nvdec_h264_capability: String::new(),
            nvdec_hevc_capability: String::new(),
            nvdec_hevc_main10_capability: String::new(),
            failure_reason,
            run_skipped: false,
            experience: Some(experience),
            run_passed,
        };

        (summary, probe)
    }

    fn harness_failure_reason(
        metrics: &crate::test_harness::HarnessMetrics,
        renderer_backend: &str,
        first_frame_time_ms: Option<f64>,
        probe_complete: bool,
    ) -> Option<String> {
        if let Some(message) = metrics.error_message.as_deref() {
            return Some(message.to_string());
        }
        if metrics.encoded_units == 0 {
            return Some("encoder produced no non-empty access units".to_string());
        }
        if metrics.decoded_frames == 0 {
            return Some("decoder produced no frames".to_string());
        }
        if first_frame_time_ms.is_none() {
            return Some("first successful present was not observed before timeout".to_string());
        }
        if !probe_complete {
            return Some("benchmark probe did not collect all required stage metrics".to_string());
        }
        if let Some(reason) = BenchmarkSummary::render_health_failure_reason(
            renderer_backend,
            BenchmarkSummary::nonzero_counter(Some(metrics.render_submitted_frames)),
            BenchmarkSummary::nonzero_counter(Some(metrics.render_uploaded_frames)),
            Some(metrics.render_presented_frames),
        ) {
            return Some(reason);
        }
        None
    }

    fn wait_for_first_presented_frame(harness: &TestHarness, timeout: Duration) -> Option<f64> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            let metrics = harness.get_metrics();
            if metrics.render_presented_frames > 0 {
                return Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn probe_from_metrics(
        manifest: &BenchmarkManifest,
        session_id: &SessionId,
        metrics: &crate::test_harness::HarnessMetrics,
        bitrate_kbps: f64,
    ) -> PipelineProbeSnapshot {
        PipelineProbeSnapshot::from_parts(
            session_id.clone(),
            "session-primary".into(),
            Some(manifest.capture_backend.clone()),
            Some(manifest.encode_backend.clone()),
            Some(manifest.transport.clone()),
            observed_fps_for_summary(manifest, metrics),
            bitrate_kbps,
            metrics.dropped_frames as u64,
            0,
            render_counters_from_metrics(metrics),
            vec![
                (
                    StageId::EncodeTotal,
                    stats_from_metrics(
                        metrics.encode_latency_avg_ms,
                        metrics.encode_latency_p50_ms,
                        metrics.encode_latency_p95_ms,
                        metrics.total_bitstream_bytes as u64,
                    ),
                ),
                (
                    StageId::SendWrite,
                    stats_from_metrics(
                        metrics.transport_latency_avg_ms,
                        metrics.transport_latency_p50_ms,
                        metrics.transport_latency_p95_ms,
                        metrics.total_bitstream_bytes as u64,
                    ),
                ),
                (
                    StageId::DecodeTotal,
                    stats_from_metrics(
                        metrics.decode_latency_avg_ms,
                        metrics.decode_latency_p50_ms,
                        metrics.decode_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::FrameSinkIngest,
                    stats_from_metrics(
                        metrics.interactive_latency_avg_ms,
                        metrics.interactive_latency_p50_ms,
                        metrics.interactive_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderUpload,
                    stats_from_metrics(
                        metrics.render_latency_avg_ms,
                        metrics.render_latency_p50_ms,
                        metrics.render_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderSubmitWait,
                    stats_from_metrics(
                        metrics.render_submit_wait_latency_avg_ms,
                        metrics.render_submit_wait_latency_p50_ms,
                        metrics.render_submit_wait_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderExecute,
                    stats_from_metrics(
                        metrics.render_execute_latency_avg_ms,
                        metrics.render_execute_latency_p50_ms,
                        metrics.render_execute_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderPrepareWait,
                    stats_from_metrics(
                        metrics.render_prepare_wait_latency_avg_ms,
                        metrics.render_prepare_wait_latency_p50_ms,
                        metrics.render_prepare_wait_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderSharedResource,
                    stats_from_metrics(
                        metrics.render_shared_resource_latency_avg_ms,
                        metrics.render_shared_resource_latency_p50_ms,
                        metrics.render_shared_resource_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderDrawPresent,
                    stats_from_metrics(
                        metrics.render_draw_present_latency_avg_ms,
                        metrics.render_draw_present_latency_p50_ms,
                        metrics.render_draw_present_latency_p95_ms,
                        0,
                    ),
                ),
                (
                    StageId::RenderPresent,
                    stats_from_metrics(
                        metrics.render_present_gap_avg_ms,
                        metrics.render_present_gap_p50_ms,
                        metrics.render_present_gap_p95_ms,
                        0,
                    ),
                ),
            ],
        )
    }

    fn observed_fps_for_summary(
        manifest: &BenchmarkManifest,
        metrics: &crate::test_harness::HarnessMetrics,
    ) -> f64 {
        if parse_decoder_backend(&manifest.decode_backend) != DecoderType::None
            && metrics.decoded_fps > 0.0
        {
            metrics.decoded_fps
        } else {
            metrics.capture_fps
        }
    }

    fn render_counters_from_metrics(
        metrics: &crate::test_harness::HarnessMetrics,
    ) -> Vec<(String, u64)> {
        vec![
            (
                "render_submitted_frames".to_string(),
                metrics.render_submitted_frames,
            ),
            (
                "render_uploaded_frames".to_string(),
                metrics.render_uploaded_frames,
            ),
            (
                "render_presented_frames".to_string(),
                metrics.render_presented_frames,
            ),
            (
                "render_present_skipped_frames".to_string(),
                metrics.render_present_skipped_frames,
            ),
            (
                "render_queue_replacements".to_string(),
                metrics.render_queue_replacements,
            ),
            (
                "render_stale_frame_drops".to_string(),
                metrics.render_stale_frame_drops,
            ),
            (
                "nvdec_shared_copy_attempts".to_string(),
                metrics.nvdec_shared_copy_attempts,
            ),
            (
                "nvdec_shared_copy_successes".to_string(),
                metrics.nvdec_shared_copy_successes,
            ),
            (
                "nvdec_shared_copy_failures".to_string(),
                metrics.nvdec_shared_copy_failures,
            ),
        ]
    }

    fn stats_from_metrics(avg: f64, p50: f64, p95: f64, bytes: u64) -> StageStatsSnapshot {
        let mut values = Vec::new();
        for value in [avg, p50, p95] {
            if value.is_finite() && value > 0.0 {
                values.push(value);
            }
        }
        if values.is_empty() {
            return StageStatsSnapshot::from_durations_ms(&[], bytes);
        }

        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        StageStatsSnapshot {
            count: values.len() as u64,
            bytes,
            avg_ms: nonzero_option(avg),
            p50_ms: nonzero_option(p50),
            p95_ms: nonzero_option(p95),
            p99_ms: nonzero_option(p95),
            max_ms: Some(max),
            jitter_ms: None,
        }
    }

    fn parse_capture_backend(value: &str) -> CaptureType {
        match value {
            "synthetic" => CaptureType::Synthetic,
            "winrt" => CaptureType::Winrt,
            #[cfg(target_os = "linux")]
            "linux" | "pipewire" => CaptureType::Linux,
            "macos" => CaptureType::Macos,
            _ => CaptureType::Dxgi,
        }
    }

    fn parse_encoder_backend(value: &str) -> EncoderType {
        match value {
            "none" => EncoderType::None,
            "openh264" | "openh264_speed" => EncoderType::OpenH264,
            "videotoolbox" | "videotoolbox_h264" => EncoderType::VideoToolboxH264,
            "videotoolbox_hevc" => EncoderType::VideoToolboxHevc,
            "nvenc_hevc" => EncoderType::NvencHevc,
            "nvenc_hevc_main10" => EncoderType::NvencHevcMain10,
            "nvenc_av1" => EncoderType::NvencAv1,
            "software_vvc" | "vvc_software" | "software_h266" | "h266_software"
            | "software-vvc" | "vvc-software" | "software-h266" | "h266-software" | "vvenc"
            | "vvc" | "h266" | "h.266" => EncoderType::SoftwareVvc,
            _ => EncoderType::NvencH264,
        }
    }

    fn parse_decoder_backend(value: &str) -> DecoderType {
        match value {
            "none" => DecoderType::None,
            "software"
            | "h264_software"
            | "openh264"
            | "software_h264"
            | "software_hevc"
            | "hevc_software"
            | "software_hevc_main10"
            | "hevc_main10_software"
            | "software_av1"
            | "av1_software"
            | "software_vvc"
            | "vvc_software"
            | "software_h266"
            | "h266_software" => DecoderType::Software,
            "ffmpeg_vvc" | "vvc_ffmpeg" | "ffmpeg_h266" | "h266_ffmpeg" => DecoderType::FfmpegVvc,
            #[cfg(target_os = "linux")]
            "linux_h264" => DecoderType::LinuxH264,
            #[cfg(target_os = "linux")]
            "linux_hevc" => DecoderType::LinuxHevc,
            #[cfg(target_os = "linux")]
            "linux_hevc_main10" => DecoderType::LinuxHevcMain10,
            "videotoolbox" => DecoderType::VideoToolbox,
            _ => DecoderType::Nvdec,
        }
    }

    fn parse_transport_backend(value: &str) -> TransportKind {
        match value {
            "quic" | "quic_datagram" | "quic-datagram" => TransportKind::QuicDatagram,
            "loopback" => TransportKind::Loopback,
            _ => TransportKind::WebrtcRtp,
        }
    }

    fn unsupported_encoder_backend_reason(value: &str) -> Option<String> {
        if matches!(value, "nvenc_av1" | "av1_nvenc" | "nvenc-av1") {
            #[cfg(any(windows, target_os = "linux"))]
            {
                if let Err(error) = NvencAv1Encoder::probe_av1_available() {
                    return Some(format!(
                        "NVENC AV1 benchmark skipped: current GPU/driver does not expose AV1 encode support ({error:?})"
                    ));
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                return Some(
                    "NVENC AV1 benchmark skipped: NVENC AV1 is unavailable on this platform"
                        .to_string(),
                );
            }
        }

        if matches!(
            value,
            "software_vvc"
                | "vvc_software"
                | "software_h266"
                | "h266_software"
                | "software-vvc"
                | "vvc-software"
                | "software-h266"
                | "h266-software"
                | "vvenc"
                | "vvc"
                | "h266"
                | "h.266"
        ) && !mrd_encode_vvenc::vvenc_software_compiled()
        {
            return Some(
                "H.266/VVC benchmark encode requires mrd-encode-vvenc feature software-vvenc and libvvenc >= 1.13.0".to_string(),
            );
        }

        None
    }

    fn unsupported_benchmark_result(
        manifest: &BenchmarkManifest,
        session_id: &SessionId,
        reason: String,
    ) -> (BenchmarkSummary, PipelineProbeSnapshot) {
        let probe = PipelineProbeSnapshot::from_parts(
            session_id.clone(),
            "benchmark".into(),
            Some(manifest.encode_backend.clone()),
            Some("vvc".into()),
            Some(manifest.transport.clone()),
            0.0,
            0.0,
            0,
            0,
            vec![],
            vec![],
        );
        let summary = BenchmarkSummary {
            run_id: manifest.run_id.clone(),
            scenario: manifest.scenario.clone(),
            transport: manifest.transport.clone(),
            capture_backend: manifest.capture_backend.clone(),
            encode_backend: manifest.encode_backend.clone(),
            decode_backend: manifest.decode_backend.clone(),
            renderer_backend: manifest.renderer_backend.clone(),
            width: manifest.width,
            height: manifest.height,
            fps_target: manifest.fps,
            duration_secs: manifest.duration_secs,
            session_established: false,
            first_frame_seen: false,
            first_frame_time_ms: None,
            probe_complete: false,
            fps_observed: 0.0,
            bitrate_kbps: 0.0,
            target_bitrate_kbps: configured_target_bitrate_kbps(),
            encoded_fps: None,
            decoded_fps: None,
            zero_copy_enabled: Some(benchmark_zero_copy_enabled(manifest)),
            total_bitstream_bytes: Some(0),
            keyframes: 0,
            dropped_frames: 0,
            quic_receiver_completed_frames: None,
            quic_receiver_expired_frames: None,
            quic_receiver_evicted_frames: None,
            quic_receiver_duplicate_fragments: None,
            quic_receiver_rejected_fragments: None,
            quic_receiver_pending_frames: None,
            quic_receiver_reassembly_drops: None,
            zero_write_access_unit_count: 0,
            warning_count: 0,
            error_count: 0,
            restart_count: 0,
            encode_total_p95_ms: None,
            send_write_p95_ms: None,
            decode_total_p95_ms: None,
            frame_sink_ingest_p95_ms: None,
            render_upload_p95_ms: None,
            render_submit_wait_p95_ms: None,
            render_execute_p95_ms: None,
            render_prepare_wait_p95_ms: None,
            render_shared_resource_p95_ms: None,
            render_draw_present_p95_ms: None,
            render_present_p95_ms: None,
            render_submitted_frames: None,
            render_uploaded_frames: None,
            render_presented_frames: None,
            render_present_skipped_frames: None,
            render_queue_replacements: None,
            render_stale_frame_drops: None,
            swap_chain_max_frame_latency: None,
            swap_chain_allow_tearing: None,
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            render_pixel_format: None,
            color_mode: Some(
                benchmark_color_mode_from_env()
                    .unwrap_or_default()
                    .as_str()
                    .to_string(),
            ),
            color_pipeline: Some(
                benchmark_color_pipeline_from_env()
                    .unwrap_or_default()
                    .as_str()
                    .to_string(),
            ),
            nvdec_shared_copy_attempts: None,
            nvdec_shared_copy_successes: None,
            nvdec_shared_copy_failures: None,
            nvdec_shared_copy_last_stage: None,
            nvdec_shared_copy_last_api: None,
            nvdec_shared_copy_last_error: None,
            nvdec_runtime_summary: String::new(),
            nvdec_h264_capability: String::new(),
            nvdec_hevc_capability: String::new(),
            nvdec_hevc_main10_capability: String::new(),
            failure_reason: Some(reason),
            run_skipped: true,
            experience: None,
            run_passed: false,
        };

        (summary, probe)
    }

    fn benchmark_zero_copy_enabled(manifest: &BenchmarkManifest) -> bool {
        matches!(
            manifest.decode_backend.as_str(),
            "nvdec" | "nvdec_av1" | "nvdec_hevc" | "nvdec_hevc_main10"
        ) && matches!(manifest.renderer_backend.as_str(), "d3d11" | "d3d11_shared")
            && matches!(manifest.capture_backend.as_str(), "dxgi" | "winrt")
    }

    fn parse_renderer_backend(value: &str) -> RendererType {
        match value {
            "macos" | "metal" => RendererType::Macos,
            #[cfg(target_os = "linux")]
            "linux" => RendererType::Linux,
            _ => RendererType::D3d11,
        }
    }

    fn nonzero_option(value: f64) -> Option<f64> {
        if value.is_finite() && value > 0.0 {
            Some(value)
        } else {
            None
        }
    }

    fn nonzero_u64_option(value: u64) -> Option<u64> {
        if value > 0 {
            Some(value)
        } else {
            None
        }
    }

    fn configured_target_bitrate_kbps() -> Option<f64> {
        std::env::var("MRD_BENCH_BITRATE_BPS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value / 1000.0)
    }

    fn env_string(key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }

    fn env_u32(key: &str, default: u32) -> u32 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    fn env_u64(key: &str, default: u64) -> u64 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    fn env_bool(key: &str, default: bool) -> bool {
        std::env::var(key)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(default)
    }

    fn benchmark_color_mode_from_env() -> Option<ColorMode> {
        env_string("MRD_BENCH_COLOR_MODE").and_then(|value| parse_color_mode(&value))
    }

    fn benchmark_color_pipeline_from_env() -> Option<ColorPipeline> {
        env_string("MRD_BENCH_COLOR_PIPELINE").and_then(|value| parse_color_pipeline(&value))
    }

    fn parse_color_mode(value: &str) -> Option<ColorMode> {
        match value.trim().to_ascii_lowercase().as_str() {
            "full" | "color" | "colour" => Some(ColorMode::Full),
            "grayscale" | "greyscale" | "gray" | "grey" => Some(ColorMode::Grayscale),
            "monochrome" | "mono" | "black_white" | "black-white" | "bw" => {
                Some(ColorMode::Monochrome)
            }
            "low_chroma" | "low-chroma" | "lowchroma" | "reduced_chroma" | "reduced-chroma" => {
                Some(ColorMode::LowChroma)
            }
            _ => None,
        }
    }

    fn parse_color_pipeline(value: &str) -> Option<ColorPipeline> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sdr8" | "sdr_8" | "sdr-8" | "8bit" | "8-bit" => Some(ColorPipeline::Sdr8),
            "hdr_main10" | "hdr-main10" | "main10" | "hevc_main10" | "hevc-main10" => {
                Some(ColorPipeline::HdrMain10)
            }
            _ => None,
        }
    }

    #[test]
    fn benchmark_color_mode_env_parses_requested_mode() {
        std::env::set_var("MRD_BENCH_COLOR_MODE", "monochrome");
        std::env::set_var("MRD_BENCH_COLOR_PIPELINE", "hdr_main10");

        assert_eq!(benchmark_color_mode_from_env(), Some(ColorMode::Monochrome));
        assert_eq!(
            benchmark_color_pipeline_from_env(),
            Some(ColorPipeline::HdrMain10)
        );

        std::env::remove_var("MRD_BENCH_COLOR_MODE");
        std::env::remove_var("MRD_BENCH_COLOR_PIPELINE");
    }

    #[test]
    fn harness_summary_prefers_decoded_fps_when_decode_backend_is_active() {
        let manifest = BenchmarkManifest {
            run_id: "quick-webrtc-20260529-fps".into(),
            scenario: "quick.transport".into(),
            transport: "webrtc".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "nvenc".into(),
            decode_backend: "nvdec".into(),
            renderer_backend: "d3d11".into(),
            width: 2560,
            height: 1440,
            fps: 144,
            duration_secs: 10,
            git_commit: "abc123".into(),
        };
        let metrics = crate::test_harness::HarnessMetrics {
            capture_fps: 144.0,
            decoded_fps: 118.0,
            ..crate::test_harness::HarnessMetrics::default()
        };

        assert_eq!(observed_fps_for_summary(&manifest, &metrics), 118.0);
    }

    #[test]
    fn harness_probe_exports_render_upload_and_present_gap_p95() {
        let manifest = BenchmarkManifest {
            run_id: "quick-webrtc-20260529-render".into(),
            scenario: "quick.transport".into(),
            transport: "webrtc".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "nvenc".into(),
            decode_backend: "nvdec".into(),
            renderer_backend: "d3d11".into(),
            width: 2560,
            height: 1440,
            fps: 144,
            duration_secs: 10,
            git_commit: "abc123".into(),
        };
        let metrics = crate::test_harness::HarnessMetrics {
            capture_fps: 144.0,
            decoded_fps: 118.0,
            render_latency_avg_ms: 0.20,
            render_latency_p50_ms: 0.18,
            render_latency_p95_ms: 0.35,
            render_submit_wait_latency_avg_ms: 0.03,
            render_submit_wait_latency_p50_ms: 0.02,
            render_submit_wait_latency_p95_ms: 0.06,
            render_execute_latency_avg_ms: 0.17,
            render_execute_latency_p50_ms: 0.16,
            render_execute_latency_p95_ms: 0.29,
            render_prepare_wait_latency_avg_ms: 0.01,
            render_prepare_wait_latency_p50_ms: 0.01,
            render_prepare_wait_latency_p95_ms: 0.02,
            render_shared_resource_latency_avg_ms: 0.08,
            render_shared_resource_latency_p50_ms: 0.07,
            render_shared_resource_latency_p95_ms: 0.11,
            render_draw_present_latency_avg_ms: 0.08,
            render_draw_present_latency_p50_ms: 0.08,
            render_draw_present_latency_p95_ms: 0.16,
            render_present_gap_avg_ms: 6.94,
            render_present_gap_p50_ms: 6.90,
            render_present_gap_p95_ms: 7.40,
            render_submitted_frames: 1_440,
            render_uploaded_frames: 1_439,
            render_presented_frames: 1_438,
            render_present_skipped_frames: 2,
            ..crate::test_harness::HarnessMetrics::default()
        };

        let probe = probe_from_metrics(
            &manifest,
            &SessionId("session-render".into()),
            &metrics,
            0.0,
        );
        let render_upload = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderUpload)
            .map(|(_, stats)| stats)
            .expect("render upload stage");
        let render_present = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderPresent)
            .map(|(_, stats)| stats)
            .expect("render present stage");
        let render_submit_wait = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderSubmitWait)
            .map(|(_, stats)| stats)
            .expect("render submit wait stage");
        let render_execute = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderExecute)
            .map(|(_, stats)| stats)
            .expect("render execute stage");
        let render_prepare_wait = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderPrepareWait)
            .map(|(_, stats)| stats)
            .expect("render prepare wait stage");
        let render_shared_resource = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderSharedResource)
            .map(|(_, stats)| stats)
            .expect("render shared resource stage");
        let render_draw_present = probe
            .stages
            .iter()
            .find(|(stage, _)| *stage == StageId::RenderDrawPresent)
            .map(|(_, stats)| stats)
            .expect("render draw present stage");

        assert_eq!(probe.fps, 118.0);
        assert_eq!(render_upload.p50_ms, Some(0.18));
        assert_eq!(render_upload.p95_ms, Some(0.35));
        assert_eq!(render_submit_wait.p50_ms, Some(0.02));
        assert_eq!(render_submit_wait.p95_ms, Some(0.06));
        assert_eq!(render_execute.p50_ms, Some(0.16));
        assert_eq!(render_execute.p95_ms, Some(0.29));
        assert_eq!(render_prepare_wait.p50_ms, Some(0.01));
        assert_eq!(render_prepare_wait.p95_ms, Some(0.02));
        assert_eq!(render_shared_resource.p50_ms, Some(0.07));
        assert_eq!(render_shared_resource.p95_ms, Some(0.11));
        assert_eq!(render_draw_present.p50_ms, Some(0.08));
        assert_eq!(render_draw_present.p95_ms, Some(0.16));
        assert_eq!(render_present.p50_ms, Some(6.90));
        assert_eq!(render_present.p95_ms, Some(7.40));
        assert!(probe
            .counters
            .iter()
            .any(|(name, value)| name == "render_presented_frames" && *value == 1_438));
        assert!(probe
            .counters
            .iter()
            .any(|(name, value)| name == "render_present_skipped_frames" && *value == 2));
    }

    #[test]
    fn benchmark_summary_extracts_key_metrics_from_probe() {
        let probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-bench".into()),
            "session-primary".into(),
            Some("dxgi".into()),
            Some("h264".into()),
            Some("webrtc".into()),
            29.7,
            1812.5,
            0,
            3,
            vec![
                ("render_submitted_frames".into(), 1_440),
                ("render_uploaded_frames".into(), 1_438),
                ("render_presented_frames".into(), 1_437),
                ("render_present_skipped_frames".into(), 1),
                ("render_queue_replacements".into(), 2),
                ("render_stale_frame_drops".into(), 2),
            ],
            vec![
                (
                    StageId::EncodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0, 6.0], 3000),
                ),
                (
                    StageId::SendWrite,
                    StageStatsSnapshot::from_durations_ms(&[1.0, 2.0, 4.0], 1500),
                ),
                (
                    StageId::DecodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[3.0, 4.0, 5.0], 1200),
                ),
                (
                    StageId::FrameSinkIngest,
                    StageStatsSnapshot::from_durations_ms(&[1.0, 1.2, 1.5], 600),
                ),
            ],
        );

        let summary = BenchmarkSummary::from_probe(
            &BenchmarkManifest {
                run_id: "quick-webrtc-20260308-abc123".into(),
                scenario: "quick.transport".into(),
                transport: "webrtc".into(),
                capture_backend: "dxgi".into(),
                encode_backend: "openh264".into(),
                decode_backend: "h264_software".into(),
                renderer_backend: "d3d11".into(),
                width: 1280,
                height: 720,
                fps: 30,
                duration_secs: 20,
                git_commit: "abc123".into(),
            },
            &probe,
            true,
            true,
            180.0,
        );

        assert_eq!(summary.transport, "webrtc");
        assert!(summary.run_passed);
        assert_eq!(summary.first_frame_time_ms, Some(180.0));
        assert_eq!(summary.encode_total_p95_ms, Some(6.0));
        assert_eq!(summary.send_write_p95_ms, Some(4.0));
        assert_eq!(summary.decode_total_p95_ms, Some(5.0));
        assert_eq!(summary.frame_sink_ingest_p95_ms, Some(1.5));
        assert_eq!(summary.render_upload_p95_ms, None);
        assert_eq!(summary.render_submitted_frames, Some(1_440));
        assert_eq!(summary.render_uploaded_frames, Some(1_438));
        assert_eq!(summary.render_presented_frames, Some(1_437));
        assert_eq!(summary.render_present_skipped_frames, Some(1));
        assert_eq!(summary.render_queue_replacements, Some(2));
        assert_eq!(summary.render_stale_frame_drops, Some(2));
        assert_eq!(summary.quic_receiver_completed_frames, None);
        assert!(!summary.nvdec_runtime_summary.is_empty());
        assert!(!summary.nvdec_h264_capability.is_empty());
        assert!(!summary.nvdec_hevc_capability.is_empty());
        assert!(!summary.nvdec_hevc_main10_capability.is_empty());
    }

    #[test]
    fn benchmark_summary_fails_when_renderer_present_collapses() {
        let probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-present-collapse".into()),
            "session-primary".into(),
            Some("dxgi".into()),
            Some("av1".into()),
            Some("webrtc".into()),
            134.0,
            80_000.0,
            0,
            0,
            vec![
                ("render_submitted_frames".into(), 1_575),
                ("render_uploaded_frames".into(), 1_575),
                ("render_presented_frames".into(), 2),
                ("render_present_skipped_frames".into(), 1_093),
                ("render_queue_replacements".into(), 1_573),
                ("render_stale_frame_drops".into(), 1_573),
            ],
            vec![
                (
                    StageId::EncodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0], 3000),
                ),
                (
                    StageId::SendWrite,
                    StageStatsSnapshot::from_durations_ms(&[0.1, 0.2], 3000),
                ),
                (
                    StageId::DecodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0], 3000),
                ),
                (
                    StageId::FrameSinkIngest,
                    StageStatsSnapshot::from_durations_ms(&[0.2, 0.4], 3000),
                ),
                (
                    StageId::RenderUpload,
                    StageStatsSnapshot::from_durations_ms(&[1.0, 20.0], 3000),
                ),
                (
                    StageId::RenderPresent,
                    StageStatsSnapshot::from_durations_ms(&[6.0, 8.0], 3000),
                ),
            ],
        );

        let summary = BenchmarkSummary::from_probe(
            &BenchmarkManifest {
                run_id: "quick-webrtc-20260531-av1-collapse".into(),
                scenario: "quick.transport".into(),
                transport: "webrtc".into(),
                capture_backend: "dxgi".into(),
                encode_backend: "nvenc_av1".into(),
                decode_backend: "nvdec_av1".into(),
                renderer_backend: "d3d11".into(),
                width: 2560,
                height: 1440,
                fps: 144,
                duration_secs: 20,
                git_commit: "abc123".into(),
            },
            &probe,
            true,
            true,
            386.0,
        );

        assert!(!summary.run_passed);
        assert!(summary
            .failure_reason
            .as_deref()
            .expect("failure reason")
            .contains("render present collapse"));
    }

    #[test]
    fn benchmark_summary_fails_when_renderer_upload_starves() {
        let probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-upload-starvation".into()),
            "session-primary".into(),
            Some("dxgi".into()),
            Some("av1".into()),
            Some("webrtc".into()),
            122.0,
            80_000.0,
            0,
            0,
            vec![
                ("render_submitted_frames".into(), 2_430),
                ("render_uploaded_frames".into(), 25),
                ("render_presented_frames".into(), 25),
                ("render_present_skipped_frames".into(), 590),
                ("render_queue_replacements".into(), 1_813),
                ("render_stale_frame_drops".into(), 1_813),
            ],
            vec![
                (
                    StageId::EncodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0], 3000),
                ),
                (
                    StageId::SendWrite,
                    StageStatsSnapshot::from_durations_ms(&[0.1, 0.2], 3000),
                ),
                (
                    StageId::DecodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0], 3000),
                ),
                (
                    StageId::FrameSinkIngest,
                    StageStatsSnapshot::from_durations_ms(&[0.2, 0.4], 3000),
                ),
                (
                    StageId::RenderUpload,
                    StageStatsSnapshot::from_durations_ms(&[1.0, 20.0], 3000),
                ),
                (
                    StageId::RenderPresent,
                    StageStatsSnapshot::from_durations_ms(&[6.0, 39.0], 3000),
                ),
            ],
        );

        let summary = BenchmarkSummary::from_probe(
            &BenchmarkManifest {
                run_id: "quick-webrtc-20260531-av1-upload-starvation".into(),
                scenario: "quick.transport".into(),
                transport: "webrtc".into(),
                capture_backend: "dxgi".into(),
                encode_backend: "nvenc_av1".into(),
                decode_backend: "nvdec_av1".into(),
                renderer_backend: "d3d11".into(),
                width: 2560,
                height: 1440,
                fps: 144,
                duration_secs: 20,
                git_commit: "abc123".into(),
            },
            &probe,
            true,
            true,
            384.0,
        );

        assert!(!summary.run_passed);
        assert!(summary
            .failure_reason
            .as_deref()
            .expect("failure reason")
            .contains("render upload starvation"));
    }

    #[test]
    fn transport_summary_uses_decoded_frame_count_for_observed_fps() {
        let manifest = BenchmarkManifest {
            run_id: "quick-quic-20260308-abc123".into(),
            scenario: "quick.transport".into(),
            transport: "quic_quinn".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "nvenc".into(),
            decode_backend: "nvdec".into(),
            renderer_backend: "d3d11".into(),
            width: 1920,
            height: 1080,
            fps: 60,
            duration_secs: 10,
            git_commit: "abc123".into(),
        };
        let sender_probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-bench".into()),
            "session-primary-sender".into(),
            Some("synthetic+nvenc".into()),
            Some("h264".into()),
            Some("quic_quinn".into()),
            10.0,
            1000.0,
            0,
            1,
            vec![],
            vec![
                (
                    StageId::EncodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[2.0, 4.0], 3000),
                ),
                (
                    StageId::SendWrite,
                    StageStatsSnapshot::from_durations_ms(&[0.1, 0.2], 3000),
                ),
            ],
        );
        let receiver_probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-bench".into()),
            "session-primary".into(),
            Some("nvdec".into()),
            Some("h264".into()),
            Some("quic_quinn".into()),
            10.0,
            1000.0,
            0,
            1,
            vec![],
            vec![
                (
                    StageId::DecodeTotal,
                    StageStatsSnapshot::from_durations_ms(&[1.0, 2.0], 3000),
                ),
                (
                    StageId::FrameSinkIngest,
                    StageStatsSnapshot::from_durations_ms(&[0.1, 0.2], 3000),
                ),
            ],
        );

        let summary = BenchmarkSummary::from_transport_probes(
            &manifest,
            &sender_probe,
            &receiver_probe,
            true,
            true,
            50.0,
            0,
            Some(327),
        );

        assert_eq!(summary.fps_observed, 32.7);
    }

    #[test]
    fn benchmark_transport_parser_maps_quic_to_ui_datagram_harness() {
        assert_eq!(parse_transport_backend("quic"), TransportKind::QuicDatagram);
        assert_eq!(
            parse_transport_backend("quic_datagram"),
            TransportKind::QuicDatagram
        );
        assert_eq!(parse_transport_backend("webrtc"), TransportKind::WebrtcRtp);
    }

    #[test]
    fn benchmark_vvc_encoder_backend_parses_to_software_vvc() {
        assert_eq!(
            parse_encoder_backend("software_vvc"),
            EncoderType::SoftwareVvc
        );
        assert_eq!(
            parse_encoder_backend("vvc_software"),
            EncoderType::SoftwareVvc
        );
        assert_eq!(
            parse_encoder_backend("software_h266"),
            EncoderType::SoftwareVvc
        );
        assert_eq!(parse_encoder_backend("vvenc"), EncoderType::SoftwareVvc);
    }

    #[cfg(not(feature = "production-vvc-software-codec"))]
    #[test]
    fn benchmark_h266_encoder_backend_is_capability_gated() {
        let manifest = BenchmarkManifest {
            run_id: "quick-webrtc-20260308-vvc".into(),
            scenario: "quick.transport".into(),
            transport: "webrtc".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "software_vvc".into(),
            decode_backend: "software_vvc".into(),
            renderer_backend: "d3d11".into(),
            width: 2560,
            height: 1440,
            fps: 144,
            duration_secs: 20,
            git_commit: "abc123".into(),
        };
        let session_id = SessionId("session-vvc".into());

        let (summary, probe) = run_harness_benchmark(&manifest, &session_id);

        assert!(!summary.run_passed);
        assert!(summary.run_skipped);
        assert_eq!(summary.fps_observed, 0.0);
        assert_eq!(summary.error_count, 0);
        assert_eq!(summary.encode_backend, "software_vvc");
        assert_eq!(summary.decode_backend, "software_vvc");
        assert!(summary
            .failure_reason
            .as_deref()
            .expect("failure reason")
            .contains("mrd-encode-vvenc feature software-vvenc"));
        assert_eq!(probe.codec.as_deref(), Some("vvc"));
    }

    #[cfg(windows)]
    #[test]
    fn benchmark_nvenc_av1_capability_probe_does_not_reject_supported_hardware() {
        if NvencAv1Encoder::probe_av1_available().is_ok() {
            assert_eq!(unsupported_encoder_backend_reason("nvenc_av1"), None);
        }
    }

    #[test]
    fn benchmark_enables_zero_copy_for_nvdec_d3d11_runs() {
        let manifest = BenchmarkManifest {
            run_id: "quick-quic-20260308-abc123".into(),
            scenario: "quick.transport".into(),
            transport: "quic".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "nvenc".into(),
            decode_backend: "nvdec".into(),
            renderer_backend: "d3d11".into(),
            width: 1920,
            height: 1080,
            fps: 60,
            duration_secs: 10,
            git_commit: "abc123".into(),
        };

        assert!(benchmark_zero_copy_enabled(&manifest));
    }

    #[test]
    fn benchmark_paths_place_outputs_under_artifacts_tree() {
        let root = PathBuf::from(r"G:\Project\mini-remote-desktop");
        let paths = BenchmarkPaths::new(
            &root,
            "2026-03-08".into(),
            "transport-webrtc-baseline".into(),
            "quick-webrtc-20260308-abc123".into(),
        );

        assert!(paths.run_dir.ends_with(r"artifacts\benchmarks\2026-03-08\transport-webrtc-baseline\quick-webrtc-20260308-abc123"));
        assert!(paths.summary_json.ends_with(r"summary.json"));
        assert!(paths.summary_csv.ends_with(r"summary.csv"));
        assert!(paths.report_md.ends_with(r"reports\markdown-report.md"));
        assert!(paths.host_stdout.ends_with(r"logs\host.stdout.log"));
        assert!(paths
            .probe_json("session-bench")
            .ends_with(r"sessions\session-bench.probe.json"));
    }

    #[test]
    fn benchmark_summary_csv_row_uses_stable_columns() {
        let summary = BenchmarkSummary {
            run_id: "quick-webrtc-20260308-abc123".into(),
            scenario: "quick.transport".into(),
            transport: "webrtc".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "openh264".into(),
            decode_backend: "h264_software".into(),
            renderer_backend: "d3d11".into(),
            width: 1280,
            height: 720,
            fps_target: 30,
            duration_secs: 20,
            session_established: true,
            first_frame_seen: true,
            first_frame_time_ms: Some(10.0),
            probe_complete: false,
            fps_observed: 29.5,
            bitrate_kbps: 1400.0,
            target_bitrate_kbps: Some(8000.0),
            encoded_fps: Some(30.0),
            decoded_fps: Some(29.5),
            zero_copy_enabled: Some(false),
            total_bitstream_bytes: Some(3_500_000),
            keyframes: 1,
            dropped_frames: 0,
            quic_receiver_completed_frames: None,
            quic_receiver_expired_frames: None,
            quic_receiver_evicted_frames: None,
            quic_receiver_duplicate_fragments: None,
            quic_receiver_rejected_fragments: None,
            quic_receiver_pending_frames: None,
            quic_receiver_reassembly_drops: None,
            zero_write_access_unit_count: 0,
            warning_count: 0,
            error_count: 0,
            restart_count: 0,
            encode_total_p95_ms: Some(1.0),
            send_write_p95_ms: None,
            decode_total_p95_ms: None,
            frame_sink_ingest_p95_ms: None,
            render_upload_p95_ms: None,
            render_submit_wait_p95_ms: None,
            render_execute_p95_ms: None,
            render_prepare_wait_p95_ms: None,
            render_shared_resource_p95_ms: None,
            render_draw_present_p95_ms: None,
            render_present_p95_ms: None,
            render_submitted_frames: Some(10),
            render_uploaded_frames: Some(9),
            render_presented_frames: Some(8),
            render_present_skipped_frames: Some(1),
            render_queue_replacements: Some(2),
            render_stale_frame_drops: Some(2),
            swap_chain_max_frame_latency: Some(1),
            swap_chain_allow_tearing: Some(true),
            swap_chain_waitable_object: Some(true),
            swap_chain_present_mode: Some("waitable".into()),
            display_refresh_hz: Some(144),
            render_thread_priority: Some("above_normal".into()),
            render_pixel_format: Some("D3D11SharedP010".into()),
            color_mode: Some("grayscale".into()),
            color_pipeline: Some("sdr8".into()),
            nvdec_shared_copy_attempts: Some(100),
            nvdec_shared_copy_successes: Some(98),
            nvdec_shared_copy_failures: Some(2),
            nvdec_shared_copy_last_stage: Some("copy".into()),
            nvdec_shared_copy_last_api: Some("cuMemcpy2D_v2:UV".into()),
            nvdec_shared_copy_last_error: Some("CUDA_ERROR_UNKNOWN".into()),
            nvdec_runtime_summary: "nvdec runtime libraries and core exports are present".into(),
            nvdec_h264_capability: "runtime=true wired=true".into(),
            nvdec_hevc_capability: "runtime=true wired=false".into(),
            nvdec_hevc_main10_capability: "runtime=false wired=false".into(),
            failure_reason: Some("encode produced no HEVC Main10 access units".into()),
            run_skipped: false,
            experience: None,
            run_passed: false,
        };

        let header = BenchmarkSummary::csv_header();
        let row = summary.csv_row();

        assert_eq!(header.len(), row.len());
        assert_eq!(header[0], "run_id");
        assert_eq!(row[0], "quick-webrtc-20260308-abc123");
        assert_eq!(row[1], "quick.transport");
        assert_eq!(row[2], "webrtc");
        assert!(header.contains(&"quic_receiver_completed_frames"));
        assert!(header.contains(&"render_queue_replacements"));
        assert!(header.contains(&"render_prepare_wait_p95_ms"));
        assert!(header.contains(&"render_shared_resource_p95_ms"));
        assert!(header.contains(&"render_draw_present_p95_ms"));
        assert!(header.contains(&"target_bitrate_kbps"));
        assert!(header.contains(&"encoded_fps"));
        assert!(header.contains(&"decoded_fps"));
        assert!(header.contains(&"zero_copy_enabled"));
        assert!(header.contains(&"total_bitstream_bytes"));
        assert!(header.contains(&"swap_chain_max_frame_latency"));
        assert!(header.contains(&"swap_chain_allow_tearing"));
        assert!(header.contains(&"render_pixel_format"));
        assert!(header.contains(&"color_mode"));
        assert!(header.contains(&"color_pipeline"));
        assert!(header.contains(&"nvdec_shared_copy_attempts"));
        assert!(header.contains(&"nvdec_shared_copy_successes"));
        assert!(header.contains(&"nvdec_shared_copy_failures"));
        assert!(header.contains(&"nvdec_shared_copy_last_stage"));
        assert!(header.contains(&"nvdec_shared_copy_last_api"));
        assert!(header.contains(&"nvdec_shared_copy_last_error"));
        assert!(header.contains(&"nvdec_hevc_main10_capability"));
        let target_bitrate_index = header
            .iter()
            .position(|column| *column == "target_bitrate_kbps")
            .expect("target bitrate column");
        assert_eq!(row[target_bitrate_index], "8000");
        let encoded_fps_index = header
            .iter()
            .position(|column| *column == "encoded_fps")
            .expect("encoded fps column");
        assert_eq!(row[encoded_fps_index], "30");
        let zero_copy_index = header
            .iter()
            .position(|column| *column == "zero_copy_enabled")
            .expect("zero copy column");
        assert_eq!(row[zero_copy_index], "false");
        let bitstream_index = header
            .iter()
            .position(|column| *column == "total_bitstream_bytes")
            .expect("total bitstream bytes column");
        assert_eq!(row[bitstream_index], "3500000");
        let render_replacements_index = header
            .iter()
            .position(|column| *column == "render_queue_replacements")
            .expect("render replacement column");
        assert_eq!(row[render_replacements_index], "2");
        let present_mode_index = header
            .iter()
            .position(|column| *column == "swap_chain_present_mode")
            .expect("swapchain present mode column");
        assert_eq!(row[present_mode_index], "waitable");
        let frame_latency_index = header
            .iter()
            .position(|column| *column == "swap_chain_max_frame_latency")
            .expect("swapchain frame latency column");
        assert_eq!(row[frame_latency_index], "1");
        let allow_tearing_index = header
            .iter()
            .position(|column| *column == "swap_chain_allow_tearing")
            .expect("swapchain tearing column");
        assert_eq!(row[allow_tearing_index], "true");
        let refresh_index = header
            .iter()
            .position(|column| *column == "display_refresh_hz")
            .expect("display refresh column");
        assert_eq!(row[refresh_index], "144");
        let render_pixel_format_index = header
            .iter()
            .position(|column| *column == "render_pixel_format")
            .expect("render pixel format column");
        assert_eq!(row[render_pixel_format_index], "D3D11SharedP010");
        let color_mode_index = header
            .iter()
            .position(|column| *column == "color_mode")
            .expect("color mode column");
        assert_eq!(row[color_mode_index], "grayscale");
        let color_pipeline_index = header
            .iter()
            .position(|column| *column == "color_pipeline")
            .expect("color pipeline column");
        assert_eq!(row[color_pipeline_index], "sdr8");
        let shared_copy_attempts_index = header
            .iter()
            .position(|column| *column == "nvdec_shared_copy_attempts")
            .expect("shared copy attempts column");
        assert_eq!(row[shared_copy_attempts_index], "100");
        let shared_copy_last_error_index = header
            .iter()
            .position(|column| *column == "nvdec_shared_copy_last_error")
            .expect("shared copy last error column");
        assert_eq!(row[shared_copy_last_error_index], "CUDA_ERROR_UNKNOWN");
        let failure_reason_index = header
            .iter()
            .position(|column| *column == "failure_reason")
            .expect("failure_reason column");
        assert_eq!(
            row[failure_reason_index],
            "encode produced no HEVC Main10 access units"
        );
    }

    #[test]
    fn writing_benchmark_artifacts_creates_expected_files() {
        let temp_root =
            std::env::temp_dir().join(format!("mrd-bench-artifacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_root);
        let paths = BenchmarkPaths::new(
            &temp_root,
            "2026-03-08".into(),
            "transport-webrtc-baseline".into(),
            "quick-webrtc-20260308-abc123".into(),
        );
        let manifest = BenchmarkManifest {
            run_id: "quick-webrtc-20260308-abc123".into(),
            scenario: "quick.transport".into(),
            transport: "webrtc".into(),
            capture_backend: "dxgi".into(),
            encode_backend: "openh264".into(),
            decode_backend: "h264_software".into(),
            renderer_backend: "d3d11".into(),
            width: 1280,
            height: 720,
            fps: 30,
            duration_secs: 20,
            git_commit: "abc123".into(),
        };
        let probe = PipelineProbeSnapshot::from_parts(
            SessionId("session-bench".into()),
            "session-primary".into(),
            Some("dxgi".into()),
            Some("h264".into()),
            Some("webrtc".into()),
            29.7,
            1812.5,
            0,
            3,
            vec![],
            vec![(
                StageId::EncodeTotal,
                StageStatsSnapshot::from_durations_ms(&[2.0, 4.0, 6.0], 3000),
            )],
        );
        let summary = BenchmarkSummary::from_probe(&manifest, &probe, true, true, 180.0);

        super::write_benchmark_artifacts(&paths, &manifest, &summary, "session-bench", &probe)
            .expect("write benchmark artifacts");

        assert!(paths.manifest_json.exists());
        assert!(paths.summary_json.exists());
        assert!(paths.summary_csv.exists());
        assert!(paths.report_md.exists());
        assert!(paths.probe_json("session-bench").exists());
        let report = std::fs::read_to_string(&paths.report_md).expect("read benchmark report");
        assert!(report.contains("nvdec_hevc_main10"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
