import type {
  EnvironmentSnapshot,
  ExternalTestRunRecord,
  LanPeerInfo,
  TestConfig,
  TestStage,
  TestRunSummary,
} from "../adapters/tauri/types";
import type { LanE2EAutomationReport } from "./lanE2eAutomationService";
import { deriveTestClassification } from "./testClassificationService";

const MAINLINE_E2E_ARTIFACT_KIND = "mainline_e2e_artifacts_v1";

export interface MainlineE2EArtifactPayload {
  kind: typeof MAINLINE_E2E_ARTIFACT_KIND;
  schema_version: 1;
  run_id: string;
  artifact_date: string;
  generated_at: number;
  git_commit: string;
  scenario_id: string;
  producer_status: LanE2EAutomationReport["status"];
  gate_status: "PASS" | "PRODUCT_FAIL" | "INFRA_FAIL" | "INVALID_ARTIFACT" | "ALLOWED_SKIP" | "UNKNOWN";
  final_status: LanE2EAutomationReport["status"];
  failure_reason?: LanE2EAutomationReport["failureReason"];
  script_classification: MainlineE2EScriptClassification;
  human_message?: string;
  controller: {
    device_id?: string | null;
    capability_snapshot?: EnvironmentSnapshot | null;
  };
  agent?: {
    device_id: string;
    device_name: string;
    platform?: string | null;
    service_build_id?: string | null;
    protocol_version?: number | null;
    media_protocol_version?: number | null;
    media_capabilities?: string[];
  };
  requested_profile?: LanE2EAutomationReport["requestedProfile"];
  selected_profile?: LanE2EAutomationReport["requestedProfile"];
  transport_kind: "quic" | "webrtc";
  capture_source?: LanE2EAutomationReport["captureSource"];
  display_mode?: LanE2EAutomationReport["displayModeChange"];
  first_frame_time_ms?: number;
  max_zero_frame_window_after_first_frame_ms?: number;
  stage_events: LanE2EAutomationReport["stages"];
  fault_events: LanE2EAutomationReport["faultEvents"];
  runtime_snapshots: Array<NonNullable<LanE2EAutomationReport["sessionSnapshot"]>>;
  probe_snapshots: Array<NonNullable<LanE2EAutomationReport["probeSnapshot"]>>;
  media_pipeline_snapshot?: LanE2EAutomationReport["mediaPipelineSnapshot"];
  metric_series: E2EMetricSeriesRow[];
  summary: TestRunSummary;
  classification: ReturnType<typeof deriveTestClassification>;
  artifacts: MainlineE2EArtifactDescriptor[];
  metrics_csv: string;
  report: LanE2EAutomationReport;
}

export interface MainlineE2EArtifactDescriptor {
  path: string;
  kind:
    | "summary_json"
    | "timeline_json"
    | "metrics_csv"
    | "structured_log"
    | "failure_text"
    | "frame_png";
  required: boolean;
  status: "generated" | "missing";
  description?: string;
}

function gateStatusFromLanE2EReport(
  report: LanE2EAutomationReport
): MainlineE2EArtifactPayload["gate_status"] {
  const typedReport = report as LanE2EAutomationReport & {
    gate_status?: MainlineE2EArtifactPayload["gate_status"];
    gate?: { verdict?: MainlineE2EArtifactPayload["gate_status"] };
  };
  const candidate = typedReport.gate_status ?? typedReport.gate?.verdict;
  if (
    candidate === "PASS" ||
    candidate === "PRODUCT_FAIL" ||
    candidate === "INFRA_FAIL" ||
    candidate === "INVALID_ARTIFACT" ||
    candidate === "ALLOWED_SKIP"
  ) {
    return candidate;
  }
  return "UNKNOWN";
}

export interface E2EMetricSeriesRow {
  timestamp: number;
  sample_duration_ms: number;
  frames_decoded: number;
  frames_dropped: number;
  render_frames_presented: number;
  observed_fps?: number | null;
  observed_render_fps?: number | null;
  queue_depth?: number | null;
  render_queue_replacements?: number | null;
  render_present_skips?: number | null;
  receiver_active?: boolean | null;
  first_frame_time_ms?: number | null;
  max_zero_frame_window_after_first_frame_ms?: number | null;
}

