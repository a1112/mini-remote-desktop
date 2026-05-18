import { useState, useEffect, useRef, type ReactNode } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { Play, Square, Monitor, Clock, Zap, Activity, Video } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot, TestConfig, HarnessMetrics } from "../../adapters/tauri/types";
import {
  runLanE2EAutomation,
  type CrossDeviceScenarioId,
  type LanE2EAutomationCommands,
  type LanE2EAutomationOptions,
  type LanE2EAutomationReport,
  type LanE2EStatus,
} from "../../services/lanE2eAutomationService";
import { capabilityAvailable, chooseCapability } from "./capabilityMeta";

function buildDefaultConfig(capabilities: EnvironmentSnapshot | null): TestConfig {
  const capture = chooseCapability(
    ["macos", "linux", "dxgi", "synthetic"],
    capabilities,
    "available_captures",
    "synthetic"
  );
  const encoder = chooseCapability(
    capture === "macos"
      ? ["videotoolbox_h264", "openh264"]
      : capture === "linux"
        ? ["nvenc_h264", "openh264"]
        : ["nvenc_h264", "openh264"],
    capabilities,
    "available_encoders",
    "openh264"
  );
  const decoder = chooseCapability(
    capture === "linux"
      ? ["linux_h264", "software", "none"]
      : capture === "macos"
        ? ["videotoolbox", "software", "none"]
        : ["nvdec", "software", "none"],
    capabilities,
    "available_decoders",
    "none"
  );
  const renderer = capabilityAvailable(capabilities, "available_renderers", "macos")
    ? "macos"
    : capabilityAvailable(capabilities, "available_renderers", "linux")
      ? "linux"
      : capabilityAvailable(capabilities, "available_renderers", "d3d11")
      ? "d3d11"
      : undefined;

  return {
    capture_type: capture,
    encoder_type: encoder,
    decoder_type: decoder,
    renderer_type: renderer,
    render_display: renderer ? true : undefined,
    zero_copy: renderer === "d3d11" && encoder.startsWith("nvenc") ? true : undefined,
    resolution: [1920, 1080],
    fps: 60,
    bitrate: 5000000,
    duration_ms: 10000,
    warmup_ms: 2000,
    input_source: capture === "synthetic" ? "synthetic" : "screen",
  };
}

const lanAutomationCommands: LanE2EAutomationCommands = {
  serviceBootstrapIfNeeded: commands.serviceBootstrapIfNeeded,
  serviceWaitForHealthy: (timeoutSecs = 10) => commands.serviceWaitForHealthy(timeoutSecs),
  ipcRuntimeSnapshot: commands.ipcRuntimeSnapshot,
  getHardwareInfo: commands.getHardwareInfo,
  ipcRegisterDevice: commands.ipcRegisterDevice,
  ipcRefreshLanDiscovery: commands.ipcRefreshLanDiscovery,
  ipcStartLanRemoteSession: commands.ipcStartLanRemoteSession,
  ipcConfigureMediaAdaptation: commands.ipcConfigureMediaAdaptation,
  ipcListRemoteCaptureSources: commands.ipcListRemoteCaptureSources,
  ipcSelectRemoteCaptureSource: commands.ipcSelectRemoteCaptureSource,
  ipcListRemoteDisplayModes: commands.ipcListRemoteDisplayModes,
  ipcSetRemoteDisplayMode: commands.ipcSetRemoteDisplayMode,
  ipcRestoreRemoteDisplayMode: commands.ipcRestoreRemoteDisplayMode,
  ipcStartReceiver: commands.ipcStartReceiver,
  openRemoteDisplayWindow: commands.openRemoteDisplayWindow,
  ipcSessionSnapshot: commands.ipcSessionSnapshot,
  ipcProbeSnapshot: commands.ipcProbeSnapshot,
  ipcMediaPipelineSnapshot: commands.ipcMediaPipelineSnapshot,
  ipcStopSession: commands.ipcStopSession,
};

