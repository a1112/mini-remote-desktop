import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Play, Grid3x3, CheckCircle2, XCircle, Clock, Loader2, Square, RefreshCw } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type {
  EnvironmentSnapshot,
  LanPeerInfo,
  MediaProfile,
  TestConfig,
  TestRun,
  TestRunSummary,
} from "../../adapters/tauri/types";
import {
  buildCapabilitySnapshotFromIpc,
  buildCapabilitySnapshotFromEnvironment,
  capabilityForOption,
  capabilityOptionState,
  environmentSnapshotFromCapabilitySnapshot,
  evaluateCapabilityCombination,
  shouldShowCapabilityOptionForSnapshot,
  type CapabilitySnapshot,
} from "../../services/capabilityMatrix";
import {
  runLanE2EAutomation,
  type LanE2EAutomationCommands,
  type LanE2EAutomationReport,
} from "../../services/lanE2eAutomationService";
import {
  readShowUnavailableCapabilities,
  useShowUnavailableCapabilities,
} from "./useCapabilityVisibility";

interface MatrixDimension {
  id: string;
  name: string;
  options: MatrixOption[];
}

interface MatrixOption {
  id: string;
  name: string;
  enabled: boolean;
  available?: boolean;
  statusLabel?: string;
  unavailableReason?: string;
  scopeBlockedReason?: string;
  defaultEnabledOn?: HostOs[];
}

type HostOs = "windows" | "macos" | "linux" | "other";

function isHevcEncoder(encoder?: TestConfig["encoder_type"]): boolean {
  return encoder === "nvenc_hevc" || encoder === "nvenc_hevc_main10";
}

const MATRIX_DIMENSIONS: MatrixDimension[] = [
  {
    id: "capture",
    name: "捕获",
    options: [
      { id: "dxgi", name: "DXGI", enabled: true },
      { id: "winrt", name: "WinRT", enabled: false },
      { id: "macos", name: "macOS", enabled: false, defaultEnabledOn: ["macos"] },
      { id: "linux", name: "Linux", enabled: false },
      { id: "synthetic", name: "Synthetic", enabled: false, defaultEnabledOn: ["linux"] },
    ],
  },
  {
    id: "encoder",
    name: "编码器",
    options: [
      { id: "nvenc_h264", name: "NVENC H.264", enabled: true },
      { id: "nvenc_hevc", name: "NVENC HEVC Main", enabled: false },
      { id: "nvenc_hevc_main10", name: "NVENC HEVC Main10", enabled: false },
      { id: "openh264", name: "OpenH264", enabled: true },
      { id: "nvenc_av1", name: "NVENC AV1", enabled: false },
      {
        id: "videotoolbox_h264",
        name: "VideoToolbox H.264",
        enabled: false,
        defaultEnabledOn: ["macos"],
      },
    ],
  },
  {
    id: "decoder",
    name: "解码器",
    options: [
      { id: "none", name: "None / encode only", enabled: false },
      { id: "nvdec", name: "NVDEC", enabled: true },
      { id: "software", name: "软件", enabled: true },
      {
        id: "linux_h264",
        name: "Linux H.264 HW",
        enabled: false,
        defaultEnabledOn: ["linux"],
      },
      {
        id: "linux_hevc",
        name: "Linux HEVC HW",
        enabled: false,
      },
      {
        id: "linux_hevc_main10",
        name: "Linux HEVC Main10 HW",
        enabled: false,
      },
      {
        id: "videotoolbox",
        name: "VideoToolbox",
        enabled: false,
        defaultEnabledOn: ["macos"],
      },
    ],
  },
  {
    id: "transport",
    name: "传输层",
    options: [
      { id: "loopback", name: "Loopback", enabled: true },
      { id: "webrtc", name: "WebRTC RTP", enabled: false },
      { id: "quic", name: "QUIC Datagram", enabled: false },
    ],
  },
  {
    id: "renderer",
    name: "渲染",
    options: [
      { id: "renderer_none", name: "No display", enabled: true },
      { id: "d3d11", name: "DX11 popup", enabled: false },
      { id: "opengl", name: "OpenGL", enabled: false },
      { id: "d3d12_native", name: "DX12 native", enabled: false },
      { id: "macos", name: "Metal", enabled: false },
      { id: "linux", name: "Linux", enabled: false },
    ],
  },
  {
    id: "memory",
    name: "Memory",
    options: [
      { id: "cpu", name: "CPU", enabled: true },
      { id: "d3d11_shared", name: "D3D11 shared texture", enabled: false },
    ],
  },
  {
    id: "resolution",
    name: "分辨率",
    options: [
      { id: "1280x720", name: "720p", enabled: true },
      { id: "1920x1080", name: "1080p", enabled: true },
      { id: "2560x1440", name: "1440p", enabled: false },
      { id: "2560x1600", name: "1600p", enabled: false },
      { id: "3440x1440", name: "UWQHD", enabled: false },
      { id: "3840x2160", name: "4K", enabled: false },
    ],
  },
  {
    id: "fps",
    name: "帧率",
    options: [
      { id: "24", name: "24 FPS", enabled: false },
      { id: "30", name: "30 FPS", enabled: true },
      { id: "45", name: "45 FPS", enabled: false },
      { id: "60", name: "60 FPS", enabled: true },
      { id: "90", name: "90 FPS", enabled: false },
      { id: "120", name: "120 FPS", enabled: false },
      { id: "144", name: "144 FPS", enabled: false },
      { id: "165", name: "165 FPS", enabled: false },
      { id: "180", name: "180 FPS", enabled: false },
      { id: "249", name: "249 FPS", enabled: false },
    ],
  },
  {
    id: "bitrate",
    name: "码率",
    options: [
      { id: "3000000", name: "3 Mbps", enabled: false },
      { id: "5000000", name: "5 Mbps", enabled: true },
      { id: "8000000", name: "8 Mbps", enabled: false },
      { id: "12000000", name: "12 Mbps", enabled: false },
      { id: "20000000", name: "20 Mbps", enabled: false },
      { id: "50000000", name: "50 Mbps", enabled: false },
      { id: "80000000", name: "80 Mbps", enabled: false },
      { id: "100000000", name: "100 Mbps", enabled: false },
      { id: "120000000", name: "120 Mbps", enabled: false },
    ],
  },
  {
    id: "adaptive",
    name: "自适应",
    options: [
      { id: "off", name: "固定", enabled: true },
      { id: "on", name: "关键帧阶梯", enabled: false },
    ],
  },
  {
    id: "duration",
    name: "时长",
    options: [
      { id: "3000", name: "3 秒", enabled: false },
      { id: "5000", name: "5 秒", enabled: true },
      { id: "10000", name: "10 秒", enabled: false },
      { id: "30000", name: "30 秒", enabled: false },
    ],
  },
];

function normalizeHostOs(osType?: string): HostOs {
  const normalized = osType?.toLowerCase() ?? "";
  if (normalized.includes("windows") || normalized === "win32") return "windows";
  if (normalized.includes("mac") || normalized === "darwin") return "macos";
  if (normalized.includes("linux")) return "linux";
  return "other";
}

function defaultCapturesForOs(os: HostOs): string[] {
  if (os === "windows") return ["dxgi", "winrt", "synthetic"];
  if (os === "macos") return ["macos", "synthetic"];
  if (os === "linux") return ["linux", "synthetic"];
  return ["synthetic"];
}

function defaultEncodersForOs(os: HostOs): string[] {
  if (os === "windows") {
    return ["nvenc_h264", "nvenc_hevc", "nvenc_hevc_main10", "openh264", "nvenc_av1"];
  }
  if (os === "macos") return ["videotoolbox_h264", "openh264"];
  return ["openh264"];
}

function defaultDecodersForOs(os: HostOs): string[] {
  if (os === "windows") return ["nvdec", "software", "none"];
  if (os === "macos") return ["software", "none"];
  if (os === "linux") return ["linux_h264", "linux_hevc", "linux_hevc_main10", "software", "none"];
  return ["software", "none"];
}

function defaultRenderersForOs(os: HostOs): string[] {
  if (os === "windows") return ["none", "d3d11", "opengl"];
  if (os === "macos") return ["none", "macos"];
  if (os === "linux") return ["none", "linux"];
  return ["none"];
}

function defaultMemoryModesForOs(os: HostOs): string[] {
  return os === "windows" ? ["cpu", "d3d11_shared"] : ["cpu"];
}