export type MainlineE2EScriptClassification =
  | "completed"
  | "skipped"
  | "unsupported"
  | "peer_version_mismatch"
  | "display_refresh_limited"
  | "profile_downgraded"
  | "display_mode_failed"
  | "capture_error"
  | "transport_loss"
  | "decode_error"
  | "render_error"
  | "threshold_miss"
  | "service_crash"
  | "runtime_error"
  | "visual_integrity_risk"
  | "fault_injection_unsupported"
  | "fault_injection_failed"
  | "stop_failed"
  | "failed";

export function summaryFromLanE2EReport(report: LanE2EAutomationReport): TestRunSummary {
  const probe = report.probeSnapshot;
  const adaptation = report.mediaAdaptationSnapshot ?? report.mediaPipelineSnapshot?.adaptation;
  const renderFrameCount = report.sampleRenderFramesPresented;
  const renderQueueReplacements =
    report.sampleRenderQueueReplacements ??
    report.mediaPipelineSnapshot?.render_queue_replacements;
  const renderPresentSkips =
    report.sampleRenderPresentSkips ??
    report.mediaPipelineSnapshot?.render_present_skips;
  return {
    total_duration_ms: Math.max(0, report.finishedAt - report.startedAt),
    first_frame_latency_ms: report.firstFrameTimeMs,
    capture_fps:
      report.sampleObservedFpsAtTargetDuration ??
      report.sampleObservedFps ??
      probe?.current_fps ??
      undefined,
    dropped_frames: report.sampleFramesDropped ?? probe?.frames_dropped ?? 0,
    frame_count: report.sampleFramesDecoded ?? probe?.frames_decoded ?? 0,
    render_fps:
      report.sampleObservedRenderFpsAtTargetDuration ??
      report.sampleObservedRenderFps,
    render_frame_count: renderFrameCount,
    render_queue_replacements: renderQueueReplacements,
    render_queue_replacement_ratio: finiteRatio(renderQueueReplacements, renderFrameCount),
    render_present_skips: renderPresentSkips,
    render_present_skip_ratio: finiteRatio(renderPresentSkips, renderFrameCount),
    adaptation_state: adaptation?.state,
    adaptation_ladder_index: adaptation?.ladder_index,
    adaptation_current_profile: adaptation
      ? formatMediaProfile(adaptation.current_profile)
      : undefined,
    adaptation_target_profile: adaptation
      ? formatMediaProfile(adaptation.target_profile)
      : undefined,
    adaptation_reason: adaptation?.last_reason ?? undefined,
    error_message: report.errorMessage,
    failure_reason: report.failureReason ? "validation_failure" : undefined,
  };
}

function finiteRatio(numerator: number | undefined, denominator: number | undefined) {
  if (
    typeof numerator !== "number" ||
    typeof denominator !== "number" ||
    !Number.isFinite(numerator) ||
    !Number.isFinite(denominator) ||
    denominator <= 0
  ) {
    return undefined;
  }
  return numerator / denominator;
}

export function externalRunRecordFromLanE2EReport(
  report: LanE2EAutomationReport,
  config: TestConfig,
  options: {
    environment?: EnvironmentSnapshot | null;
    peer?: LanPeerInfo | null;
    runMode?: ExternalTestRunRecord["run_mode"];
    runIdPrefix?: string;
  } = {}
): ExternalTestRunRecord {
  const reportJson = JSON.stringify(report);
  const runId = `${options.runIdPrefix ?? "lan"}-${safeRunIdPart(
    report.sessionId ?? `${report.startedAt}-${report.finishedAt}`
  )}`;
  return {
    run_id: runId,
    scenario_id: report.scenarioId,
    run_mode: options.runMode ?? "matrix",
    status:
      report.status === "completed"
        ? "completed"
        : report.status === "skipped"
          ? "cancelled"
          : "failed",
    started_at: report.startedAt,
    finished_at: report.finishedAt,
    config_snapshot: config,
    environment_snapshot: options.environment ?? undefined,
    summary: summaryFromLanE2EReport(report),
    classification: deriveTestClassification(config, options.environment, {
      runScope: "cross_device",
      peer: options.peer ?? report.peer ?? null,
    }),
    events: report.stages.map((stage) => ({
      stage: testStageFromLanStage(stage.stage),
      status: stage.status,
      timestamp: stage.timestamp,
      error: stage.error,
    })),
    artifacts: [
      {
        artifact_id: `${runId}-report`,
        kind: "lan_e2e_report",
        run_id: runId,
        created_at: report.finishedAt,
        data: reportJson,
        metadata: {
          format: "json",
          size_bytes: reportJson.length,
        },
      },
    ],
  };
}