export function E2ETestPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const autorunStartedRef = useRef(false);
  const [isRunning, setIsRunning] = useState(false);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<HarnessMetrics | null>(null);
  const [capturedFrame, setCapturedFrame] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [lanRunState, setLanRunState] = useState<LanE2EStatus | "idle">("idle");
  const [lanReport, setLanReport] = useState<LanE2EAutomationReport | null>(null);
  const [lanScenarioId, setLanScenarioId] =
    useState<CrossDeviceScenarioId>("cross.e2e.remote_display_smoke");
  const currentConfig = buildDefaultConfig(capabilities);

  useEffect(() => {
    let cancelled = false;

    commands.testGetCapabilities().then((result) => {
      if (!cancelled && result.ok) {
        setCapabilities(result.value);
      }
    });

    return () => {
      cancelled = true;
    };
  }, []);

  // Poll for run status
  useEffect(() => {
    if (!currentRunId || !isRunning) return;

    const interval = setInterval(async () => {
      const runResult = await commands.testGetRun(currentRunId);
      if (runResult.ok && runResult.value) {
        if (runResult.value.status !== "running") {
          setIsRunning(false);
          // Navigate to detail page when done
          navigate(`/test/run/${currentRunId}`);
        }
      }
    }, 500);

    return () => clearInterval(interval);
  }, [currentRunId, isRunning, navigate]);

  // Poll for metrics using legacy harness
  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const metricsResult = await commands.testHarnessGetMetrics();
      if (metricsResult.ok) {
        setMetrics(metricsResult.value);
      }

      const framesResult = await commands.testHarnessGetFrames();
      if (framesResult.ok && framesResult.value[0]) {
        setCapturedFrame(framesResult.value[0][0]);
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    const scenarioId =
      currentConfig.capture_type === "linux"
        ? "e2e.linux_local"
        : currentConfig.capture_type === "macos"
          ? "e2e.macos_local"
          : "e2e.local";
    const result = await commands.testStartRun({
      scenarioId,
      config: currentConfig,
    });

    if (result.ok) {
      setCurrentRunId(result.value);
      setIsRunning(true);
    }
  };

  const handleStop = async () => {
    if (currentRunId) {
      await commands.testStopRun(currentRunId);
      setIsRunning(false);
    }
  };

  const startLanE2E = async (optionOverrides: LanE2EAutomationOptions = {}) => {
    setLanRunState("running");
    setLanReport(null);
    publishLanAutomationStatus("running");

    const report = await runLanE2EAutomation(lanAutomationCommands, {
      ...optionOverrides,
      scenarioId: optionOverrides.scenarioId ?? lanScenarioId,
      transportKind: optionOverrides.transportKind ?? "quic",
      timeoutMs: optionOverrides.timeoutMs ?? 15_000,
      sampleIntervalMs: optionOverrides.sampleIntervalMs ?? 500,
      minDecodedFrames: optionOverrides.minDecodedFrames ?? 20,
      minFps: optionOverrides.minFps ?? 2,
    });

    setLanReport(report);
    setLanRunState(report.status);
    publishLanAutomationReport(report);
    void commands.automationWriteReport(report).then((result) => {
      if (!result.ok) {
        console.error("Failed to write LAN E2E automation report", result.error);
      }
    });
  };

  const handleStartLanE2E = async () => {
    await startLanE2E();
  };

  useEffect(() => {
    if (autorunStartedRef.current || searchParams.get("autorun") !== "lan-e2e") return;
    autorunStartedRef.current = true;
    void startLanE2E(buildLanAutomationOptionsFromSearchParams(searchParams));
  }, [searchParams]);

  return (
    <div className="p-6">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground">端到端本地测试</h1>
        <p className="text-muted-foreground">
          测试完整的采集→编码→解码→渲染流程
        </p>
        {currentRunId && (
          <p className="text-xs text-muted-foreground mt-1">
            运行 ID: {currentRunId}
          </p>
        )}
      </div>

      {/* Configuration Panel */}
      <section className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">测试配置</h2>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
          <div>
            <span className="text-muted-foreground">采集:</span> {currentConfig.capture_type}
          </div>
          <div>
            <span className="text-muted-foreground">编码:</span> {currentConfig.encoder_type}
          </div>
          <div>
            <span className="text-muted-foreground">解码:</span> {currentConfig.decoder_type}
          </div>
          <div>
            <span className="text-muted-foreground">渲染:</span> {currentConfig.renderer_type ?? "none"}
          </div>
          <div>
            <span className="text-muted-foreground">内存路径:</span>{" "}
            {currentConfig.zero_copy ? "D3D11 Shared" : "CPU"}
          </div>
          <div>
            <span className="text-muted-foreground">分辨率:</span> 1920x1080
          </div>
          <div>
            <span className="text-muted-foreground">帧率:</span> 60 FPS
          </div>
          <div>
            <span className="text-muted-foreground">码率:</span> 5000 kbps
          </div>
          <div>
            <span className="text-muted-foreground">时长:</span> 30 秒
          </div>
        </div>
      </section>

      {/* Control Panel */}
      <section className="mb-6">
        {!isRunning ? (
          <button
            onClick={handleStart}
            className="flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors"
          >
            <Play className="h-4 w-4" />
            启动测试
          </button>
        ) : (
          <button
            onClick={handleStop}
            className="flex items-center gap-2 px-4 py-2 bg-destructive text-destructive-foreground rounded-lg hover:bg-destructive/90 transition-colors"
          >
            <Square className="h-4 w-4" />
            停止测试
          </button>
        )}
      </section>

      {/* LAN E2E Automation */}
      <section
        className="bg-card rounded-lg border p-4 mb-6"
        data-lan-e2e-status={lanRunState}
      >
        <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
          <div>
            <h2 className="text-lg font-semibold mb-2">LAN E2E 自动化</h2>
            <p className="text-sm text-muted-foreground max-w-3xl">
              两端打开同款 Rdesk 后，自动拉起/检查 mrd-service，刷新局域网发现，按跨设备场景执行
              discovery、远程显示 smoke、媒体画像或故障恢复预检，并输出结构化报告。
            </p>
          </div>
          <div className="flex flex-col gap-2 sm:min-w-72">
            <label className="text-xs font-medium text-muted-foreground" htmlFor="lan-e2e-scenario">
              跨设备场景
            </label>
            <select
              id="lan-e2e-scenario"
              aria-label="跨设备场景"
              value={lanScenarioId}
              disabled={lanRunState === "running"}
              onChange={(event) => {
                setLanScenarioId(parseCrossDeviceScenarioId(event.target.value) ?? "cross.e2e.remote_display_smoke");
              }}
              className="rounded-lg border bg-background px-3 py-2 text-sm"
            >
              <option value="cross.e2e.discovery">发现/配对预检</option>
              <option value="cross.e2e.remote_display_smoke">远程显示 Smoke</option>
              <option value="cross.e2e.media_profile">媒体画像校验</option>
              <option value="cross.fault.recovery">故障恢复预检</option>
            </select>
            <button
              type="button"
              onClick={handleStartLanE2E}
              disabled={lanRunState === "running"}
              className="flex items-center justify-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60 transition-colors"
            >
              <Play className="h-4 w-4" />
              {lanRunState === "running" ? "跨设备 E2E 运行中" : "开始跨设备 E2E"}
            </button>
          </div>
        </div>

        <div className="mt-4 grid gap-3 md:grid-cols-5">
          <AutomationStatusCard
            label="状态"
            value={formatLanStatus(lanRunState)}
            tone={lanRunState === "completed" ? "success" : lanRunState === "failed" ? "danger" : lanRunState === "skipped" ? "warning" : "default"}
          />
          <AutomationStatusCard
            label="场景"
            value={formatCrossDeviceScenario(lanReport?.scenarioId ?? lanScenarioId)}
          />
          <AutomationStatusCard
            label="目标设备"
            value={lanReport?.peer?.device_name ?? lanReport?.peer?.device_id ?? "等待发现"}
          />
          <AutomationStatusCard
            label="捕获源"
            value={formatCaptureSourceSummary(lanReport)}
          />
          <AutomationStatusCard
            label="探针反馈"
            value={formatProbeSummary(lanReport)}
          />
        </div>

        {lanReport && (
          <div className="mt-4 rounded-lg border bg-muted/30 p-3 text-sm">
            <div className="flex flex-wrap gap-x-6 gap-y-2">
              <span>
                <span className="text-muted-foreground">Scenario:</span>{" "}
                {formatCrossDeviceScenario(lanReport.scenarioId)}
              </span>
              <span>
                <span className="text-muted-foreground">Session:</span>{" "}
                {lanReport.sessionId ?? "n/a"}
              </span>
              <span>
                <span className="text-muted-foreground">Peer:</span>{" "}
                {lanReport.peer?.device_name ?? lanReport.peer?.device_id ?? "n/a"}
              </span>
              <span>
                <span className="text-muted-foreground">Window:</span>{" "}
                {lanReport.displayWindow?.label ?? "n/a"}
              </span>
              <span>
                <span className="text-muted-foreground">Capture:</span>{" "}
                {formatCaptureSourceSummary(lanReport)}
              </span>
              <span>
                <span className="text-muted-foreground">Requested:</span>{" "}
                {formatRequestedProfile(lanReport)}
              </span>
            </div>
            {(lanReport.status === "failed" || lanReport.status === "skipped") && (
              <p className={`mt-2 ${lanReport.status === "skipped" ? "text-yellow-500" : "text-red-500"}`}>
                {lanReport.failureReason}: {lanReport.errorMessage ?? "未知错误"}
              </p>
            )}
          </div>
        )}
      </section>

      {/* Real-time Metrics */}
      {metrics && (
        <section className="mb-6 grid grid-cols-2 md:grid-cols-4 gap-4">
          <MetricCard
            icon={<Activity className="h-4 w-4" />}
            label="Pipeline FPS"
            value={`${metrics.capture_fps.toFixed(1)} FPS`}
            color={getFpsColor(metrics.capture_fps)}
          />
          <MetricCard
            icon={<Clock className="h-4 w-4" />}
            label="编码延迟 P95"
            value={`${metrics.encode_latency_p95_ms.toFixed(2)} ms`}
            color={getLatencyColor(metrics.encode_latency_p95_ms, 10, 20)}
          />
          <MetricCard
            icon={<Zap className="h-4 w-4" />}
            label="解码延迟 P95"
            value={`${metrics.decode_latency_p95_ms.toFixed(2)} ms`}
            color={getLatencyColor(metrics.decode_latency_p95_ms, 10, 30)}
          />
          <MetricCard
            icon={<Video className="h-4 w-4" />}
            label="总帧数"
            value={`${metrics.frame_count}`}
          />
        </section>
      )}

      {/* Frame Display */}
      <section className="bg-card rounded-lg border p-4">
        <h2 className="text-sm font-medium mb-2 flex items-center gap-2">
          <Monitor className="h-4 w-4" />
          实时画面
        </h2>
        <div className="aspect-video bg-black rounded flex items-center justify-center">
          {capturedFrame ? (
            <img
              src={`data:image/png;base64,${capturedFrame}`}
              alt="Captured frame"
              className="max-w-full max-h-full"
            />
          ) : (
            <p className="text-muted-foreground text-sm">
              {isRunning ? "正在捕获..." : "等待启动..."}
            </p>
          )}
        </div>
      </section>
    </div>
  );
}

