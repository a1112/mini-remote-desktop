import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams, useSearchParams } from "react-router";
import {
  ArrowLeft,
  Circle,
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
  browserWebrtcPreviewStart,
  browserWebrtcPreviewStop,
  closeRemoteDisplayWindow,
  configureRemoteDisplayNativeSurface,
  currentRemoteDisplayWindowContext,
  ipcMediaPipelineSnapshot,
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
  type TestConfig,
  type TestMatrixConfig,
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
type ResolutionKey = "1280x720" | "1920x1080" | "2560x1440" | "2560x1600" | "3440x1440";
type FpsKey = "30" | "60" | "120" | "144" | "165" | "180" | "249";
type BitrateKey = "8" | "20" | "50" | "80" | "100" | "120";
type TestStatus = "idle" | "starting" | "running" | "stopping" | "completed" | "failed";
type WebPreviewMode = "idle" | "connecting" | "webrtc" | "failed";
type CaptureSourcePickerMode = "dropdown" | "modal";

const METRICS_POLL_MS = 500;
const WEB_PREVIEW_CONNECT_TIMEOUT_MS = 3_000;
const WEB_VIEW_MAX_FPS = 60;

type Option<T extends string> = {
  value: T;
  label: string;
};

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

function formatMbps(value?: number | null) {
  return typeof value === "number" ? `${value.toFixed(value >= 10 ? 1 : 2)} Mbps` : "-";
}

function formatPercent(value: number) {
  return `${Math.max(0, value).toFixed(value >= 10 ? 0 : 1)}%`;
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

function fpsForWebView(fps: FpsKey): FpsKey {
  return Number(fps) > WEB_VIEW_MAX_FPS ? "60" : fps;
}

type LocalWebViewProfile = {
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport: TransportKind;
  fps: FpsKey;
};

type LocalWebViewPlan = {
  profile: LocalWebViewProfile | null;
  reason: string | null;
  changed: boolean;
  message: string | null;
};

function resolveLocalWebViewPlan({
  capabilities,
  hostOs,
  capture,
  encoder,
  decoder,
  transport,
  fps,
}: {
  capabilities: EnvironmentSnapshot | null;
  hostOs: HostOs;
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport: TransportKind;
  fps: FpsKey;
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
        ? ["openh264", "nvenc_h264"]
        : ["openh264"];
  const encoderCandidates = uniqueValues([
    ...preferredEncoders,
    ...(isH264PreviewEncoder(encoder) ? [encoder] : []),
  ]);
  const nextEncoder = pickCapability(
    encoderCandidates,
    capabilities?.available_encoders
  );

  if (!nextEncoder) {
    return {
      profile: null,
      reason: "Web View 需要可输出 H.264 的编码器",
      changed: false,
      message: null,
    };
  }

  const nextDecoder: DecoderType = "none";

  const profile: LocalWebViewProfile = {
    capture: nextCapture,
    encoder: nextEncoder,
    decoder: nextDecoder,
    transport: "webrtc",
    fps: fpsForWebView(fps),
  };
  const changed =
    profile.capture !== capture ||
    profile.encoder !== encoder ||
    profile.decoder !== decoder ||
    profile.transport !== transport ||
    profile.fps !== fps;

  return {
    profile,
    reason: null,
    changed,
    message: changed
      ? `Web View 已切换到 ${optionLabel(captureOptions, profile.capture)} / ${optionLabel(
          encoderOptions,
          profile.encoder
        )} / ${optionLabel(decoderOptions, profile.decoder)} / ${optionLabel(
          transportOptions,
          profile.transport
        )} / ${optionLabel(fpsOptions, profile.fps)}`
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
  className = "",
}: {
  label: string;
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  className?: string;
}) {
  return (
    <label
      className={`flex h-9 min-w-0 items-center gap-1 rounded-md border border-white/10 bg-black/20 px-2 text-[10px] text-slate-400 ${className}`}
      title={label}
    >
      <span className="shrink-0 uppercase tracking-normal">{label}</span>
      <select
        className="min-w-0 bg-transparent text-[11px] font-medium text-slate-100 outline-none"
        value={value}
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

export function RemoteDisplayWindowPage() {
  const { id } = useParams();
  const [searchParams] = useSearchParams();
  const surfaceId = searchParams.get("surface") ?? "surface-1";
  const renderAreaRef = useRef<HTMLDivElement | null>(null);
  const syncAnimationFrameRef = useRef<number | null>(null);
  const syncTimerIdsRef = useRef<number[]>([]);
  const webPreviewVideoRef = useRef<HTMLVideoElement | null>(null);
  const webPreviewPeerRef = useRef<RTCPeerConnection | null>(null);
  const webPreviewSessionRef = useRef<string | null>(null);
  const autoCaptureSourceRequestedRef = useRef<string | null>(null);
  const linuxNativeProfileAppliedRef = useRef(false);

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
  const [resolution, setResolution] = useState<ResolutionKey>("1920x1080");
  const [fps, setFps] = useState<FpsKey>("144");
  const [bitrate, setBitrate] = useState<BitrateKey>("20");
  const [isMaximized, setIsMaximized] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [lastError, setLastError] = useState<string | null>(null);
  const [testSettingsOpen, setTestSettingsOpen] = useState(false);
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<HarnessMetrics | null>(null);
  const [webPreviewMode, setWebPreviewMode] = useState<WebPreviewMode>("idle");
  const [webPreviewError, setWebPreviewError] = useState<string | null>(null);
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

  const sessionId = id ?? context?.session_id ?? "local-preview";
  const activeSurfaceId = context?.surface_id ?? surfaceId;
  const isLocalPipelinePreview = isLocalPipelinePreviewSession(sessionId);
  const hostOs = normalizeOs(capabilities?.os_type);
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
      }),
    [capabilities, capture, decoder, encoder, fps, hostOs, transport]
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
  const visibleDecoderOptions = useMemo(
    () =>
      capabilities?.available_decoders?.length
        ? decoderOptions.filter((option) => capabilities.available_decoders.includes(option.value))
        : decoderOptions,
    [capabilities]
  );

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
  const remoteLatencyMs =
    mediaPipelineSnapshot?.stage_metrics?.find((metric) => metric.stage === "receiver.decode")
      ?.p95_ms ?? null;
  const remoteQuality =
    (probeSnapshot?.last_error || mediaPipelineSnapshot?.codec_fallback_reason)
      ? "降级"
      : remoteDropRatio <= 0.5 && (probeSnapshot?.current_fps ?? 0) >= 55
        ? "流畅"
        : hasRemoteFrames
          ? "一般"
          : "等待";
  const diagnosticsVisible = diagnosticsOpen || diagnosticsPinned;
  const diagnosticsCodec = codecLabel(
    mediaPipelineSnapshot?.active_codec ?? mediaProfileNegotiation?.selected.codec,
    mediaPipelineSnapshot?.active_codec_profile ??
      mediaProfileNegotiation?.selected.codec_profile
  );
  const diagnosticsChroma =
    mediaPipelineSnapshot?.active_chroma_subsampling ??
    mediaProfileNegotiation?.selected.chroma_subsampling ??
    "-";
  const diagnosticsPixelFormat =
    mediaPipelineSnapshot?.active_pixel_format ??
    mediaProfileNegotiation?.selected.pixel_format ??
    probeSnapshot?.latest_frame_pixel_format ??
    "-";
  const diagnosticsBitDepth =
    mediaPipelineSnapshot?.active_bit_depth ?? mediaProfileNegotiation?.selected.bit_depth ?? null;
  const diagnosticsHdrEnabled =
    mediaPipelineSnapshot?.active_hdr_enabled ?? mediaProfileNegotiation?.selected.hdr_enabled;
  const diagnosticsResolution =
    mediaPipelineSnapshot?.active_width && mediaPipelineSnapshot?.active_height
      ? `${mediaPipelineSnapshot.active_width}x${mediaPipelineSnapshot.active_height}`
      : probeSnapshot?.media_probe_width && probeSnapshot?.media_probe_height
      ? `${probeSnapshot.media_probe_width}x${probeSnapshot.media_probe_height}`
      : remoteProbeTarget?.split("@")[0] ?? "-";
  const diagnosticsTarget =
    mediaPipelineSnapshot?.active_width &&
    mediaPipelineSnapshot?.active_height &&
    mediaPipelineSnapshot?.active_fps
      ? `${mediaPipelineSnapshot.active_width}x${mediaPipelineSnapshot.active_height}@${mediaPipelineSnapshot.active_fps}`
      : remoteProbeTarget ??
    (mediaProfileNegotiation?.selected
      ? `${mediaProfileNegotiation.selected.width}x${mediaProfileNegotiation.selected.height}@${mediaProfileNegotiation.selected.fps}`
      : "-");

  const title = useMemo(() => {
    if (context?.label) return context.label;
    return `display-${sessionId}`;
  }, [context?.label, sessionId]);

  const testDescription = useMemo(
    () =>
      `${optionLabel(captureOptions, capture)} -> ${optionLabel(
        encoderOptions,
        encoder
      )} -> ${optionLabel(decoderOptions, decoder)} / ${optionLabel(
        transportOptions,
        transport
      )} / ${optionLabel(resolutionOptions, resolution)} @ ${optionLabel(
        fpsOptions,
        fps
      )} / ${optionLabel(bitrateOptions, bitrate)}`,
    [bitrate, capture, decoder, encoder, fps, resolution, transport]
  );
  const buildTestConfig = useCallback((rendererTargetHwnd?: string | null, selection?: Partial<LocalWebViewProfile>) => {
    const selectedCapture = selection?.capture ?? capture;
    const selectedEncoder = selection?.encoder ?? encoder;
    const selectedDecoder = selection?.decoder ?? decoder;
    const selectedTransport = selection?.transport ?? transport;
    const selectedFps = selection?.fps ?? fps;
    const [width, height] = resolution.split("x").map(Number) as [number, number];
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
      bitrate: Number(bitrate) * 1_000_000,
      duration_ms: 30_000,
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
    transport,
  ]);
  const testConfig = useMemo(
    () => buildTestConfig(nativeSurface?.hwnd),
    [buildTestConfig, nativeSurface?.hwnd]
  );
  const localStartBlockReason =
    isLocalPipelinePreview && renderMode === "web" ? localWebViewPlan.reason : null;
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
  const isTestBusy =
    testStatus === "starting" || testStatus === "running" || testStatus === "stopping";
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
          setRenderMode("web");
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

    const previewSessionId = webPreviewSessionRef.current;
    webPreviewSessionRef.current = null;
    if (stopHost && previewSessionId && isTauriRuntime()) {
      void browserWebrtcPreviewStop(previewSessionId);
    }
  }, []);

  const switchToNativeRender = useCallback(() => {
    if (!nativeRendererAvailableForHost) return;
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setRenderMode(nativeRenderMode);
  }, [closeWebPreviewPeer, nativeRenderMode, nativeRendererAvailableForHost]);

  const switchToD3d12Render = useCallback(() => {
    if (!d3d12RendererAvailable || localRenderSwitchLocked) return;
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setRenderMode("d3d12_native");
  }, [closeWebPreviewPeer, d3d12RendererAvailable, localRenderSwitchLocked]);

  const switchToWebRender = useCallback(() => {
    if (isLocalPipelinePreview && isTestBusy) {
      setTestMessage("请先停止测试再切换 Web View");
      return;
    }

    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    setRenderMode("web");
  }, [closeWebPreviewPeer, isLocalPipelinePreview, isTestBusy]);

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
  }, [currentRunId, isLocalPipelinePreview, isTestBusy, testStatus]);

  useEffect(() => {
    if (!isLocalPipelinePreview || !isTestBusy || isNative) {
      closeWebPreviewPeer();
      setWebPreviewMode("idle");
      setWebPreviewError(null);
      return;
    }

    if (localStartBlockReason) {
      closeWebPreviewPeer();
      setWebPreviewMode("failed");
      setWebPreviewError(localStartBlockReason);
      return;
    }

    if (!isTauriRuntime() || typeof RTCPeerConnection === "undefined") {
      setWebPreviewMode("failed");
      setWebPreviewError("WebRTC is unavailable in this runtime");
      return;
    }

    if (encoder === "nvenc_av1") {
      setWebPreviewMode("failed");
      setWebPreviewError("Browser WebRTC preview currently supports H.264 output");
      return;
    }

    if (!browserSupportsH264WebrtcVideo()) {
      setWebPreviewMode("failed");
      setWebPreviewError("Browser WebRTC video renderer does not advertise H.264 receive support");
      return;
    }

    let cancelled = false;
    let renderedVideoFrame = false;
    let connectTimeoutId: number | null = null;
    const peer = new RTCPeerConnection({ iceServers: [] });
    webPreviewPeerRef.current = peer;
    webPreviewSessionRef.current = sessionId;
    setWebPreviewMode("connecting");
    setWebPreviewError(null);

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

    peer.addTransceiver("video", { direction: "recvonly" });
    peer.ontrack = (event) => {
      if (cancelled) return;
      const stream = event.streams[0] ?? new MediaStream([event.track]);

      const bindStreamToVideo = () => {
        if (cancelled) return;
        const video = webPreviewVideoRef.current;
        if (!video) {
          window.requestAnimationFrame(bindStreamToVideo);
          return;
        }

        video.srcObject = stream;
        video.muted = true;
        video.playsInline = true;
        const videoWithFrameCallback = video as HTMLVideoElement & {
          requestVideoFrameCallback?: (callback: () => void) => number;
        };
        videoWithFrameCallback.requestVideoFrameCallback?.(() => markVideoRendered());
        video.addEventListener("loadeddata", markVideoRendered, { once: true });
        video.addEventListener("playing", markVideoRendered, { once: true });
        void video.play().catch((error) => {
          if (cancelled) return;
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
          h264Profile: encoder === "nvenc_h264" && decoder === "nvdec" ? "high" : "baseline",
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
      closeWebPreviewPeer();
    };
  }, [
    closeWebPreviewPeer,
    decoder,
    encoder,
    fps,
    isLocalPipelinePreview,
    isNative,
    isTestBusy,
    localStartBlockReason,
    sessionId,
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
    setMetrics(null);
    setWebPreviewMode("idle");
    setWebPreviewError(null);

    await testHarnessStop();

    let configForRun = testConfig;
    if (!isNative && localWebViewPlan.profile) {
      if (localWebViewPlan.changed) {
        setCapture(localWebViewPlan.profile.capture);
        setEncoder(localWebViewPlan.profile.encoder);
        setDecoder(localWebViewPlan.profile.decoder);
        setTransport(localWebViewPlan.profile.transport);
        setFps(localWebViewPlan.profile.fps);
        if (localWebViewPlan.message) setTestMessage(localWebViewPlan.message);
      }
      configForRun = buildTestConfig(null, localWebViewPlan.profile);
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
      configForRun = buildTestConfig(rendererTargetHwnd);
    } else if (isNative) {
      configForRun = buildTestConfig(null);
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

  const handleStopTest = async () => {
    if (!isLocalPipelinePreview) {
      setTestStatus("idle");
      setTestMessage("远程接收由 mrd-service 管理，未停止会话");
      return;
    }

    setTestStatus("stopping");
    closeWebPreviewPeer();
    setWebPreviewMode("idle");
    const result = currentRunId
      ? await testStopRun(currentRunId)
      : await testHarnessStop();
    await testHarnessStop();

    if (result.ok) {
      setTestStatus("idle");
      setCurrentRunId(null);
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
  const webPreviewUsesVideo =
    isLocalPipelinePreview &&
    !isNative &&
    (webPreviewMode === "connecting" || webPreviewMode === "webrtc");
  const effectiveRenderLabel =
    isLocalPipelinePreview && !isNative
      ? webPreviewMode === "webrtc"
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
  const settingsFooterMessage = localStartBlockReason
    ? localStartBlockReason
    : isLocalPipelinePreview
      ? metrics
        ? `${metrics.capture_fps.toFixed(1)} FPS / ${metrics.frame_count} frames`
        : "等待开始测试"
      : mediaProfileNegotiation
        ? `远端 ${mediaProfileNegotiation.selected.width}x${mediaProfileNegotiation.selected.height}@${mediaProfileNegotiation.selected.fps} / ${mediaProfileNegotiation.selected.bitrate_mbps} Mbps`
        : "远程参数将通过协商层下发";
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
            className="relative hidden md:block"
            onMouseEnter={() => setDiagnosticsOpen(true)}
            onMouseLeave={() => {
              if (!diagnosticsPinned) setDiagnosticsOpen(false);
            }}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                setDiagnosticsOpen(false);
                setDiagnosticsPinned(false);
              }
            }}
          >
            <button
              type="button"
              aria-label="连接诊断"
              aria-expanded={diagnosticsVisible}
              className="inline-flex items-center gap-2 rounded-md border border-emerald-400/20 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-50 hover:bg-emerald-500/16"
              onClick={() => {
                setDiagnosticsPinned((value) => !value);
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
              <span>{formatFps(probeSnapshot?.current_fps)}</span>
              <span className="text-emerald-200/60">/</span>
              <span>{remoteLatencyMs !== null ? `${remoteLatencyMs.toFixed(1)} ms` : "-"}</span>
            </button>
            {diagnosticsVisible ? (
              <div className="absolute right-0 top-9 z-50 max-h-[min(72vh,520px)] w-[420px] overflow-y-auto rounded-md border border-emerald-400/20 bg-[#03140f]/95 p-4 text-[11px] text-emerald-50 shadow-2xl shadow-emerald-950/60 backdrop-blur">
                <div className="mb-3 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="h-2 w-2 rounded-full bg-emerald-300" />
                    <div className="text-sm font-semibold text-white">远程诊断</div>
                  </div>
                  <div className="text-emerald-200/70">{formatTime(elapsed)}</div>
                </div>
                <div className="grid gap-4">
                  <DiagnosticGroup
                    title="连接"
                    rows={[
                      ["连接时间", formatTime(elapsed)],
                      ["连接质量", remoteQuality],
                      ["帧率", formatFps(probeSnapshot?.current_fps)],
                      ["延迟", remoteLatencyMs !== null ? `${remoteLatencyMs.toFixed(1)} ms` : "-"],
                      ["丢包/掉帧", formatPercent(remoteDropRatio)],
                      [
                        "码率",
                        formatMbps(
                          probeSnapshot?.bitrate_mbps ??
                            mediaPipelineSnapshot?.active_bitrate_mbps ??
                            null
                        ),
                      ],
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
          <div className="flex overflow-hidden rounded-md border border-white/10">
            <button
              className={`px-2.5 py-1 text-[11px] ${
                renderMode === "web"
                  ? "bg-white/14 text-white"
                  : localRenderSwitchLocked
                    ? "cursor-not-allowed text-slate-600"
                    : "text-slate-400 hover:bg-white/8"
              }`}
              onClick={switchToWebRender}
              disabled={localRenderSwitchLocked}
              title={renderSwitchLockedTitle}
            >
              Web View
            </button>
            <button
              className={`px-2.5 py-1 text-[11px] ${
                renderMode === nativeRenderMode
                  ? "bg-cyan-500/25 text-cyan-100"
                  : nativeRendererAvailableForHost && !localRenderSwitchLocked
                    ? "text-slate-400 hover:bg-white/8"
                    : "cursor-not-allowed text-slate-600"
              }`}
              onClick={switchToNativeRender}
              disabled={!nativeRendererAvailableForHost || localRenderSwitchLocked}
              title={renderSwitchLockedTitle}
            >
              {nativeRenderLabel}
            </button>
            <button
              className={`px-2.5 py-1 text-[11px] ${
                renderMode === "d3d12_native"
                  ? "bg-cyan-500/25 text-cyan-100"
                  : d3d12RendererAvailable && !localRenderSwitchLocked
                    ? "text-slate-400 hover:bg-white/8"
                    : "cursor-not-allowed text-slate-600"
              }`}
              onClick={switchToD3d12Render}
              disabled={!d3d12RendererAvailable || localRenderSwitchLocked}
              title={
                localRenderSwitchLocked
                  ? renderSwitchLockedTitle
                  : d3d12RendererAvailable
                    ? undefined
                    : d3d12UnavailableTitle
              }
            >
              DX12 native
            </button>
          </div>
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
              </div>
              <button
                className="inline-flex h-8 w-8 items-center justify-center rounded-md text-slate-400 hover:bg-white/10 hover:text-white"
                onClick={closeTestSettings}
                title="Close"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="grid min-h-0 gap-3 overflow-y-auto px-4 py-4 sm:grid-cols-2 lg:grid-cols-3">
              <TitleSelect
                label="CAP"
                value={capture}
                options={visibleCaptureOptions}
                onChange={setCapture}
              />
              <TitleSelect
                label="ENC"
                value={encoder}
                options={visibleEncoderOptions}
                onChange={setEncoder}
              />
              <TitleSelect
                label="DEC"
                value={decoder}
                options={visibleDecoderOptions}
                onChange={setDecoder}
              />
              <TitleSelect
                label="NET"
                value={transport}
                options={transportOptions}
                onChange={setTransport}
              />
              <TitleSelect
                label="SIZE"
                value={resolution}
                options={resolutionOptions}
                onChange={setResolution}
              />
              <TitleSelect label="FPS" value={fps} options={fpsOptions} onChange={setFps} />
              <TitleSelect
                label="BR"
                value={bitrate}
                options={bitrateOptions}
                onChange={setBitrate}
              />
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
            autoPlay
            muted
            playsInline
          />
        )}
        {isLocalPipelinePreview && !isNative && !webPreviewUsesVideo && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="text-center">
              <PanelTop className="mx-auto mb-3 h-9 w-9 text-slate-500" />
              <div className="text-sm font-medium text-slate-300">
                {isTestBusy ? "等待 WebRTC 视频帧" : "点击开始显示本机 WebRTC 画面"}
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
              <div className="text-xs font-medium text-slate-200">正在启动 WebRTC 视频</div>
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
          <span className="hidden xl:inline">
            memory: {usesNativeSharedTexture ? "D3D11 shared" : nativeRendererType === "macos" ? "Metal upload" : nativeRendererType === "linux" ? "Linux upload" : isLocalPipelinePreview && !isNative ? "WebRTC MediaStream" : "CPU preview"}
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
