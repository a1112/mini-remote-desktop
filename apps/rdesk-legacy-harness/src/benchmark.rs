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
    pub nvdec_runtime_summary: String,
    pub nvdec_h264_capability: String,
    pub nvdec_hevc_capability: String,
    pub nvdec_hevc_main10_capability: String,
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
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
            run_passed: session_established && first_frame_seen && probe_complete,
        }
    }

    pub fn from_transport_probes(
        manifest: &BenchmarkManifest,
        sender_probe: &PipelineProbeSnapshot,
        receiver_probe: &PipelineProbeSnapshot,
        session_established: bool,
        first_frame_seen: bool,
        first_frame_time_ms: f64,
        zero_write_access_unit_count: u64,
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
            fps_observed: receiver_probe.fps,
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
            nvdec_runtime_summary,
            nvdec_h264_capability,
            nvdec_hevc_capability,
            nvdec_hevc_main10_capability,
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
            "nvdec_runtime_summary",
            "nvdec_h264_capability",
            "nvdec_hevc_capability",
            "nvdec_hevc_main10_capability",
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
            self.nvdec_runtime_summary.clone(),
            self.nvdec_h264_capability.clone(),
            self.nvdec_hevc_capability.clone(),
            self.nvdec_hevc_main10_capability.clone(),
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

fn option_u64(value: Option<u64>) -> String {
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
        fps_observed = summary.fps_observed,
        bitrate_kbps = summary.bitrate_kbps,
        encode_p95 = option_f64(summary.encode_total_p95_ms),
        send_p95 = option_f64(summary.send_write_p95_ms),
        decode_p95 = option_f64(summary.decode_total_p95_ms),
        frame_sink_p95 = option_f64(summary.frame_sink_ingest_p95_ms),
        render_p95 = option_f64(summary.render_upload_p95_ms),
        present_p95 = option_f64(summary.render_present_p95_ms),
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
    use std::path::PathBuf;

    use mrd_observability::{PipelineProbeSnapshot, StageId, StageStatsSnapshot};
    use mrd_proto::SessionId;

    use super::{BenchmarkManifest, BenchmarkPaths, BenchmarkSummary};

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
            vec![],
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
        assert_eq!(summary.quic_receiver_completed_frames, None);
        assert!(!summary.nvdec_runtime_summary.is_empty());
        assert!(!summary.nvdec_h264_capability.is_empty());
        assert!(!summary.nvdec_hevc_capability.is_empty());
        assert!(!summary.nvdec_hevc_main10_capability.is_empty());
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
            nvdec_runtime_summary: "nvdec runtime libraries and core exports are present".into(),
            nvdec_h264_capability: "runtime=true wired=true".into(),
            nvdec_hevc_capability: "runtime=true wired=false".into(),
            nvdec_hevc_main10_capability: "runtime=false wired=false".into(),
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
        assert!(header.contains(&"nvdec_hevc_main10_capability"));
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