function MetricCard({
  icon,
  label,
  value,
  color = "text-foreground",
}: {
  icon: ReactNode;
  label: string;
  value: string;
  color?: string;
}) {
  return (
    <div className="bg-card rounded-lg p-4 border">
      <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
        {icon}
        <span>{label}</span>
      </div>
      <div className={`text-xl font-semibold ${color}`}>{value}</div>
    </div>
  );
}

function getFpsColor(fps: number): string {
  if (fps >= 30) return "text-green-500";
  if (fps >= 15) return "text-yellow-500";
  return "text-red-500";
}

function getLatencyColor(ms: number, good: number, warning: number): string {
  if (ms <= good) return "text-green-500";
  if (ms <= warning) return "text-yellow-500";
  return "text-red-500";
}

function AutomationStatusCard({
  label,
  value,
  tone = "default",
}: {
  label: string;
  value: string;
  tone?: "default" | "success" | "danger" | "warning";
}) {
  const color =
    tone === "success"
      ? "text-green-500"
      : tone === "danger"
        ? "text-red-500"
        : tone === "warning"
          ? "text-yellow-500"
          : "text-foreground";

  return (
    <div className="rounded-lg border bg-background/60 p-3">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={`mt-1 text-sm font-semibold ${color}`}>{value}</div>
    </div>
  );
}

