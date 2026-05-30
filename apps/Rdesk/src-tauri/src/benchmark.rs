use std::{
    fs,
    path::{Path, PathBuf},
};

use mrd_decode_nvdec::{probe_runtime as probe_nvdec_runtime, NvdecCapabilityProbe};
use mrd_observability::{PipelineProbeSnapshot, StageId};
use serde::{Deserialize, Serialize};

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
    pub swap_chain_waitable_object: Option<bool>,
    #[serde(default)]
    pub swap_chain_present_mode: Option<String>,
    #[serde(default)]
    pub display_refresh_hz: Option<u32>,
    #[serde(default)]
    pub render_thread_priority: Option<String>,
    pub nvdec_runtime_summary: String,
    pub nvdec_h264_capability: String,
    pub nvdec_hevc_capability: String,
    pub nvdec_hevc_main10_capability: String,
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub run_skipped: bool,
    pub run_passed: bool,
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
            render_present_p95_ms: stage_p95(StageId::RenderPresent),
            render_submitted_frames: Self::counter(probe, "render_submitted_frames"),
            render_uploaded_frames: Self::counter(probe, "render_uploaded_frames"),
            render_presented_frames: Self::counter(probe, "render_presented_frames"),
            render_present_skipped_frames: Self::counter(probe, "render_present_skipped_frames"),
            render_queue_replacements: Self::counter(probe, "render_queue_replacements"),
            render_stale_frame_drops: Self::counter(probe, "render_stale_frame_drops"),
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
            failure_reason: None,
            run_skipped: false,
            run_passed: session_established && first_frame_seen && probe_complete,
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
            render_present_p95_ms: find_stage(receiver_probe, StageId::RenderPresent),
            render_submitted_frames: Self::counter(receiver_probe, "render_submitted_frames"),
            render_uploaded_frames: Self::counter(receiver_probe, "render_uploaded_frames"),
            render_presented_frames: Self::counter(receiver_probe, "render_presented_frames"),
            render_present_skipped_frames: Self::counter(
                receiver_probe,
                "render_present_skipped_frames",
            ),
            render_queue_replacements: Self::counter(receiver_probe, "render_queue_replacements"),
            render_stale_frame_drops: Self::counter(receiver_probe, "render_stale_frame_drops"),
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
            failure_reason: None,
            run_skipped: false,
            run_passed: session_established && first_frame_seen && probe_complete,
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
            "render_present_p95_ms",
            "render_submitted_frames",
            "render_uploaded_frames",
            "render_presented_frames",
            "render_present_skipped_frames",
            "render_queue_replacements",
            "render_stale_frame_drops",
            "swap_chain_waitable_object",
            "swap_chain_present_mode",
            "display_refresh_hz",
            "render_thread_priority",
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
            option_f64(self.render_present_p95_ms),
            option_u64(self.render_submitted_frames),
            option_u64(self.render_uploaded_frames),
            option_u64(self.render_presented_frames),
            option_u64(self.render_present_skipped_frames),
            option_u64(self.render_queue_replacements),
            option_u64(self.render_stale_frame_drops),
            option_bool(self.swap_chain_waitable_object),
            self.swap_chain_present_mode.clone().unwrap_or_default(),
            option_u32(self.display_refresh_hz),
            self.render_thread_priority.clone().unwrap_or_default(),
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
| render_present_p95_ms | {present_p95} |\n\
| swap_chain_waitable_object | {swap_chain_waitable} |\n\
| swap_chain_present_mode | {swap_chain_present_mode} |\n\
| display_refresh_hz | {display_refresh_hz} |\n\
| render_thread_priority | {render_thread_priority} |\n\
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
        present_p95 = option_f64(summary.render_present_p95_ms),
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
    use mrd_proto::SessionId;

    use crate::test_harness::{
        CaptureType, DecoderType, EncoderType, RendererType, TestChain, TestConfig, TestHarness,
        TransportKind,
    };

    use super::{BenchmarkManifest, BenchmarkPaths, BenchmarkSummary};

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
            renderer: Some(parse_renderer_backend(&manifest.renderer_backend)),
            transport: Some(parse_transport_backend(&manifest.transport)),
            zero_copy: Some(benchmark_zero_copy_enabled(manifest)),
            pace_to_fps: Some(env_bool("MRD_BENCH_PACE_TO_FPS", false)),
            visual_preview: Some(false),
            ..Default::default()
        });

        let started = Instant::now();
        harness.start().expect("start benchmark harness");
        let first_frame_time_ms = wait_for_first_decoded_frame(&harness, Duration::from_secs(8));
        thread::sleep(Duration::from_secs(manifest.duration_secs));
        harness.stop().expect("stop benchmark harness");
        let metrics = harness.get_metrics();
        let elapsed_secs = started.elapsed().as_secs_f64().max(0.001);
        let bitrate_kbps = (metrics.total_bitstream_bytes as f64 * 8.0) / elapsed_secs / 1000.0;
        let first_frame_seen =
            first_frame_time_ms.is_some() || metrics.decoded_frames > 0 || metrics.frame_count > 0;
        let probe = probe_from_metrics(manifest, session_id, &metrics, bitrate_kbps);
        let render_probe_complete = manifest.renderer_backend == "none"
            || (metrics.render_latency_p95_ms > 0.0 && metrics.render_present_gap_p95_ms > 0.0);
        let probe_complete = metrics.encoded_units > 0
            && metrics.decoded_frames > 0
            && metrics.encode_latency_p95_ms > 0.0
            && metrics.decode_latency_p95_ms > 0.0
            && render_probe_complete;
        let failure_reason = harness_failure_reason(&metrics, first_frame_time_ms, probe_complete);
        let run_passed = first_frame_seen
            && probe_complete
            && metrics.encode_failures == 0
            && metrics.decode_failures == 0
            && metrics.error_message.is_none();

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
            render_present_p95_ms: nonzero_option(metrics.render_present_gap_p95_ms),
            render_submitted_frames: Some(metrics.render_submitted_frames),
            render_uploaded_frames: Some(metrics.render_uploaded_frames),
            render_presented_frames: Some(metrics.render_presented_frames),
            render_present_skipped_frames: Some(metrics.render_present_skipped_frames),
            render_queue_replacements: Some(metrics.render_queue_replacements),
            render_stale_frame_drops: Some(metrics.render_stale_frame_drops),
            swap_chain_waitable_object: metrics.swap_chain_waitable_object,
            swap_chain_present_mode: metrics.swap_chain_present_mode.clone(),
            display_refresh_hz: metrics.display_refresh_hz,
            render_thread_priority: metrics.render_thread_priority.clone(),
            nvdec_runtime_summary: String::new(),
            nvdec_h264_capability: String::new(),
            nvdec_hevc_capability: String::new(),
            nvdec_hevc_main10_capability: String::new(),
            failure_reason,
            run_skipped: false,
            run_passed,
        };

        (summary, probe)
    }

    fn harness_failure_reason(
        metrics: &crate::test_harness::HarnessMetrics,
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
            return Some("first decoded frame was not observed before timeout".to_string());
        }
        if !probe_complete {
            return Some("benchmark probe did not collect all required stage metrics".to_string());
        }
        None
    }

    fn wait_for_first_decoded_frame(harness: &TestHarness, timeout: Duration) -> Option<f64> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            let metrics = harness.get_metrics();
            if metrics.decoded_frames > 0 || metrics.frame_count > 0 {
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
                if let Err(error) = NvencAv1Encoder::new(64, 64, 30) {
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
            render_present_p95_ms: None,
            render_submitted_frames: None,
            render_uploaded_frames: None,
            render_presented_frames: None,
            render_present_skipped_frames: None,
            render_queue_replacements: None,
            render_stale_frame_drops: None,
            swap_chain_waitable_object: None,
            swap_chain_present_mode: None,
            display_refresh_hz: None,
            render_thread_priority: None,
            nvdec_runtime_summary: String::new(),
            nvdec_h264_capability: String::new(),
            nvdec_hevc_capability: String::new(),
            nvdec_hevc_main10_capability: String::new(),
            failure_reason: Some(reason),
            run_skipped: true,
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

        assert_eq!(probe.fps, 118.0);
        assert_eq!(render_upload.p50_ms, Some(0.18));
        assert_eq!(render_upload.p95_ms, Some(0.35));
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
            render_present_p95_ms: None,
            render_submitted_frames: Some(10),
            render_uploaded_frames: Some(9),
            render_presented_frames: Some(8),
            render_present_skipped_frames: Some(1),
            render_queue_replacements: Some(2),
            render_stale_frame_drops: Some(2),
            swap_chain_waitable_object: Some(true),
            swap_chain_present_mode: Some("waitable".into()),
            display_refresh_hz: Some(144),
            render_thread_priority: Some("above_normal".into()),
            nvdec_runtime_summary: "nvdec runtime libraries and core exports are present".into(),
            nvdec_h264_capability: "runtime=true wired=true".into(),
            nvdec_hevc_capability: "runtime=true wired=false".into(),
            nvdec_hevc_main10_capability: "runtime=false wired=false".into(),
            failure_reason: Some("encode produced no HEVC Main10 access units".into()),
            run_skipped: false,
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
        assert!(header.contains(&"nvdec_hevc_main10_capability"));
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
        let refresh_index = header
            .iter()
            .position(|column| *column == "display_refresh_hz")
            .expect("display refresh column");
        assert_eq!(row[refresh_index], "144");
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