function optionEnabledForOs(option: MatrixOption, os: HostOs): boolean {
  return option.defaultEnabledOn ? option.defaultEnabledOn.includes(os) : option.enabled;
}

function hasLinuxHardwareDecoder(availableDecoders: string[]): boolean {
  return availableDecoders.some((decoder) =>
    decoder === "linux_h264" ||
    decoder === "linux_hevc" ||
    decoder === "linux_hevc_main10"
  );
}

function shouldEnableOptionByDefault(
  dimensionId: string,
  option: MatrixOption,
  os: HostOs,
  availableDecoders: string[]
): boolean {
  if (
    os === "linux" &&
    dimensionId === "decoder" &&
    option.id === "software" &&
    hasLinuxHardwareDecoder(availableDecoders)
  ) {
    return false;
  }
  return optionEnabledForOs(option, os);
}

function createMatrixDimensions(
  capabilities?: EnvironmentSnapshot | null,
  capabilitySnapshot?: CapabilitySnapshot | null,
  showUnavailable = false
): MatrixDimension[] {
  const os = normalizeHostOs(capabilities?.os_type ?? "windows");
  const availableCaptures = capabilities?.available_captures ?? defaultCapturesForOs(os);
  const availableEncoders = capabilities?.available_encoders ?? defaultEncodersForOs(os);
  const availableDecoders = [
    "none",
    ...(capabilities?.available_decoders ?? defaultDecodersForOs(os)),
  ].filter((value, index, values) => values.indexOf(value) === index);
  const availableRenderers = capabilities?.available_renderers ?? defaultRenderersForOs(os);
  const availableMemoryModes =
    capabilities?.available_memory_modes ?? defaultMemoryModesForOs(os);

  const optionAvailable = (dimensionId: string, optionId: string): boolean => {
    if (capabilitySnapshot) {
      return capabilityOptionState(capabilitySnapshot, dimensionId, optionId) !== "disabled";
    }
    switch (dimensionId) {
      case "capture":
        return availableCaptures.includes(optionId);
      case "encoder":
        return availableEncoders.includes(optionId);
      case "decoder":
        return availableDecoders.includes(optionId);
      case "renderer":
        if (optionId === "renderer_none") {
          return availableRenderers.includes("none");
        }
        if (optionId === "d3d12_native") {
          return (
            availableRenderers.includes("d3d12") ||
            availableRenderers.includes("d3d12_native")
          );
        }
        return availableRenderers.includes(optionId);
      case "memory":
        return availableMemoryModes.includes(optionId);
      default:
        return true;
    }
  };

  return MATRIX_DIMENSIONS.map((dimension) => ({
    ...dimension,
    options: dimension.options
      .map((option) => {
        const state = capabilityOptionState(capabilitySnapshot, dimension.id, option.id);
        const capability = capabilityForOption(capabilitySnapshot, dimension.id, option.id);
        const available = optionAvailable(dimension.id, option.id);
        const selectable = state === "selectable" && available;
        return {
          ...option,
          available,
          statusLabel: capability?.status,
          unavailableReason: capability?.reason ?? capability?.detail,
          enabled:
            selectable &&
            shouldEnableOptionByDefault(dimension.id, option, os, availableDecoders),
        };
      })
      .filter(
        (option) =>
          shouldShowCapabilityOptionForSnapshot(
            capabilitySnapshot,
            dimension.id,
            option.id,
            showUnavailable
          ) && (showUnavailable || option.available)
      ),
  })).filter((dimension) => dimension.options.length > 0);
}

interface MatrixTest {
  id: string;
  config: TestConfig;
  status: "pending" | "running" | "completed" | "failed" | "skipped";
  result?: TestRunSummary;
  duration?: number;
  skipReason?: string;
  failureReason?: string;
}

interface SelectedMatrixOption {
  dimensionId: string;
  option: MatrixOption;
}

interface MatrixGenerationResult {
  tests: MatrixTest[];
  truncated: boolean;
}

function buildConfig(options: SelectedMatrixOption[]): TestConfig {
  const config: TestConfig = {};
  options.forEach(({ dimensionId, option }) => {
    switch (dimensionId) {
      case "capture":
        config.capture_type = option.id as TestConfig["capture_type"];
        break;
      case "encoder":
        config.encoder_type = option.id as TestConfig["encoder_type"];
        break;
      case "decoder":
        config.decoder_type = option.id as TestConfig["decoder_type"];
        break;
      case "transport":
        config.transport_kind = option.id as TestConfig["transport_kind"];
        break;
      case "renderer":
        if (option.id === "d3d11") {
          config.renderer_type = "d3d11";
          config.render_display = true;
        } else if (option.id === "d3d12_native") {
          config.renderer_type = "d3d12";
          config.render_display = true;
        } else if (option.id === "opengl") {
          config.renderer_type = "opengl";
          config.render_display = true;
        } else if (option.id === "macos") {
          config.renderer_type = "macos";
          config.render_display = true;
        } else if (option.id === "linux") {
          config.renderer_type = "linux";
          config.render_display = true;
        } else {
          config.render_display = false;
        }
        break;
      case "memory":
        config.zero_copy = option.id === "d3d11_shared";
        break;
      case "resolution": {
        const [w, h] = option.id.split("x").map(Number);
        if (w && h) {
          config.resolution = [w, h];
        }
        break;
      }
      case "fps":
        config.fps = Number(option.id);
        break;
      case "bitrate":
        config.bitrate = Number(option.id);
        break;
      case "adaptive":
        config.adaptive_media = option.id === "on";
        break;
      case "duration":
        config.duration_ms = Number(option.id);
        break;
    }
  });

  config.transport_kind ??= "loopback";
  config.render_display ??= false;
  config.zero_copy ??= false;
  config.bitrate ??= 5000000;
  config.duration_ms ??= 5000;
  config.warmup_ms = 1000;
  config.visual_preview = false;

  return config;
}

interface MatrixAcceptanceResult {
  acceptable: boolean;
  reason?: string;
}

function evaluateMatrixRun(
  config: TestConfig,
  summary?: TestRunSummary
): MatrixAcceptanceResult {
  if (!summary || summary.frame_count <= 0 || summary.error_message) {
    return {
      acceptable: false,
      reason: summary?.error_message ?? "No frames were produced",
    };
  }

  const targetFps = Math.max(1, config.fps ?? 60);
  const minFps = minimumExpectedFps(config, targetFps);
  const captureFps = summary.capture_fps ?? 0;
  if (captureFps < minFps) {
    return {
      acceptable: false,
      reason: `Pipeline FPS ${captureFps.toFixed(1)} < ${minFps.toFixed(1)} expected`,
    };
  }

  const maxTotalP95Ms = maximumExpectedLatencyMs(config, targetFps);
  const totalLatencyP95 = summary.total_latency_p95 ?? Number.POSITIVE_INFINITY;
  if (totalLatencyP95 > maxTotalP95Ms) {
    const [slowestStage, slowestStageMs] = slowestPipelineStage(summary);
    return {
      acceptable: false,
      reason: `${slowestStage} P95 ${slowestStageMs.toFixed(
        2
      )} ms; total P95 ${totalLatencyP95.toFixed(2)} ms > ${maxTotalP95Ms.toFixed(2)} ms budget`,
    };
  }

  return { acceptable: true };
}

function minimumExpectedFps(config: TestConfig, targetFps: number): number {
  if (config.capture_type === "macos") {
    if (config.encoder_type === "openh264") {
      return targetFps * 0.3;
    }
    if (config.encoder_type === "videotoolbox_h264") {
      return targetFps * 0.35;
    }
  }
  if (config.encoder_type === "openh264") {
    return targetFps * 0.35;
  }
  if (config.decoder_type === "software") {
    return targetFps * 0.45;
  }
  return targetFps * 0.6;
}

function maximumExpectedLatencyMs(config: TestConfig, targetFps: number): number {
  const frameBudgetMs = 1000 / targetFps;
  if (config.encoder_type === "openh264") {
    return Math.max(120, frameBudgetMs * 8);
  }
  if (config.decoder_type === "software") {
    return Math.max(80, frameBudgetMs * 5);
  }
  return Math.max(100, frameBudgetMs * 4);
}

function slowestPipelineStage(summary: TestRunSummary): readonly [string, number] {
  const stages = [
    ["encode", summary.encode_latency_p95 ?? 0],
    ["transport", summary.transport_latency_p95 ?? 0],
    ["decode", summary.decode_latency_p95 ?? 0],
  ] as const;

  return stages.reduce((slowest, current) =>
    current[1] > slowest[1] ? current : slowest
  );
}