function formatLanStatus(status: LanE2EStatus | "idle"): string {
  switch (status) {
    case "running":
      return "LAN E2E 运行中";
    case "completed":
      return "LAN E2E 完成";
    case "failed":
      return "LAN E2E 失败";
    case "skipped":
      return "LAN E2E 跳过";
    default:
      return "等待启动";
  }
}

function formatCrossDeviceScenario(scenarioId: CrossDeviceScenarioId): string {
  switch (scenarioId) {
    case "cross.e2e.discovery":
      return "发现/配对预检";
    case "cross.e2e.remote_display_smoke":
      return "远程显示 Smoke";
    case "cross.e2e.media_profile":
      return "媒体画像校验";
    case "cross.fault.recovery":
      return "故障恢复预检";
    default:
      return "LAN 远程显示";
  }
}

function formatProbeSummary(report: LanE2EAutomationReport | null): string {
  const probe = report?.probeSnapshot;
  if (!probe) return "等待采样";
  const fps = probe.current_fps ?? 0;
  const seconds = ((report.sampleDurationMs ?? 0) / 1000).toFixed(1);
  const mediaProbe = probe.media_probe_valid
    ? `, media ${formatMediaProbeTarget(probe)} ${probe.media_probe_format ?? "unknown"} #${probe.last_media_sequence ?? "-"} ${
        probe.last_media_payload_hash ?? ""
      }`
    : "";
  return `${formatValidationMode(report.validationMode)} decoded ${probe.frames_decoded}, received ${probe.frames_received}, fps ${fps}, ${seconds}s${mediaProbe}`;
}

