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

export function summaryFromLanE2EReport(report: LanE2EAutomationReport): TestRunSummary {
  const probe = report.probeSnapshot;
  const adaptation = report.mediaAdaptationSnapshot ?? report.mediaPipelineSnapshot?.adaptation;
  return {
    total_duration_ms: Math.max(0, report.finishedAt - report.startedAt),
    capture_fps: report.sampleObservedFps ?? probe?.current_fps ?? undefined,
    dropped_frames: report.sampleFramesDropped ?? probe?.frames_dropped ?? 0,
    frame_count: report.sampleFramesDecoded ?? probe?.frames_decoded ?? 0,
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