function unsupportedMatrixReason(config: TestConfig): string | null {
  if (
    config.zero_copy &&
    config.capture_type !== "dxgi" &&
    config.capture_type !== "winrt"
  ) {
    return "D3D11 shared texture path requires DXGI or WinRT capture";
  }
  if (
    config.zero_copy &&
    config.encoder_type !== "none" &&
    config.encoder_type !== "nvenc_h264" &&
    config.encoder_type !== "nvenc_hevc" &&
    config.encoder_type !== "nvenc_hevc_main10" &&
    config.encoder_type !== "nvenc_av1"
  ) {
    return "D3D11 shared texture input requires direct render or NVENC GPU encoders";
  }
  if (isHevcEncoder(config.encoder_type) && config.decoder_type === "linux_h264") {
    return "Linux H.264 hardware decoder cannot decode NVENC HEVC output";
  }
  if (config.encoder_type === "nvenc_hevc_main10" && config.decoder_type === "linux_hevc") {
    return "NVENC HEVC Main10 requires the Linux HEVC Main10 decoder path";
  }
  if (config.encoder_type === "nvenc_av1" && config.decoder_type === "linux_h264") {
    return "Linux H.264 hardware decoder cannot decode NVENC AV1 output";
  }
  if (
    config.encoder_type === "nvenc_av1" &&
    (config.decoder_type === "linux_hevc" || config.decoder_type === "linux_hevc_main10")
  ) {
    return "Linux HEVC hardware decoder cannot decode NVENC AV1 output";
  }
  if (
    (config.encoder_type === "nvenc_h264" ||
      config.encoder_type === "openh264" ||
      config.encoder_type === "videotoolbox_h264") &&
    (config.decoder_type === "linux_hevc" || config.decoder_type === "linux_hevc_main10")
  ) {
    return "Linux HEVC hardware decoder cannot decode H.264 output";
  }
  if (
    config.zero_copy &&
    config.encoder_type !== "none" &&
    config.decoder_type !== "nvdec" &&
    config.decoder_type !== "none"
  ) {
    return "D3D11 shared texture path requires NVDEC";
  }
  if (
    config.zero_copy &&
    config.decoder_type !== "none" &&
    (config.renderer_type !== "d3d11" || !config.render_display)
  ) {
    return "D3D11 shared texture path requires DX11 popup renderer";
  }
  if (config.renderer_type === "d3d11" && config.capture_type === "macos") {
    return "DX11 popup renderer is Windows-only";
  }
  if (config.renderer_type === "macos" && config.zero_copy) {
    return "Metal renderer does not accept D3D11 shared texture input";
  }
  if (config.renderer_type === "opengl" && config.zero_copy) {
    return "OpenGL renderer requires CPU memory input";
  }
  if (config.encoder_type === "videotoolbox_h264" && config.decoder_type === "nvdec") {
    return "VideoToolbox H.264 output should use VideoToolbox, software, or encode-only decode modes";
  }
  if (
    (config.encoder_type === "nvenc_av1" || isHevcEncoder(config.encoder_type)) &&
    config.decoder_type === "videotoolbox"
  ) {
    return "VideoToolbox decoder path is H.264-only in this matrix";
  }
  return null;
}

function matrixCapabilitySkipReason(
  config: TestConfig,
  capabilitySnapshot: CapabilitySnapshot | null
): string | null {
  if (!capabilitySnapshot) return null;

  const evaluation = evaluateCapabilityCombination(
    {
      capture: config.capture_type,
      encoder: config.encoder_type,
      decoder: config.decoder_type,
      renderer:
        config.render_display && config.renderer_type === "d3d12"
          ? "d3d12_native"
          : config.render_display
            ? config.renderer_type
            : undefined,
      memory: config.zero_copy ? "d3d11_shared" : "cpu",
      transport: config.transport_kind,
      allowCpuCopy: false,
    },
    capabilitySnapshot
  );

  if (evaluation.status !== "blocked" && evaluation.status !== "skipped") {
    return null;
  }

  return evaluation.reasons.join("; ") || "Capability combination is not runnable";
}

function staticMatrixSkipReason(
  config: TestConfig,
  capabilitySnapshot: CapabilitySnapshot | null
): string | null {
  return matrixCapabilitySkipReason(config, capabilitySnapshot) ?? unsupportedMatrixReason(config);
}

function capabilitySkipReason(config: TestConfig, message: string): string | null {
  if (/not supported on/i.test(message)) {
    return message;
  }
  if (
    config.encoder_type === "nvenc_av1" &&
    /NVENC AV1 unavailable|AV1 codec not supported|NVENC AV1 preset query failed/i.test(message)
  ) {
    return message;
  }
  if (
    isHevcEncoder(config.encoder_type) &&
    /NVENC HEVC unavailable|HEVC codec not supported|NVENC HEVC preset query failed/i.test(message)
  ) {
    return message;
  }
  return null;
}

const STATUS_LABELS: Record<MatrixTest["status"], string> = {
  pending: "待执行",
  running: "运行中",
  completed: "完成",
  failed: "失败",
  skipped: "跳过",
};

const MAX_MATRIX_RUNS = 300;
const MAX_MATRIX_RENDER_ROWS = 250;
const SKIP_YIELD_BATCH_SIZE = 20;
const LOCAL_LAN_TARGET_ID = "__local__";
const CROSS_DEVICE_LOOPBACK_REASON =
  "Loopback 仅支持本机进程内测试；跨设备矩阵请显式选择 QUIC Datagram 或 WebRTC RTP。";

type MatrixRunScope = "local" | "cross-device";