function formatCaptureSourceSummary(report: LanE2EAutomationReport | null): string {
  const source = report?.captureSourceSelection?.source ?? report?.captureSource;
  if (!source) return "等待选择";
  return `${formatCaptureSourceKind(source.source_kind)} / ${source.title} / ${source.width}x${source.height}`;
}

function formatCaptureSourceKind(kind: string): string {
  switch (kind) {
    case "display_shared":
      return "全屏 shared";
    case "display":
      return "全屏 copy";
    case "window":
      return "窗口";
    default:
      return kind;
  }
}

function formatRequestedProfile(report: LanE2EAutomationReport | null): string {
  const profile = report?.requestedProfile;
  if (!profile) return "n/a";
  return `${profile.width}x${profile.height} @ ${profile.fps} FPS / ${profile.bitrate_mbps} Mbps`;
}

function formatMediaProbeTarget(
  probe: NonNullable<LanE2EAutomationReport["probeSnapshot"]>
): string {
  const width = probe.media_probe_width ?? 0;
  const height = probe.media_probe_height ?? 0;
  const targetFps = probe.media_probe_target_fps ?? 0;
  const targetBitrate = probe.media_probe_target_bitrate_mbps ?? 0;
  if (width > 0 && height > 0 && targetFps > 0) {
    const bitrate = targetBitrate > 0 ? ` target ${targetBitrate}Mbps` : "";
    return `${width}x${height}@${targetFps}${bitrate}`;
  }
  return "unknown-target";
}

