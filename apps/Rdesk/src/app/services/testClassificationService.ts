import type {
  EnvironmentSnapshot,
  LanPeerInfo,
  TestClassification,
  TestConfig,
  TestDeviceDescriptor,
  TestRun,
  TestRunScope,
} from "../adapters/tauri/types";

export interface PerformanceComparisonRow {
  runId: string;
  scenarioId: string;
  status: string;
  startedAt: number;
  label: string;
  deviceLabel: string;
  runScope: TestClassification["run_scope"];
  memoryPath: TestClassification["memory_path"];
  encodeAccel: TestClassification["encode_accel"];
  decodeAccel: TestClassification["decode_accel"];
  transportPath: TestClassification["transport_path"];
  renderPath: TestClassification["render_path"];
  resolution: string;
  targetFps: number | null;
  bitrateMbps: number | null;
  fpsAvg: number | null;
  fpsMin: number | null;
  latencyP50Ms: number | null;
  latencyP95Ms: number | null;
  threeFrameBudgetMs: number | null;
  droppedFrames: number;
  dropRatePct: number | null;
  frameCount: number;
  cpuP95Percent: number | null;
  gpuP95Percent: number | null;
  memoryPeakMb: number | null;
  networkPeakMbps: number | null;
}

export interface MatrixPerformanceSummary {
  rows: PerformanceComparisonRow[];
  completed: number;
  failed: number;
  skipped: number;
}

export function deriveTestClassification(
  config: TestConfig = {},
  environment?: EnvironmentSnapshot | null,
  options: {
    runScope?: TestRunScope;
    peer?: LanPeerInfo | TestDeviceDescriptor | null;
    localDeviceId?: string | null;
    localDeviceName?: string | null;
  } = {}
): TestClassification {
  const runScope = options.runScope ?? "local";
  const transportPath = transportPathFromConfig(config);
  const renderPath = renderPathFromConfig(config, transportPath);
  const memoryPath = memoryPathFromConfig(config, transportPath, renderPath);
  return {
    run_scope: runScope,
    memory_path: memoryPath,
    encode_accel: encodeAccelFromConfig(config),
    decode_accel: decodeAccelFromConfig(config, transportPath),
    transport_path: transportPath,
    render_path: renderPath,
    local_device: environment
      ? {
          device_id: options.localDeviceId ?? null,
          device_name: options.localDeviceName ?? "local",
          platform: environment.os_type ?? null,
          cpu: environment.cpu_brand ?? null,
          gpu: environment.gpu_info ?? null,
        }
      : null,
    peer_device: options.peer ? deviceDescriptorFromPeer(options.peer) : null,
  };
}

export function classificationForRun(run: TestRun): TestClassification {
  return (
    run.classification ??
    deriveTestClassification(run.config_snapshot, run.environment_snapshot, {
      runScope: run.scenario_id.startsWith("cross.") || run.scenario_id.startsWith("lan.")
        ? "cross_device"
        : "local",
    })
  );
}

export function performanceRowFromRun(run: TestRun): PerformanceComparisonRow {
  const classification = classificationForRun(run);
  const config = run.config_snapshot ?? {};
  const summary = run.summary;
  const resolution = config.resolution ? `${config.resolution[0]}x${config.resolution[1]}` : "unknown";
  const targetFps = positiveFiniteOrNull(config.fps);
  const bitrate = positiveFiniteOrNull(config.bitrate);
  const bitrateMbps = bitrate == null ? null : bitrate / 1_000_000;
  const fpsAvg = finiteOrNull(summary?.capture_fps);
  const frameCount = summary?.frame_count ?? 0;
  const droppedFrames = summary?.dropped_frames ?? 0;
  const totalFrames = frameCount + droppedFrames;
  const dropRatePct = totalFrames > 0 ? (droppedFrames / totalFrames) * 100 : null;
  const latencyP95Ms = finiteOrNull(summary?.total_latency_p95);
  const latencyP50Ms = finiteOrNull(summary?.first_frame_latency_ms);
  const deviceLabel = deviceComparisonLabel(classification);

  return {
    runId: run.run_id,
    scenarioId: run.scenario_id,
    status: run.status,
    startedAt: run.started_at,
    label: `${deviceLabel} / ${classification.memory_path} / ${classification.encode_accel}->${classification.decode_accel}`,
    deviceLabel,
    runScope: classification.run_scope,
    memoryPath: classification.memory_path,
    encodeAccel: classification.encode_accel,
    decodeAccel: classification.decode_accel,
    transportPath: classification.transport_path,
    renderPath: classification.render_path,
    resolution,
    targetFps,
    bitrateMbps,
    fpsAvg,
    fpsMin: null,
    latencyP50Ms,
    latencyP95Ms,
    threeFrameBudgetMs: targetFps ? (3 * 1000) / targetFps : null,
    droppedFrames,
    dropRatePct,
    frameCount,
    cpuP95Percent: finiteOrNull(summary?.cpu_p95_percent),
    gpuP95Percent: finiteOrNull(summary?.gpu_p95_percent),
    memoryPeakMb: finiteOrNull(summary?.memory_peak_mb),
    networkPeakMbps: finiteOrNull(summary?.network_peak_mbps),
  };
}

