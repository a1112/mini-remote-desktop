import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams, useSearchParams } from "react-router";
import {
  Activity,
  ArrowLeft,
  BarChart3,
  Circle,
  Gauge,
  Loader2,
  Maximize2,
  Minimize,
  Monitor,
  MousePointer2,
  Network,
  PanelTop,
  Play,
  SlidersHorizontal,
  Square,
  X,
} from "lucide-react";
import {
  browserWebcodecsPreviewWebSocketUrl,
  browserWebrtcPreviewStart,
  browserWebrtcPreviewStop,
  closeRemoteDisplayWindow,
  configureRemoteDisplayNativeSurface,
  currentRemoteDisplayWindowContext,
  getSystemResourceSnapshot,
  ipcMediaPipelineSnapshot,
  presentRemotePreviewFrameOnNativeSurface,
  presentTestHarnessFrameOnNativeSurface,
  testGetCapabilities,
  testGetRun,
  testHarnessGetMetrics,
  testHarnessStop,
  testStartRun,
  testStopRun,
  type CaptureType,
  type DecoderType,
  type EncoderType,
  type EnvironmentSnapshot,
  type HarnessMetrics,
  type MediaPipelineSnapshot,
  type NativeRenderSurfaceSnapshot,
  type RemoteDisplayWindowContext,
  type SystemResourceSnapshot,
  type TestConfig,
  type TestMatrixConfig,
  type TestRun,
} from "../adapters/tauri";
import {
  getProbeSnapshot,
  getSessionSnapshot,
  listRemoteCaptureSources,
  selectRemoteCaptureSource,
  startReceiver,
  updateMediaProfile,
  type CaptureSource,
  type CaptureSourceSelection,
  type MediaProfileNegotiation,
  type ProbeSnapshot,
  type SessionRuntimeSnapshot,
} from "../services/ipcSessionService";
import { isTauriRuntime } from "../utils/runtime";
import { withTauriWindow } from "../utils/tauriWindow";

type RenderMode = "web" | "d3d11_native" | "d3d12_native" | "metal_native" | "linux_native";
type HostOs = "macos" | "windows" | "linux" | "other";
type TransportKind = NonNullable<TestMatrixConfig["transport"]>;
type ResolutionKey =
  | "1280x720"
  | "1920x1080"
  | "2560x1440"
  | "2560x1600"
  | "3440x1440"
  | "3840x2160";
type FpsKey = "30" | "60" | "120" | "144" | "165" | "180" | "249";
type BitrateKey = "8" | "20" | "50" | "80" | "100" | "120";
type TestStatus = "idle" | "starting" | "running" | "stopping" | "completed" | "failed";
type WebPreviewMode = "idle" | "connecting" | "webrtc" | "webcodecs" | "failed";
type WebPreviewEngine = "webrtc" | "webcodecs";
type CaptureSourcePickerMode = "dropdown" | "modal";
type LocalTestDurationMode = "30s" | "60s" | "manual";
type MatrixDimensionKey = "capture" | "encoder" | "resolution" | "fps" | "bitrate";
type LocalTestSelection = Partial<LocalWebViewProfile> & {
  resolution?: ResolutionKey;
};

const METRICS_POLL_MS = 500;
const WEB_PREVIEW_CONNECT_TIMEOUT_MS = 8_000;
const WEB_VIEW_MAX_FPS = 144;
const DIAGNOSTICS_SAMPLE_LIMIT = 60;
const DIAGNOSTICS_SAMPLE_INTERVAL_MS = 1_500;

type Option<T extends string> = {
  value: T;
  label: string;
};

type DiagnosticsSample = {
  atMs: number;
  fps: number | null;
  paintFps: number | null;
  latencyP95Ms: number | null;
  captureP95Ms: number | null;
  encodeP95Ms: number | null;
  transportP95Ms: number | null;
  decodeP95Ms: number | null;
  renderP95Ms: number | null;
  queueDepth: number | null;
  droppedFrames: number | null;
  bitrateMbps: number | null;
  serviceCpuPercent: number | null;
  serviceMemoryPercent: number | null;
  serviceMemoryMb: number | null;
  serviceGpuPercent: number | null;
  serviceGpuMemoryMb: number | null;
  serviceNetworkRxMbps: number | null;
  serviceNetworkTxMbps: number | null;
  displayCpuPercent: number | null;
  displayMemoryPercent: number | null;
  displayMemoryMb: number | null;
  displayGpuPercent: number | null;
  displayGpuMemoryMb: number | null;
  displayNetworkRxMbps: number | null;
  displayNetworkTxMbps: number | null;
};

type DiagnosticsStageRow = {
  label: string;
  value: number | null;
  samples?: number | null;
};

export type WebRtcInboundVideoCounters = {
  timestampMs: number;
  framesDecoded: number;
  framesDropped: number;
  packetsLost: number;
  jitterSeconds: number | null;
  jitterBufferDelaySeconds: number | null;
  jitterBufferEmittedCount: number | null;
  totalDecodeTimeSeconds: number | null;
  totalProcessingDelaySeconds: number | null;
  totalInterFrameDelaySeconds: number | null;
  freezeCount: number;
  frameWidth: number | null;
  frameHeight: number | null;
};

export type WebRtcReceiverStats = {
  decodedFps: number | null;
  framesDecoded: number;
  framesDropped: number;
  packetsLost: number;
  jitterMs: number | null;
  jitterBufferDelayAvgMs: number | null;
  decodeAvgMs: number | null;
  processingDelayAvgMs: number | null;
  interFrameDelayAvgMs: number | null;
  freezeCount: number;
  frameWidth: number | null;
  frameHeight: number | null;
};

export type WebRtcPresentationLatencyStats = {
  latestMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  samples: number;
  source:
    | "browser_capture_time"
    | "rtp_frame_timing_channel"
    | "frame_timing_channel"
    | "webcodecs_frame_header";
};

type WebRtcFrameTimingMetadata = {
  sequence: number;
  captureUnixUs: number;
  sentUnixUs: number | null;
  keyframe: boolean;
  rtpTimestamp: number | null;
};

type WebRtcFrameTimingDebug = {
  received: number;
  lastMessage: string | null;
  lastStats: WebRtcPresentationLatencyStats | null;
  localChannelState?: string | null;
  remoteChannelState?: string | null;
};

type WebCodecsFrameHeader = {
  type: "mrd.webcodecs.frame.v1";
  sequence: number;
  timestamp_us: number;
  duration_us: number;
  capture_unix_us: number;
  keyframe: boolean;
  codec: string;
  codec_format: "annexb";
  width: number;
  height: number;
};

type WebCodecsReadyMessage = {
  type: "mrd.webcodecs.ready.v1";
  session_id: string;
  codec: string;
  codec_format: "annexb";
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
};

type WebCodecsErrorMessage = {
  type?: string;
  code?: string;
  message?: string;
};

type WebCodecsAccessUnitMessage = {
  header: WebCodecsFrameHeader;
  payload: Uint8Array;
};

type WebCodecsWorkerCapableCanvas = {
  transferControlToOffscreen?: () => OffscreenCanvas;
};

type WebCodecsWorkerMessage =
  | {
      type: "ready";
      width: number;
      height: number;
      fps: number;
      bitrateMbps: number;
      rendererBackend: "webgl2" | "2d";
    }
  | {
      type: "stats";
      fps: number;
      paintFps: number;
      frameCount: number;
      frameIntervalP95Ms: number | null;
      latencyLatestMs: number;
      latencyP50Ms: number;
      latencyP95Ms: number;
      latencyMaxMs: number;
      latencySamples: number;
      decodeQueueSize: number;
      droppedFrames: number;
      canvasWidth: number;
      canvasHeight: number;
      rendererBackend: "webgl2" | "2d";
    }
  | {
      type: "closed";
    }
  | {
      type: "error";
      message: string;
    };

type WindowWithMrdFrameTimingDebug = Window & {
  __mrdFrameTimingDebug?: WebRtcFrameTimingDebug;
  __mrdWebPreviewPeer?: RTCPeerConnection;
};

type WebRtcVideoFrameCallbackMetadata = {
  presentedFrames?: number;
  presentationTime?: number;
  expectedDisplayTime?: number;
  captureTime?: number;
  receiveTime?: number;
  rtpTimestamp?: number;
};

const WEBRTC_FRAME_TIMING_CHANNEL = "mrd-frame-timing";
const WEBRTC_PRESENTATION_LATENCY_SAMPLE_LIMIT = 240;
const WEBRTC_FRAME_TIMING_STALE_MS = 200;

function updateWebRtcFrameTimingDebug(
  update: Partial<WebRtcFrameTimingDebug>
): void {
  if (typeof window === "undefined") return;
  const debugWindow = window as WindowWithMrdFrameTimingDebug;
  debugWindow.__mrdFrameTimingDebug = {
    received: 0,
    lastMessage: null,
    lastStats: null,
    ...debugWindow.__mrdFrameTimingDebug,
    ...update,
  };
}

const captureOptions: Option<CaptureType>[] = [
  { value: "dxgi", label: "DXGI" },
  { value: "winrt", label: "WinRT" },
  { value: "macos", label: "macOS" },
  { value: "linux", label: "Linux" },
  { value: "synthetic", label: "Synthetic" },
];

const encoderOptions: Option<EncoderType>[] = [
  { value: "none", label: "Direct" },
  { value: "nvenc_h264", label: "NVENC H.264" },
  { value: "nvenc_hevc", label: "NVENC HEVC Main" },
  { value: "nvenc_hevc_main10", label: "NVENC HEVC Main10" },
  { value: "videotoolbox_h264", label: "VideoToolbox H.264" },
  { value: "openh264", label: "OpenH264" },
  { value: "nvenc_av1", label: "NVENC AV1" },
];

const decoderOptions: Option<DecoderType>[] = [
  { value: "nvdec", label: "NVDEC" },
  { value: "linux_h264", label: "Linux H.264 HW" },
  { value: "linux_hevc", label: "Linux HEVC HW" },
  { value: "linux_hevc_main10", label: "Linux HEVC Main10 HW" },
  { value: "videotoolbox", label: "VideoToolbox" },
  { value: "software", label: "Software" },
  { value: "none", label: "No decode" },
];

const transportOptions: Option<TransportKind>[] = [
  { value: "loopback", label: "Loopback" },
  { value: "webrtc", label: "WebRTC" },
  { value: "quic", label: "QUIC" },
];

const linuxNativeEncoderPreference: EncoderType[] = ["none", "nvenc_h264", "openh264"];
const linuxNativeDecoderPreference: DecoderType[] = ["none", "linux_h264", "software"];

const resolutionOptions: Option<ResolutionKey>[] = [
  { value: "1280x720", label: "720p" },
  { value: "1920x1080", label: "1080p" },
  { value: "2560x1440", label: "1440p" },
  { value: "2560x1600", label: "1600p" },
  { value: "3440x1440", label: "UWQHD" },
  { value: "3840x2160", label: "4K" },
];

const fpsOptions: Option<FpsKey>[] = [
  { value: "30", label: "30 FPS" },
  { value: "60", label: "60 FPS" },
  { value: "120", label: "120 FPS" },
  { value: "144", label: "144 FPS" },
  { value: "165", label: "165 FPS" },
  { value: "180", label: "180 FPS" },
  { value: "249", label: "249 FPS" },
];

const bitrateOptions: Option<BitrateKey>[] = [
  { value: "8", label: "8 Mbps" },
  { value: "20", label: "20 Mbps" },
  { value: "50", label: "50 Mbps" },
  { value: "80", label: "80 Mbps" },
  { value: "100", label: "100 Mbps" },
  { value: "120", label: "120 Mbps" },
];

const captureSourcePickerOptions: Option<CaptureSourcePickerMode>[] = [
  { value: "dropdown", label: "下拉选择" },
  { value: "modal", label: "弹窗选择" },
];

const localTestDurationOptions: Option<LocalTestDurationMode>[] = [
  { value: "30s", label: "30S" },
  { value: "60s", label: "60S" },
  { value: "manual", label: "手动停止" },
];

const matrixDimensionOptions: Option<MatrixDimensionKey>[] = [
  { value: "capture", label: "CAP" },
  { value: "encoder", label: "ENC" },
  { value: "resolution", label: "SIZE" },
  { value: "fps", label: "FPS" },
  { value: "bitrate", label: "BR" },
];

export function browserWebrtcPreviewH264Profile(
  encoder: EncoderType,
  _decoder: DecoderType
): "baseline" | "high" {
  return encoder === "nvenc_h264" ? "high" : "baseline";
}

function optionLabel<T extends string>(options: Option<T>[], value: T) {
  return options.find((option) => option.value === value)?.label ?? value;
}

function captureSourceKindLabel(kind: string) {
  switch (kind) {
    case "display_shared":
      return "全屏 shared";
    case "display":
      return "全屏 copy";
    case "window":
      return "单窗口";
    default:
      return kind || "未知";
  }
}

function pickPreferredCaptureSource(sources: CaptureSource[]) {
  return (
    sources.find((source) => source.source_kind === "display_shared") ??
    sources.find((source) => source.source_kind === "display") ??
    sources.find((source) => source.source_kind === "window") ??
    sources[0] ??
    null
  );
}

function isNvencSharedTextureEncoder(encoder: EncoderType) {
  return (
    encoder === "nvenc_h264" ||
    encoder === "nvenc_hevc" ||
    encoder === "nvenc_hevc_main10" ||
    encoder === "nvenc_av1"
  );
}

function isHevcEncoder(encoder: EncoderType) {
  return encoder === "nvenc_hevc" || encoder === "nvenc_hevc_main10";
}

function browserLooksLikeMacos(): boolean {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform.toLowerCase();
  const userAgent = navigator.userAgent.toLowerCase();
  return platform.includes("mac") || userAgent.includes("mac os x");
}

function browserHostOs(): HostOs {
  if (browserLooksLikeMacos()) return "macos";
  if (typeof navigator === "undefined") return "other";
  const platform = navigator.platform.toLowerCase();
  const userAgent = navigator.userAgent.toLowerCase();
  if (platform.includes("linux") || userAgent.includes("linux")) return "linux";
  if (platform.includes("win") || userAgent.includes("windows")) return "windows";
  return "other";
}

function nativeRenderModeForHost(hostOs: HostOs): RenderMode {
  if (hostOs === "macos") return "metal_native";
  if (hostOs === "linux") return "linux_native";
  if (hostOs === "windows") return "d3d11_native";
  return "web";
}

function nativeRendererForHost(hostOs: HostOs): NonNullable<TestConfig["renderer_type"]> | null {
  if (hostOs === "macos") return "macos";
  if (hostOs === "linux") return "linux";
  if (hostOs === "windows") return "d3d11";
  return null;
}

function dash(value: string | number | null | undefined) {
  if (value === null || value === undefined || value === "") return "-";
  return String(value);
}

function formatFps(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(value >= 100 ? 0 : 1)} FPS` : "-";
}

function formatHz(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(value >= 100 ? 0 : 1)} Hz` : "-";
}

function formatMbps(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(value >= 10 ? 1 : 2)} Mbps` : "-";
}

function formatMs(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(value >= 10 ? 1 : 2)} ms` : "-";
}

function formatPercent(value: number) {
  return `${Math.max(0, value).toFixed(value >= 10 ? 0 : 1)}%`;
}

function formatOptionalPercent(value?: number | null) {
  return typeof value === "number" ? formatPercent(value) : "-";
}

function formatMb(value?: number | null) {
  return typeof value === "number" ? `${value.toLocaleString()} MB` : "-";
}

function bpsToMbps(value?: number | null) {
  return typeof value === "number" ? value * 8 / 1_000_000 : null;
}

function sumNullable(...values: Array<number | null | undefined>) {
  const finiteValues = values.filter((value): value is number => typeof value === "number");
  if (finiteValues.length === 0) return null;
  return finiteValues.reduce((total, value) => total + value, 0);
}

function formatCount(value?: number | null) {
  return typeof value === "number" ? value.toLocaleString() : "-";
}

function localTestDurationMs(mode: LocalTestDurationMode): number {
  if (mode === "60s") return 60_000;
  if (mode === "manual") return 24 * 60 * 60 * 1000;
  return 30_000;
}

function formatSummaryFps(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(1)} FPS` : "-";
}

function formatSummaryMs(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(1)} ms` : "-";
}

function formatDropped(value?: number | null) {
  return typeof value === "number" ? `${value.toLocaleString()} dropped` : "-";
}