function formatValidationMode(mode: LanE2EAutomationReport["validationMode"]): string {
  return mode === "webrtc_rtp" ? "WebRTC RTP" : "QUIC datagram";
}

function buildLanAutomationOptionsFromSearchParams(
  searchParams: URLSearchParams
): LanE2EAutomationOptions {
  return {
    scenarioId: parseCrossDeviceScenarioId(searchParams.get("scenarioId") ?? searchParams.get("scenario")),
    targetDeviceId: searchParams.get("targetDeviceId") ?? searchParams.get("target") ?? undefined,
    transportKind: parseTransportKind(searchParams.get("transport")),
    timeoutMs: parsePositiveNumber(searchParams.get("timeoutMs")),
    minSampleDurationMs: parsePositiveNumber(searchParams.get("minSampleDurationMs")),
    minDecodedFrames: parsePositiveNumber(searchParams.get("minDecodedFrames")),
    minFps: parsePositiveNumber(searchParams.get("minFps")),
    stopOnComplete: parseOptionalBoolean(searchParams.get("stopOnComplete")),
    displayModePolicy: parseDisplayModePolicy(searchParams.get("displayModePolicy")),
    preferredCaptureSourceId:
      searchParams.get("captureSourceId") ?? searchParams.get("sourceId") ?? undefined,
    preferredCaptureSourceKind:
      searchParams.get("captureSourceKind") ?? searchParams.get("captureKind") ?? undefined,
    expectedPeerBuildId: searchParams.get("expectedPeerBuildId") ?? undefined,
    adaptive: parseOptionalBoolean(searchParams.get("adaptive")),
    requestedProfile: parseRequestedProfile(searchParams),
  };
}

function parseRequestedProfile(searchParams: URLSearchParams): LanE2EAutomationOptions["requestedProfile"] {
  const width = parsePositiveNumber(searchParams.get("width") ?? searchParams.get("profileWidth"));
  const height = parsePositiveNumber(searchParams.get("height") ?? searchParams.get("profileHeight"));
  const fps = parsePositiveNumber(searchParams.get("fps") ?? searchParams.get("profileFps"));
  const bitrate = parsePositiveNumber(
    searchParams.get("bitrateMbps") ?? searchParams.get("profileBitrateMbps")
  );
  if (!width || !height || !fps || !bitrate) return undefined;
  return {
    width,
    height,
    fps,
    bitrate_mbps: bitrate,
    codec: "h264",
  };
}

function parseCrossDeviceScenarioId(value: string | null): CrossDeviceScenarioId | undefined {
  if (
    value === "lan.e2e.remote_display" ||
    value === "cross.e2e.discovery" ||
    value === "cross.e2e.remote_display_smoke" ||
    value === "cross.e2e.media_profile" ||
    value === "cross.fault.recovery"
  ) {
    return value;
  }
  return undefined;
}

function parseTransportKind(value: string | null): LanE2EAutomationOptions["transportKind"] {
  return value === "webrtc" ? "webrtc" : value === "quic" ? "quic" : undefined;
}

function parseDisplayModePolicy(value: string | null): LanE2EAutomationOptions["displayModePolicy"] {
  return value === "temporary" || value === "required" || value === "none" ? value : undefined;
}

function parsePositiveNumber(value: string | null): number | undefined {
  if (!value) return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

function parseOptionalBoolean(value: string | null): boolean | undefined {
  if (value === "true") return true;
  if (value === "false") return false;
  return undefined;
}

function publishLanAutomationStatus(status: LanE2EStatus): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.lanE2eStatus = status;
}

function publishLanAutomationReport(report: LanE2EAutomationReport): void {
  if (typeof window === "undefined") return;

  const automationWindow = window as Window & {
    __MRD_LAN_E2E_REPORT__?: LanE2EAutomationReport;
  };
  automationWindow.__MRD_LAN_E2E_REPORT__ = report;
  document.documentElement.dataset.lanE2eStatus = report.status;
  window.dispatchEvent(new CustomEvent("mrd:lan-e2e-report", { detail: report }));
}