const lanAutomationCommands: LanE2EAutomationCommands = {
  serviceBootstrapIfNeeded: commands.serviceBootstrapIfNeeded,
  serviceWaitForHealthy: (timeoutSecs = 10) =>
    commands.serviceWaitForHealthy(timeoutSecs),
  ipcRuntimeSnapshot: commands.ipcRuntimeSnapshot,
  getHardwareInfo: commands.getHardwareInfo,
  ipcRegisterDevice: commands.ipcRegisterDevice,
  ipcRefreshLanDiscovery: commands.ipcRefreshLanDiscovery,
  ipcStartLanRemoteSession: commands.ipcStartLanRemoteSession,
  ipcUpdateMediaProfile: commands.ipcUpdateMediaProfile,
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

function yieldToUi(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function formatMs(value: number | null | undefined): string {
  return value != null && Number.isFinite(value) ? `${value.toFixed(2)} ms` : "-";
}

interface MatrixTestPageProps {
  runDelayMs?: number;
}

function setOptionEnabled(
  dimensions: MatrixDimension[],
  dimensionId: string,
  optionId: string,
  enabled: boolean
): MatrixDimension[] {
  return dimensions.map((dim) =>
    dim.id === dimensionId
      ? {
          ...dim,
          options: dim.options.map((opt) =>
            opt.id === optionId ? { ...opt, enabled } : opt
          ),
        }
      : dim
  );
}

function applyRunScopeToDimensions(
  dimensions: MatrixDimension[],
  runScope: MatrixRunScope
): MatrixDimension[] {
  if (runScope !== "cross-device") {
    return dimensions.map((dimension) => ({
      ...dimension,
      options: dimension.options.map((option) => ({
        ...option,
        scopeBlockedReason: undefined,
      })),
    }));
  }

  return dimensions.map((dimension) =>
    dimension.id === "transport"
      ? {
          ...dimension,
          options: dimension.options.map((option) =>
            option.id === "loopback"
              ? {
                  ...option,
                  enabled: false,
                  scopeBlockedReason: CROSS_DEVICE_LOOPBACK_REASON,
                }
              : option
          ),
        }
      : dimension
  );
}

function optionScopeBlockedReason(
  runScope: MatrixRunScope,
  dimensionId: string,
  optionId: string
): string | null {
  return runScope === "cross-device" && dimensionId === "transport" && optionId === "loopback"
    ? CROSS_DEVICE_LOOPBACK_REASON
    : null;
}

function matrixGenerationBlockedReason(
  dimensions: MatrixDimension[],
  runScope: MatrixRunScope
): string | null {
  if (runScope !== "cross-device") return null;
  const transportDimension = dimensions.find((dimension) => dimension.id === "transport");
  const hasCrossDeviceTransport =
    transportDimension?.options.some(
      (option) =>
        option.enabled &&
        !option.scopeBlockedReason &&
        (option.id === "quic" || option.id === "webrtc")
    ) ?? false;
  return hasCrossDeviceTransport ? null : CROSS_DEVICE_LOOPBACK_REASON;
}

function isOptionEnabled(
  dimensions: MatrixDimension[],
  dimensionId: string,
  optionId: string
): boolean {
  return (
    dimensions
      .find((dim) => dim.id === dimensionId)
      ?.options.some((opt) => opt.id === optionId && opt.enabled) ?? false
  );
}

function matrixConfigKey(config: TestConfig): string {
  return JSON.stringify({
    capture_type: config.capture_type,
    encoder_type: config.encoder_type,
    decoder_type: config.decoder_type,
    transport_kind: config.transport_kind,
    renderer_type: config.renderer_type,
    render_display: config.render_display,
    zero_copy: config.zero_copy,
    adaptive_media: config.adaptive_media,
    resolution: config.resolution,
    fps: config.fps,
    bitrate: config.bitrate,
    duration_ms: config.duration_ms,
    warmup_ms: config.warmup_ms,
  });
}

function crossDeviceMatrixKey(config: TestConfig): string {
  const transportKind = crossDeviceTransportFromConfig(config);
  return JSON.stringify({
    transport_kind: transportKind ?? config.transport_kind ?? "loopback",
    adaptive_media: config.adaptive_media,
    resolution: config.resolution,
    fps: config.fps,
    bitrate: config.bitrate,
    duration_ms: config.duration_ms,
  });
}

function createCrossDeviceMatrixTests(matrixTests: MatrixTest[]): MatrixTest[] {
  const seen = new Set<string>();
  const tests: MatrixTest[] = [];

  for (const test of matrixTests) {
    const key = crossDeviceMatrixKey(test.config);
    if (seen.has(key)) continue;
    seen.add(key);
    tests.push({
      ...test,
      id: `cross_device_${tests.length}`,
      status: "pending",
      skipReason: test.skipReason ?? crossDeviceUnsupportedTransportReason(test.config) ?? undefined,
      failureReason: undefined,
      result: undefined,
      duration: undefined,
    });
  }

  return tests;
}

function crossDeviceTransportFromConfig(config: TestConfig): "quic" | "webrtc" | null {
  if (config.transport_kind === "quic" || config.transport_kind === "webrtc") {
    return config.transport_kind;
  }
  return null;
}

function crossDeviceUnsupportedTransportReason(config: TestConfig): string | null {
  const transportKind = config.transport_kind ?? "loopback";
  if (transportKind === "loopback") {
    return CROSS_DEVICE_LOOPBACK_REASON;
  }
  if (transportKind !== "quic" && transportKind !== "webrtc") {
    return `跨设备矩阵不支持传输层 ${transportKind}；请使用 QUIC Datagram 或 WebRTC RTP。`;
  }
  return null;
}

export function mediaProfileFromConfig(config: TestConfig): MediaProfile {
  const [width, height] = config.resolution ?? [1920, 1080];
  const hevc = config.encoder_type === "nvenc_hevc" || config.encoder_type === "nvenc_hevc_main10";
  const main10 = config.encoder_type === "nvenc_hevc_main10";
  const profile: MediaProfile = {
    width,
    height,
    fps: config.fps ?? 60,
    bitrate_mbps: Math.max(1, Math.round((config.bitrate ?? 20_000_000) / 1_000_000)),
    codec: hevc ? "hevc" : "h264",
  };
  if (hevc) {
    profile.codec_profile = main10 ? "main10" : "main";
    profile.bit_depth = main10 ? 10 : 8;
    profile.chroma_subsampling = "4:2:0";
    profile.pixel_format = main10 ? "p010" : "nv12";
    profile.hdr_enabled = false;
  }
  return profile;
}

function crossDeviceMinimumExpectedFps(profile: MediaProfile): number {
  return Math.max(1, Math.floor(Math.max(1, profile.fps) * 0.8));
}

function summaryFromLanReport(report: LanE2EAutomationReport): TestRunSummary {
  const probe = report.probeSnapshot;
  const adaptation = report.mediaAdaptationSnapshot ?? report.mediaPipelineSnapshot?.adaptation;
  return {
    total_duration_ms: Math.max(0, report.finishedAt - report.startedAt),
    capture_fps: probe?.current_fps ?? undefined,
    dropped_frames: probe?.frames_dropped ?? 0,
    frame_count: probe?.frames_decoded ?? 0,
    adaptation_state: adaptation?.state,
    adaptation_ladder_index: adaptation?.ladder_index,
    adaptation_current_profile: adaptation
      ? formatMatrixMediaProfile(adaptation.current_profile)
      : undefined,
    adaptation_target_profile: adaptation
      ? formatMatrixMediaProfile(adaptation.target_profile)
      : undefined,
    adaptation_reason: adaptation?.last_reason ?? undefined,
    error_message: report.errorMessage,
    failure_reason: report.failureReason ? "validation_failure" : undefined,
  };
}

export function formatMatrixMediaProfile(profile: MediaProfile): string {
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

export function crossDevicePeerSkipReason(
  peer: LanPeerInfo,
  transportKind: "quic" | "webrtc",
  profile?: MediaProfile
): string | null {
  const transports = peer.transports.map((transport) => transport.toLowerCase());
  const transportList = peer.transports.length > 0 ? peer.transports.join(", ") : "none";

  if (!peer.p2p_available) {
    return `LAN peer is discovered but not P2P available: ${peer.device_id}`;
  }

  if (transportKind === "webrtc") {
    return transports.includes("webrtc")
      ? null
      : `LAN peer does not support webrtc: ${peer.device_id} supports ${transportList}`;
  }

  const requiredQuicCapabilities = [
    "quic_datagram",
    "quic_datagram_2k144",
    "media_profile_control_v1",
  ];
  const missing = requiredQuicCapabilities.filter(
    (capability) => !transports.includes(capability)
  );
  if (missing.length > 0) {
    return `LAN peer does not support required QUIC media capabilities [${missing.join(
      ", "
    )}]: ${peer.device_id} supports ${transportList}`;
  }

  const mediaProtocolVersion = peer.media_protocol_version ?? 0;
  const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
    capability.toLowerCase()
  );
  const hasMediaV3 =
    mediaProtocolVersion >= 3 &&
    (transports.includes("quic_datagram_media_v3") ||
      mediaCapabilities.includes("quic_datagram_media_v3"));
  const hasMediaV2 =
    mediaProtocolVersion >= 2 &&
    (transports.includes("quic_datagram_media_v2") ||
      mediaCapabilities.includes("quic_datagram_media_v2"));
  const codec = profile?.codec?.toLowerCase() ?? "h264";
  const requiredMediaCapabilities =
    codec === "hevc"
      ? [
          ["dxgi_capture"],
          ["nvenc_hevc", "encode.nvenc_hevc"],
          ["nvdec_hevc", "decode.nvdec_hevc"],
          ["d3d11_native_render"],
          ["media.hevc_main_420_8bit"],
        ]
      : [
          ["dxgi_capture"],
          ["nvenc_h264", "encode.nvenc_h264"],
          ["nvdec", "decode.nvdec"],
          ["d3d11_native_render"],
        ];
  const missingMediaCapabilities = requiredMediaCapabilities
    .filter((aliases) => !aliases.some((capability) => mediaCapabilities.includes(capability)))
    .map((aliases) => aliases[0]);
  if (!hasMediaV3 && !hasMediaV2) {
    return `LAN peer is not on a compatible QUIC media protocol: ${peer.device_id} reports media protocol ${
      mediaProtocolVersion || "unknown"
    }`;
  }
  return missingMediaCapabilities.length === 0
    ? null
    : `LAN peer is missing required Windows media capabilities [${missingMediaCapabilities.join(
        ", "
      )}]: ${peer.device_id}`;
}

function crossDeviceReportSkipReason(report: LanE2EAutomationReport): string | null {
  if (
    report.failureReason === "peer_not_ready" ||
    report.failureReason === "media_profile_mismatch" ||
    report.failureReason === "profile_downgraded"
  ) {
    return report.errorMessage ?? report.failureReason;
  }
  return null;
}

function sanitizeSessionPart(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-");
}

function localRendererForOs(
  os: HostOs,
  availableRenderers: string[]
): TestConfig["renderer_type"] | null {
  if (os === "linux" && availableRenderers.includes("linux")) return "linux";
  if (os === "macos" && availableRenderers.includes("macos")) return "macos";
  if (os === "windows" && availableRenderers.includes("d3d11")) return "d3d11";
  return null;
}

function createLocalUiDebugMatrixTests(
  capabilities: EnvironmentSnapshot | null,
  capabilitySnapshot: CapabilitySnapshot | null
): MatrixTest[] {
  const os = normalizeHostOs(capabilities?.os_type ?? "windows");
  const availableCaptures = capabilities?.available_captures ?? defaultCapturesForOs(os);
  const availableEncoders = capabilities?.available_encoders ?? defaultEncodersForOs(os);
  const availableDecoders = [
    "none",
    ...(capabilities?.available_decoders ?? defaultDecodersForOs(os)),
  ];
  const availableRenderers = capabilities?.available_renderers ?? defaultRenderersForOs(os);

  if (!availableCaptures.includes("synthetic") || !availableEncoders.includes("openh264")) {
    return [];
  }

  const baseConfig = {
    capture_type: "synthetic",
    encoder_type: "openh264",
    transport_kind: "loopback",
    resolution: [1280, 720],
    fps: 30,
    bitrate: 3_000_000,
    duration_ms: 3_000,
    warmup_ms: 250,
    zero_copy: false,
    visual_preview: true,
    input_source: "synthetic",
  } satisfies TestConfig;

  const configs: TestConfig[] = [
    {
      ...baseConfig,
      decoder_type: "none",
      render_display: false,
    },
  ];

  if (availableDecoders.includes("software")) {
    configs.push({
      ...baseConfig,
      decoder_type: "software",
      render_display: false,
    });
  }

  const localRenderer = localRendererForOs(os, availableRenderers);
  if (localRenderer) {
    configs.push({
      ...baseConfig,
      decoder_type: "none",
      renderer_type: localRenderer,
      render_display: true,
    });
  }

  if (availableEncoders.includes("nvenc_h264")) {
    configs.push({
      ...baseConfig,
      encoder_type: "nvenc_h264",
      decoder_type: "none",
      bitrate: 5_000_000,
      render_display: false,
    });
  }

  return configs.map((config, index) => ({
    id: `local_ui_debug_${index}`,
    config,
    status: "pending",
    skipReason: staticMatrixSkipReason(config, capabilitySnapshot) ?? undefined,
  }));
}

export function MatrixTestPage({ runDelayMs = 7000 }: MatrixTestPageProps = {}) {
  const [showUnavailable] = useShowUnavailableCapabilities();
  const [dimensions, setDimensions] = useState<MatrixDimension[]>(() =>
    createMatrixDimensions(null, null, readShowUnavailableCapabilities())
  );
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [tests, setTests] = useState<MatrixTest[]>([]);
  const [currentTestIndex, setCurrentTestIndex] = useState(0);
  const [completedCount, setCompletedCount] = useState(0);
  const [failedCount, setFailedCount] = useState(0);
  const [skippedCount, setSkippedCount] = useState(0);
  const [matrixNotice, setMatrixNotice] = useState<string | null>(null);
  const [runScope, setRunScope] = useState<MatrixRunScope>("local");
  const [lanPeers, setLanPeers] = useState<LanPeerInfo[]>([]);
  const [selectedLanTargetId, setSelectedLanTargetId] =
    useState(LOCAL_LAN_TARGET_ID);
  const [isRefreshingLanPeers, setIsRefreshingLanPeers] = useState(false);
  const stopRequestedRef = useRef(false);
  const activeRunIdRef = useRef<string | null>(null);
  const environmentCapabilitySnapshot = useMemo(
    () => (capabilities ? buildCapabilitySnapshotFromEnvironment(capabilities) : null),
    [capabilities]
  );
  const capabilitySnapshot =
    serviceCapabilitySnapshot ?? environmentCapabilitySnapshot;
  const scopedDimensions = useMemo(
    () => applyRunScopeToDimensions(dimensions, runScope),
    [dimensions, runScope]
  );
  const selectionBlockedReason = useMemo(
    () => matrixGenerationBlockedReason(scopedDimensions, runScope),
    [scopedDimensions, runScope]
  );

  useEffect(() => {
    let cancelled = false;
    let legacyEnvironment: EnvironmentSnapshot | null = null;
    let serviceSnapshot: CapabilitySnapshot | null = null;

    const applyLegacyEnvironment = (environment: EnvironmentSnapshot) => {
      if (cancelled) {
        return;
      }
      legacyEnvironment = environment;
      if (serviceSnapshot) {
        const mergedEnvironment = environmentSnapshotFromCapabilitySnapshot(
          serviceSnapshot,
          legacyEnvironment
        );
        setCapabilities(mergedEnvironment);
        setDimensions(createMatrixDimensions(mergedEnvironment, serviceSnapshot, showUnavailable));
        return;
      }
      setCapabilities(environment);
      setServiceCapabilitySnapshot(null);
      setDimensions(createMatrixDimensions(environment, null, showUnavailable));
    };

    const applyServiceSnapshot = (snapshot: CapabilitySnapshot) => {
      if (cancelled) {
        return;
      }
      serviceSnapshot = snapshot;
      const environment = environmentSnapshotFromCapabilitySnapshot(snapshot, legacyEnvironment);
      setCapabilities(environment);
      setServiceCapabilitySnapshot(snapshot);
      setDimensions(createMatrixDimensions(environment, snapshot, showUnavailable));
    };

    void commands.testGetCapabilities().then((legacyResult) => {
      if (legacyResult.ok && legacyResult.value) {
        applyLegacyEnvironment(legacyResult.value);
      }
    });

    void commands.ipcCapabilitySnapshot().then((serviceResult) => {
      if (serviceResult.ok && serviceResult.value) {
        applyServiceSnapshot(buildCapabilitySnapshotFromIpc(serviceResult.value));
      } else if (!cancelled) {
        setServiceCapabilitySnapshot(null);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [showUnavailable]);

  const refreshLanPeers = useCallback(async () => {
    setIsRefreshingLanPeers(true);
    const result = await commands.ipcRefreshLanDiscovery();
    if (result.ok) {
      const peers = result.value?.peers ?? [];
      setLanPeers(peers);
      setSelectedLanTargetId((current) =>
        current === LOCAL_LAN_TARGET_ID ||
        peers.some((peer) => peer.device_id === current)
          ? current
          : LOCAL_LAN_TARGET_ID
      );
    } else {
      setMatrixNotice(`刷新 LAN 发现失败：${result.error.message}`);
    }
    setIsRefreshingLanPeers(false);
  }, []);

  useEffect(() => {
    if (runScope !== "cross-device") return;
    void refreshLanPeers();
  }, [refreshLanPeers, runScope]);

  const toggleOption = (dimensionId: string, optionId: string) => {
    setMatrixNotice(null);
    const scopeBlockedReason = optionScopeBlockedReason(runScope, dimensionId, optionId);
    if (scopeBlockedReason) {
      setMatrixNotice(scopeBlockedReason);
      return;
    }
    setDimensions((current) => {
      const option = current
        .find((dim) => dim.id === dimensionId)
        ?.options.find((opt) => opt.id === optionId);
      if (option?.available === false) return current;

      let next = current.map((dim) =>
        dim.id === dimensionId
          ? {
              ...dim,
              options: dim.options.map((opt) =>
                opt.id === optionId ? { ...opt, enabled: !opt.enabled } : opt
              ),
            }
          : dim
      );

      if (
        dimensionId === "memory" &&
        optionId === "d3d11_shared" &&
        isOptionEnabled(next, "memory", "d3d11_shared")
      ) {
        next = setOptionEnabled(next, "renderer", "d3d11", true);
        next = setOptionEnabled(next, "renderer", "opengl", false);
        next = setOptionEnabled(next, "renderer", "renderer_none", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId === "d3d11" &&
        !isOptionEnabled(next, "renderer", "d3d11")
      ) {
        next = setOptionEnabled(next, "memory", "d3d11_shared", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId === "opengl" &&
        isOptionEnabled(next, "renderer", "opengl")
      ) {
        next = setOptionEnabled(next, "memory", "d3d11_shared", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId !== "renderer_none" &&
        isOptionEnabled(next, "renderer", optionId)
      ) {
        next = setOptionEnabled(next, "renderer", "renderer_none", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId === "renderer_none" &&
        isOptionEnabled(next, "renderer", "renderer_none")
      ) {
        next = setOptionEnabled(next, "renderer", "d3d11", false);
        next = setOptionEnabled(next, "renderer", "opengl", false);
        next = setOptionEnabled(next, "renderer", "d3d12_native", false);
        next = setOptionEnabled(next, "renderer", "macos", false);
        next = setOptionEnabled(next, "renderer", "linux", false);
        next = setOptionEnabled(next, "memory", "d3d11_shared", false);
      }

      return next;
    });
  };

  const generateMatrix = useCallback((): MatrixGenerationResult => {
    if (selectionBlockedReason) {
      return { tests: [], truncated: false };
    }

    const enabledOptions = scopedDimensions
      .map((dim) =>
        dim.options
          .filter((o) => o.enabled && !o.scopeBlockedReason)
          .map((option) => ({
            dimensionId: dim.id,
            option,
          }))
      )
      .filter((opts) => opts.length > 0);

    if (enabledOptions.length === 0) {
      return { tests: [], truncated: false };
    }

    const combinations: MatrixTest[] = [];
    const seenConfigs = new Set<string>();
    let truncated = false;
    const generate = (index: number, current: SelectedMatrixOption[]) => {
      if (truncated) return;

      if (index >= enabledOptions.length) {
        const config = buildConfig(current);

        const configKey = matrixConfigKey(config);
        if (seenConfigs.has(configKey)) {
          return;
        }
        seenConfigs.add(configKey);

        if (combinations.length >= MAX_MATRIX_RUNS) {
          truncated = true;
          return;
        }

        combinations.push({
          id: `matrix_${combinations.length}`,
          config,
          status: "pending",
          skipReason: staticMatrixSkipReason(config, capabilitySnapshot) ?? undefined,
        });
        return;
      }

      const options = enabledOptions[index];
      if (!options) return;

      for (const option of options) {
        if (truncated) break;
        generate(index + 1, [...current, option]);
      }
    };

    generate(0, []);
    return { tests: combinations, truncated };
  }, [capabilitySnapshot, scopedDimensions, selectionBlockedReason]);

  const waitForRunCompletion = async (runId: string, config: TestConfig): Promise<TestRun | null> => {
    const timeoutMs = Math.max(
      runDelayMs,
      (config.duration_ms ?? 5000) + (config.warmup_ms ?? 0) + 3000
    );
    const startedAt = Date.now();
    let lastRun: TestRun | null = null;

    while (Date.now() - startedAt <= timeoutMs) {
      if (stopRequestedRef.current) {
        return lastRun;
      }

      const runResult = await commands.testGetRun(runId);
      if (!runResult.ok) {
        throw new Error(runResult.error.message);
      }
      if (!runResult.value) {
        return null;
      }

      lastRun = runResult.value;
      if (
        runResult.value.status === "completed" ||
        runResult.value.status === "failed" ||
        runResult.value.status === "cancelled"
      ) {
        return runResult.value;
      }

      await new Promise((resolve) => setTimeout(resolve, 250));
    }

    return lastRun;
  };

  const handleStop = useCallback(async () => {
    stopRequestedRef.current = true;
    setMatrixNotice("正在停止当前矩阵测试...");

    const activeRunId = activeRunIdRef.current;
    if (activeRunId) {
      await commands.testStopRun(activeRunId);
    }
  }, []);

  const runMatrixTests = async (matrixTests: MatrixTest[], initialNotice: string | null) => {
    if (matrixTests.length === 0) return;

    stopRequestedRef.current = false;
    activeRunIdRef.current = null;
    setMatrixNotice(initialNotice);
    setTests(matrixTests);
    setIsRunning(true);
    setCurrentTestIndex(0);
    setCompletedCount(0);
    setFailedCount(0);
    setSkippedCount(0);

    await yieldToUi();

    for (let i = 0; i < matrixTests.length; i++) {
      if (stopRequestedRef.current) {
        break;
      }

      setCurrentTestIndex(i);

      const test = matrixTests[i];
      if (!test) continue;

      const staticSkipReason =
        test.skipReason ?? staticMatrixSkipReason(test.config, capabilitySnapshot);
      if (staticSkipReason) {
        setSkippedCount((count) => count + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i ? { ...t, status: "skipped" as const, skipReason: staticSkipReason } : t
          )
        );
        if ((i + 1) % SKIP_YIELD_BATCH_SIZE === 0) {
          await yieldToUi();
        }
        continue;
      }

      setTests((prev) =>
        prev.map((t, idx) =>
          idx === i ? { ...t, status: "running" as const } : t
        )
      );

      const startTime = Date.now();
      const markFailed = (
        duration = Date.now() - startTime,
        result?: TestRunSummary,
        failureReason?: string
      ) => {
        setFailedCount((f) => f + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i
              ? {
                  ...t,
                  status: "failed" as const,
                  result,
                  duration,
                  failureReason,
                }
              : t
          )
        );
      };
      const markSkipped = (skipReason: string, duration = Date.now() - startTime) => {
        setSkippedCount((count) => count + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i ? { ...t, status: "skipped" as const, skipReason, duration } : t
          )
        );
      };

      let activeRunId: string | null = null;
      try {
        const result = await commands.testStartRun({
          scenarioId: "matrix",
          config: test.config,
        });

        if (!result.ok) {
          const skipReason = capabilitySkipReason(test.config, result.error.message);
          if (skipReason) {
            markSkipped(skipReason);
            continue;
          }
          markFailed(undefined, undefined, result.error.message);
          continue;
        }

        activeRunId = result.value;
        activeRunIdRef.current = activeRunId;

        const run = await waitForRunCompletion(activeRunId, test.config);
        if (stopRequestedRef.current) {
          markSkipped("用户停止矩阵测试");
          await commands.testStopRun(activeRunId);
          break;
        }
        if (!run) {
          markFailed();
          await commands.testStopRun(activeRunId);
          continue;
        }

        const duration = Date.now() - startTime;

        const acceptance = evaluateMatrixRun(test.config, run.summary);
        if (run.status === "completed" && acceptance.acceptable) {
          setCompletedCount((c) => c + 1);
          setTests((prev) =>
            prev.map((t, idx) =>
              idx === i
                ? {
                    ...t,
                    status: "completed" as const,
                    result: run.summary,
                    duration,
              }
                : t
            )
          );
        } else {
          markFailed(
            duration,
            run.summary,
            run.summary?.error_message ?? acceptance.reason ?? run.status
          );
        }

        await commands.testStopRun(activeRunId);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (stopRequestedRef.current) {
          markSkipped("用户停止矩阵测试");
          if (activeRunId) {
            await commands.testStopRun(activeRunId);
          }
          break;
        }
        const skipReason = capabilitySkipReason(test.config, message);
        if (skipReason) {
          markSkipped(skipReason);
          continue;
        }
        markFailed(undefined, undefined, message);
        if (activeRunId) {
          await commands.testStopRun(activeRunId);
        }
      } finally {
        if (activeRunIdRef.current === activeRunId) {
          activeRunIdRef.current = null;
        }
      }

      await yieldToUi();
    }

    activeRunIdRef.current = null;
    setIsRunning(false);
    if (stopRequestedRef.current) {
      setMatrixNotice("矩阵测试已停止。");
    } else if (initialNotice) {
      setMatrixNotice("本地 UI 调试矩阵已结束。");
    }
  };

  const runCrossDeviceMatrixTests = async (
    matrixTests: MatrixTest[],
    targetPeer: LanPeerInfo
  ) => {
    const crossDeviceTests = createCrossDeviceMatrixTests(matrixTests);
    if (crossDeviceTests.length === 0) return;

    stopRequestedRef.current = false;
    activeRunIdRef.current = null;
    setMatrixNotice(
      `正在运行跨设备矩阵：目标 ${targetPeer.device_name} (${targetPeer.ip})。`
    );
    setTests(crossDeviceTests);
    setIsRunning(true);
    setCurrentTestIndex(0);
    setCompletedCount(0);
    setFailedCount(0);
    setSkippedCount(0);

    await yieldToUi();

    for (let i = 0; i < crossDeviceTests.length; i++) {
      if (stopRequestedRef.current) break;

      setCurrentTestIndex(i);
      const test = crossDeviceTests[i];
      if (!test) continue;

      setTests((prev) =>
        prev.map((t, idx) =>
          idx === i ? { ...t, status: "running" as const } : t
        )
      );

      const startTime = Date.now();
      const markFailed = (
        duration = Date.now() - startTime,
        result?: TestRunSummary,
        failureReason?: string
      ) => {
        setFailedCount((f) => f + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i
              ? {
                  ...t,
                  status: "failed" as const,
                  result,
                  duration,
                  failureReason,
                }
              : t
          )
        );
      };
      const markSkipped = (
        skipReason: string,
        duration = Date.now() - startTime,
        result?: TestRunSummary
      ) => {
        setSkippedCount((count) => count + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i
              ? {
                  ...t,
                  status: "skipped" as const,
                  result,
                  skipReason,
                  duration,
                }
              : t
          )
        );
      };

      try {
        const profile = mediaProfileFromConfig(test.config);
        const transportKind = crossDeviceTransportFromConfig(test.config);
        if (!transportKind) {
          markSkipped(
            crossDeviceUnsupportedTransportReason(test.config) ??
              "跨设备矩阵需要显式选择 QUIC Datagram 或 WebRTC RTP。"
          );
          await yieldToUi();
          continue;
        }
        const peerSkipReason = crossDevicePeerSkipReason(targetPeer, transportKind, profile);
        if (peerSkipReason) {
          markSkipped(peerSkipReason);
          await yieldToUi();
          continue;
        }

        const durationMs = test.config.duration_ms ?? 5000;
        const report = await runLanE2EAutomation(lanAutomationCommands, {
          scenarioId: "cross.e2e.remote_display_smoke",
          targetDeviceId: targetPeer.device_id,
          transportKind,
          requestedProfile: profile,
          adaptive: test.config.adaptive_media === true,
          displayModePolicy: "temporary",
          timeoutMs: Math.max(10_000, durationMs + (test.config.warmup_ms ?? 0) + 5000),
          sampleIntervalMs: 500,
          minSampleDurationMs: Math.min(1000, durationMs),
          minDecodedFrames: 1,
          minFps: crossDeviceMinimumExpectedFps(profile),
          createSessionId: () =>
            `matrix-lan-${sanitizeSessionPart(targetPeer.device_id)}-${Date.now()}-${i}`,
        });

        const summary = summaryFromLanReport(report);
        const duration = Date.now() - startTime;

        if (stopRequestedRef.current) {
          markSkipped("用户停止矩阵测试", duration);
          break;
        }

        if (report.status === "completed") {
          setCompletedCount((count) => count + 1);
          setTests((prev) =>
            prev.map((t, idx) =>
              idx === i
                ? {
                    ...t,
                    status: "completed" as const,
                    result: summary,
                    duration,
                  }
                : t
            )
          );
        } else if (report.status === "skipped") {
          markSkipped(
            report.errorMessage ?? report.failureReason ?? "跨设备用例跳过",
            duration,
            summary
          );
        } else {
          const skipReason = crossDeviceReportSkipReason(report);
          if (skipReason) {
            markSkipped(skipReason, duration, summary);
          } else {
            markFailed(
              duration,
              summary,
              report.errorMessage ?? report.failureReason ?? "跨设备矩阵用例失败"
            );
          }
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (stopRequestedRef.current) {
          markSkipped("用户停止矩阵测试");
          break;
        }
        markFailed(undefined, undefined, message);
      }

      await yieldToUi();
    }

    activeRunIdRef.current = null;
    setIsRunning(false);
    setMatrixNotice(
      stopRequestedRef.current
        ? "跨设备矩阵测试已停止。"
        : "跨设备矩阵测试已结束。"
    );
  };

  const handleStart = async () => {
    const matrixGeneration = generateMatrix();
    if (matrixGeneration.truncated) {
      setTests(matrixGeneration.tests);
      setMatrixNotice(
        `当前选择超过 ${MAX_MATRIX_RUNS} 个组合。请减少勾选项后再启动，避免 UI 和测试管线被一次性压满。`
      );
      return;
    }

    if (runScope === "cross-device" && selectedLanTargetId !== LOCAL_LAN_TARGET_ID) {
      const selectedPeer = lanPeers.find(
        (peer) => peer.device_id === selectedLanTargetId
      );
      if (!selectedPeer) {
        setMatrixNotice("未找到选中的跨设备目标，请刷新发现设备后重试。");
        return;
      }
      await runCrossDeviceMatrixTests(matrixGeneration.tests, selectedPeer);
      return;
    }

    await runMatrixTests(matrixGeneration.tests, null);
  };

  const handleStartLocalUiDebug = async () => {
    const localTests = createLocalUiDebugMatrixTests(capabilities, capabilitySnapshot);
    if (localTests.length === 0) {
      setTests([]);
      setMatrixNotice("本地 UI 调试矩阵不可用：当前能力快照没有 synthetic capture 或 OpenH264。");
      return;
    }

    await runMatrixTests(
      localTests,
      "正在运行本地 UI 调试矩阵：使用 Synthetic capture，不触发 Linux 屏幕共享授权弹窗。"
    );
  };

  const matrixGeneration = useMemo(() => generateMatrix(), [generateMatrix]);
  const isRemoteCrossDeviceRun =
    runScope === "cross-device" && selectedLanTargetId !== LOCAL_LAN_TARGET_ID;
  const plannedMatrixTests = useMemo(
    () =>
      isRemoteCrossDeviceRun
        ? createCrossDeviceMatrixTests(matrixGeneration.tests)
        : matrixGeneration.tests,
    [isRemoteCrossDeviceRun, matrixGeneration.tests]
  );
  const plannedTotalTests = plannedMatrixTests.length;
  const totalTests = tests.length > 0 ? tests.length : plannedTotalTests;
  const finishedCount = completedCount + failedCount + skippedCount;
  const progress = totalTests > 0 ? (finishedCount / totalTests) * 100 : 0;
  const platformLabel = capabilities?.os_type ?? "windows";
  const visibleTests = tests.slice(0, MAX_MATRIX_RENDER_ROWS);
  const hiddenResultRows = tests.length - visibleTests.length;

  const getStatusIcon = (status: MatrixTest["status"]) => {
    switch (status) {
      case "pending":
        return <div className="w-4 h-4 rounded-full border-2 border-gray-300" />;
      case "running":
        return <Loader2 className="h-4 w-4 text-blue-500 animate-spin" />;
      case "completed":
        return <CheckCircle2 className="h-4 w-4 text-green-500" />;
      case "failed":
        return <XCircle className="h-4 w-4 text-red-500" />;
      case "skipped":
        return <Clock className="h-4 w-4 text-gray-400" />;
    }
  };

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Grid3x3 className="h-6 w-6" />
          矩阵测试
        </h1>
        <p className="text-muted-foreground">
          批量参数组合测试，当前平台矩阵：{platformLabel}
        </p>
      </div>

      {/* Execution Scope */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">执行目标</h2>
        <div className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
          <label className="block">
            <span className="mb-2 block text-sm font-medium">执行范围</span>
            <select
              aria-label="执行范围"
              value={runScope}
              disabled={isRunning}
              onChange={(event) => {
                const nextScope = event.target.value as MatrixRunScope;
                setRunScope(nextScope);
                setMatrixNotice(null);
              }}
              className="w-full rounded border border-border bg-background px-3 py-2 text-sm"
            >
              <option value="local">本机</option>
              <option value="cross-device">跨设备</option>
            </select>
          </label>

          {runScope === "cross-device" && (
            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
              <label className="block">
                <span className="mb-2 block text-sm font-medium">跨设备目标设备</span>
                <select
                  aria-label="跨设备目标设备"
                  value={selectedLanTargetId}
                  disabled={isRunning}
                  onChange={(event) => setSelectedLanTargetId(event.target.value)}
                  className="w-full rounded border border-border bg-background px-3 py-2 text-sm"
                >
                  <option value={LOCAL_LAN_TARGET_ID}>本机</option>
                  {lanPeers.map((peer) => (
                    <option key={peer.device_id} value={peer.device_id}>
                      {peer.device_name} ({peer.ip})
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                onClick={() => void refreshLanPeers()}
                disabled={isRunning || isRefreshingLanPeers}
                className="flex items-center justify-center gap-2 rounded border border-border bg-secondary px-3 py-2 text-sm text-secondary-foreground hover:bg-secondary/80 disabled:opacity-50"
              >
                <RefreshCw
                  className={`h-4 w-4 ${isRefreshingLanPeers ? "animate-spin" : ""}`}
                />
                刷新发现设备
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Dimension Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择测试维度</h2>
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {scopedDimensions.map((dim) => (
            <div key={dim.id}>
              <h3 className="font-medium text-sm mb-2">{dim.name}</h3>
              <div className="space-y-1">
                {dim.options.map((opt) => {
                  const scopeBlocked = Boolean(opt.scopeBlockedReason);
                  return (
                    <label
                      key={opt.id}
                      className={`flex items-center gap-2 p-2 rounded hover:bg-muted ${
                        opt.available === false || scopeBlocked
                          ? "cursor-not-allowed opacity-50"
                          : "cursor-pointer"
                      }`}
                    >
                      <input
                        type="checkbox"
                        aria-label={opt.name}
                        checked={opt.enabled}
                        onChange={() => toggleOption(dim.id, opt.id)}
                        disabled={isRunning || opt.available === false || scopeBlocked}
                        className="rounded"
                      />
                      <span className="text-sm">{opt.name}</span>
                      {scopeBlocked && (
                        <span
                          className="text-xs bg-slate-100 text-slate-700 px-1 rounded dark:bg-slate-800 dark:text-slate-200"
                          title={opt.scopeBlockedReason}
                        >
                          仅本机
                        </span>
                      )}
                      {opt.available === false && (
                        <span
                          className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded"
                          title={opt.unavailableReason ?? opt.statusLabel}
                        >
                          不可用
                        </span>
                      )}
                      {opt.available !== false && opt.statusLabel === "degraded" && (
                        <span
                          className="text-xs bg-amber-100 text-amber-800 px-1 rounded"
                          title={opt.unavailableReason ?? "degraded"}
                        >
                          degraded
                        </span>
                      )}
                      {opt.available !== false && opt.statusLabel === "supported" && (
                        <span
                          className="text-xs bg-blue-100 text-blue-800 px-1 rounded"
                          title={opt.unavailableReason ?? "supported"}
                        >
                          待探测
                        </span>
                      )}
                      {opt.id === "nvenc_av1" && (
                        <span className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded">
                          NVDEC
                        </span>
                      )}
                    </label>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Summary */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">测试概览</h2>
          {!isRunning ? (
            <div className="flex flex-wrap justify-end gap-2">
              <button
                onClick={handleStartLocalUiDebug}
                className="flex items-center gap-2 px-4 py-2 rounded border border-border bg-secondary text-secondary-foreground hover:bg-secondary/80"
              >
                <Play className="h-4 w-4" />
                本地 UI 调试矩阵
              </button>
              <button
                onClick={handleStart}
                disabled={
                  plannedTotalTests === 0 ||
                  matrixGeneration.truncated ||
                  Boolean(selectionBlockedReason)
                }
                className="flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50"
              >
                <Play className="h-4 w-4" />
                启动矩阵测试 ({plannedTotalTests}
                {matrixGeneration.truncated ? "+" : ""} 个组合)
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-4">
              <span className="text-sm text-muted-foreground">
                {currentTestIndex + 1} / {totalTests}
              </span>
              <div className="w-32 h-2 bg-muted rounded-full overflow-hidden">
                <div
                  className="h-full bg-primary transition-all"
                  style={{ width: `${progress}%` }}
                />
              </div>
              <button
                onClick={handleStop}
                className="flex items-center gap-2 px-3 py-2 bg-destructive text-destructive-foreground rounded hover:bg-destructive/90"
              >
                <Square className="h-4 w-4" />
                停止
              </button>
            </div>
          )}
        </div>

        {matrixGeneration.truncated && !isRunning && (
          <div className="mb-4 rounded border border-yellow-500/40 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-700 dark:text-yellow-300">
            当前选择超过 {MAX_MATRIX_RUNS} 个组合。减少勾选项后再启动，避免矩阵一次性生成和跳过过多用例导致界面无响应。
          </div>
        )}

        {selectionBlockedReason && !isRunning && (
          <div className="mb-4 rounded border border-yellow-500/40 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-700 dark:text-yellow-300">
            {selectionBlockedReason}
          </div>
        )}

        {matrixNotice && (
          <div className="mb-4 rounded border border-border bg-muted px-3 py-2 text-sm text-muted-foreground">
            {matrixNotice}
          </div>
        )}

        {tests.length > 0 && (
          <div className="grid grid-cols-4 gap-4 text-center">
            <div>
              <p className="text-2xl font-bold">{totalTests}</p>
              <p className="text-xs text-muted-foreground">总计</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-green-600">{completedCount}</p>
              <p className="text-xs text-muted-foreground">成功</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-red-600">{failedCount}</p>
              <p className="text-xs text-muted-foreground">失败</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-gray-400">
                {skippedCount}
              </p>
              <p className="text-xs text-muted-foreground">跳过</p>
            </div>
          </div>
        )}
      </div>

      {/* Test Results Grid */}
      {tests.length > 0 && (
        <div className="bg-card rounded-lg border overflow-x-auto">
          <table className="w-full min-w-[1840px]">
            <thead className="bg-muted">
              <tr>
                <th className="px-4 py-2 text-left text-sm font-medium">状态</th>
                <th className="px-4 py-2 text-left text-sm font-medium">捕获</th>
                <th className="px-4 py-2 text-left text-sm font-medium">编码器</th>
                <th className="px-4 py-2 text-left text-sm font-medium">解码器</th>
                <th className="px-4 py-2 text-left text-sm font-medium">传输</th>
                <th className="px-4 py-2 text-left text-sm font-medium">渲染</th>
                <th className="px-4 py-2 text-left text-sm font-medium">分辨率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">帧率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">码率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">自适应</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Pipeline FPS</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Memory</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Encode P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Transport P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Decode P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">延迟 P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">时长</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {visibleTests.map((test) => (
                <tr
                  key={test.id}
                  className={
                    test.status === "running"
                      ? "bg-blue-50 dark:bg-blue-900/10"
                      : ""
                  }
                >
                  <td className="px-4 py-2 flex items-center gap-2">
                    {getStatusIcon(test.status)}
                    <div className="min-w-0">
                      <span className="text-xs text-muted-foreground">
                        {STATUS_LABELS[test.status]}
                      </span>
                      {test.skipReason && (
                        <div
                          className="max-w-[220px] truncate text-[11px] text-muted-foreground"
                          title={test.skipReason}
                        >
                          {test.skipReason}
                        </div>
                      )}
                      {test.failureReason && (
                        <div
                          className="max-w-[260px] truncate text-[11px] text-destructive"
                          title={test.failureReason}
                        >
                          {test.failureReason}
                        </div>
                      )}
                    </div>
                  </td>
                  <td className="px-4 py-2 text-sm">{test.config.capture_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.encoder_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.decoder_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.transport_kind}</td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.render_display ? test.config.renderer_type ?? "native" : "none"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.resolution?.join("x")}
                  </td>
                  <td className="px-4 py-2 text-sm">{test.config.fps}</td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.bitrate ? `${(test.config.bitrate / 1000000).toFixed(0)} Mbps` : "-"}
                  </td>
                  <td className="px-4 py-2 text-xs">
                    {test.config.adaptive_media ? (
                      <div
                        className="max-w-[210px] truncate"
                        title={[
                          test.result?.adaptation_current_profile,
                          test.result?.adaptation_target_profile,
                          test.result?.adaptation_reason,
                        ].filter(Boolean).join(" -> ")}
                      >
                        {test.result?.adaptation_state ?? "enabled"}
                        {test.result?.adaptation_ladder_index != null
                          ? ` #${test.result.adaptation_ladder_index}`
                          : ""}
                        {test.result?.adaptation_target_profile
                          ? ` ${test.result.adaptation_target_profile}`
                          : ""}
                      </div>
                    ) : (
                      "固定"
                    )}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.result?.capture_fps?.toFixed(1) ?? "-"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.zero_copy ? "d3d11_shared" : "cpu"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.encode_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.transport_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.decode_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.total_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.duration ? `${(test.duration / 1000).toFixed(1)}s` : "-"}
                  </td>
                </tr>
              ))}
              {hiddenResultRows > 0 && (
                <tr>
                  <td colSpan={16} className="px-4 py-3 text-center text-sm text-muted-foreground">
                    仅显示前 {MAX_MATRIX_RENDER_ROWS} 条结果，剩余 {hiddenResultRows} 条仍会按顺序执行。
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