export function buildMatrixPerformanceSummary(
  runs: Array<{
    id: string;
    config: TestConfig;
    status: "pending" | "running" | "completed" | "failed" | "skipped";
    result?: TestRun["summary"];
    duration?: number;
  }>,
  environment?: EnvironmentSnapshot | null,
  options: { runScope?: TestRunScope; peer?: LanPeerInfo | null } = {}
): MatrixPerformanceSummary {
  const now = Date.now();
  const rows = runs
    .filter((run) => run.result)
    .map((run, index) =>
      performanceRowFromRun({
        run_id: run.id,
        scenario_id: options.runScope === "cross_device" ? "cross.e2e.remote_display_smoke" : "matrix",
        run_mode: "matrix",
        status: matrixStatusToRunStatus(run.status),
        started_at: now + index,
        finished_at: run.duration ? now + index + run.duration : undefined,
        config_snapshot: run.config,
        environment_snapshot: environment ?? unknownEnvironment(),
        summary: run.result,
        classification: deriveTestClassification(run.config, environment, {
          runScope: options.runScope ?? "local",
          peer: options.peer ?? null,
        }),
      })
    );

  return {
    rows,
    completed: runs.filter((run) => run.status === "completed").length,
    failed: runs.filter((run) => run.status === "failed").length,
    skipped: runs.filter((run) => run.status === "skipped").length,
  };
}

export function groupRowsByClassification(rows: PerformanceComparisonRow[]) {
  const groups = new Map<string, PerformanceComparisonRow[]>();
  for (const row of rows) {
    const key = [
      row.deviceLabel,
      row.runScope,
      row.memoryPath,
      row.encodeAccel,
      row.decodeAccel,
      row.transportPath,
      row.renderPath,
    ].join(" / ");
    groups.set(key, [...(groups.get(key) ?? []), row]);
  }

  return Array.from(groups.entries()).map(([key, groupRows]) => ({
    key,
    label: key,
    count: groupRows.length,
    fpsAvg: average(groupRows.map((row) => row.fpsAvg)),
    latencyP95Ms: average(groupRows.map((row) => row.latencyP95Ms)),
    dropRatePct: average(groupRows.map((row) => row.dropRatePct)),
    cpuP95Percent: average(groupRows.map((row) => row.cpuP95Percent)),
    gpuP95Percent: average(groupRows.map((row) => row.gpuP95Percent)),
    memoryPeakMb: max(groupRows.map((row) => row.memoryPeakMb)),
    networkPeakMbps: max(groupRows.map((row) => row.networkPeakMbps)),
  }));
}

function transportPathFromConfig(config: TestConfig): TestClassification["transport_path"] {
  switch (config.transport_kind) {
    case "webrtc":
      return "webrtc";
    case "quic":
      return "quic";
    case "loopback":
      return "loopback";
    case undefined:
      return "loopback";
    default:
      return "unknown";
  }
}