export function mainlineE2EArtifactPayloadFromReport(
  report: LanE2EAutomationReport,
  config: TestConfig,
  options: {
    environment?: EnvironmentSnapshot | null;
    peer?: LanPeerInfo | null;
    runMode?: ExternalTestRunRecord["run_mode"];
    runIdPrefix?: string;
    gitCommit?: string;
    generatedAt?: number;
  } = {}
): MainlineE2EArtifactPayload {
  const externalRecord = externalRunRecordFromLanE2EReport(report, config, options);
  const generatedAt = options.generatedAt ?? report.finishedAt;
  const metricSeries = metricSeriesFromLanE2EReport(report);
  const peer = options.peer ?? report.peer ?? null;
  const finalStatus = report.status;
  const failureReason = report.failureReason;
  const gateStatus = gateStatusFromLanE2EReport(report);
  return {
    kind: MAINLINE_E2E_ARTIFACT_KIND,
    schema_version: 1,
    run_id: externalRecord.run_id ?? defaultLanRunId(report, options.runIdPrefix),
    artifact_date: dateKeyFromTimestamp(generatedAt),
    generated_at: generatedAt,
    git_commit: normalizeGitCommit(options.gitCommit ?? runtimeGitCommit()),
    scenario_id: report.scenarioId,
    producer_status: finalStatus,
    gate_status: gateStatus,
    final_status: finalStatus,
    failure_reason: failureReason,
    script_classification: scriptClassificationFromLanE2EReport(report),
    human_message: report.errorMessage,
    controller: {
      device_id: report.controllerDeviceId ?? null,
      capability_snapshot: options.environment ?? null,
    },
    agent: peer
      ? {
          device_id: peer.device_id,
          device_name: peer.device_name,
          platform: peer.device_type,
          service_build_id: peer.service_build_id ?? null,
          protocol_version: peer.protocol_version,
          media_protocol_version: peer.media_protocol_version ?? null,
          media_capabilities: peer.media_capabilities ?? [],
        }
      : undefined,
    requested_profile: report.requestedProfile,
    selected_profile: selectedProfileFromLanE2EReport(report),
    transport_kind: report.validationMode === "webrtc_rtp" ? "webrtc" : "quic",
    capture_source: report.captureSource,
    display_mode: report.displayModeChange,
    first_frame_time_ms: report.firstFrameTimeMs,
    max_zero_frame_window_after_first_frame_ms: report.maxZeroFrameWindowAfterFirstFrameMs,
    stage_events: report.stages,
    fault_events: report.faultEvents,
    runtime_snapshots: report.sessionSnapshot ? [report.sessionSnapshot] : [],
    probe_snapshots: report.probeSnapshot ? [report.probeSnapshot] : [],
    media_pipeline_snapshot: report.mediaPipelineSnapshot,
    metric_series: metricSeries,
    summary: externalRecord.summary ?? summaryFromLanE2EReport(report),
    classification: externalRecord.classification ?? deriveTestClassification(config, options.environment, {
      runScope: "cross_device",
      peer,
    }),
    artifacts: mainlineArtifactDescriptors(Boolean(peer), finalStatus, failureReason, gateStatus),
    metrics_csv: metricsCsvFromRows(metricSeries),
    report,
  };
}