function formatDurationMs(value?: number | null) {
  if (typeof value !== "number") return "-";
  const totalSeconds = Math.max(0, Math.round(value / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0
    ? `${minutes}m ${seconds.toString().padStart(2, "0")}s`
    : `${seconds}s`;
}

function formatTimestamp(value?: number | null) {
  if (typeof value !== "number") return "-";
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function runStatusLabel(status?: TestRun["status"] | null) {
  if (status === "completed") return "完成";
  if (status === "failed") return "失败";
  if (status === "cancelled") return "已停止";
  if (status === "running") return "运行中";
  if (status === "preparing") return "准备中";
  if (status === "queued") return "排队中";
  return "-";
}

function average(values: Array<number | null | undefined>) {
  const finiteValues = values.filter((value): value is number => typeof value === "number");
  if (finiteValues.length === 0) return null;
  return finiteValues.reduce((total, value) => total + value, 0) / finiteValues.length;
}

function maxValue(values: Array<number | null | undefined>) {
  const finiteValues = values.filter((value): value is number => typeof value === "number");
  return finiteValues.length > 0 ? Math.max(...finiteValues) : null;
}

function latestValue(values: Array<number | null | undefined>) {
  for (let index = values.length - 1; index >= 0; index -= 1) {
    const value = values[index];
    if (typeof value === "number") return value;
  }
  return null;
}

function configResolutionLabel(config?: TestConfig | null) {
  const resolution = config?.resolution;
  return resolution ? `${resolution[0]}x${resolution[1]}` : "-";
}

function configBitrateMbps(config?: TestConfig | null) {
  if (typeof config?.bitrate !== "number") return null;
  return config.bitrate / 1_000_000;
}

function configCodecLabel(config?: TestConfig | null) {
  return `${dash(config?.capture_type)} -> ${dash(config?.encoder_type)} -> ${dash(
    config?.decoder_type
  )} / ${dash(config?.transport_kind)}`;
}

function percentile(values: number[], percentileValue: number) {
  const finiteValues = values.filter((value) => Number.isFinite(value));
  if (finiteValues.length === 0) return null;

  const sorted = [...finiteValues].sort((a, b) => a - b);
  const index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(sorted.length * percentileValue) - 1)
  );
  return sorted[index] ?? null;
}

const WEBCODECS_CHUNK_MAGIC = "MRDWC01\0";
const WEBCODECS_BINARY_HEADER_LEN = 12;

export function parseWebCodecsAccessUnitMessage(
  data: ArrayBuffer
): WebCodecsAccessUnitMessage | null {
  if (data.byteLength < WEBCODECS_BINARY_HEADER_LEN) return null;
  const view = new DataView(data);
  let magic = "";
  for (let index = 0; index < 8; index += 1) {
    magic += String.fromCharCode(view.getUint8(index));
  }
  if (magic !== WEBCODECS_CHUNK_MAGIC) return null;
  const headerLength = view.getUint32(8, true);
  const payloadOffset = WEBCODECS_BINARY_HEADER_LEN + headerLength;
  if (payloadOffset > data.byteLength) return null;
  try {
    const headerBytes = new Uint8Array(data, WEBCODECS_BINARY_HEADER_LEN, headerLength);
    const header = JSON.parse(new TextDecoder().decode(headerBytes)) as WebCodecsFrameHeader;
    if (header.type !== "mrd.webcodecs.frame.v1") return null;
    return {
      header,
      payload: new Uint8Array(data, payloadOffset),
    };
  } catch {
    return null;
  }
}

function parseWebRtcFrameTimingMetadata(data: unknown): WebRtcFrameTimingMetadata | null {
  let raw: unknown = data;
  if (typeof data === "string") {
    try {
      raw = JSON.parse(data);
    } catch {
      return null;
    }
  }
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Record<string, unknown>;
  if (record.type !== "mrd.frame_timing.v1") return null;
  const sequence = numberFromStats(record.sequence);
  const captureUnixUs = numberFromStats(record.capture_unix_us);
  if (sequence === null || captureUnixUs === null) return null;
  return {
    sequence,
    captureUnixUs,
    sentUnixUs: numberFromStats(record.sent_unix_us),
    keyframe: record.keyframe === true,
    rtpTimestamp: numberFromStats(record.rtp_timestamp),
  };
}

export class WebRtcPresentationLatencyTracker {
  private readonly timeOriginMs: number;
  private readonly maxSamples: number;
  private readonly pendingBySequence: WebRtcFrameTimingMetadata[] = [];
  private readonly pendingByRtpTimestamp = new Map<number, WebRtcFrameTimingMetadata>();
  private readonly samplesMs: number[] = [];
  private lastPresentedFrames = 0;

  constructor(options: { timeOriginMs?: number; maxSamples?: number } = {}) {
    this.timeOriginMs =
      options.timeOriginMs ??
      (typeof performance !== "undefined" && typeof performance.now === "function"
        ? Date.now() - performance.now()
        : Date.now());
    this.maxSamples = options.maxSamples ?? WEBRTC_PRESENTATION_LATENCY_SAMPLE_LIMIT;
  }

  addMetadata(data: unknown): void {
    const metadata = parseWebRtcFrameTimingMetadata(data);
    if (!metadata) return;
    const insertAt = this.pendingBySequence.findIndex(
      (entry) => entry.sequence > metadata.sequence
    );
    if (insertAt >= 0) {
      this.pendingBySequence.splice(insertAt, 0, metadata);
    } else {
      this.pendingBySequence.push(metadata);
    }
    if (typeof metadata.rtpTimestamp === "number") {
      this.pendingByRtpTimestamp.set(metadata.rtpTimestamp, metadata);
    }
    while (this.pendingBySequence.length > this.maxSamples) {
      const dropped = this.pendingBySequence.shift();
      if (typeof dropped?.rtpTimestamp === "number") {
        this.pendingByRtpTimestamp.delete(dropped.rtpTimestamp);
      }
    }
  }

  observeFrame(
    nowMs: number,
    metadata: WebRtcVideoFrameCallbackMetadata
  ): WebRtcPresentationLatencyStats | null {
    const presentationTimeMs = metadata.presentationTime ?? metadata.expectedDisplayTime ?? nowMs;
    const browserCaptureTimeMs = numberFromStats(metadata.captureTime);
    if (browserCaptureTimeMs !== null) {
      return this.recordLatency(
        presentationTimeMs - browserCaptureTimeMs,
        "browser_capture_time"
      );
    }

    const presentationUnixMs = this.timeOriginMs + presentationTimeMs;
    const rtpTimestamp = numberFromStats(metadata.rtpTimestamp);
    const rtpTiming = rtpTimestamp === null ? null : this.consumeRtpTiming(rtpTimestamp);
    const timing = rtpTiming ?? this.consumeSequenceTiming(metadata.presentedFrames, presentationUnixMs);
    if (!timing) return this.currentStats();

    return this.recordLatency(
      presentationUnixMs - timing.captureUnixUs / 1000,
      rtpTiming ? "rtp_frame_timing_channel" : "frame_timing_channel"
    );
  }

  private consumeRtpTiming(rtpTimestamp: number): WebRtcFrameTimingMetadata | null {
    const timing = this.pendingByRtpTimestamp.get(rtpTimestamp) ?? null;
    if (!timing) return null;
    this.pendingByRtpTimestamp.delete(rtpTimestamp);
    const index = this.pendingBySequence.findIndex((entry) => entry === timing);
    if (index >= 0) {
      this.pendingBySequence.splice(0, index + 1);
    }
    return timing;
  }

  private consumeSequenceTiming(
    presentedFrames: number | undefined,
    presentationUnixMs: number
  ): WebRtcFrameTimingMetadata | null {
    while (
      this.pendingBySequence.length > 1 &&
      presentationUnixMs - (this.pendingBySequence[0]?.captureUnixUs ?? 0) / 1000 >
        WEBRTC_FRAME_TIMING_STALE_MS
    ) {
      const dropped = this.pendingBySequence.shift();
      if (typeof dropped?.rtpTimestamp === "number") {
        this.pendingByRtpTimestamp.delete(dropped.rtpTimestamp);
      }
    }

    const frameDelta =
      typeof presentedFrames === "number"
        ? this.lastPresentedFrames > 0
          ? Math.max(1, presentedFrames - this.lastPresentedFrames)
          : Math.max(1, Math.min(presentedFrames, this.pendingBySequence.length))
        : 1;
    if (typeof presentedFrames === "number") {
      this.lastPresentedFrames = presentedFrames;
    }

    let timing: WebRtcFrameTimingMetadata | null = null;
    for (let index = 0; index < frameDelta; index += 1) {
      const next = this.pendingBySequence.shift();
      if (!next) break;
      if (typeof next.rtpTimestamp === "number") {
        this.pendingByRtpTimestamp.delete(next.rtpTimestamp);
      }
      timing = next;
    }
    return timing;
  }

  private recordLatency(
    latencyMs: number,
    source: WebRtcPresentationLatencyStats["source"]
  ): WebRtcPresentationLatencyStats | null {
    if (!Number.isFinite(latencyMs) || latencyMs < 0 || latencyMs > 10_000) {
      return this.currentStats();
    }
    this.samplesMs.push(latencyMs);
    while (this.samplesMs.length > this.maxSamples) {
      this.samplesMs.shift();
    }
    return this.currentStats(source);
  }

  private currentStats(
    source: WebRtcPresentationLatencyStats["source"] = "frame_timing_channel"
  ): WebRtcPresentationLatencyStats | null {
    if (this.samplesMs.length === 0) return null;
    const latestMs = this.samplesMs[this.samplesMs.length - 1] ?? 0;
    return {
      latestMs,
      p50Ms: percentile(this.samplesMs, 0.5) ?? 0,
      p95Ms: percentile(this.samplesMs, 0.95) ?? 0,
      maxMs: Math.max(...this.samplesMs),
      samples: this.samplesMs.length,
      source,
    };
  }
}

function findStageMetric(
  snapshot: MediaPipelineSnapshot | null,
  stageNames: string[]
): MediaPipelineSnapshot["stage_metrics"][number] | null {
  const metrics = snapshot?.stage_metrics ?? [];
  for (const stageName of stageNames) {
    const exact = metrics.find((metric) => metric.stage === stageName);
    if (exact) return exact;
  }

  return (
    metrics.find((metric) =>
      stageNames.some((stageName) => metric.stage.toLowerCase().includes(stageName.toLowerCase()))
    ) ?? null
  );
}

function findStageP95(snapshot: MediaPipelineSnapshot | null, stageNames: string[]) {
  return findStageMetric(snapshot, stageNames)?.p95_ms ?? null;
}

function hasDiagnosticsSampleValue(sample: DiagnosticsSample) {
  return [
    sample.fps,
    sample.paintFps,
    sample.latencyP95Ms,
    sample.captureP95Ms,
    sample.encodeP95Ms,
    sample.transportP95Ms,
    sample.decodeP95Ms,
    sample.renderP95Ms,
    sample.queueDepth,
    sample.droppedFrames,
    sample.bitrateMbps,
    sample.serviceCpuPercent,
    sample.serviceMemoryPercent,
    sample.serviceGpuPercent,
    sample.serviceNetworkRxMbps,
    sample.serviceNetworkTxMbps,
    sample.displayCpuPercent,
    sample.displayMemoryPercent,
    sample.displayGpuPercent,
    sample.displayNetworkRxMbps,
    sample.displayNetworkTxMbps,
  ].some((value) => typeof value === "number");
}

function numberFromStats(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function stringFromStats(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function deltaAverageSeconds(
  currentTotal: number | null,
  previousTotal: number | null,
  currentCount: number | null,
  previousCount: number | null
) {
  if (
    currentTotal === null ||
    previousTotal === null ||
    currentCount === null ||
    previousCount === null
  ) {
    return null;
  }
  const countDelta = currentCount - previousCount;
  if (countDelta <= 0) return null;
  return Math.max(0, currentTotal - previousTotal) / countDelta * 1000;
}

function inboundVideoCountersFromStats(
  report: RTCStatsReport
): WebRtcInboundVideoCounters | null {
  for (const rawStats of report.values()) {
    const stats = rawStats as Record<string, unknown>;
    if (
      stringFromStats(stats.type) !== "inbound-rtp" ||
      stringFromStats(stats.kind) !== "video"
    ) {
      continue;
    }

    const framesDecoded = numberFromStats(stats.framesDecoded);
    const timestampMs = numberFromStats(stats.timestamp);
    if (framesDecoded === null || timestampMs === null) return null;

    return {
      timestampMs,
      framesDecoded,
      framesDropped: numberFromStats(stats.framesDropped) ?? 0,
      packetsLost: numberFromStats(stats.packetsLost) ?? 0,
      jitterSeconds: numberFromStats(stats.jitter),
      jitterBufferDelaySeconds: numberFromStats(stats.jitterBufferDelay),
      jitterBufferEmittedCount: numberFromStats(stats.jitterBufferEmittedCount),
      totalDecodeTimeSeconds: numberFromStats(stats.totalDecodeTime),
      totalProcessingDelaySeconds: numberFromStats(stats.totalProcessingDelay),
      totalInterFrameDelaySeconds: numberFromStats(stats.totalInterFrameDelay),
      freezeCount: numberFromStats(stats.freezeCount) ?? 0,
      frameWidth: numberFromStats(stats.frameWidth),
      frameHeight: numberFromStats(stats.frameHeight),
    };
  }
  return null;
}

export function summarizeWebRtcInboundVideoStats(
  report: RTCStatsReport,
  previous: WebRtcInboundVideoCounters | null,
  fallbackNowMs: number
): { stats: WebRtcReceiverStats | null; counters: WebRtcInboundVideoCounters | null } {
  const counters = inboundVideoCountersFromStats(report);
  if (!counters) return { stats: null, counters: null };

  const previousTimestamp = previous?.timestampMs ?? fallbackNowMs;
  const elapsedSeconds = Math.max(0.001, (counters.timestampMs - previousTimestamp) / 1000);
  const decodedDelta =
    previous ? Math.max(0, counters.framesDecoded - previous.framesDecoded) : null;
  const decodedFps = decodedDelta === null ? null : decodedDelta / elapsedSeconds;
  const decodeAvgMs = previous
    ? deltaAverageSeconds(
        counters.totalDecodeTimeSeconds,
        previous.totalDecodeTimeSeconds,
        counters.framesDecoded,
        previous.framesDecoded
      )
    : null;
  const processingDelayAvgMs = previous
    ? deltaAverageSeconds(
        counters.totalProcessingDelaySeconds,
        previous.totalProcessingDelaySeconds,
        counters.framesDecoded,
        previous.framesDecoded
      )
    : null;
  const interFrameDelayAvgMs = previous
    ? deltaAverageSeconds(
        counters.totalInterFrameDelaySeconds,
        previous.totalInterFrameDelaySeconds,
        counters.framesDecoded,
        previous.framesDecoded
      )
    : null;
  const jitterBufferDelayAvgMs = previous
    ? deltaAverageSeconds(
        counters.jitterBufferDelaySeconds,
        previous.jitterBufferDelaySeconds,
        counters.jitterBufferEmittedCount,
        previous.jitterBufferEmittedCount
      )
    : null;

  return {
    counters,
    stats: {
      decodedFps,
      framesDecoded: counters.framesDecoded,
      framesDropped: counters.framesDropped,
      packetsLost: counters.packetsLost,
      jitterMs: counters.jitterSeconds === null ? null : counters.jitterSeconds * 1000,
      jitterBufferDelayAvgMs,
      decodeAvgMs,
      processingDelayAvgMs,
      interFrameDelayAvgMs,
      freezeCount: counters.freezeCount,
      frameWidth: counters.frameWidth,
      frameHeight: counters.frameHeight,
    },
  };
}

export function buildWebRtcDiagnosticsStageRows(
  stats: WebRtcReceiverStats | null,
  presentationLatencyStats: WebRtcPresentationLatencyStats | null = null
): DiagnosticsStageRow[] {
  const rows: DiagnosticsStageRow[] = [];
  if (presentationLatencyStats) {
    rows.push(
      {
        label: "e2e.capture_to_present_p50",
        value: presentationLatencyStats.p50Ms,
        samples: presentationLatencyStats.samples,
      },
      {
        label: "e2e.capture_to_present_p95",
        value: presentationLatencyStats.p95Ms,
        samples: presentationLatencyStats.samples,
      },
      {
        label: "e2e.capture_to_present_max",
        value: presentationLatencyStats.maxMs,
        samples: presentationLatencyStats.samples,
      }
    );
  }
  if (!stats) return rows;
  rows.push(
    { label: "webrtc.decode_avg", value: stats.decodeAvgMs },
    { label: "webrtc.jitter_buffer_avg", value: stats.jitterBufferDelayAvgMs },
    { label: "webrtc.processing_avg", value: stats.processingDelayAvgMs },
    { label: "webrtc.render_interval_avg", value: stats.interFrameDelayAvgMs },
  );
  return rows.filter((row) => typeof row.value === "number");
}

function codecLabel(codec?: string | null, profile?: string | null) {
  const normalized = codec?.toLowerCase();
  const family =
    normalized === "hevc" || normalized === "h265"
      ? "H.265"
      : normalized === "h264"
        ? "H.264"
        : normalized === "av1"
          ? "AV1"
          : dash(codec);
  if (!profile) return family;
  return `${family} ${profile.toLowerCase() === "main" ? "Main" : profile}`;
}

function codecFromEncoder(encoder: EncoderType): string | null {
  if (encoder.includes("hevc")) return "hevc";
  if (encoder.includes("h264") || encoder === "openh264") return "h264";
  if (encoder.includes("av1")) return "av1";
  return null;
}

function decoderLabel(decoder?: string | null) {
  switch (decoder) {
    case "nvdec_hevc_d3d11_shared":
    case "nvdec_d3d11_shared_hevc":
      return "NVDEC HEVC / D3D11";
    case "nvdec_hevc":
      return "NVDEC HEVC";
    case "nvdec":
    case "nvdec_d3d11_shared":
      return "NVDEC";
    case "h264_software":
    case "software":
      return "Software";
    default:
      return dash(decoder);
  }
}

function captureMethodLabel(source?: CaptureSourceSelection | null, capture?: CaptureType) {
  const sourceKind = source?.source.source_kind;
  if (source?.source.platform === "windows" && sourceKind === "display_shared") {
    return "DXGINative";
  }
  if (capture === "dxgi") return "DXGINative";
  if (capture === "winrt") return "WinRT";
  if (capture === "linux") return "PipeWire";
  return dash(capture);
}

function defaultNativeRenderMode(): RenderMode {
  return nativeRenderModeForHost(browserHostOs());
}

function normalizeOs(osType?: string): HostOs {
  const os = osType?.toLowerCase() ?? "";
  if (os.includes("mac")) return "macos";
  if (os.includes("win")) return "windows";
  if (os.includes("linux")) return "linux";
  return "other";
}

function pickAvailable<T extends string>(
  current: T,
  available: readonly string[] | undefined,
  preferred: readonly T[],
  fallback: T
): T {
  if (!available || available.length === 0) return current;
  if (available.includes(current)) return current;
  return preferred.find((value) => available.includes(value)) ?? fallback;
}

function uniqueValues<T extends string>(values: readonly T[]): T[] {
  return values.filter((value, index) => values.indexOf(value) === index);
}

function pickCapability<T extends string>(
  candidates: readonly T[],
  available: readonly string[] | undefined
): T | null {
  if (!available || available.length === 0) return candidates[0] ?? null;
  return candidates.find((value) => available.includes(value)) ?? null;
}

function isH264PreviewEncoder(encoder: EncoderType) {
  return (
    encoder === "openh264" ||
    encoder === "videotoolbox_h264" ||
    encoder === "nvenc_h264"
  );
}

function browserSupportsH264WebrtcVideo(): boolean {
  if (typeof RTCRtpReceiver === "undefined") return true;
  const capabilities = RTCRtpReceiver.getCapabilities?.("video");
  if (!capabilities?.codecs?.length) return true;
  return capabilities.codecs.some(
    (codec) => codec.mimeType.toLowerCase() === "video/h264"
  );
}

export function browserSupportsWebCodecsH264(): boolean {
  if (typeof window === "undefined") return false;
  const maybeWindow = window as Window & {
    VideoDecoder?: unknown;
    EncodedVideoChunk?: unknown;
  };
  return Boolean(maybeWindow.VideoDecoder && maybeWindow.EncodedVideoChunk);
}

export function browserSupportsWebCodecsWorkerRendering(
  canvas: WebCodecsWorkerCapableCanvas | null
): boolean {
  if (!browserSupportsWebCodecsH264()) return false;
  if (typeof window === "undefined") return false;
  const maybeWindow = window as Window & {
    Worker?: unknown;
    OffscreenCanvas?: unknown;
  };
  return Boolean(
    maybeWindow.Worker &&
      maybeWindow.OffscreenCanvas &&
      canvas?.transferControlToOffscreen
  );
}

export function webCodecsMemoryPathLabelFromState(
  channelState: string | null | undefined
): string {
  if (channelState === "webcodecs-worker:webgl2") return "WebGL2 OffscreenCanvas";
  if (channelState === "webcodecs-worker:2d") return "OffscreenCanvas 2D";
  if (channelState?.startsWith("webcodecs-worker")) return "OffscreenCanvas";
  return "WebCodecs canvas";
}

export function webPreviewDecoderLabel(
  engine: WebPreviewEngine,
  fallbackLabel: string
): string {
  if (engine === "webcodecs") return "Browser WebCodecs";
  if (engine === "webrtc") return "Browser video decode";
  return fallbackLabel;
}

export function webPreviewTransportLabel(
  engine: WebPreviewEngine,
  fallbackLabel: string
): string {
  if (engine === "webcodecs") return "WebSocket AU bridge";
  if (engine === "webrtc") return "WebRTC RTP";
  return fallbackLabel;
}

const WEBRTC_LOW_LATENCY_PLAYOUT_SECONDS = 0.02;
const WEBRTC_VIDEO_BACKLOG_SWITCH_MIN_FPS = 90;
const WEBRTC_VIDEO_BACKLOG_SWITCH_LATENCY_MS = 120;
const WEBRTC_VIDEO_BACKLOG_SWITCH_METADATA_AGE_MS = 80;
const WEBRTC_VIDEO_BACKLOG_SWITCH_JITTER_BUFFER_MS = 35;
const WEBRTC_VIDEO_BACKLOG_SWITCH_FPS_RATIO = 0.75;

export function applyWebRtcReceiverLowLatencyHint(receiver?: RTCRtpReceiver | null) {
  if (!receiver) return;
  const tunableReceiver = receiver as RTCRtpReceiver & {
    playoutDelayHint?: number;
    jitterBufferTarget?: number;
  };
  try {
    tunableReceiver.playoutDelayHint = WEBRTC_LOW_LATENCY_PLAYOUT_SECONDS;
  } catch {
    // Optional browser API.
  }
  try {
    tunableReceiver.jitterBufferTarget = WEBRTC_LOW_LATENCY_PLAYOUT_SECONDS;
  } catch {
    // Optional browser API.
  }
}

export function applyWebRtcVideoMotionHint(track?: MediaStreamTrack | null) {
  if (!track) return;
  try {
    track.contentHint = "motion";
  } catch {
    // Optional browser hint.
  }
}

export type WebRtcVideoToWebCodecsSwitchInput = {
  targetFps: number;
  actualFps: number | null;
  latencyP95Ms: number | null;
  metadataAgeMs: number | null;
  jitterBufferMs: number | null;
  webCodecsAvailable: boolean;
  allowAutoSwitch: boolean;
  alreadyAttempted: boolean;
};

export type WebRtcVideoToWebCodecsSwitchDecision = {
  shouldSwitch: boolean;
  reason: string | null;
};

export function shouldAutoSwitchWebRtcVideoToWebCodecs({
  targetFps,
  actualFps,
  latencyP95Ms,
  metadataAgeMs,
  jitterBufferMs,
  webCodecsAvailable,
  allowAutoSwitch,
  alreadyAttempted,
}: WebRtcVideoToWebCodecsSwitchInput): WebRtcVideoToWebCodecsSwitchDecision {
  if (
    !allowAutoSwitch ||
    !webCodecsAvailable ||
    alreadyAttempted ||
    targetFps < WEBRTC_VIDEO_BACKLOG_SWITCH_MIN_FPS
  ) {
    return { shouldSwitch: false, reason: null };
  }

  const highLatency =
    typeof latencyP95Ms === "number" &&
    latencyP95Ms >= WEBRTC_VIDEO_BACKLOG_SWITCH_LATENCY_MS;
  const staleMetadata =
    typeof metadataAgeMs === "number" &&
    metadataAgeMs >= WEBRTC_VIDEO_BACKLOG_SWITCH_METADATA_AGE_MS;
  const jitterBacklog =
    typeof jitterBufferMs === "number" &&
    jitterBufferMs >= WEBRTC_VIDEO_BACKLOG_SWITCH_JITTER_BUFFER_MS;
  const lowFps =
    typeof actualFps === "number" &&
    actualFps > 0 &&
    actualFps < targetFps * WEBRTC_VIDEO_BACKLOG_SWITCH_FPS_RATIO;

  if (!highLatency || (!staleMetadata && !jitterBacklog && !lowFps)) {
    return { shouldSwitch: false, reason: null };
  }

  return {
    shouldSwitch: true,
    reason: `WebRTC video backlog: p95 ${latencyP95Ms.toFixed(1)} ms, metadata age ${
      typeof metadataAgeMs === "number" ? metadataAgeMs.toFixed(1) : "-"
    } ms, fps ${typeof actualFps === "number" ? actualFps.toFixed(1) : "-"}/${targetFps}. Switching to WebCodecs web path.`,
  };
}

function fpsForWebView(fps: FpsKey): FpsKey {
  return Number(fps) > WEB_VIEW_MAX_FPS ? "144" : fps;
}

function optionValueFromSearch<T extends string>(
  options: Option<T>[],
  value: string | null
): T | null {
  if (!value) return null;
  return options.some((option) => option.value === value) ? (value as T) : null;
}

function resolutionFromSearch(searchParams: URLSearchParams): ResolutionKey | null {
  const direct = optionValueFromSearch(resolutionOptions, searchParams.get("resolution"));
  if (direct) return direct;
  const width = searchParams.get("width") ?? searchParams.get("profileWidth");
  const height = searchParams.get("height") ?? searchParams.get("profileHeight");
  if (!width || !height) return null;
  return optionValueFromSearch(resolutionOptions, `${width}x${height}`);
}

function fpsFromSearch(searchParams: URLSearchParams): FpsKey | null {
  return optionValueFromSearch(fpsOptions, searchParams.get("fps") ?? searchParams.get("profileFps"));
}

function bitrateFromSearch(searchParams: URLSearchParams): BitrateKey | null {
  return optionValueFromSearch(
    bitrateOptions,
    searchParams.get("bitrateMbps") ?? searchParams.get("profileBitrateMbps")
  );
}

type LocalWebViewProfile = {
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport: TransportKind;
  fps: FpsKey;
  bitrate: BitrateKey;
};

type LocalWebViewPlan = {
  profile: LocalWebViewProfile | null;
  reason: string | null;
  changed: boolean;
  message: string | null;
};

function isExplicitBrowser2k144LowLatencyProfile({
  capture,
  encoder,
  decoder,
  transport,
  fps,
  bitrate,
  resolution,
}: LocalWebViewProfile & { resolution: ResolutionKey }): boolean {
  return (
    capture === "dxgi" &&
    encoder === "nvenc_h264" &&
    decoder === "none" &&
    transport === "webrtc" &&
    resolution === "2560x1440" &&
    fps === "144" &&
    bitrate === "20"
  );
}

function resolveLocalWebViewPlan({
  capabilities,
  hostOs,
  capture,
  encoder,
  decoder,
  transport,
  fps,
  bitrate,
  capHighFpsBitrate,
}: {
  capabilities: EnvironmentSnapshot | null;
  hostOs: HostOs;
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport: TransportKind;
  fps: FpsKey;
  bitrate: BitrateKey;
  capHighFpsBitrate: boolean;
}): LocalWebViewPlan {
  const captureDefaults: CaptureType[] =
    hostOs === "macos"
      ? ["macos", "synthetic"]
      : hostOs === "windows"
        ? ["dxgi", "winrt", "synthetic"]
        : hostOs === "linux"
          ? ["linux", "synthetic"]
        : ["synthetic"];
  const nextCapture = pickCapability(
    uniqueValues([capture, ...captureDefaults]),
    capabilities?.available_captures
  );

  if (!nextCapture) {
    return {
      profile: null,
      reason: "Web View 未找到可用采集源",
      changed: false,
      message: null,
    };
  }

  const preferredEncoders: EncoderType[] =
    hostOs === "macos"
      ? ["openh264", "videotoolbox_h264"]
      : hostOs === "windows"
        ? ["nvenc_h264", "openh264"]
        : ["openh264"];
  const targetFps = fpsForWebView(fps);
  const targetFpsNumber = Number(targetFps);
  const hardwareH264Encoders: EncoderType[] =
    hostOs === "macos" ? ["videotoolbox_h264"] : ["nvenc_h264"];
  const requiresHardwareH264 = targetFpsNumber > 30;
  const previewEncoderCandidates = requiresHardwareH264
    ? hardwareH264Encoders
    : preferredEncoders;
  const encoderCandidates = uniqueValues([
    ...previewEncoderCandidates,
    ...(isH264PreviewEncoder(encoder) && !requiresHardwareH264 ? [encoder] : []),
  ]);
  const nextEncoder = pickCapability(
    encoderCandidates,
    capabilities?.available_encoders
  );

  if (!nextEncoder) {
    return {
      profile: null,
      reason: requiresHardwareH264
        ? `网页 ${targetFps} FPS 本机采集需要硬件 H.264 编码器；当前 service 未报告 NVENC/VideoToolbox H.264 可用，OpenH264 仅作为 <=30 FPS 诊断兜底。`
        : "Web View 需要可输出 H.264 的编码器",
      changed: false,
      message: null,
    };
  }

  const nextDecoder: DecoderType = "none";
  const nextBitrate: BitrateKey =
    capHighFpsBitrate && targetFpsNumber >= 120 && Number(bitrate) > 8 ? "8" : bitrate;

  const profile: LocalWebViewProfile = {
    capture: nextCapture,
    encoder: nextEncoder,
    decoder: nextDecoder,
    transport: "webrtc",
    fps: targetFps,
    bitrate: nextBitrate,
  };
  const changed =
    profile.capture !== capture ||
    profile.encoder !== encoder ||
    profile.decoder !== decoder ||
    profile.transport !== transport ||
    profile.fps !== fps ||
    profile.bitrate !== bitrate;

  return {
    profile,
    reason: null,
    changed,
    message: changed
      ? `Web View 已切换到 ${optionLabel(captureOptions, profile.capture)} / ${optionLabel(
          encoderOptions,
          profile.encoder
        )} / Browser video decode / WebRTC RTP / ${optionLabel(fpsOptions, profile.fps)} / ${optionLabel(
          bitrateOptions,
          profile.bitrate
        )}`
      : null,
  };
}

export function isLocalPipelinePreviewSession(sessionId: string): boolean {
  return sessionId === "local-preview" || sessionId.startsWith("local-display-test");
}

function waitForIceGatheringComplete(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === "complete") {
    return Promise.resolve();
  }

  return new Promise((resolve) => {
    const timeout = window.setTimeout(done, 1500);

    function done() {
      window.clearTimeout(timeout);
      peer.removeEventListener("icegatheringstatechange", onChange);
      resolve();
    }

    function onChange() {
      if (peer.iceGatheringState === "complete") {
        done();
      }
    }

    peer.addEventListener("icegatheringstatechange", onChange);
  });
}

function TitleSelect<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled = false,
  title,
  className = "",
}: {
  label: string;
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  title?: string;
  className?: string;
}) {
  return (
    <label
      className={`flex h-9 min-w-0 items-center gap-1 rounded-md border border-white/10 bg-black/20 px-2 text-[10px] text-slate-400 ${
        disabled ? "opacity-60" : ""
      } ${className}`}
      title={title ?? label}
    >
      <span className="shrink-0 uppercase tracking-normal">{label}</span>
      <select
        className="min-w-0 bg-transparent text-[11px] font-medium text-slate-100 outline-none disabled:cursor-not-allowed disabled:text-slate-500"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value as T)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} className="bg-[#111827] text-slate-100">
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function ReadonlyTitleValue({
  label,
  value,
  title,
}: {
  label: string;
  value: string;
  title?: string;
}) {
  return (
    <div
      className="flex h-9 min-w-0 items-center gap-1 rounded-md border border-white/10 bg-black/20 px-2 text-[10px] text-slate-400"
      title={title ?? label}
    >
      <span className="shrink-0 uppercase tracking-normal">{label}</span>
      <span className="min-w-0 truncate text-[11px] font-medium text-slate-100">
        {value}
      </span>
    </div>
  );
}

function TileOptionGroup<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled = false,
  title,
}: {
  label: string;
  value: T;
  options: Array<Option<T> & { disabledReason?: string | null }>;
  onChange: (value: T) => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <section
      className="min-w-0 rounded-lg border border-white/10 bg-black/18 p-2"
      title={title ?? label}
    >
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-normal text-slate-400">
        {label}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {options.map((option) => {
          const optionDisabled = disabled || Boolean(option.disabledReason);
          const selected = option.value === value;
          return (
            <button
              key={option.value}
              type="button"
              aria-label={`${label} ${option.label}`}
              className={`min-h-8 rounded-md border px-2 py-1 text-[11px] font-medium transition ${
                selected
                  ? "border-cyan-300/60 bg-cyan-500/20 text-cyan-50"
                  : optionDisabled
                    ? "cursor-not-allowed border-white/8 bg-white/[0.03] text-slate-600"
                    : "border-white/10 bg-white/[0.03] text-slate-200 hover:border-cyan-300/45 hover:bg-cyan-500/12"
              }`}
              disabled={optionDisabled}
              title={option.disabledReason ?? title ?? option.label}
              onClick={() => onChange(option.value)}
            >
              {option.label}
            </button>
          );
        })}
      </div>
    </section>
  );
}

function MultiTileOptionGroup<T extends string>({
  label,
  values,
  options,
  onToggle,
  disabled = false,
  title,
}: {
  label: string;
  values: readonly T[];
  options: Array<Option<T> & { disabledReason?: string | null }>;
  onToggle: (value: T) => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-black/18 p-2" title={title ?? label}>
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-normal text-slate-400">
        {label}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {options.map((option) => {
          const optionDisabled = disabled || Boolean(option.disabledReason);
          const selected = values.includes(option.value);
          return (
            <button
              key={option.value}
              type="button"
              aria-label={`${label} ${option.label}`}
              className={`min-h-8 rounded-md border px-2 py-1 text-[11px] font-medium transition ${
                selected
                  ? "border-violet-300/60 bg-violet-500/20 text-violet-50"
                  : optionDisabled
                    ? "cursor-not-allowed border-white/8 bg-white/[0.03] text-slate-600"
                    : "border-white/10 bg-white/[0.03] text-slate-200 hover:border-violet-300/45 hover:bg-violet-500/12"
              }`}
              disabled={optionDisabled}
              title={option.disabledReason ?? title ?? option.label}
              onClick={() => onToggle(option.value)}
            >
              {option.label}
            </button>
          );
        })}
      </div>
    </section>
  );
}