function matrixStatusToRunStatus(status: "pending" | "running" | "completed" | "failed" | "skipped") {
  switch (status) {
    case "pending":
      return "queued" as const;
    case "skipped":
      return "skipped" as const;
    default:
      return status;
  }
}

function renderPathFromConfig(
  config: TestConfig,
  transportPath: TestClassification["transport_path"]
): TestClassification["render_path"] {
  if (transportPath === "webrtc" && (!config.render_display || config.renderer_type === "webview")) {
    return "browser_video";
  }
  if (!config.render_display) return "none";
  switch (config.renderer_type) {
    case "d3d11":
      return "native_d3d11";
    case "d3d12":
      return "native_d3d12";
    case "opengl":
      return "native_opengl";
    case "macos":
      return "native_macos";
    case "linux":
      return "native_linux";
    case "webview":
      return "browser_video";
    default:
      return "unknown";
  }
}

function memoryPathFromConfig(
  config: TestConfig,
  transportPath: TestClassification["transport_path"],
  renderPath: TestClassification["render_path"]
): TestClassification["memory_path"] {
  if (transportPath === "webrtc" && renderPath === "browser_video") {
    return "webrtc_media_stream";
  }
  if (config.zero_copy) return "zero_copy_d3d11_shared";
  if (!config.capture_type && !config.encoder_type && !config.decoder_type) return "unknown";
  return "cpu_copy";
}

function encodeAccelFromConfig(config: TestConfig): TestClassification["encode_accel"] {
  switch (config.encoder_type) {
    case "none":
      return "none";
    case "nvenc_h264":
    case "nvenc_hevc":
    case "nvenc_hevc_main10":
    case "nvenc_av1":
    case "videotoolbox_h264":
    case "videotoolbox_hevc":
      return "hardware";
    case "openh264":
    case "software_vvc":
      return "software";
    case undefined:
      return "unknown";
    default:
      return "unknown";
  }
}

function decodeAccelFromConfig(
  config: TestConfig,
  transportPath: TestClassification["transport_path"]
): TestClassification["decode_accel"] {
  if (config.decoder_type === "none" && transportPath === "webrtc") return "browser";
  switch (config.decoder_type) {
    case "none":
      return "none";
    case "nvdec":
    case "linux_h264":
    case "linux_hevc":
    case "linux_hevc_main10":
    case "videotoolbox":
      return "hardware";
    case "software":
    case "ffmpeg_h264":
    case "ffmpeg_hevc":
    case "ffmpeg_vvc":
      return "software";
    case undefined:
      return "unknown";
    default:
      return "unknown";
  }
}

function deviceDescriptorFromPeer(peer: LanPeerInfo | TestDeviceDescriptor): TestDeviceDescriptor {
  if ("device_id" in peer && "transports" in peer) {
    return {
      device_id: peer.device_id,
      device_name: peer.device_name,
      platform: peer.device_type,
      service_build_id: peer.service_build_id ?? null,
      protocol_version: peer.protocol_version,
      media_protocol_version: peer.media_protocol_version ?? null,
    };
  }
  return peer;
}

function deviceComparisonLabel(classification: TestClassification): string {
  const local = classification.local_device?.device_name || classification.local_device?.platform || "local";
  const peer = classification.peer_device?.device_name || classification.peer_device?.device_id;
  return classification.run_scope === "cross_device" && peer ? `${local} -> ${peer}` : local;
}

function unknownEnvironment(): EnvironmentSnapshot {
  return {
    os_type: "unknown",
    cpu_brand: "Unknown CPU",
    cpu_cores: 0,
    memory_gb: 0,
    gpu_info: "Unknown GPU",
    available_captures: [],
    available_encoders: [],
    available_decoders: [],
    available_renderers: [],
    available_memory_modes: [],
  };
}

function finiteOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function positiveFiniteOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function average(values: Array<number | null>) {
  const finite = values.filter((value): value is number => value != null && Number.isFinite(value));
  if (finite.length === 0) return null;
  return finite.reduce((sum, value) => sum + value, 0) / finite.length;
}

function max(values: Array<number | null>) {
  const finite = values.filter((value): value is number => value != null && Number.isFinite(value));
  if (finite.length === 0) return null;
  return Math.max(...finite);
}