function mainlineArtifactDescriptors(
  hasAgent: boolean,
  status: LanE2EAutomationReport["status"],
  failureReason?: LanE2EAutomationReport["failureReason"],
  gateStatus: MainlineE2EArtifactPayload["gate_status"] = "UNKNOWN"
): MainlineE2EArtifactDescriptor[] {
  const artifacts: MainlineE2EArtifactDescriptor[] = [
    {
      path: "summary.json",
      kind: "summary_json",
      required: true,
      status: "generated",
      description: "Canonical run summary and source report payload.",
    },
    {
      path: "timeline.json",
      kind: "timeline_json",
      required: true,
      status: "generated",
      description: "Stage and fault timeline used by scripts and CI artifacts.",
    },
    {
      path: "metrics.csv",
      kind: "metrics_csv",
      required: true,
      status: "generated",
      description: "Sample-window metric series.",
    },
    {
      path: "controller.log",
      kind: "structured_log",
      required: true,
      status: "generated",
      description: "Controller build, device, capability, and summary snapshot.",
    },
    {
      path: "agent.log",
      kind: "structured_log",
      required: hasAgent,
      status: "generated",
      description: hasAgent
        ? "Agent build, discovery, protocol, and capability snapshot."
        : "Generated placeholder stating that no cross-device agent was present.",
    },
    {
      path: "first-frame.png",
      kind: "frame_png",
      required: false,
      status: "missing",
      description: "Written only when the runtime provides first_frame_png_base64.",
    },
    {
      path: "last-frame.png",
      kind: "frame_png",
      required: false,
      status: "missing",
      description: "Written only when the runtime provides last_frame_png_base64.",
    },
  ];

  if (status !== "completed" || failureReason || gateStatus !== "PASS") {
    artifacts.push({
      path: "failure.txt",
      kind: "failure_text",
      required: true,
      status: "generated",
      description: "Human-readable terminal status, failure reason, and message.",
    });
  }

  return artifacts;
}

export function scriptClassificationFromLanE2EReport(
  report: Pick<LanE2EAutomationReport, "status" | "failureReason" | "errorMessage">
): MainlineE2EScriptClassification {
  if (report.status === "completed") return "completed";
  if (report.status === "skipped") {
    return skippedScriptClassification(report.failureReason);
  }

  switch (report.failureReason) {
    case "peer_not_ready":
    case "peer_not_found":
      return "unsupported";
    case "peer_version_mismatch":
      return "peer_version_mismatch";
    case "media_profile_mismatch":
    case "profile_downgraded":
      return "profile_downgraded";
    case "display_mode_failed":
      return "display_mode_failed";
    case "capture_source_failed":
      return "capture_error";
    case "display_window_failed":
      return "render_error";
    case "control_input_unsupported":
      return "unsupported";
    case "control_input_failed":
      return "runtime_error";
    case "receiver_start_failed":
    case "session_start_failed":
      return "transport_loss";
    case "no_remote_frames":
    case "performance_threshold":
      return "threshold_miss";
    case "service_unhealthy":
    case "local_device_registration_failed":
      return "service_crash";
    case "fault_injection_unsupported":
      return "fault_injection_unsupported";
    case "fault_injection_failed":
      return "fault_injection_failed";
    case "stop_failed":
      return "stop_failed";
    case "runtime_error":
      return runtimeErrorScriptClassification(report.errorMessage);
    case undefined:
      return report.status === "failed" ? "failed" : "skipped";
  }
}

function skippedScriptClassification(
  reason: LanE2EAutomationReport["failureReason"]
): MainlineE2EScriptClassification {
  switch (reason) {
    case "peer_not_found":
    case "peer_not_ready":
    case "control_input_unsupported":
      return "unsupported";
    case "peer_version_mismatch":
      return "peer_version_mismatch";
    case "media_profile_mismatch":
    case "profile_downgraded":
      return "profile_downgraded";
    case "fault_injection_unsupported":
      return "fault_injection_unsupported";
    case undefined:
      return "skipped";
    default:
      return "skipped";
  }
}

function runtimeErrorScriptClassification(
  errorMessage: string | undefined
): MainlineE2EScriptClassification {
  if (errorMessage && /decode|h\.264|nvdec/i.test(errorMessage)) {
    return "decode_error";
  }
  if (errorMessage && /transport|quic|timeout/i.test(errorMessage)) {
    return "transport_loss";
  }
  return "runtime_error";
}

function defaultLanRunId(
  report: LanE2EAutomationReport,
  runIdPrefix?: string
): string {
  return `${runIdPrefix ?? "lan"}-${safeRunIdPart(
    report.sessionId ?? `${report.startedAt}-${report.finishedAt}`
  )}`;
}

function selectedProfileFromLanE2EReport(
  report: LanE2EAutomationReport
): LanE2EAutomationReport["requestedProfile"] {
  return (
    report.mediaAdaptationSnapshot?.current_profile ??
    report.mediaPipelineSnapshot?.adaptation?.current_profile ??
    report.requestedProfile
  );
}