function DiagnosticGroup({
  title,
  rows,
}: {
  title: string;
  rows: Array<[string, string | number]>;
}) {
  return (
    <section className="rounded-md border border-emerald-400/10 bg-emerald-950/20 p-3">
      <div className="mb-2 text-[12px] font-semibold text-emerald-100">{title}</div>
      <div className="grid gap-1.5">
        {rows.map(([label, value]) => (
          <div key={label} className="grid grid-cols-[92px_1fr] gap-3">
            <span className="text-emerald-200/60">{label}</span>
            <span className="min-w-0 truncate text-emerald-50">{value}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function DiagnosticsSparkline({
  samples,
  value,
  colorClass,
}: {
  samples: DiagnosticsSample[];
  value: (sample: DiagnosticsSample) => number | null;
  colorClass: string;
}) {
  const points = samples
    .map((sample, index) => ({ index, metric: value(sample) }))
    .filter((point): point is { index: number; metric: number } => typeof point.metric === "number");

  if (points.length < 2) {
    return (
      <div className="flex h-12 items-center justify-center rounded border border-emerald-300/10 bg-black/20 text-[10px] text-emerald-100/45">
        等待样本
      </div>
    );
  }

  const min = Math.min(...points.map((point) => point.metric));
  const max = Math.max(...points.map((point) => point.metric));
  const span = Math.max(0.001, max - min);
  const maxIndex = Math.max(1, samples.length - 1);
  const polyline = points
    .map((point) => {
      const x = point.index / maxIndex * 100;
      const y = 36 - (point.metric - min) / span * 30;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");

  return (
    <svg
      className="h-12 w-full rounded border border-emerald-300/10 bg-black/20"
      viewBox="0 0 100 40"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <polyline
        points={polyline}
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        vectorEffect="non-scaling-stroke"
        className={colorClass}
      />
    </svg>
  );
}

function DiagnosticMetricTile({
  title,
  value,
  subtitle,
  samples,
  sampleValue,
  colorClass,
}: {
  title: string;
  value: string;
  subtitle: string;
  samples: DiagnosticsSample[];
  sampleValue: (sample: DiagnosticsSample) => number | null;
  colorClass: string;
}) {
  return (
    <div className="rounded-md border border-emerald-400/10 bg-emerald-950/20 p-3">
      <div className="mb-2 flex items-start justify-between gap-2">
        <div>
          <div className="text-[11px] font-semibold text-emerald-100">{title}</div>
          <div className="text-[10px] text-emerald-200/55">{subtitle}</div>
        </div>
        <div className="text-right text-sm font-semibold text-white">{value}</div>
      </div>
      <DiagnosticsSparkline samples={samples} value={sampleValue} colorClass={colorClass} />
    </div>
  );
}

function resourceMetric(
  snapshot: SystemResourceSnapshot | null,
  value: (snapshot: SystemResourceSnapshot) => number | null | undefined
) {
  if (!snapshot?.target_found) return null;
  const next = value(snapshot);
  return typeof next === "number" && Number.isFinite(next) ? next : null;
}

function resourceNetworkSubtitle(snapshot: SystemResourceSnapshot | null) {
  if (!snapshot?.network_metrics_available) return "网络指标不可用";
  const scope = snapshot.network_metrics_scope === "system" ? "系统网卡" : "进程";
  return `${scope} RX/TX`;
}

function resourceGpuSubtitle(snapshot: SystemResourceSnapshot | null) {
  if (!snapshot?.gpu_metrics_available) return "GPU 指标不可用";
  const scope = snapshot.gpu_metrics_scope === "process" ? "进程显存 / 系统利用率" : "系统 GPU";
  return scope;
}

function DiagnosticStageList({ rows }: { rows: DiagnosticsStageRow[] }) {
  const visibleRows = rows.filter((row) => typeof row.value === "number");
  return (
    <section className="rounded-md border border-emerald-400/10 bg-emerald-950/20 p-3">
      <div className="mb-2 flex items-center gap-2 text-[12px] font-semibold text-emerald-100">
        <Gauge className="h-3.5 w-3.5 text-emerald-300" />
        阶段延迟 P95
      </div>
      {visibleRows.length > 0 ? (
        <div className="grid gap-1.5">
          {visibleRows.map((row) => (
            <div key={row.label} className="grid grid-cols-[1fr_76px_52px] items-center gap-2">
              <span className="min-w-0 truncate text-emerald-200/70">{row.label}</span>
              <span className="text-right font-medium text-emerald-50">{formatMs(row.value)}</span>
              <span className="text-right text-emerald-200/45">
                {row.samples ? `${row.samples}x` : "-"}
              </span>
            </div>
          ))}
        </div>
      ) : (
        <div className="text-emerald-200/55">当前路径暂无阶段延迟样本</div>
      )}
    </section>
  );
}

export function RemoteDisplayWindowPage() {
  const { id } = useParams();
  const [searchParams] = useSearchParams();
  const surfaceId = searchParams.get("surface") ?? "surface-1";
  const renderAreaRef = useRef<HTMLDivElement | null>(null);
  const syncAnimationFrameRef = useRef<number | null>(null);
  const syncTimerIdsRef = useRef<number[]>([]);
  const webPreviewVideoRef = useRef<HTMLVideoElement | null>(null);
  const webCodecsCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const webCodecsTransferredCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const webCodecsWorkerInitRetryRef = useRef(0);
  const webCodecsMainCanvasRecoveringRef = useRef(false);
  const webPreviewPeerRef = useRef<RTCPeerConnection | null>(null);
  const webPreviewSessionRef = useRef<string | null>(null);
  const webRtcStatsCountersRef = useRef<WebRtcInboundVideoCounters | null>(null);
  const autoStartRequestedRef = useRef<string | null>(null);
  const autoCaptureSourceRequestedRef = useRef<string | null>(null);
  const nativePreviewFrameKeyRef = useRef<string | null>(null);
  const linuxNativeProfileAppliedRef = useRef(false);
  const diagnosticsCurrentRef = useRef<DiagnosticsSample | null>(null);
  const diagnosticsSamplesRef = useRef<DiagnosticsSample[]>([]);
  const diagnosticsPopoverRef = useRef<HTMLDivElement | null>(null);
  const matrixStopRequestedRef = useRef(false);
  const webPreviewRunStartedAtRef = useRef<number | null>(null);
  const webPreviewAutoStopTimerRef = useRef<number | null>(null);
  const webVideoFpsRef = useRef<number | null>(null);
  const webVideoFrameCountRef = useRef(0);
  const webRtcReceiverStatsRef = useRef<WebRtcReceiverStats | null>(null);
  const webPresentationLatencyStatsRef = useRef<WebRtcPresentationLatencyStats | null>(null);

  const [context, setContext] = useState<RemoteDisplayWindowContext | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [nativeSurface, setNativeSurface] =
    useState<NativeRenderSurfaceSnapshot | null>(null);
  const [renderMode, setRenderMode] = useState<RenderMode>(() =>
    isTauriRuntime() ? defaultNativeRenderMode() : "web"
  );
  const [capture, setCapture] = useState<CaptureType>("dxgi");
  const [encoder, setEncoder] = useState<EncoderType>("nvenc_hevc");
  const [decoder, setDecoder] = useState<DecoderType>("nvdec");
  const [transport, setTransport] = useState<TransportKind>("quic");
  const [resolution, setResolution] = useState<ResolutionKey>(
    () => resolutionFromSearch(searchParams) ?? "1920x1080"
  );
  const [fps, setFps] = useState<FpsKey>(() => fpsFromSearch(searchParams) ?? "144");
  const [bitrate, setBitrate] = useState<BitrateKey>(
    () => bitrateFromSearch(searchParams) ?? "20"
  );
  const [isMaximized, setIsMaximized] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [lastError, setLastError] = useState<string | null>(null);
  const [testSettingsOpen, setTestSettingsOpen] = useState(false);
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [lastCompletedRun, setLastCompletedRun] = useState<TestRun | null>(null);
  const [metrics, setMetrics] = useState<HarnessMetrics | null>(null);
  const [durationMode, setDurationMode] = useState<LocalTestDurationMode>("30s");
  const [matrixModeEnabled, setMatrixModeEnabled] = useState(false);
  const [matrixDimensions, setMatrixDimensions] =
    useState<MatrixDimensionKey[]>(["fps"]);
  const [matrixRunProgress, setMatrixRunProgress] =
    useState<{ current: number; total: number; label: string } | null>(null);
  const [queryProfileAppliedKey, setQueryProfileAppliedKey] = useState<string | null>(null);
  const [webPreviewMode, setWebPreviewMode] = useState<WebPreviewMode>("idle");
  const [webPreviewEngine, setWebPreviewEngine] = useState<WebPreviewEngine>("webrtc");
  const [webPreviewError, setWebPreviewError] = useState<string | null>(null);
  const [webCodecsCanvasEpoch, setWebCodecsCanvasEpoch] = useState(0);
  const [webVideoFps, setWebVideoFps] = useState<number | null>(null);
  const [webPaintFps, setWebPaintFps] = useState<number | null>(null);
  const [webFrameIntervalP95Ms, setWebFrameIntervalP95Ms] = useState<number | null>(null);
  const [webVideoFrameCount, setWebVideoFrameCount] = useState(0);
  const [webPresentationLatencyStats, setWebPresentationLatencyStats] =
    useState<WebRtcPresentationLatencyStats | null>(null);
  const [webFrameTimingMetadataCount, setWebFrameTimingMetadataCount] = useState(0);
  const [webFrameTimingChannelState, setWebFrameTimingChannelState] =
    useState<string | null>(null);
  const [webFrameTimingMetadataAgeMs, setWebFrameTimingMetadataAgeMs] =
    useState<number | null>(null);
  const [webRtcReceiverStats, setWebRtcReceiverStats] =
    useState<WebRtcReceiverStats | null>(null);
  const [testMessage, setTestMessage] = useState<string | null>(null);
  const [sessionSnapshot, setSessionSnapshot] =
    useState<SessionRuntimeSnapshot | null>(null);
  const [probeSnapshot, setProbeSnapshot] = useState<ProbeSnapshot | null>(null);
  const [mediaPipelineSnapshot, setMediaPipelineSnapshot] =
    useState<MediaPipelineSnapshot | null>(null);
  const [mediaProfileNegotiation, setMediaProfileNegotiation] =
    useState<MediaProfileNegotiation | null>(null);
  const [captureSources, setCaptureSources] = useState<CaptureSource[]>([]);
  const [captureSourcesLoading, setCaptureSourcesLoading] = useState(false);
  const [captureSourceSelection, setCaptureSourceSelection] =
    useState<CaptureSourceSelection | null>(null);
  const [captureSourcePickerMode, setCaptureSourcePickerMode] =
    useState<CaptureSourcePickerMode>("dropdown");
  const [captureSourcePickerOpen, setCaptureSourcePickerOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [diagnosticsPinned, setDiagnosticsPinned] = useState(false);
  const [diagnosticsSamples, setDiagnosticsSamples] = useState<DiagnosticsSample[]>([]);
  const [serviceResourceSnapshot, setServiceResourceSnapshot] =
    useState<SystemResourceSnapshot | null>(null);
  const [displayResourceSnapshot, setDisplayResourceSnapshot] =
    useState<SystemResourceSnapshot | null>(null);

  const sessionId = id ?? context?.session_id ?? "local-preview";
  const activeSurfaceId = context?.surface_id ?? surfaceId;
  const isLocalPipelinePreview = isLocalPipelinePreviewSession(sessionId);
  const hostOs = normalizeOs(capabilities?.os_type);
  const requestedResolution = useMemo(() => resolutionFromSearch(searchParams), [searchParams]);
  const requestedFps = useMemo(() => fpsFromSearch(searchParams), [searchParams]);
  const requestedBitrate = useMemo(() => bitrateFromSearch(searchParams), [searchParams]);
  const queryProfileKey = useMemo(
    () =>
      requestedResolution !== null || requestedFps !== null || requestedBitrate !== null
        ? `${sessionId}:${requestedResolution ?? ""}:${requestedFps ?? ""}:${
            requestedBitrate ?? ""
          }`
        : null,
    [requestedBitrate, requestedFps, requestedResolution, sessionId]
  );
  const queryProfileNeedsApply =
    queryProfileKey !== null && queryProfileAppliedKey !== queryProfileKey;
  const explicitBrowser2k144LowLatencyProfile =
    renderMode === "web" &&
    webPreviewEngine === "webrtc" &&
    isExplicitBrowser2k144LowLatencyProfile({
      capture,
      encoder,
      decoder,
      transport,
      resolution,
      fps,
      bitrate,
    });
  const localWebViewPlan = useMemo(
    () =>
      resolveLocalWebViewPlan({
        capabilities,
        hostOs,
        capture,
        encoder,
        decoder,
        transport,
        fps,
        bitrate,
        capHighFpsBitrate:
          requestedBitrate === null && !explicitBrowser2k144LowLatencyProfile,
      }),
    [
      bitrate,
      capabilities,
      capture,
      decoder,
      encoder,
      explicitBrowser2k144LowLatencyProfile,
      fps,
      hostOs,
      requestedBitrate,
      transport,
    ]
  );
  const nativeRenderMode = nativeRenderModeForHost(hostOs);
  const nativeRendererType =
    renderMode === "metal_native"
      ? "macos"
      : renderMode === "linux_native"
        ? "linux"
      : renderMode === "d3d11_native"
        ? "d3d11"
        : null;
  const isNative = nativeRendererType !== null;
  const requiresEmbeddedNativeSurface =
    nativeRendererType === "d3d11" ||
    nativeRendererType === "macos" ||
    nativeRendererType === "linux";
  const nativeRendererTypeForHost = nativeRendererForHost(hostOs);
  const currentNativeRendererAvailable =
    isTauriRuntime() &&
    (!capabilities
      ? true
      : nativeRendererType
        ? capabilities.available_renderers?.includes(nativeRendererType) ?? false
        : false);
  const nativeRendererAvailableForHost =
    isTauriRuntime() &&
    nativeRendererTypeForHost !== null &&
    (!capabilities
      ? true
      : capabilities.available_renderers?.includes(nativeRendererTypeForHost) ?? false);
  const d3d12RendererAvailable = false;
  const d3d12UnavailableTitle =
    "D3D12 目前仅接入渲染测试页的独立 probe，尚未接入远程显示主链路。";
  const nativeRenderAvailable = isNative
    ? currentNativeRendererAvailable
    : nativeRendererAvailableForHost;
  const usesNativeSharedTexture =
    nativeRendererType === "d3d11" &&
    capture === "dxgi" &&
    isNvencSharedTextureEncoder(encoder) &&
    decoder === "nvdec";
  const visibleCaptureOptions = useMemo(
    () =>
      capabilities?.available_captures?.length
        ? captureOptions.filter((option) => capabilities.available_captures?.includes(option.value))
        : captureOptions,
    [capabilities]
  );
  const visibleEncoderOptions = useMemo(
    () =>
      capabilities?.available_encoders?.length
        ? encoderOptions.filter((option) => capabilities.available_encoders.includes(option.value))
        : encoderOptions,
    [capabilities]
  );
  const browserPreviewEncoderOptions = useMemo(() => {
    const targetFps = Number(fps);
    const needsHardwareH264 = Number.isFinite(targetFps) && targetFps > 30;
    const allowed = visibleEncoderOptions.filter((option) => {
      if (!isH264PreviewEncoder(option.value)) return false;
      if (!needsHardwareH264) return true;
      return option.value === "nvenc_h264" || option.value === "videotoolbox_h264";
    });
    return allowed.length > 0 ? allowed : visibleEncoderOptions;
  }, [fps, visibleEncoderOptions]);
  const visibleDecoderOptions = useMemo(
    () =>
      capabilities?.available_decoders?.length
        ? decoderOptions.filter((option) => capabilities.available_decoders.includes(option.value))
        : decoderOptions,
    [capabilities]
  );
  const activeEncoderOptions =
    isLocalPipelinePreview && renderMode === "web"
      ? browserPreviewEncoderOptions
      : visibleEncoderOptions;
  const browserEncoderConstraintTitle =
    isLocalPipelinePreview && renderMode === "web"
      ? Number(fps) > 30
        ? "网页渲染路径当前只接入 H.264；高帧率需要硬件 H.264，HEVC/AV1/OpenH264 不进入该路径。"
        : "网页渲染路径当前只接入 H.264 access unit，HEVC/AV1 尚未接入浏览器预览路径。"
      : undefined;
  const selectedDurationMs = localTestDurationMs(durationMode);
  const activeEncoderValues = useMemo(
    () => new Set(activeEncoderOptions.map((option) => option.value)),
    [activeEncoderOptions]
  );
  const captureTileOptions = useMemo(
    () =>
      captureOptions.map((option) => ({
        ...option,
        disabledReason:
          capabilities?.available_captures?.length &&
          !capabilities.available_captures.includes(option.value)
            ? "当前平台能力未报告该采集路径"
            : null,
      })),
    [capabilities]
  );
  const encoderTileOptions = useMemo(
    () =>
      encoderOptions.map((option) => ({
        ...option,
        disabledReason: capabilities?.available_encoders?.length &&
          !capabilities.available_encoders.includes(option.value)
          ? "当前平台能力未报告该编码器"
          : isLocalPipelinePreview &&
              renderMode === "web" &&
              !activeEncoderValues.has(option.value)
            ? browserEncoderConstraintTitle ??
              "当前网页渲染路径未接入该编码器"
            : null,
      })),
    [activeEncoderValues, browserEncoderConstraintTitle, capabilities, isLocalPipelinePreview, renderMode]
  );
  const decoderTileOptions = useMemo(
    () =>
      decoderOptions.map((option) => ({
        ...option,
        disabledReason:
          capabilities?.available_decoders?.length &&
          !capabilities.available_decoders.includes(option.value)
            ? "当前平台能力未报告该解码器"
            : null,
      })),
    [capabilities]
  );
  const durationTileOptions = useMemo(
    () =>
      localTestDurationOptions.map((option) => ({
        ...option,
        disabledReason:
          matrixModeEnabled && option.value === "manual"
            ? "多选矩阵需要固定时长；手动停止仅用于单次测试"
            : null,
      })),
    [matrixModeEnabled]
  );
  const browserWebViewFpsLimitReason = `Web View 当前接入上限为 ${WEB_VIEW_MAX_FPS} FPS；更高档位需要 native 渲染或后续浏览器媒体链路。`;
  const browserHardwareH264Available = useMemo(() => {
    if (!capabilities?.available_encoders?.length) return true;
    const hardwareEncoders =
      hostOs === "macos" ? ["videotoolbox_h264"] : ["nvenc_h264"];
    return hardwareEncoders.some((encoder) =>
      capabilities.available_encoders.includes(encoder)
    );
  }, [capabilities, hostOs]);
  const fpsTileOptions = useMemo(
    () =>
      fpsOptions.map((option) => ({
        ...option,
        disabledReason:
          isLocalPipelinePreview &&
          renderMode === "web" &&
          Number(option.value) > WEB_VIEW_MAX_FPS
            ? browserWebViewFpsLimitReason
            : isLocalPipelinePreview &&
                renderMode === "web" &&
                Number(option.value) > 30 &&
                !browserHardwareH264Available
              ? "网页高帧率预览需要硬件 H.264 编码器；当前平台只允许 30 FPS 诊断档。"
              : null,
      })),
    [
      browserHardwareH264Available,
      browserWebViewFpsLimitReason,
      isLocalPipelinePreview,
      renderMode,
    ]
  );
  const bitrateTileOptions = useMemo(
    () =>
      bitrateOptions.map((option) => ({
        ...option,
        disabledReason:
          isLocalPipelinePreview &&
          renderMode === "web" &&
          requestedBitrate === null &&
          !explicitBrowser2k144LowLatencyProfile &&
          Number(fpsForWebView(fps)) >= 120 &&
          Number(option.value) > 8
            ? "当前 Web View 高帧率默认保护档会把码率限制到 8 Mbps；使用 2K144 预设或 URL 显式码率可运行更高码率。"
            : null,
      })),
    [
      explicitBrowser2k144LowLatencyProfile,
      fps,
      isLocalPipelinePreview,
      renderMode,
      requestedBitrate,
    ]
  );
  const resolutionTileOptions = useMemo(
    () =>
      resolutionOptions.map((option) => ({
        ...option,
        disabledReason: null,
      })),
    []
  );

  useEffect(() => {
    if (!isLocalPipelinePreview || renderMode !== "web") return;
    const firstOption = activeEncoderOptions[0];
    if (!firstOption) return;
    if (activeEncoderOptions.some((option) => option.value === encoder)) return;
    setEncoder(firstOption.value);
  }, [activeEncoderOptions, encoder, isLocalPipelinePreview, renderMode]);

  useEffect(() => {
    if (matrixModeEnabled && durationMode === "manual") {
      setDurationMode("30s");
    }
  }, [durationMode, matrixModeEnabled]);

  useEffect(() => {
    if (isHevcEncoder(encoder) && transport === "webrtc") {
      setTransport("quic");
    }
    if (isHevcEncoder(encoder) && (decoder === "software" || decoder === "linux_h264")) {
      const preferredLinuxDecoder = encoder === "nvenc_hevc_main10" ? "linux_hevc_main10" : "linux_hevc";
      setDecoder(
        capabilities?.available_decoders.includes("nvdec")
          ? "nvdec"
          : capabilities?.available_decoders.includes(preferredLinuxDecoder)
            ? preferredLinuxDecoder
            : "none"
      );
    }
    if (
      encoder === "nvenc_av1" &&
      (decoder === "linux_h264" || decoder === "linux_hevc" || decoder === "linux_hevc_main10")
    ) {
      setDecoder(capabilities?.available_decoders.includes("nvdec") ? "nvdec" : "none");
    }
    if (
      (encoder === "nvenc_h264" || encoder === "openh264" || encoder === "videotoolbox_h264") &&
      (decoder === "linux_hevc" || decoder === "linux_hevc_main10")
    ) {
      setDecoder(capabilities?.available_decoders.includes("linux_h264") ? "linux_h264" : "software");
    }
  }, [capabilities?.available_decoders, decoder, encoder, transport]);

  const renderModeLabel =
    renderMode === "metal_native"
      ? "Metal native"
      : renderMode === "d3d12_native"
        ? "DX12 native"
        : renderMode === "linux_native"
          ? "Linux native"
        : renderMode === "d3d11_native"
          ? hostOs === "linux"
            ? "Linux native"
            : "D3D11 native"
          : "Web View";
  const nativeRenderLabel =
    hostOs === "macos"
      ? "Metal native"
      : hostOs === "linux"
        ? "Linux native"
        : hostOs === "windows"
          ? "DX11 native"
          : "Native";
  const remoteFramesReceived = probeSnapshot?.frames_received ?? 0;
  const remoteFramesDecoded = probeSnapshot?.frames_decoded ?? 0;
  const remoteFrameDataUrl = probeSnapshot?.latest_frame_data_url ?? null;
  const nativeSurfaceAttached =
    isNative && Boolean(nativeSurface?.attached || context?.native_surface_attached);
  const showRemotePreviewFrame = !nativeSurfaceAttached && Boolean(remoteFrameDataUrl);
  const remoteFrameAspectRatio =
    probeSnapshot?.latest_frame_width && probeSnapshot?.latest_frame_height
      ? `${probeSnapshot.latest_frame_width} / ${probeSnapshot.latest_frame_height}`
      : probeSnapshot?.media_probe_width && probeSnapshot?.media_probe_height
        ? `${probeSnapshot.media_probe_width} / ${probeSnapshot.media_probe_height}`
        : undefined;
  const hasRemoteFrames = remoteFramesReceived > 0 || remoteFramesDecoded > 0;
  const remoteProbeTarget =
    probeSnapshot?.media_probe_width &&
    probeSnapshot?.media_probe_height &&
    probeSnapshot?.media_probe_target_fps
      ? `${probeSnapshot.media_probe_width}x${probeSnapshot.media_probe_height}@${probeSnapshot.media_probe_target_fps}`
      : null;
  const remoteDropTotal =
    (probeSnapshot?.frames_dropped ?? 0) + (mediaPipelineSnapshot?.dropped_frames ?? 0);
  const remoteFrameTotal = Math.max(
    1,
    (probeSnapshot?.frames_received ?? 0) + remoteDropTotal
  );
  const remoteDropRatio = remoteDropTotal / remoteFrameTotal * 100;
  const stageCaptureP95Ms = findStageP95(mediaPipelineSnapshot, [
    "sender.capture",
    "capture",
  ]);
  const stageEncodeP95Ms = findStageP95(mediaPipelineSnapshot, [
    "sender.encode",
    "encode",
  ]);
  const stageTransportP95Ms = findStageP95(mediaPipelineSnapshot, [
    "sender.send_datagram",
    "sender.transport",
    "transport",
    "fragment/send",
  ]);
  const stageDecodeP95Ms = findStageP95(mediaPipelineSnapshot, [
    "receiver.decode",
    "decode",
  ]);
  const stageRenderP95Ms = findStageP95(mediaPipelineSnapshot, [
    "receiver.present",
    "receiver.render_upload",
    "render_upload",
    "render_present",
    "present",
    "render",
  ]);
  const diagnosticsFps =
    probeSnapshot?.current_fps ??
    webVideoFps ??
    webRtcReceiverStats?.decodedFps ??
    metrics?.decoded_fps ??
    metrics?.capture_fps ??
    null;
  const diagnosticsVisualFps = webVideoFps ?? diagnosticsFps;
  const diagnosticsCaptureP95Ms =
    metrics?.capture_latency_p95_ms ?? stageCaptureP95Ms;
  const diagnosticsEncodeP95Ms =
    metrics?.encode_latency_p95_ms ?? stageEncodeP95Ms;
  const diagnosticsTransportP95Ms =
    metrics?.transport_latency_p95_ms ?? stageTransportP95Ms;
  const diagnosticsDecodeP95Ms =
    metrics?.decode_latency_p95_ms ?? stageDecodeP95Ms ?? webRtcReceiverStats?.decodeAvgMs ?? null;
  const diagnosticsRenderP95Ms = stageRenderP95Ms;
  const diagnosticsLatencyP95Ms =
    webPresentationLatencyStats?.p95Ms ??
    metrics?.total_latency_p95_ms ??
    stageDecodeP95Ms ??
    webFrameIntervalP95Ms ??
    null;
  const diagnosticsLatencyIsPrecise =
    webPresentationLatencyStats?.source === "browser_capture_time" ||
    webPresentationLatencyStats?.source === "rtp_frame_timing_channel";
  const diagnosticsLatencyLabel = webPresentationLatencyStats
    ? diagnosticsLatencyIsPrecise
      ? "真实端到端 p95"
      : "端到端估算 p95"
    : typeof metrics?.total_latency_p95_ms === "number"
    ? "端到端 p95"
    : typeof stageDecodeP95Ms === "number"
      ? "解码 p95"
      : typeof webFrameIntervalP95Ms === "number"
        ? "Web 呈现间隔 p95"
      : "延迟 p95";
  const diagnosticsE2eLabelPrefix = diagnosticsLatencyIsPrecise ? "真实 E2E" : "估算 E2E";
  const diagnosticsBitrateMbps =
    probeSnapshot?.bitrate_mbps ?? mediaPipelineSnapshot?.active_bitrate_mbps ?? Number(bitrate);
  const diagnosticsDroppedFrames =
    metrics?.dropped_frames ?? webRtcReceiverStats?.framesDropped ?? remoteDropTotal ?? null;
  const diagnosticsRenderQueueReplacements =
    mediaPipelineSnapshot?.render_queue_replacements ?? null;
  const diagnosticsRenderLockDrops = mediaPipelineSnapshot?.render_lock_drops ?? null;
  const diagnosticsRenderPresentSkips = mediaPipelineSnapshot?.render_present_skips ?? null;
  const diagnosticsDropRatio =
    metrics && metrics.frame_count > 0
      ? metrics.dropped_frames / metrics.frame_count * 100
      : remoteDropRatio;
  const diagnosticsQueueDepth = mediaPipelineSnapshot?.queue_depth ?? null;
  const diagnosticsServiceCpuPercent = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => snapshot.cpu_metrics_available === false ? null : snapshot.cpu_usage_percent
  );
  const diagnosticsServiceMemoryPercent = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => snapshot.memory_usage_percent
  );
  const diagnosticsServiceMemoryMb = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => snapshot.memory_used_mb
  );
  const diagnosticsServiceGpuPercent = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => snapshot.gpu_usage_percent
  );
  const diagnosticsServiceGpuMemoryMb = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => snapshot.gpu_memory_used_mb
  );
  const diagnosticsServiceNetworkRxMbps = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => bpsToMbps(snapshot.network_rx_bps)
  );
  const diagnosticsServiceNetworkTxMbps = resourceMetric(
    serviceResourceSnapshot,
    (snapshot) => bpsToMbps(snapshot.network_tx_bps)
  );
  const diagnosticsDisplayCpuPercent = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => snapshot.cpu_metrics_available === false ? null : snapshot.cpu_usage_percent
  );
  const diagnosticsDisplayMemoryPercent = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => snapshot.memory_usage_percent
  );
  const diagnosticsDisplayMemoryMb = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => snapshot.memory_used_mb
  );
  const diagnosticsDisplayGpuPercent = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => snapshot.gpu_usage_percent
  );
  const diagnosticsDisplayGpuMemoryMb = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => snapshot.gpu_memory_used_mb
  );
  const diagnosticsDisplayNetworkRxMbps = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => bpsToMbps(snapshot.network_rx_bps)
  );
  const diagnosticsDisplayNetworkTxMbps = resourceMetric(
    displayResourceSnapshot,
    (snapshot) => bpsToMbps(snapshot.network_tx_bps)
  );
  const diagnosticsTargetFps =
    mediaPipelineSnapshot?.active_fps ??
    probeSnapshot?.media_probe_target_fps ??
    Number(fps);
  const diagnosticsBrowserPaintLimited =
    Number(fps) >= 100 &&
    typeof webVideoFps === "number" &&
    webVideoFps < diagnosticsTargetFps * 0.85 &&
    typeof webRtcReceiverStats?.decodedFps === "number" &&
    webRtcReceiverStats.decodedFps >= diagnosticsTargetFps * 0.9 &&
    (webRtcReceiverStats.framesDropped ?? 0) === 0 &&
    (webRtcReceiverStats.packetsLost ?? 0) === 0;
  const diagnosticsCurrent: DiagnosticsSample = {
    atMs: 0,
    fps: diagnosticsVisualFps,
    paintFps: webPaintFps,
    latencyP95Ms: diagnosticsLatencyP95Ms,
    captureP95Ms: diagnosticsCaptureP95Ms,
    encodeP95Ms: diagnosticsEncodeP95Ms,
    transportP95Ms: diagnosticsTransportP95Ms,
    decodeP95Ms: diagnosticsDecodeP95Ms,
    renderP95Ms: diagnosticsRenderP95Ms,
    queueDepth: diagnosticsQueueDepth,
    droppedFrames: diagnosticsDroppedFrames,
    bitrateMbps: diagnosticsBitrateMbps,
    serviceCpuPercent: diagnosticsServiceCpuPercent,
    serviceMemoryPercent: diagnosticsServiceMemoryPercent,
    serviceMemoryMb: diagnosticsServiceMemoryMb,
    serviceGpuPercent: diagnosticsServiceGpuPercent,
    serviceGpuMemoryMb: diagnosticsServiceGpuMemoryMb,
    serviceNetworkRxMbps: diagnosticsServiceNetworkRxMbps,
    serviceNetworkTxMbps: diagnosticsServiceNetworkTxMbps,
    displayCpuPercent: diagnosticsDisplayCpuPercent,
    displayMemoryPercent: diagnosticsDisplayMemoryPercent,
    displayMemoryMb: diagnosticsDisplayMemoryMb,
    displayGpuPercent: diagnosticsDisplayGpuPercent,
    displayGpuMemoryMb: diagnosticsDisplayGpuMemoryMb,
    displayNetworkRxMbps: diagnosticsDisplayNetworkRxMbps,
    displayNetworkTxMbps: diagnosticsDisplayNetworkTxMbps,
  };
  const diagnosticsStageRows = useMemo<DiagnosticsStageRow[]>(() => {
    const pipelineRows =
      mediaPipelineSnapshot?.stage_metrics
        ?.filter((metric) => typeof metric.p95_ms === "number")
        .map((metric) => ({
          label: metric.stage,
          value: metric.p95_ms ?? null,
          samples: (metric as { samples?: number | null }).samples,
        })) ?? [];

    if (pipelineRows.length > 0) return pipelineRows;

    const webRtcRows = buildWebRtcDiagnosticsStageRows(
      webRtcReceiverStats,
      webPresentationLatencyStats
    );
    if (webRtcRows.length > 0) return webRtcRows;

    return [
      { label: "capture", value: metrics?.capture_latency_p95_ms ?? null },
      { label: "encode", value: metrics?.encode_latency_p95_ms ?? null },
      { label: "transport", value: metrics?.transport_latency_p95_ms ?? null },
      { label: "decode", value: metrics?.decode_latency_p95_ms ?? null },
      { label: "total", value: metrics?.total_latency_p95_ms ?? null },
      { label: "web.frame_interval", value: webFrameIntervalP95Ms },
    ];
  }, [
    mediaPipelineSnapshot?.stage_metrics,
    metrics?.capture_latency_p95_ms,
    metrics?.decode_latency_p95_ms,
    metrics?.encode_latency_p95_ms,
    metrics?.total_latency_p95_ms,
    metrics?.transport_latency_p95_ms,
    webPresentationLatencyStats,
    webRtcReceiverStats,
    webFrameIntervalP95Ms,
  ]);
  const remoteQuality =
    (probeSnapshot?.last_error || mediaPipelineSnapshot?.codec_fallback_reason)
      ? "降级"
      : diagnosticsBrowserPaintLimited
        ? "受限"
      : diagnosticsDropRatio <= 0.5 &&
          typeof diagnosticsVisualFps === "number" &&
          diagnosticsVisualFps >= diagnosticsTargetFps * 0.9
        ? "流畅"
        : hasRemoteFrames || typeof diagnosticsVisualFps === "number"
          ? "一般"
          : "等待";
  const diagnosticsVisible = diagnosticsOpen || diagnosticsPinned;
  const diagnosticsSelectedCodec =
    mediaPipelineSnapshot?.active_codec ??
    mediaProfileNegotiation?.selected.codec ??
    codecFromEncoder(encoder);
  const diagnosticsCodec = codecLabel(
    diagnosticsSelectedCodec,
    mediaPipelineSnapshot?.active_codec_profile ??
      mediaProfileNegotiation?.selected.codec_profile
  );
  const diagnosticsChroma =
    mediaPipelineSnapshot?.active_chroma_subsampling ??
    mediaProfileNegotiation?.selected.chroma_subsampling ??
    (diagnosticsSelectedCodec ? "4:2:0" : "-");
  const diagnosticsPixelFormat =
    mediaPipelineSnapshot?.active_pixel_format ??
    mediaProfileNegotiation?.selected.pixel_format ??
    probeSnapshot?.latest_frame_pixel_format ??
    (renderMode === "web" && diagnosticsSelectedCodec
      ? webPreviewEngine === "webcodecs"
        ? "WebCodecs H.264 Annex B"
        : "WebRTC 4:2:0"
      : "-");
  const diagnosticsBitDepth =
    mediaPipelineSnapshot?.active_bit_depth ??
    mediaProfileNegotiation?.selected.bit_depth ??
    (diagnosticsSelectedCodec ? 8 : null);
  const diagnosticsHdrEnabled =
    mediaPipelineSnapshot?.active_hdr_enabled ?? mediaProfileNegotiation?.selected.hdr_enabled;
  const diagnosticsResolution =
    mediaPipelineSnapshot?.active_width && mediaPipelineSnapshot?.active_height
      ? `${mediaPipelineSnapshot.active_width}x${mediaPipelineSnapshot.active_height}`
      : probeSnapshot?.media_probe_width && probeSnapshot?.media_probe_height
      ? `${probeSnapshot.media_probe_width}x${probeSnapshot.media_probe_height}`
      : remoteProbeTarget?.split("@")[0] ?? resolution;
  const diagnosticsTarget =
    mediaPipelineSnapshot?.active_width &&
    mediaPipelineSnapshot?.active_height &&
    mediaPipelineSnapshot?.active_fps
      ? `${mediaPipelineSnapshot.active_width}x${mediaPipelineSnapshot.active_height}@${mediaPipelineSnapshot.active_fps}`
      : remoteProbeTarget ??
    (mediaProfileNegotiation?.selected
      ? `${mediaProfileNegotiation.selected.width}x${mediaProfileNegotiation.selected.height}@${mediaProfileNegotiation.selected.fps}`
      : `${resolution}@${fps}`);

  useEffect(() => {
    diagnosticsCurrentRef.current = diagnosticsCurrent;
  }, [diagnosticsCurrent]);

  useEffect(() => {
    webVideoFpsRef.current = webVideoFps;
  }, [webVideoFps]);

  useEffect(() => {
    webVideoFrameCountRef.current = webVideoFrameCount;
  }, [webVideoFrameCount]);

  useEffect(() => {
    webRtcReceiverStatsRef.current = webRtcReceiverStats;
  }, [webRtcReceiverStats]);

  useEffect(() => {
    webPresentationLatencyStatsRef.current = webPresentationLatencyStats;
  }, [webPresentationLatencyStats]);

  useEffect(() => {
    diagnosticsSamplesRef.current = [];
    setDiagnosticsSamples([]);
  }, [sessionId]);

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const refreshResources = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const [serviceResult, displayResult] = await Promise.all([
          getSystemResourceSnapshot("mrd-service"),
          getSystemResourceSnapshot("display"),
        ]);
        if (cancelled) return;
        if (serviceResult.ok) setServiceResourceSnapshot(serviceResult.value);
        if (displayResult.ok) setDisplayResourceSnapshot(displayResult.value);
      } finally {
        inFlight = false;
      }
    };

    void refreshResources();
    const interval = window.setInterval(() => void refreshResources(), 2_000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    const record = () => {
      const current = diagnosticsCurrentRef.current;
      if (!current || !hasDiagnosticsSampleValue(current)) return;

      const nextSample = {
        ...current,
        atMs: Date.now(),
      };
      const next = [...diagnosticsSamplesRef.current, nextSample];
      const bounded = next.slice(Math.max(0, next.length - DIAGNOSTICS_SAMPLE_LIMIT));
      diagnosticsSamplesRef.current = bounded;
      if (diagnosticsVisible) {
        setDiagnosticsSamples(bounded);
      }
    };

    record();
    const interval = window.setInterval(record, DIAGNOSTICS_SAMPLE_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [diagnosticsVisible, sessionId]);

  useEffect(() => {
    if (!diagnosticsVisible) return;

    const closeIfOutside = (event: MouseEvent | PointerEvent) => {
      const target = event.target as Node | null;
      if (target && diagnosticsPopoverRef.current?.contains(target)) return;
      setDiagnosticsOpen(false);
      setDiagnosticsPinned(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setDiagnosticsOpen(false);
      setDiagnosticsPinned(false);
    };

    document.addEventListener("pointerdown", closeIfOutside, true);
    document.addEventListener("mousedown", closeIfOutside, true);
    document.addEventListener("click", closeIfOutside, true);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeIfOutside, true);
      document.removeEventListener("mousedown", closeIfOutside, true);
      document.removeEventListener("click", closeIfOutside, true);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [diagnosticsVisible]);

  const title = useMemo(() => {
    if (context?.label) return context.label;
    return `display-${sessionId}`;
  }, [context?.label, sessionId]);

  const displayDecoderLabel =
    isLocalPipelinePreview && renderMode === "web"
      ? webPreviewDecoderLabel(webPreviewEngine, optionLabel(decoderOptions, decoder))
      : optionLabel(decoderOptions, decoder);
  const displayTransportLabel =
    isLocalPipelinePreview && renderMode === "web"
      ? webPreviewTransportLabel(webPreviewEngine, optionLabel(transportOptions, transport))
      : optionLabel(transportOptions, transport);
  const testDescription = useMemo(
    () =>
      `${optionLabel(captureOptions, capture)} -> ${optionLabel(
        encoderOptions,
        encoder
      )} -> ${displayDecoderLabel} / ${displayTransportLabel} / ${optionLabel(resolutionOptions, resolution)} @ ${optionLabel(
        fpsOptions,
        fps
      )} / ${optionLabel(bitrateOptions, bitrate)}`,
    [bitrate, capture, displayDecoderLabel, displayTransportLabel, encoder, fps, resolution]
  );
  const buildTestConfig = useCallback((rendererTargetHwnd?: string | null, selection?: LocalTestSelection) => {
    const selectedCapture = selection?.capture ?? capture;
    const selectedEncoder = selection?.encoder ?? encoder;
    const selectedDecoder = selection?.decoder ?? decoder;
    const selectedTransport = selection?.transport ?? transport;
    const selectedFps = selection?.fps ?? fps;
    const selectedBitrate = selection?.bitrate ?? bitrate;
    const selectedResolution = selection?.resolution ?? resolution;
    const [width, height] = selectedResolution.split("x").map(Number) as [number, number];
    const selectedUsesNativeSharedTexture =
      nativeRendererType === "d3d11" &&
      selectedCapture === "dxgi" &&
      isNvencSharedTextureEncoder(selectedEncoder) &&
      selectedDecoder === "nvdec";

    return {
      capture_type: selectedCapture,
      encoder_type: selectedEncoder,
      decoder_type: selectedDecoder,
      transport_kind: selectedTransport,
      resolution: [width, height],
      fps: Number(selectedFps),
      bitrate: Number(selectedBitrate) * 1_000_000,
      duration_ms: selectedDurationMs,
      warmup_ms: 500,
      input_source: selectedCapture === "synthetic" ? "synthetic" : "screen",
      output_validation: true,
      visual_preview: false,
      render_display: Boolean(
        isNative && (rendererTargetHwnd || nativeRendererType === "linux")
      ),
      zero_copy: selectedUsesNativeSharedTexture,
      ...(nativeRendererType ? { renderer_type: nativeRendererType } : {}),
      ...(isNative && rendererTargetHwnd ? { renderer_target_hwnd: rendererTargetHwnd } : {}),
    } satisfies TestConfig;
  }, [
    bitrate,
    capture,
    decoder,
    encoder,
    fps,
    isNative,
    nativeRendererType,
    resolution,
    selectedDurationMs,
    transport,
  ]);
  const testConfig = useMemo(
    () => buildTestConfig(nativeSurface?.hwnd),
    [buildTestConfig, nativeSurface?.hwnd]
  );
  const toggleMatrixDimension = useCallback((value: MatrixDimensionKey) => {
    setMatrixDimensions((current) => {
      if (current.includes(value)) {
        const next = current.filter((item) => item !== value);
        return next.length > 0 ? next : current;
      }
      return [...current, value];
    });
  }, []);
  const buildLocalMatrixSelections = useCallback((): LocalTestSelection[] => {
    if (!matrixModeEnabled) return [{}];
    const dimensionValues: Array<Array<LocalTestSelection>> = matrixDimensions.map((dimension) => {
      switch (dimension) {
        case "capture":
          return captureTileOptions
            .filter((option) => !option.disabledReason)
            .map((option) => ({ capture: option.value }));
        case "encoder":
          return encoderTileOptions
            .filter((option) => !option.disabledReason)
            .map((option) => ({ encoder: option.value }));
        case "resolution":
          return resolutionTileOptions
            .filter((option) => !option.disabledReason)
            .map((option) => ({ resolution: option.value }));
        case "fps":
          return fpsTileOptions
            .filter((option) => !option.disabledReason)
            .map((option) => ({ fps: option.value }));
        case "bitrate":
          return bitrateTileOptions
            .filter((option) => !option.disabledReason)
            .map((option) => ({ bitrate: option.value }));
        default:
          return [{}];
      }
    });

    const combinations = dimensionValues.reduce<LocalTestSelection[]>(
      (acc, values) =>
        acc.flatMap((base) =>
          values.map((value) => ({
            ...base,
            ...value,
          }))
        ),
      [{}]
    );

    return combinations.slice(0, 36);
  }, [
    activeEncoderOptions,
    bitrateTileOptions,
    captureTileOptions,
    encoderTileOptions,
    fpsTileOptions,
    matrixDimensions,
    matrixModeEnabled,
    resolutionTileOptions,
    visibleCaptureOptions,
  ]);
  const matrixSelectionCount = matrixModeEnabled ? buildLocalMatrixSelections().length : 1;
  const isTestBusy =
    testStatus === "starting" || testStatus === "running" || testStatus === "stopping";
  const webCodecsStartBlockReason =
    isLocalPipelinePreview && renderMode === "web" && webPreviewEngine === "webcodecs"
      ? browserSupportsWebCodecsH264()
        ? null
        : "WebCodecs 超低延迟路径需要浏览器 VideoDecoder / EncodedVideoChunk 支持。"
      : null;
  const localStartBlockReason =
    webCodecsStartBlockReason ??
    (isLocalPipelinePreview && renderMode === "web" ? localWebViewPlan.reason : null);
  const browserWebRtc2k144BlockReason = !isLocalPipelinePreview
    ? "仅本机 Web View 测试可用"
    : isTestBusy
      ? "请先停止当前测试再切换测试档位"
      : !browserSupportsH264WebrtcVideo()
        ? "当前浏览器未声明 H.264 WebRTC 接收能力"
        : capabilities &&
            (!(capabilities.available_captures ?? []).includes("dxgi") ||
              !capabilities.available_encoders.includes("nvenc_h264"))
          ? "WebRTC 2K144 需要 Windows DXGI + NVENC H.264"
          : null;
  const browserWebCodecsUltraBlockReason = !isLocalPipelinePreview
    ? "仅本机 Web View 测试可用"
    : isTestBusy
      ? "请先停止当前测试再切换测试档位"
      : !browserSupportsWebCodecsH264()
        ? "当前浏览器缺少 VideoDecoder / EncodedVideoChunk"
        : capabilities &&
            (!(capabilities.available_captures ?? []).includes("dxgi") ||
              !capabilities.available_encoders.includes("nvenc_h264"))
          ? "WebCodecs 2K144 需要 Windows DXGI + NVENC H.264"
          : null;
  const buildRemoteMediaProfile = useCallback(() => {
    const [width, height] = resolution.split("x").map(Number) as [number, number];
    const hevc = isHevcEncoder(encoder);
    const main10 = encoder === "nvenc_hevc_main10";
    return {
      width,
      height,
      fps: Number(fps),
      bitrate_mbps: Number(bitrate),
      codec: hevc ? "hevc" : "h264",
      codec_profile: hevc ? (main10 ? "main10" : "main") : "high",
      bit_depth: main10 ? 10 : 8,
      chroma_subsampling: "4:2:0",
      pixel_format: main10 ? "p010" : "nv12",
      hdr_enabled: false,
    };
  }, [bitrate, encoder, fps, resolution]);
  const localRenderSwitchLocked = isLocalPipelinePreview && isTestBusy;

  useEffect(() => {
    const timer = window.setInterval(() => setElapsed((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void testGetCapabilities().then((result) => {
      if (result.ok) setCapabilities(result.value);
    });
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    void currentRemoteDisplayWindowContext().then((result) => {
      if (result.ok) {
        setContext(result.value);
        if (result.value?.render_mode === "macos_native") {
          setRenderMode("metal_native");
        } else if (result.value?.render_mode === "linux_native") {
          setRenderMode("linux_native");
        } else if (result.value?.render_mode === "d3d11_native") {
          setRenderMode("d3d11_native");
        } else if (result.value?.render_mode === "d3d12_native") {
          setRenderMode("d3d11_native");
        } else if (result.value?.render_mode === "web") {
          const contextSessionId = result.value.session_id;
          if (!contextSessionId || isLocalPipelinePreviewSession(contextSessionId)) {
            setRenderMode("web");
          }
        }
      }
    });
    void withTauriWindow(async (appWindow) => {
      const next = await appWindow.isMaximized();
      if (typeof next === "boolean") setIsMaximized(next);
    });
  }, []);

  useEffect(() => {
    if (!capabilities) return;

    const os = normalizeOs(capabilities.os_type);
    if (os === "macos") {
      setCapture((value) =>
        pickAvailable(value, capabilities.available_captures, ["macos", "synthetic"], "macos")
      );
      setEncoder((value) =>
        pickAvailable(
          value,
          capabilities.available_encoders,
          ["videotoolbox_h264", "openh264"],
          "videotoolbox_h264"
        )
      );
      setDecoder((value) =>
        pickAvailable(
          value,
          capabilities.available_decoders,
          ["videotoolbox", "software", "none"],
          "videotoolbox"
        )
      );
      setFps((value) => (value === "120" || value === "144" ? "60" : value));
      setRenderMode((value) => (value === "d3d11_native" ? "metal_native" : value));
      return;
    }

    if (os === "windows") {
      setCapture((value) =>
        pickAvailable(value, capabilities.available_captures, ["dxgi", "winrt", "synthetic"], "dxgi")
      );
      setEncoder((value) =>
        pickAvailable(
          value,
          capabilities.available_encoders,
          ["nvenc_hevc", "nvenc_h264", "nvenc_av1", "openh264"],
          "nvenc_hevc"
        )
      );
      setDecoder((value) =>
        pickAvailable(value, capabilities.available_decoders, ["nvdec", "software", "none"], "nvdec")
      );
      setRenderMode((value) => (value === "metal_native" ? "d3d11_native" : value));
      return;
    }

    if (os === "linux") {
      const localCapturePreference: CaptureType[] = ["linux", "synthetic"];
      const localDecoderPreference: DecoderType[] = isLocalPipelinePreview
        ? ["software", "linux_h264", "none"]
        : ["linux_h264", "software", "none"];
      const fallbackCapture = localCapturePreference[0] ?? "synthetic";
      const fallbackDecoder = localDecoderPreference[0] ?? "software";
      setCapture((value) =>
        pickAvailable(value, capabilities.available_captures, localCapturePreference, fallbackCapture)
      );
      setEncoder((value) =>
        pickAvailable(value, capabilities.available_encoders, ["openh264"], "openh264")
      );
      setDecoder((value) =>
        pickAvailable(
          value,
          capabilities.available_decoders,
          localDecoderPreference,
          fallbackDecoder
        )
      );
      setRenderMode((value) => (value === "linux_native" ? value : "web"));
      return;
    }

    setRenderMode("web");
  }, [capabilities]);

  useEffect(() => {
    if (
      !isLocalPipelinePreview ||
      renderMode !== "web" ||
      isTestBusy ||
      !localWebViewPlan.profile ||
      !localWebViewPlan.changed
    ) {
      return;
    }

    setCapture(localWebViewPlan.profile.capture);
    setEncoder(localWebViewPlan.profile.encoder);
    setDecoder(localWebViewPlan.profile.decoder);
    setTransport(localWebViewPlan.profile.transport);
    setFps(localWebViewPlan.profile.fps);
    setBitrate(localWebViewPlan.profile.bitrate);
    setLastError(null);
    if (localWebViewPlan.message) setTestMessage(localWebViewPlan.message);
  }, [isLocalPipelinePreview, isTestBusy, localWebViewPlan, renderMode]);

  useEffect(() => {
    if (
      !isLocalPipelinePreview ||
      hostOs !== "linux" ||
      renderMode !== "linux_native" ||
      isTestBusy ||
      !capabilities
    ) {
      return;
    }

    const availableCaptures = capabilities.available_captures ?? [];
    if (availableCaptures.includes("linux")) {
      setCapture("linux");
    } else {
      setCapture((value) =>
        pickAvailable(value, availableCaptures, ["synthetic"], "synthetic")
      );
    }

    if (!linuxNativeProfileAppliedRef.current) {
      const availableEncoders = capabilities.available_encoders ?? [];
      const availableDecoders = capabilities.available_decoders ?? [];
      setEncoder(
        pickCapability(linuxNativeEncoderPreference, availableEncoders) ?? "openh264"
      );
      setDecoder(
        pickCapability(linuxNativeDecoderPreference, availableDecoders) ?? "none"
      );
      setTransport("loopback");
      linuxNativeProfileAppliedRef.current = true;
    }
  }, [capabilities, hostOs, isLocalPipelinePreview, isTestBusy, renderMode]);

  useEffect(() => {
    if (renderMode !== "linux_native") {
      linuxNativeProfileAppliedRef.current = false;
    }
  }, [renderMode]);

  useEffect(() => {
    if (isNative && capabilities && !nativeRenderAvailable) {
      setRenderMode("web");
    }
  }, [capabilities, isNative, nativeRenderAvailable]);

  const syncNativeSurface = useCallback(async (options?: { visible?: boolean }) => {
    if (!isTauriRuntime()) return null;
    const element = renderAreaRef.current;
    if (!element) return null;

    const rect = element.getBoundingClientRect();
    if (isNative && (rect.width <= 0 || rect.height <= 0)) return null;

    const visible = options?.visible ?? !testSettingsOpen;
    const scale = nativeRendererType === "macos" ? 1 : window.devicePixelRatio || 1;
    const result = await configureRemoteDisplayNativeSurface({
      enabled: isNative && nativeRenderAvailable,
      visible: isNative && nativeRenderAvailable && visible,
      rect: {
        x: Math.round(rect.left * scale),
        y: Math.round(rect.top * scale),
        width: Math.round(rect.width * scale),
        height: Math.round(rect.height * scale),
      },
    });

    if (result.ok) {
      setNativeSurface(result.value);
      setLastError(null);
      return result.value;
    } else {
      setLastError(result.error.message);
      if (isNative) setRenderMode("web");
      return null;
    }
  }, [isNative, nativeRenderAvailable, nativeRendererType, testSettingsOpen]);

  const openTestSettings = useCallback(() => {
    setTestSettingsOpen(true);
    void syncNativeSurface({ visible: false });
  }, [syncNativeSurface]);

  const closeTestSettings = useCallback(() => {
    setTestSettingsOpen(false);
    void syncNativeSurface({ visible: true });
  }, [syncNativeSurface]);

  const closeWebPreviewPeer = useCallback((stopHost = true) => {
    const peer = webPreviewPeerRef.current;
    webPreviewPeerRef.current = null;
    if (peer) {
      peer.ontrack = null;
      peer.onconnectionstatechange = null;
      peer.close();
    }

    const video = webPreviewVideoRef.current;
    if (video) {
      const stream = video.srcObject instanceof MediaStream ? video.srcObject : null;
      stream?.getTracks().forEach((track) => track.stop());
      video.srcObject = null;
    }
    setWebVideoFps(null);
    setWebPaintFps(null);
    setWebFrameIntervalP95Ms(null);
    setWebVideoFrameCount(0);
    setWebRtcReceiverStats(null);
    webRtcStatsCountersRef.current = null;

    const previewSessionId = webPreviewSessionRef.current;
    webPreviewSessionRef.current = null;
    if (stopHost && previewSessionId) {
      void browserWebrtcPreviewStop(previewSessionId);
    }
  }, []);

  const switchToNativeRender = useCallback(() => {
    if (!nativeRendererAvailableForHost) return;
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setWebPreviewEngine("webrtc");
    setRenderMode(nativeRenderMode);
  }, [closeWebPreviewPeer, nativeRenderMode, nativeRendererAvailableForHost]);

  const switchToD3d12Render = useCallback(() => {
    if (!d3d12RendererAvailable || localRenderSwitchLocked) return;
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setWebPreviewEngine("webrtc");
    setRenderMode("d3d12_native");
  }, [closeWebPreviewPeer, d3d12RendererAvailable, localRenderSwitchLocked]);

  const switchToWebRtcRender = useCallback(() => {
    if (isLocalPipelinePreview && isTestBusy) {
      setTestMessage("请先停止测试再切换 WebRTC video");
      return;
    }

    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setWebPreviewEngine("webrtc");
    if (isLocalPipelinePreview) {
      setDecoder("none");
      setTransport("webrtc");
      setTestMessage("WebRTC video：浏览器视频解码 / RTP timing");
    }
    setRenderMode("web");
  }, [closeWebPreviewPeer, isLocalPipelinePreview, isTestBusy]);

  const switchToWebCodecsRender = useCallback(() => {
    if (!isLocalPipelinePreview) return;
    if (browserWebCodecsUltraBlockReason) {
      setTestMessage(browserWebCodecsUltraBlockReason);
      return;
    }

    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setWebPreviewEngine("webcodecs");
    setDecoder("none");
    setTransport("webrtc");
    setRenderMode("web");
    setTestMessage("WebCodecs WebGL2：WebSocket AU bridge / Worker + WebGL2 优先");
  }, [browserWebCodecsUltraBlockReason, closeWebPreviewPeer, isLocalPipelinePreview]);

  const clearNativeSurfaceSyncSchedule = useCallback(() => {
    if (syncAnimationFrameRef.current !== null) {
      window.cancelAnimationFrame(syncAnimationFrameRef.current);
      syncAnimationFrameRef.current = null;
    }

    for (const timerId of syncTimerIdsRef.current) {
      window.clearTimeout(timerId);
    }
    syncTimerIdsRef.current = [];
  }, []);

  const scheduleNativeSurfaceSync = useCallback(() => {
    clearNativeSurfaceSyncSchedule();

    syncAnimationFrameRef.current = window.requestAnimationFrame(() => {
      syncAnimationFrameRef.current = null;
      void syncNativeSurface();
    });

    syncTimerIdsRef.current = [50, 150, 300].map((delay) =>
      window.setTimeout(() => {
        void syncNativeSurface();
      }, delay)
    );
  }, [clearNativeSurfaceSyncSchedule, syncNativeSurface]);

  useEffect(() => {
    scheduleNativeSurfaceSync();
    return clearNativeSurfaceSyncSchedule;
  }, [clearNativeSurfaceSyncSchedule, scheduleNativeSurfaceSync]);

  useEffect(() => {
    const element = renderAreaRef.current;
    if (!element) return;

    const observer = new ResizeObserver(() => {
      scheduleNativeSurfaceSync();
    });
    observer.observe(element);
    window.addEventListener("focus", scheduleNativeSurfaceSync);
    window.addEventListener("resize", scheduleNativeSurfaceSync);
    window.visualViewport?.addEventListener("resize", scheduleNativeSurfaceSync);
    window.visualViewport?.addEventListener("scroll", scheduleNativeSurfaceSync);

    return () => {
      observer.disconnect();
      window.removeEventListener("focus", scheduleNativeSurfaceSync);
      window.removeEventListener("resize", scheduleNativeSurfaceSync);
      window.visualViewport?.removeEventListener("resize", scheduleNativeSurfaceSync);
      window.visualViewport?.removeEventListener("scroll", scheduleNativeSurfaceSync);
    };
  }, [scheduleNativeSurfaceSync]);

  useEffect(() => {
    if (!isLocalPipelinePreview || !isTestBusy) return;
    if (!isNative && !isTauriRuntime()) return;

    let cancelled = false;
    const poll = async () => {
      const metricsResult = await testHarnessGetMetrics();
      if (cancelled) return;

      if (metricsResult.ok) {
        setMetrics(metricsResult.value);
        if (metricsResult.value.error_message) {
          setTestMessage(metricsResult.value.error_message);
          setLastError(metricsResult.value.error_message);
        }
      }

      if (!currentRunId) return;
      const runResult = await testGetRun(currentRunId);
      if (cancelled || !runResult.ok || !runResult.value) return;

      if (runResult.value.status !== "running") {
        setTestStatus(runResult.value.status === "completed" ? "completed" : "failed");
        setLastCompletedRun(runResult.value);
        setTestMessage(
          runResult.value.summary?.error_message ??
            (runResult.value.status === "completed" ? "测试完成" : `测试${runResult.value.status}`)
        );
      } else if (testStatus === "starting") {
        setTestStatus("running");
        setTestMessage("测试运行中");
      }
    };

    void poll();
    const interval = window.setInterval(() => {
      void poll();
    }, METRICS_POLL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [currentRunId, isLocalPipelinePreview, isNative, isTestBusy, testStatus]);

  useEffect(() => {
    if (!isLocalPipelinePreview || !isTestBusy || isNative || webPreviewEngine !== "webrtc") {
      closeWebPreviewPeer();
      setWebPreviewMode("idle");
      setWebPreviewError(null);
      setWebPresentationLatencyStats(null);
      setWebFrameTimingMetadataCount(0);
      setWebFrameTimingChannelState(null);
      setWebFrameTimingMetadataAgeMs(null);
      return;
    }

    if (localStartBlockReason) {
      closeWebPreviewPeer();
      setWebPreviewMode("failed");
      setWebPreviewError(localStartBlockReason);
      setWebPresentationLatencyStats(null);
      setWebFrameTimingMetadataCount(0);
      setWebFrameTimingChannelState(null);
      setWebFrameTimingMetadataAgeMs(null);
      return;
    }

    if (typeof RTCPeerConnection === "undefined") {
      setWebPreviewMode("failed");
      setWebPreviewError("WebRTC is unavailable in this runtime");
      setWebPresentationLatencyStats(null);
      setWebFrameTimingMetadataCount(0);
      setWebFrameTimingChannelState(null);
      setWebFrameTimingMetadataAgeMs(null);
      return;
    }

    if (encoder === "nvenc_av1") {
      setWebPreviewMode("failed");
      setWebPreviewError("Browser WebRTC preview currently supports H.264 output");
      setWebPresentationLatencyStats(null);
      setWebFrameTimingMetadataCount(0);
      setWebFrameTimingChannelState(null);
      setWebFrameTimingMetadataAgeMs(null);
      return;
    }

    if (!browserSupportsH264WebrtcVideo()) {
      setWebPreviewMode("failed");
      setWebPreviewError("Browser WebRTC video renderer does not advertise H.264 receive support");
      setWebPresentationLatencyStats(null);
      setWebFrameTimingMetadataCount(0);
      setWebFrameTimingChannelState(null);
      setWebFrameTimingMetadataAgeMs(null);
      return;
    }

    let cancelled = false;
    let renderedVideoFrame = false;
    let connectTimeoutId: number | null = null;
    let statsIntervalId: number | null = null;
    const peer = new RTCPeerConnection({ iceServers: [] });
    const presentationLatencyTracker = new WebRtcPresentationLatencyTracker();
    let latestPresentationLatencyStats: WebRtcPresentationLatencyStats | null = null;
    (window as WindowWithMrdFrameTimingDebug).__mrdWebPreviewPeer = peer;
    updateWebRtcFrameTimingDebug({
      received: 0,
      lastMessage: null,
      lastStats: null,
      localChannelState: null,
      remoteChannelState: null,
    });
    const frameTimingChannel = peer.createDataChannel(WEBRTC_FRAME_TIMING_CHANNEL, {
      ordered: false,
      maxRetransmits: 0,
    });
    frameTimingChannel.onopen = () => {
      setWebFrameTimingChannelState(`local:${frameTimingChannel.readyState}`);
      updateWebRtcFrameTimingDebug({ localChannelState: frameTimingChannel.readyState });
    };
    frameTimingChannel.onclose = () => {
      setWebFrameTimingChannelState(`local:${frameTimingChannel.readyState}`);
      updateWebRtcFrameTimingDebug({ localChannelState: frameTimingChannel.readyState });
    };
    frameTimingChannel.onmessage = (event) => {
      setWebFrameTimingMetadataCount((count) => count + 1);
      const timing = parseWebRtcFrameTimingMetadata(event.data);
      if (timing) {
        setWebFrameTimingMetadataAgeMs(Date.now() - timing.captureUnixUs / 1000);
      }
      updateWebRtcFrameTimingDebug({
        received: ((window as WindowWithMrdFrameTimingDebug).__mrdFrameTimingDebug?.received ?? 0) + 1,
        lastMessage: typeof event.data === "string" ? event.data : String(event.data),
        localChannelState: frameTimingChannel.readyState,
      });
      presentationLatencyTracker.addMetadata(event.data);
    };
    peer.ondatachannel = (event) => {
      if (event.channel.label !== WEBRTC_FRAME_TIMING_CHANNEL) return;
      setWebFrameTimingChannelState(`remote:${event.channel.readyState}`);
      updateWebRtcFrameTimingDebug({ remoteChannelState: event.channel.readyState });
      event.channel.onopen = () => {
        setWebFrameTimingChannelState(`remote:${event.channel.readyState}`);
        updateWebRtcFrameTimingDebug({ remoteChannelState: event.channel.readyState });
      };
      event.channel.onclose = () => {
        setWebFrameTimingChannelState(`remote:${event.channel.readyState}`);
        updateWebRtcFrameTimingDebug({ remoteChannelState: event.channel.readyState });
      };
      event.channel.onmessage = (messageEvent) => {
        setWebFrameTimingMetadataCount((count) => count + 1);
        const timing = parseWebRtcFrameTimingMetadata(messageEvent.data);
        if (timing) {
          setWebFrameTimingMetadataAgeMs(Date.now() - timing.captureUnixUs / 1000);
        }
        updateWebRtcFrameTimingDebug({
          received:
            ((window as WindowWithMrdFrameTimingDebug).__mrdFrameTimingDebug?.received ?? 0) + 1,
          lastMessage:
            typeof messageEvent.data === "string" ? messageEvent.data : String(messageEvent.data),
          remoteChannelState: event.channel.readyState,
        });
        presentationLatencyTracker.addMetadata(messageEvent.data);
      };
    };
    webPreviewPeerRef.current = peer;
    webPreviewSessionRef.current = sessionId;
    setWebPreviewMode("connecting");
    setWebPreviewError(null);
    setWebPresentationLatencyStats(null);
    setWebFrameTimingMetadataCount(0);
    setWebFrameTimingChannelState("connecting");
    setWebFrameTimingMetadataAgeMs(null);

    const markVideoRendered = () => {
      if (cancelled || renderedVideoFrame) return;
      renderedVideoFrame = true;
      if (connectTimeoutId !== null) {
        window.clearTimeout(connectTimeoutId);
        connectTimeoutId = null;
      }
      setWebPreviewMode("webrtc");
      setWebPreviewError(null);
    };

    const startReceiverStatsPolling = (receiver: RTCRtpReceiver) => {
      if (statsIntervalId !== null) {
        window.clearInterval(statsIntervalId);
        statsIntervalId = null;
      }
      webRtcStatsCountersRef.current = null;
      const poll = async () => {
        try {
          const report = await receiver.getStats();
          if (cancelled) return;
          const next = summarizeWebRtcInboundVideoStats(
            report,
            webRtcStatsCountersRef.current,
            performance.now()
          );
          webRtcStatsCountersRef.current = next.counters;
          if (next.stats) {
            setWebRtcReceiverStats(next.stats);
          }
        } catch {
          // Stats are diagnostic-only; keep media playback independent.
        }
      };
      void poll();
      statsIntervalId = window.setInterval(() => {
        void poll();
      }, 1_000);
    };

    const videoTransceiver = peer.addTransceiver("video", { direction: "recvonly" });
    applyWebRtcReceiverLowLatencyHint(videoTransceiver.receiver);
    peer.ontrack = (event) => {
      if (cancelled) return;
      applyWebRtcReceiverLowLatencyHint(event.receiver);
      applyWebRtcVideoMotionHint(event.track);
      startReceiverStatsPolling(event.receiver);
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      stream.getVideoTracks().forEach(applyWebRtcVideoMotionHint);

      const bindStreamToVideo = () => {
        if (cancelled) return;
        const video = webPreviewVideoRef.current;
        if (!video) {
          window.requestAnimationFrame(bindStreamToVideo);
          return;
        }

        if (video.srcObject !== stream) {
          video.srcObject = stream;
        }
        video.muted = true;
        video.playsInline = true;
        const videoWithFrameCallback = video as HTMLVideoElement & {
          requestVideoFrameCallback?: (
            callback: (
              now: number,
              metadata: WebRtcVideoFrameCallbackMetadata
            ) => void
          ) => number;
        };
        let lastStatsAt = performance.now();
        let lastPresentedFrames = 0;
        let callbacksSinceStats = 0;
        let lastFrameCallbackAt: number | null = null;
        const frameIntervalsMs: number[] = [];
        const observeFrame = (now: number, metadata: WebRtcVideoFrameCallbackMetadata) => {
          if (cancelled) return;
          markVideoRendered();
          latestPresentationLatencyStats =
            presentationLatencyTracker.observeFrame(now, metadata) ?? latestPresentationLatencyStats;
          if (latestPresentationLatencyStats) {
            updateWebRtcFrameTimingDebug({ lastStats: latestPresentationLatencyStats });
          }
          callbacksSinceStats += 1;
          if (lastFrameCallbackAt !== null) {
            frameIntervalsMs.push(now - lastFrameCallbackAt);
            if (frameIntervalsMs.length > 240) {
              frameIntervalsMs.shift();
            }
          }
          lastFrameCallbackAt = now;
          const presentedFrames = metadata.presentedFrames ?? lastPresentedFrames + 1;
          const elapsedMs = performance.now() - lastStatsAt;
          if (elapsedMs >= 1000) {
            const deltaFrames = presentedFrames - lastPresentedFrames;
            setWebVideoFps((deltaFrames * 1000) / elapsedMs);
            setWebPaintFps((callbacksSinceStats * 1000) / elapsedMs);
            setWebFrameIntervalP95Ms(percentile(frameIntervalsMs, 0.95));
            if (latestPresentationLatencyStats) {
              setWebPresentationLatencyStats(latestPresentationLatencyStats);
            }
            setWebVideoFrameCount(presentedFrames);
            lastPresentedFrames = presentedFrames;
            callbacksSinceStats = 0;
            lastStatsAt = performance.now();
          }
          videoWithFrameCallback.requestVideoFrameCallback?.(observeFrame);
        };
        videoWithFrameCallback.requestVideoFrameCallback?.(observeFrame);
        video.addEventListener("loadeddata", markVideoRendered, { once: true });
        video.addEventListener("playing", markVideoRendered, { once: true });
        void video.play().catch((error) => {
          if (cancelled) return;
          if (error instanceof DOMException && error.name === "AbortError") {
            return;
          }
          const message = error instanceof Error ? error.message : String(error);
          if (message.toLowerCase().includes("interrupted by a new load request")) {
            return;
          }
          setWebPreviewError(error instanceof Error ? error.message : String(error));
        });
      };

      bindStreamToVideo();
    };
    peer.onconnectionstatechange = () => {
      if (cancelled) return;
      if (peer.connectionState === "failed" || peer.connectionState === "disconnected") {
        setWebPreviewMode("failed");
        setWebPreviewError(`WebRTC preview ${peer.connectionState}`);
      }
    };

    connectTimeoutId = window.setTimeout(() => {
      if (cancelled || renderedVideoFrame) return;
      closeWebPreviewPeer();
      setWebPreviewMode("failed");
      setWebPreviewError("WebRTC 视频未在超时内解码出第一帧，未使用图片回退");
    }, WEB_PREVIEW_CONNECT_TIMEOUT_MS);

    const startPreview = async () => {
      try {
        const offer = await peer.createOffer();
        await peer.setLocalDescription(offer);
        await waitForIceGatheringComplete(peer);
        const offerSdp = peer.localDescription?.sdp;
        if (!offerSdp) {
          throw new Error("WebRTC preview offer SDP is empty");
        }

        const answer = await browserWebrtcPreviewStart({
          sessionId,
          offerSdp,
          fps: Number(fps),
          width: Number(resolution.split("x")[0]),
          height: Number(resolution.split("x")[1]),
          bitrateMbps: Number(bitrate),
          h264Profile: browserWebrtcPreviewH264Profile(encoder, decoder),
        });
        if (cancelled) return;
        if (!answer.ok) {
          throw new Error(answer.error.message);
        }

        await peer.setRemoteDescription({
          type: "answer",
          sdp: answer.value.answer_sdp,
        });
      } catch (error) {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        closeWebPreviewPeer();
        setWebPreviewMode("failed");
        setWebPreviewError(message);
      }
    };

    void startPreview();

    return () => {
      cancelled = true;
      if (connectTimeoutId !== null) {
        window.clearTimeout(connectTimeoutId);
      }
      if (statsIntervalId !== null) {
        window.clearInterval(statsIntervalId);
      }
      closeWebPreviewPeer();
    };
  }, [
    closeWebPreviewPeer,
    bitrate,
    decoder,
    encoder,
    fps,
    isLocalPipelinePreview,
    isNative,
    isTestBusy,
    localStartBlockReason,
    resolution,
    sessionId,
    webCodecsCanvasEpoch,
    webPreviewEngine,
  ]);

  useEffect(() => {
    if (!isTestBusy || webPreviewEngine !== "webcodecs") {
      webCodecsWorkerInitRetryRef.current = 0;
      webCodecsMainCanvasRecoveringRef.current = false;
    }
  }, [isTestBusy, webPreviewEngine]);

  useEffect(() => {
    if (
      !isLocalPipelinePreview ||
      !isTestBusy ||
      isNative ||
      webPreviewEngine !== "webcodecs"
    ) {
      return;
    }

    if (localStartBlockReason) {
      setWebPreviewMode("failed");
      setWebPreviewError(localStartBlockReason);
      return;
    }

    const maybeWindow = window as unknown as Window & {
      VideoDecoder?: new (init: {
        output: (frame: VideoFrame) => void;
        error: (error: Error) => void;
      }) => {
        configure: (config: Record<string, unknown>) => void;
        decode: (chunk: unknown) => void;
        close: () => void;
        decodeQueueSize?: number;
      };
      EncodedVideoChunk?: new (init: {
        type: "key" | "delta";
        timestamp: number;
        duration?: number;
        data: BufferSource;
      }) => unknown;
    };
    const VideoDecoderCtor = maybeWindow.VideoDecoder;
    const EncodedVideoChunkCtor = maybeWindow.EncodedVideoChunk;
    if (!VideoDecoderCtor || !EncodedVideoChunkCtor) {
      setWebPreviewMode("failed");
      setWebPreviewError("WebCodecs VideoDecoder / EncodedVideoChunk is unavailable");
      return;
    }

    const canvasElement = webCodecsCanvasRef.current;
    const targetFps = Number(fps);
    const preferWorkerRendering =
      Number.isFinite(targetFps) &&
      targetFps <= 144 &&
      browserSupportsWebCodecsWorkerRendering(canvasElement);
    if (preferWorkerRendering) {
      if (canvasElement && webCodecsTransferredCanvasRef.current === canvasElement) {
        return;
      }
      let worker: Worker | null = null;
      let resizeObserver: ResizeObserver | null = null;
      let cancelled = false;
      let canvasTransferred = false;
      try {
        const offscreenCanvas = (
          canvasElement as HTMLCanvasElement & {
            transferControlToOffscreen: () => OffscreenCanvas;
          }
        ).transferControlToOffscreen();
        canvasTransferred = true;
        webCodecsTransferredCanvasRef.current = canvasElement;
        worker = new Worker(new URL("../workers/webCodecsPreview.worker.ts", import.meta.url), {
          type: "module",
        });

        const viewport = () => {
          const bounds = canvasElement?.getBoundingClientRect();
          const [requestedWidth, requestedHeight] = resolution.split("x").map(Number);
          return {
            viewportWidth: bounds?.width || canvasElement?.clientWidth || requestedWidth,
            viewportHeight: bounds?.height || canvasElement?.clientHeight || requestedHeight,
            devicePixelRatio: window.devicePixelRatio || 1,
          };
        };
        const postResize = () => {
          if (!worker) return;
          worker.postMessage({
            type: "resize",
            ...viewport(),
          });
        };

        worker.onmessage = (event: MessageEvent<WebCodecsWorkerMessage>) => {
          if (cancelled) return;
          const message = event.data;
          if (message.type === "ready") {
            setWebPreviewMode("webcodecs");
            setWebPreviewError(null);
            setWebFrameTimingChannelState(`webcodecs-worker:${message.rendererBackend}`);
            setTestMessage(
              `WebCodecs Worker + ${
                message.rendererBackend === "webgl2" ? "WebGL2" : "2D Canvas"
              } 本机采集运行中 (${message.width}x${message.height}@${message.fps})`
            );
            return;
          }
          if (message.type === "stats") {
            setWebVideoFps(message.fps);
            setWebPaintFps(message.paintFps);
            setWebVideoFrameCount(message.frameCount);
            setWebFrameIntervalP95Ms(message.frameIntervalP95Ms);
            setWebPresentationLatencyStats({
              latestMs: message.latencyLatestMs,
              p50Ms: message.latencyP50Ms,
              p95Ms: message.latencyP95Ms,
              maxMs: message.latencyMaxMs,
              samples: message.latencySamples,
              source: "webcodecs_frame_header",
            });
            return;
          }
          if (message.type === "error") {
            setWebPreviewMode("failed");
            setWebPreviewError(message.message);
            setWebFrameTimingChannelState("webcodecs-worker:error");
            return;
          }
          setWebFrameTimingChannelState("webcodecs-worker:closed");
        };
        worker.onerror = (event) => {
          if (cancelled) return;
          setWebPreviewMode("failed");
          setWebPreviewError(event.message || "WebCodecs worker failed");
          setWebFrameTimingChannelState("webcodecs-worker:error");
        };
        setWebPreviewMode("connecting");
        setWebPreviewError(null);
        setWebFrameTimingChannelState("webcodecs-worker:connecting");
        worker.postMessage(
          {
            type: "start",
            canvas: offscreenCanvas,
            websocketUrl: browserWebcodecsPreviewWebSocketUrl(),
            sessionId,
            fps: Number(fps),
            width: Number(resolution.split("x")[0]),
            height: Number(resolution.split("x")[1]),
            bitrateMbps: Number(bitrate),
            h264Profile: "baseline",
            ...viewport(),
          },
          [offscreenCanvas]
        );
        resizeObserver = new ResizeObserver(postResize);
        resizeObserver.observe(canvasElement as Element);

        return () => {
          cancelled = true;
          resizeObserver?.disconnect();
          worker?.postMessage({ type: "stop" });
          worker?.terminate();
          if (webCodecsTransferredCanvasRef.current === canvasElement) {
            webCodecsTransferredCanvasRef.current = null;
          }
        };
      } catch (error) {
        resizeObserver?.disconnect();
        worker?.terminate();
        if (webCodecsTransferredCanvasRef.current === canvasElement) {
          return;
        }
        const message = error instanceof Error ? error.message : String(error);
        const transferredCanvasError =
          canvasTransferred || /transferred|transferControlToOffscreen/i.test(message);
        if (transferredCanvasError && webCodecsWorkerInitRetryRef.current < 2) {
          webCodecsWorkerInitRetryRef.current += 1;
          setWebFrameTimingChannelState("webcodecs-worker:retry");
          setTestMessage(`WebCodecs canvas 已重建，正在重新启动 Worker: ${message}`);
          setWebCodecsCanvasEpoch((current) => current + 1);
          return;
        }
        if (transferredCanvasError) {
          setWebPreviewMode("failed");
          setWebPreviewError(`WebCodecs Worker 初始化失败: ${message}`);
          setWebFrameTimingChannelState("webcodecs-worker:error");
          return;
        }
        setWebFrameTimingChannelState("webcodecs-main:fallback");
        setTestMessage(`WebCodecs Worker 初始化失败，回退主线程解码: ${message}`);
      }
    }

    let cancelled = false;
    let decoderClosed = false;
    let configured = false;
    let lastOutputAt: number | null = null;
    let lastStatsAt = performance.now();
    let framesSinceStats = 0;
    let totalFrames = 0;
    const frameIntervalsMs: number[] = [];
    const latencySamplesMs: number[] = [];
    const headersByTimestamp = new Map<number, WebCodecsFrameHeader>();
    const socket = new WebSocket(browserWebcodecsPreviewWebSocketUrl());
    socket.binaryType = "arraybuffer";
    setWebFrameTimingChannelState("webcodecs-main:2d");

    const updateLatencyStats = (latestMs: number) => {
      latencySamplesMs.push(latestMs);
      if (latencySamplesMs.length > 240) latencySamplesMs.shift();
      setWebPresentationLatencyStats({
        latestMs,
        p50Ms: percentile(latencySamplesMs, 0.5) ?? latestMs,
        p95Ms: percentile(latencySamplesMs, 0.95) ?? latestMs,
        maxMs: Math.max(...latencySamplesMs),
        samples: latencySamplesMs.length,
        source: "webcodecs_frame_header",
      });
    };

    const videoDecoder = new VideoDecoderCtor({
      output: (frame: VideoFrame) => {
        if (cancelled) {
          frame.close();
          return;
        }
        const now = performance.now();
        const header = headersByTimestamp.get(frame.timestamp);
        if (header) {
          headersByTimestamp.delete(frame.timestamp);
          updateLatencyStats(performance.timeOrigin + now - header.capture_unix_us / 1000);
        }
        if (lastOutputAt !== null) {
          frameIntervalsMs.push(now - lastOutputAt);
          if (frameIntervalsMs.length > 240) frameIntervalsMs.shift();
        }
        lastOutputAt = now;
        framesSinceStats += 1;
        totalFrames += 1;
        const canvas = webCodecsCanvasRef.current;
        let context: CanvasRenderingContext2D | null = null;
        try {
          context = canvas?.getContext("2d", { alpha: false }) ?? null;
          webCodecsMainCanvasRecoveringRef.current = false;
        } catch (error) {
          if (!webCodecsMainCanvasRecoveringRef.current) {
            webCodecsMainCanvasRecoveringRef.current = true;
            const message = error instanceof Error ? error.message : String(error);
            setWebFrameTimingChannelState("webcodecs-main:recovering");
            setTestMessage(`WebCodecs canvas 已重建，正在恢复主线程绘制: ${message}`);
            setWebCodecsCanvasEpoch((current) => current + 1);
          }
          frame.close();
          return;
        }
        if (canvas && context) {
          const displayWidth = frame.displayWidth || frame.codedWidth;
          const displayHeight = frame.displayHeight || frame.codedHeight;
          const deviceScale = window.devicePixelRatio || 1;
          const maxCanvasWidth = Math.max(2, Math.round(canvas.clientWidth * deviceScale));
          const maxCanvasHeight = Math.max(2, Math.round(canvas.clientHeight * deviceScale));
          const displayAspect = displayWidth / Math.max(1, displayHeight);
          let targetWidth = Math.min(displayWidth, maxCanvasWidth);
          let targetHeight = Math.round(targetWidth / displayAspect);
          if (targetHeight > Math.min(displayHeight, maxCanvasHeight)) {
            targetHeight = Math.min(displayHeight, maxCanvasHeight);
            targetWidth = Math.round(targetHeight * displayAspect);
          }
          targetWidth = Math.max(2, targetWidth);
          targetHeight = Math.max(2, targetHeight);
          if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
            canvas.width = targetWidth;
            canvas.height = targetHeight;
          }
          context.drawImage(frame, 0, 0, canvas.width, canvas.height);
        }
        frame.close();
        const elapsedMs = now - lastStatsAt;
        if (elapsedMs >= 1000) {
          const fps = (framesSinceStats * 1000) / elapsedMs;
          setWebVideoFps(fps);
          setWebPaintFps(fps);
          setWebVideoFrameCount(totalFrames);
          setWebFrameIntervalP95Ms(percentile(frameIntervalsMs, 0.95));
          framesSinceStats = 0;
          lastStatsAt = now;
        }
      },
      error: (error: Error) => {
        if (cancelled) return;
        setWebPreviewMode("failed");
        setWebPreviewError(`WebCodecs decode failed: ${error.message}`);
      },
    });

    const configureDecoder = async (ready: WebCodecsReadyMessage) => {
      if (configured || cancelled) return;
      const config = {
        codec: ready.codec,
        codedWidth: ready.width,
        codedHeight: ready.height,
        hardwareAcceleration: "prefer-software",
        optimizeForLatency: true,
        avc: { format: "annexb" },
      };
      const supportChecker = (
        VideoDecoderCtor as unknown as {
          isConfigSupported?: (config: Record<string, unknown>) => Promise<{ supported: boolean; config: Record<string, unknown> }>;
        }
      ).isConfigSupported;
      const support = supportChecker ? await supportChecker(config).catch(() => null) : null;
      if (support && !support.supported) {
        throw new Error(`WebCodecs decoder does not support ${ready.codec} annexb`);
      }
      videoDecoder.configure(support?.config ?? config);
      configured = true;
      setWebPreviewMode("webcodecs");
      setWebPreviewError(null);
      setWebFrameTimingChannelState("webcodecs:open");
      setTestMessage(
        `WebCodecs H.264 Annex B 本机采集运行中 (${ready.width}x${ready.height}@${ready.fps})`
      );
    };

    socket.onopen = () => {
      if (cancelled) return;
      setWebPreviewMode("connecting");
      setWebPreviewError(null);
      setWebFrameTimingChannelState("webcodecs:connecting");
      socket.send(
        JSON.stringify({
          type: "start",
          session_id: sessionId,
          fps: Number(fps),
          width: Number(resolution.split("x")[0]),
          height: Number(resolution.split("x")[1]),
          bitrate_mbps: Number(bitrate),
          h264_profile: "baseline",
        })
      );
    };
    socket.onmessage = (event) => {
      if (cancelled) return;
      void (async () => {
        if (typeof event.data === "string") {
          const message = JSON.parse(event.data) as WebCodecsReadyMessage | WebCodecsErrorMessage;
          if (message.type === "mrd.webcodecs.ready.v1") {
            await configureDecoder(message as WebCodecsReadyMessage);
          } else if ("message" in message && message.message) {
            setWebPreviewMode("failed");
            setWebPreviewError(message.message);
          }
          return;
        }
        const buffer =
          event.data instanceof ArrayBuffer ? event.data : await (event.data as Blob).arrayBuffer();
        const accessUnit = parseWebCodecsAccessUnitMessage(buffer);
        if (!accessUnit || !configured) return;
        if ((videoDecoder.decodeQueueSize ?? 0) > 2 && !accessUnit.header.keyframe) {
          return;
        }
        headersByTimestamp.set(accessUnit.header.timestamp_us, accessUnit.header);
        const chunkData = accessUnit.payload.buffer.slice(
          accessUnit.payload.byteOffset,
          accessUnit.payload.byteOffset + accessUnit.payload.byteLength
        ) as ArrayBuffer;
        videoDecoder.decode(
          new EncodedVideoChunkCtor({
            type: accessUnit.header.keyframe ? "key" : "delta",
            timestamp: accessUnit.header.timestamp_us,
            duration: accessUnit.header.duration_us,
            data: chunkData,
          })
        );
      })().catch((error) => {
        if (cancelled) return;
        setWebPreviewMode("failed");
        setWebPreviewError(error instanceof Error ? error.message : String(error));
      });
    };
    socket.onerror = () => {
      if (cancelled) return;
      setWebPreviewMode("failed");
      setWebPreviewError("WebCodecs preview WebSocket failed");
    };
    socket.onclose = () => {
      if (cancelled) return;
      setWebFrameTimingChannelState("webcodecs:closed");
    };

    return () => {
      cancelled = true;
      try {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: "stop" }));
        }
      } catch {
        // Best-effort cleanup.
      }
      socket.close();
      if (!decoderClosed) {
        decoderClosed = true;
        videoDecoder.close();
      }
    };
  }, [
    bitrate,
    decoder,
    encoder,
    fps,
    isLocalPipelinePreview,
    isNative,
    isTestBusy,
    localStartBlockReason,
    resolution,
    sessionId,
    webPreviewEngine,
  ]);

  useEffect(() => {
    if (isLocalPipelinePreview || !isTauriRuntime()) return;

    let cancelled = false;
    const poll = async () => {
      try {
        const [snapshot, probe, pipeline] = await Promise.all([
          getSessionSnapshot(sessionId),
          getProbeSnapshot(sessionId),
          ipcMediaPipelineSnapshot(sessionId),
        ]);
        if (cancelled) return;

        setSessionSnapshot(snapshot);
        setProbeSnapshot(probe);
        if (pipeline.ok && pipeline.value) {
          setMediaPipelineSnapshot(pipeline.value);
        }

        const errorMessage = snapshot.last_error ?? probe.last_error ?? null;
        if (errorMessage) {
          setLastError(errorMessage);
          setTestMessage(errorMessage);
        } else if (snapshot.state === "failed") {
          setTestStatus("failed");
          setTestMessage("远程会话失败");
        } else if (snapshot.receiver_active) {
          setTestStatus("running");
          setTestMessage(
            probe.frames_decoded > 0 || probe.frames_received > 0
              ? "远程接收中"
              : "远程接收已启动，等待远端媒体帧"
          );
        } else if (testStatus === "running" || testStatus === "starting") {
          setTestStatus("idle");
          setTestMessage("远程会话已连接，等待启动接收侧");
        }
      } catch (error) {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        setLastError(message);
        setTestMessage(message);
      }
    };

    void poll();
    const interval = window.setInterval(() => {
      void poll();
    }, 1_000);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [isLocalPipelinePreview, sessionId, testStatus]);

  useEffect(() => {
    if (
      isLocalPipelinePreview ||
      !isTauriRuntime() ||
      !isNative ||
      !nativeSurfaceAttached ||
      !remoteFrameDataUrl
    ) {
      return;
    }

    const frameKey =
      probeSnapshot?.last_media_payload_hash ??
      `${probeSnapshot?.last_media_sequence ?? "unknown"}:${remoteFrameDataUrl.length}`;
    if (nativePreviewFrameKeyRef.current === frameKey) {
      return;
    }
    nativePreviewFrameKeyRef.current = frameKey;

    let cancelled = false;
    void presentRemotePreviewFrameOnNativeSurface(remoteFrameDataUrl).then((result) => {
      if (cancelled) return;
      if (!result.ok) {
        setLastError(result.error.message);
        return;
      }
      if (!result.value) {
        setLastError("Native render surface is not attached");
      }
    });

    return () => {
      cancelled = true;
    };
  }, [
    isLocalPipelinePreview,
    isNative,
    nativeSurfaceAttached,
    probeSnapshot?.last_media_payload_hash,
    probeSnapshot?.last_media_sequence,
    remoteFrameDataUrl,
  ]);

  const noDragSelector =
    'button, a, input, select, textarea, [role="button"], [data-no-drag="true"]';

  const handleDragStart = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0 || event.detail > 1) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(noDragSelector)) return;
    event.preventDefault();
    void withTauriWindow((appWindow) => appWindow.startDragging());
  };

  const handleToggleMaximize = async () => {
    await withTauriWindow(async (appWindow) => {
      await appWindow.toggleMaximize();
      const next = await appWindow.isMaximized();
      if (typeof next === "boolean") setIsMaximized(next);
    });
    scheduleNativeSurfaceSync();
  };

  const applyLowLatencyProfile = useCallback(() => {
    setWebPreviewEngine("webrtc");
    if (hostOs === "macos") {
      setCapture("macos");
      setEncoder("videotoolbox_h264");
      setDecoder(capabilities?.available_decoders.includes("videotoolbox") ? "videotoolbox" : "software");
      setTransport("quic");
      setResolution("1920x1080");
      setFps("60");
      setBitrate("20");
      setRenderMode(
        isTauriRuntime() && capabilities?.available_renderers?.includes("macos")
          ? "metal_native"
          : "web"
      );
      return;
    }

    if (hostOs === "linux") {
      const availableEncoders = capabilities?.available_encoders ?? [];
      const availableDecoders = capabilities?.available_decoders ?? [];
      setCapture("linux");
      setEncoder(
        pickCapability(linuxNativeEncoderPreference, availableEncoders) ?? "openh264"
      );
      setDecoder(
        pickCapability(linuxNativeDecoderPreference, availableDecoders) ?? "none"
      );
      setTransport("loopback");
      setResolution("1920x1080");
      setFps("60");
      setBitrate("20");
      setRenderMode(
        isTauriRuntime() && capabilities?.available_renderers?.includes("linux")
          ? "linux_native"
          : "web"
      );
      return;
    }

    setCapture("dxgi");
    setEncoder("nvenc_h264");
    setDecoder("nvdec");
    setTransport("quic");
    setResolution("1920x1080");
    setFps("144");
    setBitrate("20");
    setRenderMode(
      isTauriRuntime() && capabilities?.available_renderers?.includes("d3d11")
        ? "d3d11_native"
        : "web"
    );
  }, [capabilities, hostOs]);

  const applyBrowserWebRtc2k144LowLatencyProfile = useCallback(() => {
    if (browserWebRtc2k144BlockReason) {
      setTestMessage(browserWebRtc2k144BlockReason);
      return;
    }
    setWebPreviewEngine("webrtc");
    setCapture("dxgi");
    setEncoder("nvenc_h264");
    setDecoder("none");
    setTransport("webrtc");
    setResolution("2560x1440");
    setFps("144");
    setBitrate("20");
    setRenderMode("web");
    setTestMessage("WebRTC 2K144 低延迟档：浏览器视频解码 / RTP timing / 20 Mbps / Web View");
  }, [browserWebRtc2k144BlockReason]);

  const applyBrowserWebCodecsUltraLowLatencyProfile = useCallback(() => {
    if (browserWebCodecsUltraBlockReason) {
      setTestMessage(browserWebCodecsUltraBlockReason);
      return;
    }
    setWebPreviewEngine("webcodecs");
    setCapture("dxgi");
    setEncoder("nvenc_h264");
    setDecoder("none");
    setTransport("webrtc");
    setResolution("2560x1440");
    setFps("144");
    setBitrate("20");
    setRenderMode("web");
    setTestMessage(
      "WebCodecs 2K144：WebSocket AU bridge / Worker + WebGL2 优先，自动回退 2D/主线程"
    );
  }, [browserWebCodecsUltraBlockReason]);

  const ensureRemoteCaptureSourceSelected = useCallback(async () => {
    if (isLocalPipelinePreview) return null;
    if (captureSourceSelection) return captureSourceSelection;

    const sources =
      captureSources.length > 0
        ? captureSources
        : await listRemoteCaptureSources(sessionId, false, 24);
    const nextSources = Array.isArray(sources) ? sources : [];
    setCaptureSources(nextSources);

    const preferredSource = pickPreferredCaptureSource(nextSources);
    if (!preferredSource) {
      throw new Error("远端未发现可捕获的全屏/窗口源，无法启动接收");
    }

    const selection = await selectRemoteCaptureSource(sessionId, preferredSource.id);
    setCaptureSourceSelection(selection);
    setTestMessage(
      `默认远端捕获源: ${captureSourceKindLabel(selection.source.source_kind)} / ${selection.source.title}`
    );
    return selection;
  }, [captureSourceSelection, captureSources, isLocalPipelinePreview, sessionId]);

  const handleStartRemoteReceiver = useCallback(async () => {
    setTestSettingsOpen(false);
    setLastError(null);
    setTestMessage("启动远程接收侧");
    setTestStatus("starting");
    setCurrentRunId(null);
    setMetrics(null);

    try {
      const snapshot = await getSessionSnapshot(sessionId);
      setSessionSnapshot(snapshot);

      if (snapshot.state === "failed") {
        const message = snapshot.last_error ?? "远程会话已失败";
        setTestStatus("failed");
        setTestMessage(message);
        setLastError(message);
        return;
      }

      if (snapshot.role !== "controller" && snapshot.role !== "unknown") {
        const message = `当前窗口角色为 ${snapshot.role}，不能作为远程接收端`;
        setTestStatus("failed");
        setTestMessage(message);
        setLastError(message);
        return;
      }

      await ensureRemoteCaptureSourceSelected();

      if (!snapshot.receiver_active) {
        await startReceiver(sessionId);
      }

      const [nextSnapshot, nextProbe] = await Promise.all([
        getSessionSnapshot(sessionId),
        getProbeSnapshot(sessionId),
      ]);
      setSessionSnapshot(nextSnapshot);
      setProbeSnapshot(nextProbe);
      setTestStatus("running");
      setTestMessage(
        nextProbe.frames_decoded > 0 || nextProbe.frames_received > 0
          ? "远程接收中"
          : "远程接收已启动，等待远端媒体帧"
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setTestStatus("failed");
      setTestMessage(message);
      setLastError(message);
    }
  }, [ensureRemoteCaptureSourceSelected, sessionId]);

  const handleApplyRemoteMediaProfile = useCallback(async () => {
    if (isLocalPipelinePreview) return;
    if (transport !== "quic") {
      const message = "远端媒体参数切换当前仅支持 LAN QUIC 会话";
      setLastError(message);
      setTestMessage(message);
      return;
    }

    setLastError(null);
    setTestMessage("正在协商远端媒体参数");
    try {
      const negotiation = await updateMediaProfile(sessionId, buildRemoteMediaProfile());
      setMediaProfileNegotiation(negotiation);
      const selected = negotiation.selected;
      setTestMessage(
        `远端已切换 ${selected.width}x${selected.height}@${selected.fps} / ${selected.bitrate_mbps} Mbps (${negotiation.status})`
      );
      const probe = await getProbeSnapshot(sessionId);
      setProbeSnapshot(probe);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setLastError(message);
      setTestMessage(message);
    }
  }, [buildRemoteMediaProfile, isLocalPipelinePreview, sessionId, transport]);

  const hydrateRemoteCaptureSourcePreviews = useCallback(async (sources: CaptureSource[]) => {
    if (isLocalPipelinePreview || sources.length === 0) return;

    try {
      const previewSources = await listRemoteCaptureSources(
        sessionId,
        true,
        Math.min(sources.length, 8)
      );
      const previewById = new Map(previewSources.map((source) => [source.id, source]));
      setCaptureSources((currentSources) =>
        currentSources.map((source) => {
          const preview = previewById.get(source.id);
          if (!preview?.preview_data_url) return source;
          return {
            ...source,
            preview_data_url: preview.preview_data_url,
            preview_width: preview.preview_width,
            preview_height: preview.preview_height,
          };
        })
      );
    } catch {
      // Keep source selection usable when preview capture is slow or unsupported.
    }
  }, [isLocalPipelinePreview, sessionId]);

  const handleRefreshRemoteCaptureSources = useCallback(async () => {
    if (isLocalPipelinePreview) return;

    setCaptureSourcesLoading(true);
    setLastError(null);
    setTestMessage("正在枚举远端捕获源");
    try {
      const sources = await listRemoteCaptureSources(sessionId, false, 24);
      const nextSources = Array.isArray(sources) ? sources : [];
      setCaptureSources(nextSources);
      void hydrateRemoteCaptureSourcePreviews(nextSources);
      setTestMessage(
        nextSources.length > 0
          ? `已获取 ${nextSources.length} 个远端捕获源`
          : "未发现可捕获的远端窗口/屏幕"
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setCaptureSources([]);
      setLastError(message);
      setTestMessage(message);
    } finally {
      setCaptureSourcesLoading(false);
    }
  }, [hydrateRemoteCaptureSourcePreviews, isLocalPipelinePreview, sessionId]);

  const handleSelectRemoteCaptureSource = useCallback(
    async (source: CaptureSource) => {
      if (isLocalPipelinePreview) return;

      setLastError(null);
      setTestMessage(`正在切换远端捕获源: ${captureSourceKindLabel(source.source_kind)} / ${source.title}`);
      try {
        const selection = await selectRemoteCaptureSource(sessionId, source.id);
        setCaptureSourceSelection(selection);
        setTestMessage(
          `远端捕获源已切换: ${captureSourceKindLabel(selection.source.source_kind)} / ${selection.source.title}`
        );
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setLastError(message);
        setTestMessage(message);
      }
    },
    [isLocalPipelinePreview, sessionId]
  );

  useEffect(() => {
    if (
      isLocalPipelinePreview ||
      !isTauriRuntime() ||
      autoCaptureSourceRequestedRef.current === sessionId
    ) {
      return;
    }

    autoCaptureSourceRequestedRef.current = sessionId;
    let cancelled = false;

    void (async () => {
      try {
        const sources = await listRemoteCaptureSources(sessionId, false, 24);
        if (cancelled) return;

        const nextSources = Array.isArray(sources) ? sources : [];
        setCaptureSources(nextSources);
        const preferredSource = pickPreferredCaptureSource(nextSources);
        if (!preferredSource) {
          setTestMessage("远端未发现可捕获的全屏/窗口源");
          return;
        }

        const selection = await selectRemoteCaptureSource(sessionId, preferredSource.id);
        if (cancelled) return;

        setCaptureSourceSelection(selection);
        setTestMessage(
          `默认远端捕获源: ${captureSourceKindLabel(selection.source.source_kind)} / ${selection.source.title}`
        );
      } catch (error) {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        setTestMessage(`远端默认捕获源选择失败: ${message}`);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [isLocalPipelinePreview, sessionId]);

  const waitForLocalRunFinished = useCallback(
    async (runId: string, timeoutMs: number): Promise<TestRun | null> => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (matrixStopRequestedRef.current) return null;
        const runResult = await testGetRun(runId);
        if (runResult.ok && runResult.value?.status !== "running") {
          return runResult.value;
        }
        await new Promise((resolve) => window.setTimeout(resolve, METRICS_POLL_MS));
      }
      return null;
    },
    []
  );

  const buildBrowserPreviewCompletedRun = useCallback(
    (status: TestRun["status"], message?: string): TestRun => {
      const now = Date.now();
      const startedAt = webPreviewRunStartedAtRef.current ?? now;
      const samples = diagnosticsSamplesRef.current;
      const fpsSamples = samples.map((sample) => sample.fps);
      const latencySamples = samples.map((sample) => sample.latencyP95Ms);
      const encodeSamples = samples.map((sample) => sample.encodeP95Ms);
      const transportSamples = samples.map((sample) => sample.transportP95Ms);
      const decodeSamples = samples.map((sample) => sample.decodeP95Ms);
      const latencyNumericSamples = latencySamples.filter(
        (value): value is number => typeof value === "number"
      );
      const receiverStats = webRtcReceiverStatsRef.current;
      const presentationStats = webPresentationLatencyStatsRef.current;
      const droppedFrames =
        latestValue(samples.map((sample) => sample.droppedFrames)) ??
        receiverStats?.framesDropped ??
        0;
      const frameCount = Math.max(webVideoFrameCountRef.current, receiverStats?.framesDecoded ?? 0);

      return {
        run_id:
          currentRunId ??
          `web-preview-${startedAt.toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
        scenario_id: "browser-local-preview",
        run_mode: "manual",
        status,
        started_at: startedAt,
        finished_at: now,
        config_snapshot: testConfig,
        environment_snapshot:
          capabilities ?? {
            os_type: "browser",
            cpu_brand: "Browser",
            cpu_cores: navigator.hardwareConcurrency || 1,
            memory_gb: 0,
            gpu_info: "Browser Web View",
            available_encoders: [],
            available_decoders: [],
          },
        summary: {
          total_duration_ms: now - startedAt,
          capture_fps: average(fpsSamples) ?? webVideoFpsRef.current ?? undefined,
          encode_latency_p95: percentile(encodeSamples.filter((value): value is number => typeof value === "number"), 0.95) ?? undefined,
          transport_latency_p95: percentile(transportSamples.filter((value): value is number => typeof value === "number"), 0.95) ?? undefined,
          decode_latency_p95: percentile(decodeSamples.filter((value): value is number => typeof value === "number"), 0.95) ?? undefined,
          total_latency_p50:
            percentile(latencyNumericSamples, 0.5) ??
            presentationStats?.p50Ms ??
            undefined,
          total_latency_p95:
            percentile(latencyNumericSamples, 0.95) ??
            presentationStats?.p95Ms ??
            undefined,
          dropped_frames: droppedFrames,
          frame_count: frameCount,
          error_message: message,
          failure_reason:
            status === "failed"
              ? "runtime_failure"
              : status === "cancelled"
                ? "runtime_stopped"
                : undefined,
        },
      };
    },
    [
      capabilities,
      currentRunId,
      testConfig,
    ]
  );

  const clearBrowserPreviewAutoStopTimer = useCallback(() => {
    if (webPreviewAutoStopTimerRef.current !== null) {
      window.clearTimeout(webPreviewAutoStopTimerRef.current);
      webPreviewAutoStopTimerRef.current = null;
    }
  }, []);

  const completeBrowserPreviewRun = useCallback(
    async (status: TestRun["status"] = "completed", message?: string) => {
      clearBrowserPreviewAutoStopTimer();
      const completedRun = buildBrowserPreviewCompletedRun(status, message);
      closeWebPreviewPeer();
      await browserWebrtcPreviewStop(sessionId);
      setLastCompletedRun(completedRun);
      setCurrentRunId(null);
      setTestStatus(status === "failed" ? "failed" : "completed");
      setTestMessage(
        message ??
          (status === "cancelled"
            ? "测试已手动停止，报告已生成"
            : status === "failed"
              ? "测试失败，报告已生成"
              : "测试完成，报告已生成")
      );
      webPreviewRunStartedAtRef.current = null;
    },
    [
      buildBrowserPreviewCompletedRun,
      clearBrowserPreviewAutoStopTimer,
      closeWebPreviewPeer,
      sessionId,
    ]
  );

  useEffect(() => clearBrowserPreviewAutoStopTimer, [clearBrowserPreviewAutoStopTimer]);

  const handleStartTest = async () => {
    if (!isLocalPipelinePreview) {
      await handleStartRemoteReceiver();
      return;
    }

    if (localStartBlockReason) {
      setTestSettingsOpen(true);
      setTestStatus("idle");
      setTestMessage(localStartBlockReason);
      setLastError(localStartBlockReason);
      return;
    }

    setTestSettingsOpen(false);
    setLastError(null);
    setTestMessage("测试启动中");
    setTestStatus("starting");
    setCurrentRunId(null);
    setLastCompletedRun(null);
    setMetrics(null);
    setMatrixRunProgress(null);
    matrixStopRequestedRef.current = false;
    clearBrowserPreviewAutoStopTimer();
    webPreviewRunStartedAtRef.current = null;
    setWebPreviewMode("idle");
    setWebPreviewError(null);

    if (!isNative && !isTauriRuntime()) {
      if (localWebViewPlan.profile && localWebViewPlan.changed) {
        setCapture(localWebViewPlan.profile.capture);
        setEncoder(localWebViewPlan.profile.encoder);
        setDecoder(localWebViewPlan.profile.decoder);
        setTransport(localWebViewPlan.profile.transport);
        setFps(localWebViewPlan.profile.fps);
        setBitrate(localWebViewPlan.profile.bitrate);
      }
      const startedAt = Date.now();
      const runId = `web-preview-${startedAt.toString(36)}`;
      webPreviewRunStartedAtRef.current = startedAt;
      setCurrentRunId(runId);
      setTestStatus("running");
      setTestMessage(
        webPreviewEngine === "webcodecs"
          ? "网页 WebCodecs 本机采集运行中"
          : "网页 WebRTC 本机采集运行中"
      );
      if (durationMode !== "manual") {
        webPreviewAutoStopTimerRef.current = window.setTimeout(() => {
          void completeBrowserPreviewRun("completed");
        }, selectedDurationMs);
      }
      return;
    }

    await testHarnessStop();

    let configForRun = testConfig;
    let rendererTargetHwndForRun: string | null = null;
    let baseWebSelection: LocalTestSelection | undefined;
    if (!isNative && localWebViewPlan.profile) {
      if (localWebViewPlan.changed) {
        setCapture(localWebViewPlan.profile.capture);
        setEncoder(localWebViewPlan.profile.encoder);
        setDecoder(localWebViewPlan.profile.decoder);
        setTransport(localWebViewPlan.profile.transport);
        setFps(localWebViewPlan.profile.fps);
        setBitrate(localWebViewPlan.profile.bitrate);
        if (localWebViewPlan.message) setTestMessage(localWebViewPlan.message);
      }
      baseWebSelection = localWebViewPlan.profile;
      configForRun = buildTestConfig(null, baseWebSelection);
    }

    if (isNative && requiresEmbeddedNativeSurface) {
      const snapshot = await syncNativeSurface({ visible: true });
      const rendererTargetHwnd = snapshot?.hwnd ?? nativeSurface?.hwnd;
      if (!rendererTargetHwnd) {
        const message = `${nativeRenderLabel} render surface is not attached`;
        setTestStatus("failed");
        setTestMessage(message);
        setLastError(message);
        return;
      }
      const probe = await presentTestHarnessFrameOnNativeSurface();
      if (!probe.ok || !probe.value) {
        const message = probe.ok
          ? `${nativeRenderLabel} render probe did not present a frame`
          : probe.error.message;
        setTestStatus("failed");
        setTestMessage(message);
        setLastError(message);
        return;
      }
      rendererTargetHwndForRun = rendererTargetHwnd;
      configForRun = buildTestConfig(rendererTargetHwndForRun);
    } else if (isNative) {
      configForRun = buildTestConfig(null);
    }

    if (matrixModeEnabled) {
      const selections = buildLocalMatrixSelections();
      setMatrixRunProgress({ current: 0, total: selections.length, label: "准备矩阵" });
      let completed = 0;
      let failed = 0;
      let lastRun: TestRun | null = null;

      for (let index = 0; index < selections.length; index += 1) {
        if (matrixStopRequestedRef.current) break;
        const selection = {
          ...(baseWebSelection ?? {}),
          ...selections[index],
        };
        const label = [
          selection.capture ?? capture,
          selection.encoder ?? encoder,
          selection.resolution ?? resolution,
          `${selection.fps ?? fps} FPS`,
          `${selection.bitrate ?? bitrate} Mbps`,
        ].join(" / ");
        setMatrixRunProgress({ current: index + 1, total: selections.length, label });
        setTestMessage(`矩阵 ${index + 1}/${selections.length}: ${label}`);
        await testHarnessStop();
        const runResult = await testStartRun({
          scenarioId: "custom",
          config: buildTestConfig(rendererTargetHwndForRun, selection),
        });
        if (!runResult.ok) {
          failed += 1;
          setLastError(runResult.error.message);
          continue;
        }
        setCurrentRunId(runResult.value);
        const run = await waitForLocalRunFinished(
          runResult.value,
          (selectedDurationMs || 30_000) + 8_000
        );
        if (run) {
          lastRun = run;
          if (run.status === "completed") completed += 1;
          else failed += 1;
        } else if (!matrixStopRequestedRef.current) {
          failed += 1;
        }
      }

      await testHarnessStop();
      setCurrentRunId(null);
      setLastCompletedRun(lastRun);
      const stopped = matrixStopRequestedRef.current;
      setTestStatus(stopped ? "completed" : failed > 0 ? "failed" : "completed");
      setTestMessage(
        stopped
          ? `矩阵已手动停止: ${completed}/${selections.length} 完成`
          : `矩阵完成: ${completed}/${selections.length} 完成, ${failed} 失败`
      );
      setMatrixRunProgress(null);
      return;
    }

    const result = await testStartRun({
      scenarioId: "custom",
      config: configForRun,
    });

    if (result.ok) {
      setCurrentRunId(result.value);
      setTestStatus("running");
      setTestMessage("测试运行中");
      return;
    }

    setTestStatus("failed");
    setTestMessage(result.error.message);
    setLastError(result.error.message);
  };

  useEffect(() => {
    if (queryProfileKey === null || queryProfileAppliedKey === queryProfileKey) return;
    if (requestedResolution !== null && requestedResolution !== resolution) {
      setResolution(requestedResolution);
    }
    if (requestedFps !== null && requestedFps !== fps) {
      setFps(requestedFps);
    }
    if (requestedBitrate !== null && requestedBitrate !== bitrate) {
      setBitrate(requestedBitrate);
    }
    setQueryProfileAppliedKey(queryProfileKey);
  }, [
    bitrate,
    fps,
    queryProfileAppliedKey,
    queryProfileKey,
    requestedBitrate,
    requestedFps,
    requestedResolution,
    resolution,
  ]);

  useEffect(() => {
    if (searchParams.get("autostart") !== "1") return;
    const autostartKey = `${sessionId}:${searchParams.toString()}`;
    if (autoStartRequestedRef.current === autostartKey) return;
    if (!isLocalPipelinePreview || isTestBusy || localStartBlockReason) return;
    if (renderMode !== "web" || !capabilities) return;
    if (queryProfileNeedsApply) return;

    autoStartRequestedRef.current = autostartKey;
    void handleStartTest();
  }, [
    capabilities,
    isLocalPipelinePreview,
    isTestBusy,
    localStartBlockReason,
    queryProfileNeedsApply,
    renderMode,
    searchParams,
    sessionId,
  ]);

  const handleStopTest = async () => {
    if (!isLocalPipelinePreview) {
      setTestStatus("idle");
      setTestMessage("远程接收由 mrd-service 管理，未停止会话");
      return;
    }

    setTestStatus("stopping");
    matrixStopRequestedRef.current = true;
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    if (!isNative && !isTauriRuntime()) {
      await completeBrowserPreviewRun("cancelled");
      return;
    }
    const result = currentRunId
      ? await testStopRun(currentRunId)
      : await testHarnessStop();
    await testHarnessStop();

    if (result.ok) {
      setTestStatus("idle");
      setCurrentRunId(null);
      setMatrixRunProgress(null);
      setTestMessage("测试已停止");
      return;
    }

    setTestStatus("failed");
    setTestMessage(result.error.message);
    setLastError(result.error.message);
  };

  const handleClose = async () => {
    if (isTauriRuntime() && context?.label) {
      const result = await closeRemoteDisplayWindow(context.label);
      if (!result.ok) setLastError(result.error.message);
      return;
    }
    await withTauriWindow((appWindow) => appWindow.close());
  };

  const formatTime = (seconds: number) => {
    const minutes = Math.floor(seconds / 60);
    const rest = seconds % 60;
    return `${minutes.toString().padStart(2, "0")}:${rest
      .toString()
      .padStart(2, "0")}`;
  };

  const primaryActionLabel = isLocalPipelinePreview
    ? isTestBusy
      ? "停止测试"
      : "开始测试"
    : testStatus === "starting"
      ? "启动接收"
      : testStatus === "running"
        ? "刷新接收"
        : "开始接收";
  const statusLabel = isLocalPipelinePreview
    ? "connected"
    : sessionSnapshot?.state ?? "loading";
  const isBrowserBridgeRemote = !isTauriRuntime() && !isLocalPipelinePreview;
  const showDesktopWindowControls = isTauriRuntime();
  const webPreviewUsesVideo =
    isLocalPipelinePreview &&
    !isNative &&
    webPreviewEngine === "webrtc" &&
    (webPreviewMode === "connecting" || webPreviewMode === "webrtc");
  const webCodecsWorkerActive = webFrameTimingChannelState?.startsWith("webcodecs-worker") ?? false;
  const memoryPathLabel = usesNativeSharedTexture
    ? "D3D11 shared"
    : nativeRendererType === "macos"
      ? "Metal upload"
      : nativeRendererType === "linux"
        ? "Linux upload"
        : isLocalPipelinePreview && !isNative
          ? webPreviewEngine === "webcodecs"
            ? webCodecsMemoryPathLabelFromState(webFrameTimingChannelState)
            : "WebRTC MediaStream"
          : "CPU preview";
  const effectiveRenderLabel =
    isLocalPipelinePreview && !isNative
      ? webPreviewEngine === "webcodecs"
        ? webCodecsWorkerActive
          ? "WebCodecs worker"
          : "WebCodecs canvas"
        : webPreviewMode === "webrtc"
          ? "WebRTC video"
          : webPreviewMode === "connecting"
            ? "WebRTC connecting"
            : webPreviewMode === "failed"
              ? "WebRTC failed"
              : "WebRTC video"
      : renderModeLabel;
  const renderSwitchLockedTitle = localRenderSwitchLocked
    ? "请先停止测试再切换渲染模式"
    : undefined;
  const configChangeLockedTitle = localRenderSwitchLocked
    ? "当前测试运行中；停止后修改才会影响下一次启动"
    : undefined;
  const settingsFooterMessage = localStartBlockReason
    ? localStartBlockReason
    : isLocalPipelinePreview
      ? metrics
        ? `${metrics.capture_fps.toFixed(1)} FPS / ${metrics.frame_count} frames`
        : "等待开始测试"
      : mediaProfileNegotiation
        ? `远端 ${mediaProfileNegotiation.selected.width}x${mediaProfileNegotiation.selected.height}@${mediaProfileNegotiation.selected.fps} / ${mediaProfileNegotiation.selected.bitrate_mbps} Mbps`
        : "远程参数将通过协商层下发";
  const lastRunSummary = lastCompletedRun?.summary;
  const lastRunFps =
    lastRunSummary?.capture_fps ?? metrics?.capture_fps ?? webVideoFps ?? null;
  const lastRunLatencyP95 =
    lastRunSummary?.total_latency_p95 ??
    metrics?.total_latency_p95_ms ??
    webPresentationLatencyStats?.p95Ms ??
    null;
  const lastRunDropped = lastRunSummary?.dropped_frames ?? metrics?.dropped_frames ?? null;
  const reportConfig = lastCompletedRun?.config_snapshot ?? null;
  const reportEnvironment = lastCompletedRun?.environment_snapshot ?? capabilities ?? null;
  const reportDurationMs =
    lastRunSummary?.total_duration_ms ??
    (lastCompletedRun?.finished_at && lastCompletedRun.started_at
      ? lastCompletedRun.finished_at - lastCompletedRun.started_at
      : null);
  const reportFrameCount = lastRunSummary?.frame_count ?? metrics?.frame_count ?? null;
  const reportDropRatio =
    typeof lastRunDropped === "number" && typeof reportFrameCount === "number" && reportFrameCount > 0
      ? lastRunDropped / reportFrameCount * 100
      : diagnosticsDropRatio;
  const reportFpsAvg =
    lastRunSummary?.capture_fps ?? average(diagnosticsSamples.map((sample) => sample.fps));
  const reportFpsMin = maxValue(
    diagnosticsSamples.map((sample) =>
      typeof sample.fps === "number" ? -sample.fps : null
    )
  );
  const reportFpsMinValue = typeof reportFpsMin === "number" ? -reportFpsMin : null;
  const reportLatencyP50 =
    lastRunSummary?.total_latency_p50 ??
    webPresentationLatencyStats?.p50Ms ??
    percentile(
      diagnosticsSamples
        .map((sample) => sample.latencyP95Ms)
        .filter((value): value is number => typeof value === "number"),
      0.5
    );
  const reportLatencyP95 = lastRunLatencyP95;
  const reportServiceCpuP95 = percentile(
    diagnosticsSamples
      .map((sample) => sample.serviceCpuPercent)
      .filter((value): value is number => typeof value === "number"),
    0.95
  );
  const reportServiceMemoryPeak = maxValue(
    diagnosticsSamples.map((sample) => sample.serviceMemoryMb)
  );
  const reportServiceGpuP95 = percentile(
    diagnosticsSamples
      .map((sample) => sample.serviceGpuPercent)
      .filter((value): value is number => typeof value === "number"),
    0.95
  );
  const reportServiceNetworkPeak = maxValue(
    diagnosticsSamples.map((sample) =>
      sumNullable(sample.serviceNetworkRxMbps, sample.serviceNetworkTxMbps)
    )
  );
  const reportDisplayCpuP95 = percentile(
    diagnosticsSamples
      .map((sample) => sample.displayCpuPercent)
      .filter((value): value is number => typeof value === "number"),
    0.95
  );
  const reportDisplayMemoryPeak = maxValue(
    diagnosticsSamples.map((sample) => sample.displayMemoryMb)
  );
  const reportDisplayGpuP95 = percentile(
    diagnosticsSamples
      .map((sample) => sample.displayGpuPercent)
      .filter((value): value is number => typeof value === "number"),
    0.95
  );
  const reportDisplayNetworkPeak = maxValue(
    diagnosticsSamples.map((sample) =>
      sumNullable(sample.displayNetworkRxMbps, sample.displayNetworkTxMbps)
    )
  );
  const reportVisible =
    isLocalPipelinePreview &&
    lastCompletedRun &&
    !isTestBusy &&
    ["completed", "failed", "cancelled"].includes(lastCompletedRun.status);
  const primaryActionBlocked = Boolean(!isTestBusy && localStartBlockReason);
  const renderCaptureSourceCards = (closeAfterSelect = false) => (
    <div className="grid max-h-80 gap-3 overflow-y-auto pr-1 sm:grid-cols-2 lg:grid-cols-3">
      {captureSources.map((source) => {
        const selected = captureSourceSelection?.source.id === source.id;
        return (
          <button
            key={source.id}
            type="button"
            aria-label={`选择 ${source.title}`}
            onClick={() => {
              void handleSelectRemoteCaptureSource(source);
              if (closeAfterSelect) setCaptureSourcePickerOpen(false);
            }}
            className={[
              "overflow-hidden rounded-lg border bg-black/25 text-left transition",
              selected
                ? "border-emerald-300/70 shadow-[0_0_0_1px_rgba(110,231,183,0.35)]"
                : "border-white/10 hover:border-cyan-300/60 hover:bg-cyan-500/10",
            ].join(" ")}
          >
            <div className="aspect-video bg-slate-950">
              {source.preview_data_url ? (
                <img
                  src={source.preview_data_url}
                  alt=""
                  className="h-full w-full object-cover"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-[10px] text-slate-600">
                  无预览
                </div>
              )}
            </div>
            <div className="space-y-1 px-3 py-2">
              <div className="flex items-center gap-2">
                <span className="rounded bg-white/8 px-1.5 py-0.5 text-[9px] text-cyan-100">
                  {captureSourceKindLabel(source.source_kind)}
                </span>
                <div className="min-w-0 truncate text-xs font-medium text-slate-100">
                  {source.title}
                </div>
              </div>
              <div className="flex items-center justify-between gap-2 text-[10px] text-slate-500">
                <span className="truncate">
                  {source.app_name ?? source.class_name ?? source.platform}
                </span>
                <span className="shrink-0">
                  {source.width}x{source.height}
                </span>
              </div>
              {selected && (
                <div className="text-[10px] font-medium text-emerald-300">
                  已选中
                </div>
              )}
            </div>
          </button>
        );
      })}
    </div>
  );

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-[#080a0f] text-slate-100">
      <div
        className="flex h-14 shrink-0 select-none items-center border-b border-white/10 bg-[#111827]"
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        onMouseDown={handleDragStart}
        onDoubleClick={(event) => {
          if ((event.target as HTMLElement | null)?.closest(noDragSelector)) return;
          void handleToggleMaximize();
        }}
      >
        <div
          className="flex min-w-0 w-[310px] shrink-0 items-center gap-3 px-3"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <button
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-slate-400 hover:bg-white/10 hover:text-white"
            title="Back"
            onClick={() => history.back()}
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-cyan-500/15 text-cyan-300">
            <Monitor className="h-4 w-4" />
          </div>
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold">{title}</div>
            <div className="truncate text-[11px] text-slate-400">
              {sessionId} / {activeSurfaceId}
            </div>
          </div>
        </div>

        <div
          className="hidden min-w-0 flex-1 items-center gap-1 overflow-x-auto px-2 lg:flex"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          {isLocalPipelinePreview ? (
            <button
              className="inline-flex h-9 items-center gap-2 rounded-md border border-white/10 bg-black/20 px-3 text-[11px] font-medium text-slate-200 hover:bg-white/10"
              onClick={openTestSettings}
            >
              <SlidersHorizontal className="h-3.5 w-3.5 text-cyan-300" />
              测试配置
            </button>
          ) : (
            <div className="inline-flex h-9 items-center gap-2 rounded-md border border-cyan-400/20 bg-cyan-400/10 px-3 text-[11px] font-medium text-cyan-100">
              <Network className="h-3.5 w-3.5 text-cyan-300" />
              LAN 远程会话
            </div>
          )}
        </div>

        <div
          className="flex shrink-0 items-center gap-2 px-3"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <div
            ref={diagnosticsPopoverRef}
            className="relative hidden md:block"
            onMouseEnter={() => setDiagnosticsOpen(true)}
            onMouseLeave={() => {
              if (!diagnosticsPinned) setDiagnosticsOpen(false);
            }}
            onBlur={(event) => {
              if (diagnosticsPinned) return;
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                setDiagnosticsOpen(false);
              }
            }}
          >
            <button
              type="button"
              aria-label="连接诊断"
              aria-expanded={diagnosticsVisible}
              className="inline-flex items-center gap-2 rounded-md border border-emerald-400/20 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-50 hover:bg-emerald-500/16"
              onClick={() => {
                setDiagnosticsPinned(true);
                setDiagnosticsOpen(true);
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  setDiagnosticsOpen(false);
                  setDiagnosticsPinned(false);
                }
              }}
            >
              <span className="h-2 w-2 rounded-full bg-emerald-300 shadow-[0_0_8px_rgba(52,211,153,0.9)]" />
              <Network className="h-3.5 w-3.5 text-emerald-300" />
              <span>{remoteQuality}</span>
              <span className="text-emerald-200/60">/</span>
              <span>{formatFps(diagnosticsVisualFps)}</span>
              <span className="text-emerald-200/60">/</span>
              <span>{formatMs(diagnosticsLatencyP95Ms)}</span>
            </button>
            {diagnosticsVisible ? (
              <div
                className="fixed right-4 top-16 z-[1000] max-h-[calc(100vh-5rem)] w-[min(420px,calc(100vw-2rem))] overflow-y-auto rounded-md border border-emerald-400/20 bg-[#03140f]/95 p-4 text-[11px] text-emerald-50 shadow-2xl shadow-emerald-950/60 backdrop-blur"
                data-testid="remote-diagnostics-popover"
              >
                <div className="mb-3 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="h-2 w-2 rounded-full bg-emerald-300" />
                    <div className="text-sm font-semibold text-white">远程诊断</div>
                  </div>
                  <div className="text-emerald-200/70">{formatTime(elapsed)}</div>
                </div>
                <div className="grid gap-4">
                  <div className="grid grid-cols-2 gap-2">
                    <div className="rounded-md border border-emerald-400/10 bg-emerald-500/10 p-2">
                      <div className="flex items-center gap-1.5 text-emerald-200/65">
                        <Activity className="h-3.5 w-3.5" />
                        {webVideoFps ? "Web 视频" : "FPS"}
                      </div>
                      <div className="mt-1 text-lg font-semibold text-white">
                        {formatFps(diagnosticsVisualFps)}
                      </div>
                      {webPaintFps ? (
                        <div className="mt-0.5 text-[10px] text-emerald-200/55">
                          回调 {formatHz(webPaintFps)}
                        </div>
                      ) : null}
                    </div>
                    <div className="rounded-md border border-emerald-400/10 bg-emerald-500/10 p-2">
                      <div className="flex items-center gap-1.5 text-emerald-200/65">
                        <Gauge className="h-3.5 w-3.5" />
                        {diagnosticsLatencyLabel}
                      </div>
                      <div className="mt-1 text-lg font-semibold text-white">
                        {formatMs(diagnosticsLatencyP95Ms)}
                      </div>
                    </div>
                    <div className="rounded-md border border-emerald-400/10 bg-emerald-500/10 p-2">
                      <div className="text-emerald-200/65">码率 / 队列</div>
                      <div className="mt-1 text-sm font-semibold text-white">
                        {formatMbps(diagnosticsBitrateMbps)} / {formatCount(diagnosticsQueueDepth)}
                      </div>
                    </div>
                    <div className="rounded-md border border-emerald-400/10 bg-emerald-500/10 p-2">
                      <div className="text-emerald-200/65">掉帧 / 丢弃率</div>
                      <div className="mt-1 text-sm font-semibold text-white">
                        {formatCount(diagnosticsDroppedFrames)} / {formatPercent(diagnosticsDropRatio)}
                      </div>
                    </div>
                  </div>
                  <section className="rounded-md border border-emerald-400/10 bg-emerald-950/20 p-3">
                    <div className="mb-3 flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2 text-[12px] font-semibold text-emerald-100">
                        <BarChart3 className="h-3.5 w-3.5 text-emerald-300" />
                        性能曲线
                      </div>
                      <div className="text-[10px] text-emerald-200/55">
                        最近 {Math.min(diagnosticsSamples.length, DIAGNOSTICS_SAMPLE_LIMIT)} 秒
                      </div>
                    </div>
                    <div className="grid gap-3">
                      <DiagnosticMetricTile
                        title="FPS"
                        value={formatFps(diagnosticsVisualFps)}
                        subtitle={
                          webPaintFps
                            ? `目标 ${diagnosticsTargetFps} FPS / 回调 ${formatHz(webPaintFps)}`
                            : `目标 ${diagnosticsTargetFps} FPS`
                        }
                        samples={diagnosticsSamples}
                        sampleValue={(sample) => sample.fps}
                        colorClass="text-emerald-300"
                      />
                      <DiagnosticMetricTile
                        title={diagnosticsLatencyLabel}
                        value={formatMs(diagnosticsLatencyP95Ms)}
                        subtitle={
                          webPresentationLatencyStats
                            ? `p50 ${formatMs(webPresentationLatencyStats.p50Ms)} / 最新 ${formatMs(
                                webPresentationLatencyStats.latestMs
                              )}`
                            : "阶段指标优先，Web 路径回退帧间隔"
                        }
                        samples={diagnosticsSamples}
                        sampleValue={(sample) => sample.latencyP95Ms}
                        colorClass="text-cyan-300"
                      />
                      <DiagnosticMetricTile
                        title="码率"
                        value={formatMbps(diagnosticsBitrateMbps)}
                        subtitle="接收 probe / active profile"
                        samples={diagnosticsSamples}
                        sampleValue={(sample) => sample.bitrateMbps}
                        colorClass="text-lime-300"
                      />
                    </div>
                  </section>
                  <section className="rounded-md border border-emerald-400/10 bg-emerald-950/20 p-3">
                    <div className="mb-3 flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2 text-[12px] font-semibold text-emerald-100">
                        <Activity className="h-3.5 w-3.5 text-emerald-300" />
                        资源占用曲线
                      </div>
                      <div className="text-[10px] text-emerald-200/55">
                        mrd-service / 接收显示
                      </div>
                    </div>
                    <div className="grid gap-3">
                      <DiagnosticMetricTile
                        title="mrd-service CPU / 内存"
                        value={`${formatOptionalPercent(diagnosticsServiceCpuPercent)} / ${formatOptionalPercent(
                          diagnosticsServiceMemoryPercent
                        )}`}
                        subtitle={`${dash(serviceResourceSnapshot?.target_name)} PID ${dash(
                          serviceResourceSnapshot?.target_pid
                        )} / ${formatMb(diagnosticsServiceMemoryMb)}`}
                        samples={diagnosticsSamples}
                        sampleValue={(sample) => sample.serviceCpuPercent}
                        colorClass="text-emerald-300"
                      />
                      <DiagnosticMetricTile
                        title="mrd-service GPU / 网络"
                        value={`${formatOptionalPercent(diagnosticsServiceGpuPercent)} / ${formatMbps(
                          sumNullable(
                            diagnosticsServiceNetworkRxMbps,
                            diagnosticsServiceNetworkTxMbps
                          )
                        )}`}
                        subtitle={`${resourceGpuSubtitle(serviceResourceSnapshot)} / ${resourceNetworkSubtitle(
                          serviceResourceSnapshot
                        )}`}
                        samples={diagnosticsSamples}
                        sampleValue={(sample) =>
                          typeof sample.serviceGpuPercent === "number"
                            ? sample.serviceGpuPercent
                            : sumNullable(sample.serviceNetworkRxMbps, sample.serviceNetworkTxMbps)
                        }
                        colorClass="text-lime-300"
                      />
                      <DiagnosticMetricTile
                        title="接收显示 CPU / 内存"
                        value={`${formatOptionalPercent(diagnosticsDisplayCpuPercent)} / ${formatOptionalPercent(
                          diagnosticsDisplayMemoryPercent
                        )}`}
                        subtitle={`${dash(displayResourceSnapshot?.target_name)} / ${formatMb(
                          diagnosticsDisplayMemoryMb
                        )}`}
                        samples={diagnosticsSamples}
                        sampleValue={(sample) => sample.displayCpuPercent ?? sample.displayMemoryPercent}
                        colorClass="text-cyan-300"
                      />
                      <DiagnosticMetricTile
                        title="接收显示 GPU / 网络"
                        value={`${formatOptionalPercent(diagnosticsDisplayGpuPercent)} / ${formatMbps(
                          sumNullable(
                            diagnosticsDisplayNetworkRxMbps,
                            diagnosticsDisplayNetworkTxMbps
                          )
                        )}`}
                        subtitle={`${resourceGpuSubtitle(displayResourceSnapshot)} / ${resourceNetworkSubtitle(
                          displayResourceSnapshot
                        )}`}
                        samples={diagnosticsSamples}
                        sampleValue={(sample) =>
                          typeof sample.displayGpuPercent === "number"
                            ? sample.displayGpuPercent
                            : sumNullable(sample.displayNetworkRxMbps, sample.displayNetworkTxMbps)
                        }
                        colorClass="text-violet-300"
                      />
                    </div>
                  </section>
                  <DiagnosticStageList rows={diagnosticsStageRows} />
                  {webRtcReceiverStats ? (
                    <DiagnosticGroup
                      title="WebRTC 接收"
                      rows={[
                        ["视频呈现 FPS", formatFps(webVideoFps)],
                        ["回调频率", formatHz(webPaintFps)],
                        ["解码 FPS", formatFps(webRtcReceiverStats.decodedFps)],
                        ["解码帧", formatCount(webRtcReceiverStats.framesDecoded)],
                        ["掉帧", formatCount(webRtcReceiverStats.framesDropped)],
                        ["网络抖动", formatMs(webRtcReceiverStats.jitterMs)],
                        ["JitterBuffer", formatMs(webRtcReceiverStats.jitterBufferDelayAvgMs)],
                        ["解码均值", formatMs(webRtcReceiverStats.decodeAvgMs)],
                        ["处理均值", formatMs(webRtcReceiverStats.processingDelayAvgMs)],
                        ["渲染间隔", formatMs(webRtcReceiverStats.interFrameDelayAvgMs)],
                        [`${diagnosticsE2eLabelPrefix} p50`, formatMs(webPresentationLatencyStats?.p50Ms)],
                        [`${diagnosticsE2eLabelPrefix} p95`, formatMs(webPresentationLatencyStats?.p95Ms)],
                        [`${diagnosticsE2eLabelPrefix} max`, formatMs(webPresentationLatencyStats?.maxMs)],
                        ["E2E 样本", formatCount(webPresentationLatencyStats?.samples)],
                        [
                          "E2E metadata",
                          `${formatCount(webFrameTimingMetadataCount)} / ${dash(
                            webFrameTimingChannelState
                          )} / age ${formatMs(webFrameTimingMetadataAgeMs)}`,
                        ],
                        ["冻结次数", formatCount(webRtcReceiverStats.freezeCount)],
                        ["丢包", formatCount(webRtcReceiverStats.packetsLost)],
                      ]}
                    />
                  ) : null}
                  {diagnosticsBrowserPaintLimited ? (
                    <div className="rounded-md border border-amber-300/20 bg-amber-400/10 p-2 text-amber-100">
                      浏览器视频呈现低于目标，但 WebRTC 解码仍在高帧运行：当前 Web View
                      合成层可能限制显示节奏，建议用 Chrome 或原生渲染链路复测。
                    </div>
                  ) : null}
                  <DiagnosticGroup
                    title="连接"
                    rows={[
                      ["连接时间", formatTime(elapsed)],
                      ["连接质量", remoteQuality],
                      [
                        "帧率",
                        webVideoFps || webPaintFps
                          ? `视频 ${formatFps(diagnosticsVisualFps)} / 回调 ${formatHz(webPaintFps)} / 解码 ${formatFps(webRtcReceiverStats?.decodedFps ?? diagnosticsFps)}`
                          : formatFps(diagnosticsVisualFps),
                      ],
                      ["延迟", `${diagnosticsLatencyLabel}: ${formatMs(diagnosticsLatencyP95Ms)}`],
                      ["丢包/掉帧", `${formatPercent(diagnosticsDropRatio)} / ${formatCount(diagnosticsDroppedFrames)}`],
                      [
                        "渲染丢帧细分",
                        `队列 ${formatCount(diagnosticsRenderQueueReplacements)} / 锁 ${formatCount(
                          diagnosticsRenderLockDrops
                        )} / Present ${formatCount(diagnosticsRenderPresentSkips)}`,
                      ],
                      ["队列深度", formatCount(diagnosticsQueueDepth)],
                      ["码率", formatMbps(diagnosticsBitrateMbps)],
                    ]}
                  />
                  <DiagnosticGroup
                    title="画面"
                    rows={[
                      ["画面质量", remoteQuality === "流畅" ? "高清" : remoteQuality],
                      ["画面呈现", "自动缩放"],
                      ["编解码器", diagnosticsCodec],
                      ["位深", diagnosticsBitDepth ? `${diagnosticsBitDepth}-bit` : "-"],
                      ["色度采样", diagnosticsChroma],
                      ["像素格式", diagnosticsPixelFormat],
                      ["采集方式", captureMethodLabel(captureSourceSelection, capture)],
                      ["HDR", diagnosticsHdrEnabled ? "已开启" : "已关闭"],
                      ["分辨率/缩放", `${diagnosticsResolution} / ${diagnosticsTarget}`],
                    ]}
                  />
                  <DiagnosticGroup
                    title="双端"
                    rows={[
                      ["本机 CPU", dash(capabilities?.cpu_brand)],
                      ["本机 GPU", dash(capabilities?.gpu_info)],
                      ["本机内存", capabilities?.memory_gb ? `${capabilities.memory_gb}G` : "-"],
                      ["本机系统", dash(capabilities?.os_type)],
                      ["mrd-service 资源", `${formatOptionalPercent(diagnosticsServiceCpuPercent)} CPU / ${formatMb(diagnosticsServiceMemoryMb)}`],
                      ["接收显示资源", `${formatOptionalPercent(diagnosticsDisplayCpuPercent)} CPU / ${formatMb(diagnosticsDisplayMemoryMb)}`],
                      ["远端 Device", dash(sessionSnapshot?.session_id)],
                      ["Build ID", dash(context?.label)],
                      ["解码器", decoderLabel(mediaPipelineSnapshot?.active_decoder ?? decoder)],
                      ["渲染器", dash(mediaPipelineSnapshot?.active_renderer ?? renderModeLabel)],
                    ]}
                  />
                  {mediaPipelineSnapshot?.codec_fallback_reason ? (
                    <div className="rounded-md border border-amber-300/20 bg-amber-400/10 p-2 text-amber-100">
                      codec fallback: {mediaPipelineSnapshot.codec_fallback_reason}
                    </div>
                  ) : null}
                </div>
              </div>
            ) : null}
          </div>
          {showDesktopWindowControls ? (
            <>
              <button
                onClick={() => void withTauriWindow((appWindow) => appWindow.minimize())}
                className="inline-flex h-8 w-9 items-center justify-center rounded-sm text-slate-400 hover:bg-white/10 hover:text-white"
                title="Minimize"
              >
                <Minimize className="h-4 w-4" />
              </button>
              <button
                onClick={() => void handleToggleMaximize()}
                className="inline-flex h-8 w-9 items-center justify-center rounded-sm text-slate-400 hover:bg-white/10 hover:text-white"
                title={isMaximized ? "Restore" : "Maximize"}
              >
                {isMaximized ? <Square className="h-3 w-3" /> : <Maximize2 className="h-3.5 w-3.5" />}
              </button>
              <button
                onClick={() => void handleClose()}
                className="inline-flex h-8 w-9 items-center justify-center rounded-sm text-slate-400 hover:bg-red-500 hover:text-white"
                title="Close"
              >
                <X className="h-4 w-4" />
              </button>
            </>
          ) : null}
        </div>
      </div>

      {testSettingsOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 px-4"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          data-no-drag="true"
        >
          <div className="flex max-h-[calc(100vh-2rem)] w-full max-w-5xl flex-col rounded-lg border border-white/10 bg-[#0f1724] shadow-2xl">
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <div>
                <div className="text-sm font-semibold text-slate-100">测试配置</div>
                <div className="mt-1 text-[11px] text-slate-500">
                  渲染路径、采集/编码参数和浏览器显示路径统一在这里切换。
                </div>
              </div>
              <button
                className="inline-flex h-8 w-8 items-center justify-center rounded-md text-slate-400 hover:bg-white/10 hover:text-white"
                onClick={closeTestSettings}
                title="Close"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="border-b border-white/10 px-4 py-3">
              <div className="mb-2 flex items-center justify-between gap-3">
                <div>
                  <div className="text-xs font-semibold uppercase tracking-[0.18em] text-slate-300">
                    渲染路径
                  </div>
                  <div className="mt-1 text-[11px] text-slate-500">
                    WebRTC / WebCodecs 用浏览器解码绘制；native 路径由独立原生 surface 承载。
                  </div>
                </div>
                <div className="text-[11px] text-slate-500">当前: {effectiveRenderLabel}</div>
              </div>
              <div className="flex flex-wrap gap-2">
                {isLocalPipelinePreview ? (
                  <>
                    <button
                      className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                        renderMode === "web" && webPreviewEngine === "webrtc"
                          ? "border-cyan-300/50 bg-cyan-500/20 text-cyan-100"
                          : localRenderSwitchLocked
                            ? "cursor-not-allowed border-white/10 text-slate-600"
                            : "border-white/10 text-slate-300 hover:bg-white/10"
                      }`}
                      onClick={switchToWebRtcRender}
                      disabled={localRenderSwitchLocked}
                      title={renderSwitchLockedTitle ?? "浏览器 WebRTC video 显示路径"}
                    >
                      WebRTC video
                    </button>
                    <button
                      className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                        renderMode === "web" && webPreviewEngine === "webcodecs"
                          ? "border-violet-300/50 bg-violet-500/20 text-violet-100"
                          : localRenderSwitchLocked || browserWebCodecsUltraBlockReason
                            ? "cursor-not-allowed border-white/10 text-slate-600"
                            : "border-white/10 text-slate-300 hover:bg-white/10"
                      }`}
                      onClick={switchToWebCodecsRender}
                      disabled={localRenderSwitchLocked || Boolean(browserWebCodecsUltraBlockReason)}
                      title={
                        renderSwitchLockedTitle ??
                        browserWebCodecsUltraBlockReason ??
                        "浏览器 WebCodecs + WebGL2 优先显示路径"
                      }
                    >
                      WebCodecs WebGL2
                    </button>
                  </>
                ) : (
                  <button
                    className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                      renderMode === "web"
                        ? "border-cyan-300/50 bg-cyan-500/20 text-cyan-100"
                        : localRenderSwitchLocked
                          ? "cursor-not-allowed border-white/10 text-slate-600"
                          : "border-white/10 text-slate-300 hover:bg-white/10"
                    }`}
                    onClick={switchToWebRtcRender}
                    disabled={localRenderSwitchLocked}
                    title={renderSwitchLockedTitle ?? "浏览器 Web View 显示路径"}
                  >
                    Web View
                  </button>
                )}
                <button
                  className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                    renderMode === nativeRenderMode
                      ? "border-cyan-300/50 bg-cyan-500/20 text-cyan-100"
                      : nativeRendererAvailableForHost && !localRenderSwitchLocked
                        ? "border-white/10 text-slate-300 hover:bg-white/10"
                        : "cursor-not-allowed border-white/10 text-slate-600"
                  }`}
                  onClick={switchToNativeRender}
                  disabled={!nativeRendererAvailableForHost || localRenderSwitchLocked}
                  title={
                    localRenderSwitchLocked
                      ? renderSwitchLockedTitle
                      : nativeRendererAvailableForHost
                        ? "原生窗口渲染路径"
                        : `${nativeRenderLabel} 当前不可用`
                  }
                >
                  {nativeRenderLabel}
                </button>
                <button
                  className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                    renderMode === "d3d12_native"
                      ? "border-cyan-300/50 bg-cyan-500/20 text-cyan-100"
                      : d3d12RendererAvailable && !localRenderSwitchLocked
                        ? "border-white/10 text-slate-300 hover:bg-white/10"
                        : "cursor-not-allowed border-white/10 text-slate-600"
                  }`}
                  onClick={switchToD3d12Render}
                  disabled={!d3d12RendererAvailable || localRenderSwitchLocked}
                  title={
                    localRenderSwitchLocked
                      ? renderSwitchLockedTitle
                      : d3d12RendererAvailable
                        ? "D3D12 native 渲染路径"
                        : d3d12UnavailableTitle
                  }
                >
                  DX12 native
                </button>
              </div>
              {isLocalPipelinePreview && renderMode === "web" ? (
                <div className="mt-3 rounded-md border border-cyan-400/15 bg-cyan-500/10 px-3 py-2 text-[11px] text-cyan-100/85">
                  当前网页渲染路径会锁定浏览器侧解码和传输语义：
                  {webPreviewEngine === "webcodecs"
                    ? " DEC=Browser WebCodecs，NET=WebSocket AU。"
                    : " DEC=Browser video decode，NET=WebRTC RTP。"}
                  编码器只显示已接入浏览器预览的 H.264 路径；HEVC/AV1 属于 native/后续浏览器媒体链路。
                </div>
              ) : null}
            </div>

            <div className="min-h-0 space-y-3 overflow-y-auto px-4 py-4">
              <div className="grid gap-3 lg:grid-cols-[1.2fr_1fr]">
                <div className="rounded-lg border border-white/10 bg-black/18 p-3">
                  <div className="mb-2 flex items-center justify-between gap-2">
                    <div>
                      <div className="text-[10px] font-semibold uppercase tracking-normal text-slate-400">
                        测试模式
                      </div>
                      <div className="mt-1 text-[11px] text-slate-500">
                        单次使用当前组合；多选矩阵按所选维度顺序执行，最多 36 条。
                      </div>
                    </div>
                    <button
                      type="button"
                      className={`rounded-md border px-3 py-1.5 text-[11px] font-medium ${
                        matrixModeEnabled
                          ? "border-violet-300/50 bg-violet-500/20 text-violet-100"
                          : "border-white/10 text-slate-200 hover:bg-white/10"
                      } ${localRenderSwitchLocked ? "cursor-not-allowed opacity-60" : ""}`}
                      disabled={localRenderSwitchLocked}
                      title={configChangeLockedTitle ?? "切换单次/多选矩阵测试模式"}
                      onClick={() => setMatrixModeEnabled((value) => !value)}
                    >
                      {matrixModeEnabled ? `多选矩阵 (${matrixSelectionCount})` : "单次测试"}
                    </button>
                  </div>
                  <MultiTileOptionGroup
                    label="MATRIX"
                    values={matrixDimensions}
                    options={matrixDimensionOptions}
                    onToggle={toggleMatrixDimension}
                    disabled={localRenderSwitchLocked || !matrixModeEnabled}
                    title={
                      !matrixModeEnabled
                        ? "开启多选矩阵后选择参与展开的维度"
                        : configChangeLockedTitle
                    }
                  />
                </div>
                <TileOptionGroup
                  label="DURATION"
                  value={durationMode}
                  options={durationTileOptions}
                  onChange={setDurationMode}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle ?? "测试时长；手动停止使用长时运行并由停止按钮结束"}
                />
              </div>

              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                <TileOptionGroup
                  label="CAP"
                  value={capture}
                  options={captureTileOptions}
                  onChange={setCapture}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
                <TileOptionGroup
                  label="ENC"
                  value={encoder}
                  options={encoderTileOptions}
                  onChange={setEncoder}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle ?? browserEncoderConstraintTitle}
                />
              {isLocalPipelinePreview && renderMode === "web" ? (
                <ReadonlyTitleValue
                  label="DEC"
                  value={displayDecoderLabel}
                  title="网页显示路径由浏览器负责解码，不使用矩阵里的本机 decoder 字段。"
                />
              ) : (
                <TileOptionGroup
                  label="DEC"
                  value={decoder}
                  options={decoderTileOptions}
                  onChange={setDecoder}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
              )}
              {isLocalPipelinePreview && renderMode === "web" ? (
                <ReadonlyTitleValue
                  label="NET"
                  value={webPreviewEngine === "webcodecs" ? "WebSocket AU" : "WebRTC RTP"}
                  title={
                    webPreviewEngine === "webcodecs"
                      ? "WebCodecs 使用本机 mrd-service WebSocket 传输 H.264 access unit，不走 WebRTC RTP。"
                      : "WebRTC video 使用浏览器 MediaStream / RTP 接收，不使用矩阵里的 Loopback/QUIC 传输。"
                  }
                />
              ) : (
                <TileOptionGroup
                  label="NET"
                  value={transport}
                  options={transportOptions}
                  onChange={setTransport}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
              )}
                <TileOptionGroup
                  label="SIZE"
                  value={resolution}
                  options={resolutionTileOptions}
                  onChange={setResolution}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
                <TileOptionGroup
                  label="FPS"
                  value={fps}
                  options={fpsTileOptions}
                  onChange={setFps}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
                <TileOptionGroup
                  label="BR"
                  value={bitrate}
                  options={bitrateTileOptions}
                  onChange={setBitrate}
                  disabled={localRenderSwitchLocked}
                  title={configChangeLockedTitle}
                />
              </div>
            </div>

            {!isLocalPipelinePreview && (
              <div className="border-t border-white/10 px-4 py-4">
                <div className="mb-3 flex items-center justify-between gap-3">
                  <div>
                    <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.18em] text-slate-300">
                      <PanelTop className="h-3.5 w-3.5 text-cyan-300" />
                      远端捕获源
                    </div>
                    <div className="mt-1 text-[11px] text-slate-500">
                      默认优先全屏 shared copy，可切换全屏 copy 或单窗口源。
                    </div>
                  </div>
                  <div className="flex flex-wrap items-center justify-end gap-2">
                    <TitleSelect
                      label="PICK"
                      value={captureSourcePickerMode}
                      options={captureSourcePickerOptions}
                      onChange={setCaptureSourcePickerMode}
                    />
                    <button
                      className="inline-flex items-center gap-2 rounded-md border border-cyan-400/30 px-3 py-1.5 text-[11px] font-medium text-cyan-100 hover:bg-cyan-500/15 disabled:opacity-50"
                      onClick={() => void handleRefreshRemoteCaptureSources()}
                      disabled={captureSourcesLoading}
                    >
                      {captureSourcesLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                      刷新捕获源
                    </button>
                    {captureSourcePickerMode === "modal" && (
                      <button
                        className="rounded-md border border-white/15 px-3 py-1.5 text-[11px] font-medium text-slate-200 hover:bg-white/10"
                        onClick={() => setCaptureSourcePickerOpen(true)}
                      >
                        打开捕获源弹窗
                      </button>
                    )}
                  </div>
                </div>

                {captureSources.length === 0 ? (
                  <div className="rounded-lg border border-white/10 bg-black/20 px-3 py-4 text-center text-[11px] text-slate-500">
                    暂无捕获源。点击刷新从远端设备获取当前全屏/窗口列表。
                  </div>
                ) : captureSourcePickerMode === "dropdown" ? (
                  <label className="block">
                    <span className="sr-only">远端捕获源下拉</span>
                    <select
                      aria-label="远端捕获源下拉"
                      className="w-full rounded-lg border border-white/10 bg-black/25 px-3 py-2 text-xs text-slate-100 outline-none"
                      value={captureSourceSelection?.source.id ?? ""}
                      onChange={(event) => {
                        const source = captureSources.find(
                          (candidate) => candidate.id === event.target.value
                        );
                        if (source) void handleSelectRemoteCaptureSource(source);
                      }}
                    >
                      <option value="" className="bg-[#111827] text-slate-100">
                        选择远端捕获源
                      </option>
                      {captureSources.map((source) => (
                        <option
                          key={source.id}
                          value={source.id}
                          className="bg-[#111827] text-slate-100"
                        >
                          {captureSourceKindLabel(source.source_kind)} / {source.title} /{" "}
                          {source.width}x{source.height}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : (
                  renderCaptureSourceCards()
                )}
              </div>
            )}

            <div className="flex items-center justify-between border-t border-white/10 px-4 py-3">
              <div className="text-[11px] text-slate-500">
                {settingsFooterMessage}
              </div>
              <div className="flex items-center gap-2">
                <button
                  className="rounded-md border border-cyan-400/30 px-3 py-1.5 text-[11px] font-medium text-cyan-100 hover:bg-cyan-500/15"
                  onClick={applyLowLatencyProfile}
                >
                  Low latency
                </button>
                {isLocalPipelinePreview && (
                  <>
                    <button
                      className="rounded-md border border-sky-400/30 px-3 py-1.5 text-[11px] font-medium text-sky-100 hover:bg-sky-500/15 disabled:cursor-not-allowed disabled:opacity-45"
                      onClick={applyBrowserWebRtc2k144LowLatencyProfile}
                      disabled={Boolean(browserWebRtc2k144BlockReason)}
                      title={browserWebRtc2k144BlockReason ?? "浏览器 WebRTC H.264 RTP 预览路径"}
                    >
                      WebRTC 2K144
                    </button>
                    <button
                      className="rounded-md border border-violet-400/30 px-3 py-1.5 text-[11px] font-medium text-violet-100 hover:bg-violet-500/15 disabled:cursor-not-allowed disabled:opacity-45"
                      onClick={applyBrowserWebCodecsUltraLowLatencyProfile}
                      disabled={Boolean(browserWebCodecsUltraBlockReason)}
                      title={browserWebCodecsUltraBlockReason ?? "浏览器 WebCodecs + WebSocket AU + WebGL2 优先路径"}
                    >
                      WebCodecs 2K144
                    </button>
                  </>
                )}
                {!isLocalPipelinePreview && (
                  <button
                    className="rounded-md border border-emerald-400/30 px-3 py-1.5 text-[11px] font-medium text-emerald-100 hover:bg-emerald-500/15"
                    onClick={() => void handleApplyRemoteMediaProfile()}
                  >
                    应用远端
                  </button>
                )}
                <button
                  className="rounded-md px-3 py-1.5 text-[11px] text-slate-300 hover:bg-white/10"
                  onClick={closeTestSettings}
                >
                  关闭
                </button>
                <button
                  className="inline-flex items-center gap-2 rounded-md bg-cyan-500 px-3 py-1.5 text-[11px] font-medium text-white hover:bg-cyan-400 disabled:opacity-50"
                  onClick={() => void handleStartTest()}
                  disabled={
                    testStatus === "starting" ||
                    testStatus === "stopping" ||
                    Boolean(localStartBlockReason)
                  }
                  title={localStartBlockReason ?? undefined}
                >
                  {testStatus === "starting" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Play className="h-3.5 w-3.5" />
                  )}
                  开始测试
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {!isLocalPipelinePreview && captureSourcePickerOpen && (
        <div
          className="fixed inset-0 z-[60] flex items-center justify-center bg-black/65 px-4"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          data-no-drag="true"
        >
          <div className="flex max-h-[calc(100vh-3rem)] w-full max-w-5xl flex-col rounded-lg border border-white/10 bg-[#0f1724] shadow-2xl">
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <div>
                <div className="text-sm font-semibold text-slate-100">远端捕获源选择</div>
                <div className="mt-1 text-[11px] text-slate-500">
                  优先选择全屏 shared copy；需要应用窗口时选择单窗口源。
                </div>
              </div>
              <div className="flex items-center gap-2">
                <button
                  className="inline-flex items-center gap-2 rounded-md border border-cyan-400/30 px-3 py-1.5 text-[11px] font-medium text-cyan-100 hover:bg-cyan-500/15 disabled:opacity-50"
                  onClick={() => void handleRefreshRemoteCaptureSources()}
                  disabled={captureSourcesLoading}
                >
                  {captureSourcesLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  刷新
                </button>
                <button
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-slate-400 hover:bg-white/10 hover:text-white"
                  onClick={() => setCaptureSourcePickerOpen(false)}
                  title="Close"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            </div>
            <div className="min-h-0 overflow-y-auto px-4 py-4">
              {captureSources.length === 0 ? (
                <div className="rounded-lg border border-white/10 bg-black/20 px-3 py-8 text-center text-[11px] text-slate-500">
                  暂无捕获源。点击刷新从远端设备获取当前全屏/窗口列表。
                </div>
              ) : (
                renderCaptureSourceCards(true)
              )}
            </div>
          </div>
        </div>
      )}

      <div
        ref={renderAreaRef}
        data-native-render-area="true"
        className="relative min-h-0 flex-1 overflow-hidden bg-black"
      >
        <div className="absolute inset-0 bg-[radial-gradient(circle_at_center,#172033_0,#05070a_58%,#000_100%)]" />
        {webPreviewUsesVideo && (
          <video
            ref={webPreviewVideoRef}
            className="absolute inset-0 h-full w-full bg-black object-contain"
            style={{
              backfaceVisibility: "hidden",
              contain: "strict",
              transform: "translateZ(0)",
              willChange: "transform",
            }}
            autoPlay
            muted
            playsInline
          />
        )}
        {isLocalPipelinePreview && !isNative && webPreviewEngine === "webcodecs" && (
          <canvas
            key={webCodecsCanvasEpoch}
            ref={webCodecsCanvasRef}
            className="absolute inset-0 h-full w-full bg-black object-contain"
            style={{
              contain: "strict",
              imageRendering: "auto",
            }}
          />
        )}
        {isLocalPipelinePreview &&
          !isNative &&
          !webPreviewUsesVideo &&
          !(webPreviewEngine === "webcodecs" && ["connecting", "webcodecs"].includes(webPreviewMode)) && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="text-center">
              <PanelTop className="mx-auto mb-3 h-9 w-9 text-slate-500" />
              <div className="text-sm font-medium text-slate-300">
                {webPreviewEngine === "webcodecs"
                  ? "WebCodecs 超低延迟路径"
                  : isTestBusy
                    ? "等待 WebRTC 视频帧"
                    : "点击开始显示本机 WebRTC 画面"}
              </div>
              <div className="mt-1 text-xs text-slate-500">
                {localStartBlockReason ?? webPreviewError ?? testDescription}
              </div>
            </div>
          </div>
        )}
        {!isLocalPipelinePreview && showRemotePreviewFrame && remoteFrameDataUrl && (
          <img
            src={remoteFrameDataUrl}
            alt="Remote desktop frame"
            className="absolute inset-0 h-full w-full object-contain"
            style={remoteFrameAspectRatio ? { aspectRatio: remoteFrameAspectRatio } : undefined}
          />
        )}
        {isLocalPipelinePreview && !isNative && webPreviewMode === "connecting" && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="rounded-md border border-cyan-400/20 bg-black/55 px-4 py-3 text-center backdrop-blur">
              <Loader2 className="mx-auto mb-2 h-5 w-5 animate-spin text-cyan-300" />
              <div className="text-xs font-medium text-slate-200">
                {webPreviewEngine === "webcodecs" ? "正在启动 WebCodecs 解码" : "正在启动 WebRTC 视频"}
              </div>
              <div className="mt-1 text-[11px] text-slate-500">{testDescription}</div>
            </div>
          </div>
        )}
        {!isLocalPipelinePreview && !hasRemoteFrames && !remoteFrameDataUrl && (
          <div className="absolute inset-0 flex items-center justify-center px-6">
            <div className="max-w-xl rounded-xl border border-white/10 bg-black/45 px-6 py-5 text-center shadow-2xl backdrop-blur">
              <Network className="mx-auto mb-3 h-9 w-9 text-cyan-300" />
              <div className="text-sm font-semibold text-slate-100">等待远端媒体帧</div>
              <div className="mt-2 text-xs leading-5 text-slate-400">
                当前为 LAN 远程会话，不会再使用本机测试采集画面填充窗口。
                {sessionSnapshot?.receiver_active
                  ? " 接收侧已启动。"
                  : " 点击开始接收启动接收侧。"}
              </div>
              <div className="mt-3 grid grid-cols-3 gap-2 text-[11px] text-slate-300">
                <div className="rounded-md bg-white/8 px-2 py-1.5">
                  state: {sessionSnapshot?.state ?? "loading"}
                </div>
                <div className="rounded-md bg-white/8 px-2 py-1.5">
                  rx: {remoteFramesReceived}
                </div>
                <div className="rounded-md bg-white/8 px-2 py-1.5">
                  decoded: {remoteFramesDecoded}
                </div>
              </div>
            </div>
          </div>
        )}
        {!isLocalPipelinePreview && hasRemoteFrames && (
          <div className="absolute right-3 top-3 rounded-md border border-cyan-400/20 bg-black/45 px-3 py-2 text-[11px] text-cyan-100 backdrop-blur">
            remote rx {remoteFramesReceived} / decoded {remoteFramesDecoded}
            {remoteProbeTarget ? ` / ${remoteProbeTarget}` : ""}
          </div>
        )}
        {isBrowserBridgeRemote && (
          <div className="absolute left-3 top-3 rounded-md border border-amber-300/25 bg-black/50 px-3 py-2 text-[11px] text-amber-100 backdrop-blur">
            Web preview / mrd-service bridge / 非 native 高刷渲染
          </div>
        )}
        {reportVisible && lastCompletedRun && (
          <div className="absolute inset-x-3 top-3 z-20 mx-auto max-h-[calc(100%-1.5rem)] max-w-5xl overflow-y-auto rounded-xl border border-emerald-300/25 bg-[#03140f]/88 p-4 text-xs text-emerald-50 shadow-2xl shadow-emerald-950/60 backdrop-blur-md">
            <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
              <div>
                <div className="flex items-center gap-2 text-sm font-semibold text-white">
                  <BarChart3 className="h-4 w-4 text-emerald-300" />
                  完整测试报告
                </div>
                <div className="mt-1 max-w-3xl truncate text-[11px] text-emerald-200/65">
                  {configCodecLabel(reportConfig)} / {configResolutionLabel(reportConfig)} @{" "}
                  {dash(reportConfig?.fps)} FPS / {formatMbps(configBitrateMbps(reportConfig))}
                </div>
              </div>
              <div className="text-right">
                <div
                  className={`inline-flex rounded-full border px-2 py-1 text-[11px] font-semibold ${
                    lastCompletedRun.status === "failed"
                      ? "border-red-300/35 bg-red-500/15 text-red-100"
                      : lastCompletedRun.status === "cancelled"
                        ? "border-amber-300/35 bg-amber-500/15 text-amber-100"
                        : "border-emerald-300/35 bg-emerald-500/15 text-emerald-100"
                  }`}
                >
                  {runStatusLabel(lastCompletedRun.status)}
                </div>
                <div className="mt-1 max-w-[260px] truncate text-[10px] text-emerald-200/60">
                  {lastCompletedRun.run_id}
                </div>
              </div>
            </div>

            <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
              <div className="rounded-lg border border-white/10 bg-white/8 p-3">
                <div className="text-[10px] uppercase text-emerald-200/55">FPS 平均 / 最低</div>
                <div className="mt-1 text-lg font-semibold text-white">
                  {formatSummaryFps(reportFpsAvg)}
                </div>
                <div className="text-[10px] text-emerald-200/55">
                  min {formatSummaryFps(reportFpsMinValue)}
                </div>
              </div>
              <div className="rounded-lg border border-white/10 bg-white/8 p-3">
                <div className="text-[10px] uppercase text-emerald-200/55">E2E 延迟 p50 / p95</div>
                <div className="mt-1 text-lg font-semibold text-white">
                  {formatSummaryMs(reportLatencyP50)} / {formatSummaryMs(reportLatencyP95)}
                </div>
                <div className="text-[10px] text-emerald-200/55">
                  感知延迟中位 / 尾延迟
                </div>
              </div>
              <div className="rounded-lg border border-white/10 bg-white/8 p-3">
                <div className="text-[10px] uppercase text-emerald-200/55">掉帧 / 丢弃率</div>
                <div className="mt-1 text-lg font-semibold text-white">
                  {formatDropped(lastRunDropped)}
                </div>
                <div className="text-[10px] text-emerald-200/55">
                  {formatPercent(reportDropRatio)}
                </div>
              </div>
              <div className="rounded-lg border border-white/10 bg-white/8 p-3">
                <div className="text-[10px] uppercase text-emerald-200/55">帧数 / 时长</div>
                <div className="mt-1 text-lg font-semibold text-white">
                  {formatCount(reportFrameCount)}
                </div>
                <div className="text-[10px] text-emerald-200/55">
                  {formatDurationMs(reportDurationMs)}
                </div>
              </div>
            </div>

            <div className="mt-3 grid gap-3 lg:grid-cols-[1.1fr_1fr]">
              <section className="rounded-lg border border-white/10 bg-black/20 p-3">
                <div className="mb-2 text-[11px] font-semibold text-emerald-100">运行配置</div>
                <div className="grid gap-1.5 sm:grid-cols-2">
                  {[
                    ["状态", runStatusLabel(lastCompletedRun.status)],
                    ["开始/结束", `${formatTimestamp(lastCompletedRun.started_at)} / ${formatTimestamp(lastCompletedRun.finished_at)}`],
                    ["链路", configCodecLabel(reportConfig)],
                    ["渲染", dash(reportConfig?.renderer_type ?? effectiveRenderLabel)],
                    ["分辨率", configResolutionLabel(reportConfig)],
                    ["目标 FPS", dash(reportConfig?.fps)],
                    ["码率", formatMbps(configBitrateMbps(reportConfig))],
                    ["内存路径", memoryPathLabel],
                    ["CPU", dash(reportEnvironment?.cpu_brand)],
                    ["GPU", dash(reportEnvironment?.gpu_info)],
                  ].map(([label, value]) => (
                    <div key={label} className="grid grid-cols-[76px_1fr] gap-2">
                      <span className="text-emerald-200/55">{label}</span>
                      <span className="min-w-0 truncate text-emerald-50">{value}</span>
                    </div>
                  ))}
                </div>
              </section>

              <section className="rounded-lg border border-white/10 bg-black/20 p-3">
                <div className="mb-2 text-[11px] font-semibold text-emerald-100">阶段 P95</div>
                <div className="grid gap-1.5">
                  {[
                    ["capture", diagnosticsCaptureP95Ms],
                    ["encode", lastRunSummary?.encode_latency_p95 ?? diagnosticsEncodeP95Ms],
                    ["transport", lastRunSummary?.transport_latency_p95 ?? diagnosticsTransportP95Ms],
                    ["decode", lastRunSummary?.decode_latency_p95 ?? diagnosticsDecodeP95Ms],
                    ["render", diagnosticsRenderP95Ms],
                  ].map(([label, value]) => (
                    <div key={label} className="grid grid-cols-[1fr_76px] gap-2">
                      <span className="text-emerald-200/60">{label}</span>
                      <span className="text-right font-medium text-emerald-50">
                        {formatSummaryMs(value as number | null)}
                      </span>
                    </div>
                  ))}
                </div>
              </section>
            </div>

            <div className="mt-3 grid gap-3 lg:grid-cols-2">
              <section className="rounded-lg border border-white/10 bg-black/20 p-3">
                <div className="mb-2 text-[11px] font-semibold text-emerald-100">mrd-service 资源</div>
                <div className="grid grid-cols-2 gap-2">
                  <div>CPU p95: {formatOptionalPercent(reportServiceCpuP95)}</div>
                  <div>内存峰值: {formatMb(reportServiceMemoryPeak)}</div>
                  <div>GPU p95: {formatOptionalPercent(reportServiceGpuP95)}</div>
                  <div>网络峰值: {formatMbps(reportServiceNetworkPeak)}</div>
                </div>
              </section>
              <section className="rounded-lg border border-white/10 bg-black/20 p-3">
                <div className="mb-2 text-[11px] font-semibold text-emerald-100">接收显示资源</div>
                <div className="grid grid-cols-2 gap-2">
                  <div>CPU p95: {formatOptionalPercent(reportDisplayCpuP95)}</div>
                  <div>内存峰值: {formatMb(reportDisplayMemoryPeak)}</div>
                  <div>GPU p95: {formatOptionalPercent(reportDisplayGpuP95)}</div>
                  <div>网络峰值: {formatMbps(reportDisplayNetworkPeak)}</div>
                </div>
              </section>
            </div>

            {(lastRunSummary?.error_message || mediaPipelineSnapshot?.codec_fallback_reason) && (
              <div className="mt-3 rounded-lg border border-amber-300/25 bg-amber-500/10 p-3 text-amber-100">
                {lastRunSummary?.error_message
                  ? `error: ${lastRunSummary.error_message}`
                  : `codec fallback: ${mediaPipelineSnapshot?.codec_fallback_reason}`}
              </div>
            )}
          </div>
        )}
        {isLocalPipelinePreview && matrixRunProgress && (
          <div className="absolute right-3 top-3 max-w-md rounded-md border border-violet-300/25 bg-violet-950/65 px-3 py-2 text-[11px] text-violet-100 backdrop-blur">
            矩阵 {matrixRunProgress.current}/{matrixRunProgress.total}:{" "}
            {matrixRunProgress.label}
          </div>
        )}
        {lastError && (
          <div className="absolute bottom-3 left-3 max-w-xl rounded-md border border-red-500/30 bg-red-950/70 px-3 py-2 text-xs text-red-100">
            {lastError}
          </div>
        )}
      </div>

      <div className="flex h-10 shrink-0 items-center justify-between gap-3 border-t border-white/10 bg-[#0f1724] px-3 text-[11px] text-slate-400">
        <div className="flex min-w-0 items-center gap-4">
          <span className="inline-flex items-center gap-1.5">
            <Circle className="h-2 w-2 fill-emerald-400 text-emerald-400" />
            {statusLabel}
          </span>
          <span>render: {effectiveRenderLabel}</span>
          <span className="hidden min-w-0 truncate md:inline">
            {isLocalPipelinePreview
              ? `test: ${testDescription}`
              : `remote: ${sessionSnapshot?.transport_kind ?? "unknown"} / receiver ${
                  sessionSnapshot?.receiver_active ? "on" : "off"
                }`}
          </span>
          {metrics && (
            <span className="hidden lg:inline">
              {metrics.capture_fps.toFixed(1)} FPS / {metrics.total_latency_p95_ms.toFixed(1)} ms
            </span>
          )}
          {!metrics && webVideoFps !== null && (
            <span className="hidden lg:inline">
              web {webVideoFps.toFixed(1)} FPS / {webVideoFrameCount} frames
            </span>
          )}
          <span className="hidden xl:inline">
            memory: {memoryPathLabel}
          </span>
          {isNative && nativeSurface?.attached && (
            <span className="hidden xl:inline">
              surface: {activeSurfaceId} / handle {nativeSurface.hwnd}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {testMessage && <span className="hidden max-w-[220px] truncate md:inline">{testMessage}</span>}
          <button
            className="inline-flex h-7 items-center gap-1.5 rounded-md border border-white/10 px-2 text-slate-300 hover:bg-white/10"
            onClick={openTestSettings}
          >
            <SlidersHorizontal className="h-3.5 w-3.5" />
            配置
          </button>
          <button
            className={`inline-flex h-7 items-center gap-1.5 rounded-md px-2 font-medium ${
              isLocalPipelinePreview && isTestBusy
                ? "bg-red-500/90 text-white hover:bg-red-400"
                : primaryActionBlocked
                  ? "cursor-not-allowed bg-slate-700 text-slate-400"
                  : "bg-cyan-500 text-white hover:bg-cyan-400"
            }`}
            aria-label={
              isLocalPipelinePreview && isTestBusy
                ? "Stop local pipeline test"
                : isLocalPipelinePreview
                  ? "Start local pipeline test"
                  : "Start remote receiver"
            }
            onClick={() =>
              void (isLocalPipelinePreview && isTestBusy
                ? handleStopTest()
                : handleStartTest())
            }
            disabled={primaryActionBlocked}
            title={localStartBlockReason ?? undefined}
          >
            {testStatus === "starting" || testStatus === "stopping" ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : isLocalPipelinePreview && isTestBusy ? (
              <Square className="h-3 w-3" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            {primaryActionLabel}
          </button>
          <span className="hidden items-center gap-1.5 xl:inline-flex">
            <MousePointer2 className="h-3.5 w-3.5" />
            input ready
          </span>
        </div>
      </div>
    </div>
  );
}