function metricSeriesFromLanE2EReport(report: LanE2EAutomationReport): E2EMetricSeriesRow[] {
  return [
    {
      timestamp: report.finishedAt,
      sample_duration_ms: report.sampleDurationMs,
      frames_decoded: report.sampleFramesDecoded,
      frames_dropped: report.sampleFramesDropped,
      render_frames_presented: report.sampleRenderFramesPresented,
      observed_fps: nullableFinite(report.sampleObservedFpsAtTargetDuration ?? report.sampleObservedFps),
      observed_render_fps: nullableFinite(
        report.sampleObservedRenderFpsAtTargetDuration ?? report.sampleObservedRenderFps
      ),
      queue_depth: nullableFinite(report.mediaPipelineSnapshot?.queue_depth),
      render_queue_replacements: nullableFinite(report.sampleRenderQueueReplacements),
      render_present_skips: nullableFinite(report.sampleRenderPresentSkips),
      receiver_active: report.sessionSnapshot?.receiver_active ?? null,
      first_frame_time_ms: nullableFinite(report.firstFrameTimeMs),
      max_zero_frame_window_after_first_frame_ms: nullableFinite(
        report.maxZeroFrameWindowAfterFirstFrameMs
      ),
    },
  ];
}

function metricsCsvFromRows(rows: E2EMetricSeriesRow[]): string {
  const headers = [
    "timestamp",
    "sample_duration_ms",
    "frames_decoded",
    "frames_dropped",
    "render_frames_presented",
    "observed_fps",
    "observed_render_fps",
    "queue_depth",
    "render_queue_replacements",
    "render_present_skips",
    "receiver_active",
    "first_frame_time_ms",
    "max_zero_frame_window_after_first_frame_ms",
  ];
  const lines = rows.map((row) =>
    [
      row.timestamp,
      row.sample_duration_ms,
      row.frames_decoded,
      row.frames_dropped,
      row.render_frames_presented,
      row.observed_fps,
      row.observed_render_fps,
      row.queue_depth,
      row.render_queue_replacements,
      row.render_present_skips,
      row.receiver_active,
      row.first_frame_time_ms,
      row.max_zero_frame_window_after_first_frame_ms,
    ].map(csvCell).join(",")
  );
  return `${headers.join(",")}\n${lines.join("\n")}\n`;
}

function csvCell(value: unknown): string {
  if (value == null) return "";
  const text = String(value);
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

function nullableFinite(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function dateKeyFromTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "unknown-date";
  return date.toISOString().slice(0, 10);
}

function normalizeGitCommit(value: string | null | undefined): string {
  const trimmed = value?.trim();
  return trimmed || "unknown";
}

function runtimeGitCommit(): string {
  return (
    (import.meta as any).env?.VITE_GIT_COMMIT ??
    (import.meta as any).env?.VITE_MRD_GIT_COMMIT ??
    "unknown"
  );
}

function formatMediaProfile(profile: {
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
  codec?: string | null;
  codec_profile?: string | null;
  bit_depth?: number | null;
  chroma_subsampling?: string | null;
  pixel_format?: string | null;
}) {
  const codecParts = [
    profile.codec,
    profile.codec_profile,
    profile.bit_depth != null ? `${profile.bit_depth}-bit` : null,
    profile.chroma_subsampling,
    profile.pixel_format,
  ].filter((part): part is string => Boolean(part));
  const codecLabel = codecParts.length > 0 ? `${codecParts.join("/")} ` : "";
  return `${codecLabel}${profile.width}x${profile.height}@${profile.fps}/${profile.bitrate_mbps}Mbps`;
}

function testStageFromLanStage(stage: LanE2EAutomationReport["stages"][number]["stage"]): TestStage {
  switch (stage) {
    case "preflight":
    case "pairing":
      return "prepare";
    case "session":
    case "receiver":
    case "display":
    case "control":
      return "initialize";
    case "capture_source":
      return "capture";
    case "display_mode":
      return "render";
    case "sample":
      return "running";
    case "assert":
    case "adaptation":
    case "fault":
      return "validate";
    case "cleanup":
      return "summarize";
  }
}

function safeRunIdPart(value: string) {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-");
}
