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
  presentTestHarnessFrameOnNativeSurface,
  testGetCapabilities,
  testGetRun,
  testHarnessGetFrames,
  testHarnessGetMetrics,
  testHarnessStop,
  testStartRun,
  testStopRun,
  type CaptureType,
  type DecoderType,
  type EncoderType,
  type EnvironmentSnapshot,
  type FrameData,
  type HarnessMetrics,
  type NativeRenderSurfaceSnapshot,
  type RemoteDisplayWindowContext,
  type TestConfig,
  type TestMatrixConfig,
} from "../adapters/tauri";
import {
  getProbeSnapshot,
  getSessionSnapshot,
  startReceiver,
  type ProbeSnapshot,
  type SessionRuntimeSnapshot,
} from "../services/ipcSessionService";
import { isTauriRuntime } from "../utils/runtime";
import { withTauriWindow } from "../utils/tauriWindow";

type RenderMode = "web" | "d3d11_native" | "metal_native";
type TransportKind = NonNullable<TestMatrixConfig["transport"]>;
type ResolutionKey = "1280x720" | "1920x1080" | "2560x1440" | "2560x1600" | "3440x1440";
type FpsKey = "30" | "60" | "120" | "144";
type BitrateKey = "8" | "20" | "50" | "80";
type TestStatus = "idle" | "starting" | "running" | "stopping" | "completed" | "failed";
type WebPreviewMode = "idle" | "connecting" | "webrtc" | "fallback" | "failed";

const METRICS_POLL_MS = 500;
const WEB_PREVIEW_FRAME_POLL_MS = 80;

type Option<T extends string> = {
  value: T;
  label: string;
};

const captureOptions: Option<CaptureType>[] = [
  { value: "dxgi", label: "DXGI" },
  { value: "winrt", label: "WinRT" },
  { value: "macos", label: "macOS" },
  { value: "synthetic", label: "Synthetic" },
];

const encoderOptions: Option<EncoderType>[] = [
  { value: "nvenc_h264", label: "NVENC H.264" },
  { value: "videotoolbox_h264", label: "VideoToolbox H.264" },
  { value: "openh264", label: "OpenH264" },
  { value: "nvenc_av1", label: "NVENC AV1" },
];

const decoderOptions: Option<DecoderType>[] = [
  { value: "nvdec", label: "NVDEC" },
  { value: "videotoolbox", label: "VideoToolbox" },
  { value: "software", label: "Software" },
  { value: "none", label: "Encode only" },
];

const transportOptions: Option<TransportKind>[] = [
  { value: "loopback", label: "Loopback" },
  { value: "webrtc", label: "WebRTC" },
  { value: "quic", label: "QUIC" },
];

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
];

const bitrateOptions: Option<BitrateKey>[] = [
  { value: "8", label: "8 Mbps" },
  { value: "20", label: "20 Mbps" },
  { value: "50", label: "50 Mbps" },
  { value: "80", label: "80 Mbps" },
];

function optionLabel<T extends string>(options: Option<T>[], value: T) {
  return options.find((option) => option.value === value)?.label ?? value;
}

function browserLooksLikeMacos(): boolean {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform.toLowerCase();
  const userAgent = navigator.userAgent.toLowerCase();
  return platform.includes("mac") || userAgent.includes("mac os x");
}

function defaultNativeRenderMode(): RenderMode {
  return browserLooksLikeMacos() ? "metal_native" : "d3d11_native";
}

function normalizeOs(osType?: string): "macos" | "windows" | "other" {
  const os = osType?.toLowerCase() ?? "";
  if (os.includes("mac")) return "macos";
  if (os.includes("win")) return "windows";
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

export function RemoteDisplayWindowPage() {
  const { id } = useParams();
  const [searchParams] = useSearchParams();
  const surfaceId = searchParams.get("surface") ?? "surface-1";
  const renderAreaRef = useRef<HTMLDivElement | null>(null);
  const syncAnimationFrameRef = useRef<number | null>(null);
  const syncTimerIdsRef = useRef<number[]>([]);
  const webPreviewRenderedGenerationRef = useRef<number | null>(null);
  const webPreviewVideoRef = useRef<HTMLVideoElement | null>(null);
  const webPreviewPeerRef = useRef<RTCPeerConnection | null>(null);
  const webPreviewSessionRef = useRef<string | null>(null);

  const [context, setContext] = useState<RemoteDisplayWindowContext | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [nativeSurface, setNativeSurface] =
    useState<NativeRenderSurfaceSnapshot | null>(null);
  const [renderMode, setRenderMode] = useState<RenderMode>(() =>
    isTauriRuntime() ? defaultNativeRenderMode() : "web"
  );
  const [capture, setCapture] = useState<CaptureType>("dxgi");
  const [encoder, setEncoder] = useState<EncoderType>("nvenc_h264");
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
  const [capturedFrame, setCapturedFrame] = useState<FrameData | null>(null);
  const [webPreviewMode, setWebPreviewMode] = useState<WebPreviewMode>("idle");
  const [webPreviewError, setWebPreviewError] = useState<string | null>(null);
  const [testMessage, setTestMessage] = useState<string | null>(null);
  const [sessionSnapshot, setSessionSnapshot] =
    useState<SessionRuntimeSnapshot | null>(null);
  const [probeSnapshot, setProbeSnapshot] = useState<ProbeSnapshot | null>(null);

  const sessionId = id ?? context?.session_id ?? "local-preview";
  const activeSurfaceId = context?.surface_id ?? surfaceId;
  const isLocalPipelinePreview = isLocalPipelinePreviewSession(sessionId);
  const hostOs = normalizeOs(capabilities?.os_type);
  const nativeRenderMode: RenderMode = hostOs === "macos" ? "metal_native" : "d3d11_native";
  const nativeRendererType =
    renderMode === "metal_native" ? "macos" : renderMode === "d3d11_native" ? "d3d11" : null;
  const isNative = nativeRendererType !== null;
  const nativeRendererTypeForHost = hostOs === "macos" ? "macos" : "d3d11";
  const currentNativeRendererAvailable =
    isTauriRuntime() &&
    (!capabilities
      ? true
      : nativeRendererType
        ? capabilities.available_renderers?.includes(nativeRendererType) ?? false
        : false);
  const nativeRendererAvailableForHost =
    isTauriRuntime() &&
    (!capabilities
      ? true
      : capabilities.available_renderers?.includes(nativeRendererTypeForHost) ?? false);
  const nativeRenderAvailable = isNative
    ? currentNativeRendererAvailable
    : nativeRendererAvailableForHost;
  const usesNativeSharedTexture =
    nativeRendererType === "d3d11" &&
    capture === "dxgi" &&
    encoder === "nvenc_h264" &&
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
  const renderModeLabel =
    renderMode === "metal_native"
      ? "Metal native"
      : renderMode === "d3d11_native"
        ? "D3D11 native"
        : "Web preview";
  const nativeRenderLabel = hostOs === "macos" ? "Metal native" : "DX11 native";
  const remoteFramesReceived = probeSnapshot?.frames_received ?? 0;
  const remoteFramesDecoded = probeSnapshot?.frames_decoded ?? 0;
  const hasRemoteFrames = remoteFramesReceived > 0 || remoteFramesDecoded > 0;

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
  const buildTestConfig = useCallback((rendererTargetHwnd?: string | null) => {
    const [width, height] = resolution.split("x").map(Number) as [number, number];
    return {
      capture_type: capture,
      encoder_type: encoder,
      decoder_type: decoder,
      transport_kind: transport,
      resolution: [width, height],
      fps: Number(fps),
      bitrate: Number(bitrate) * 1_000_000,
      duration_ms: 30_000,
      warmup_ms: 500,
      input_source: capture === "synthetic" ? "synthetic" : "screen",
      output_validation: true,
      render_display: Boolean(isNative && rendererTargetHwnd),
      zero_copy: usesNativeSharedTexture,
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
    usesNativeSharedTexture,
  ]);
  const testConfig = useMemo(
    () => buildTestConfig(nativeSurface?.hwnd),
    [buildTestConfig, nativeSurface?.hwnd]
  );
  const isTestBusy =
    testStatus === "starting" || testStatus === "running" || testStatus === "stopping";

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
        } else if (result.value?.render_mode === "d3d11_native") {
          setRenderMode("d3d11_native");
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
        pickAvailable(value, capabilities.available_encoders, ["nvenc_h264", "openh264"], "nvenc_h264")
      );
      setDecoder((value) =>
        pickAvailable(value, capabilities.available_decoders, ["nvdec", "software", "none"], "nvdec")
      );
      setRenderMode((value) => (value === "metal_native" ? "d3d11_native" : value));
      return;
    }

    setRenderMode("web");
  }, [capabilities]);

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
    if (!isLocalPipelinePreview) return;
    setTestSettingsOpen(true);
    void syncNativeSurface({ visible: false });
  }, [isLocalPipelinePreview, syncNativeSurface]);

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
    webPreviewRenderedGenerationRef.current = null;
    setCapturedFrame(null);
    setRenderMode(nativeRenderMode);
  }, [closeWebPreviewPeer, nativeRenderMode, nativeRendererAvailableForHost]);

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

    if (!isTauriRuntime() || typeof RTCPeerConnection === "undefined") {
      setWebPreviewMode("fallback");
      setWebPreviewError("WebRTC is unavailable in this runtime");
      return;
    }

    if (encoder === "nvenc_av1") {
      setWebPreviewMode("fallback");
      setWebPreviewError("Browser WebRTC preview currently supports H.264 output");
      return;
    }

    let cancelled = false;
    const peer = new RTCPeerConnection({ iceServers: [] });
    webPreviewPeerRef.current = peer;
    webPreviewSessionRef.current = sessionId;
    setWebPreviewMode("connecting");
    setWebPreviewError(null);
    setCapturedFrame(null);

    peer.addTransceiver("video", { direction: "recvonly" });
    peer.ontrack = (event) => {
      if (cancelled) return;
      const video = webPreviewVideoRef.current;
      if (!video) return;
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      video.srcObject = stream;
      video.muted = true;
      video.playsInline = true;
      void video.play().catch(() => {});
      setWebPreviewMode("webrtc");
      setWebPreviewError(null);
    };
    peer.onconnectionstatechange = () => {
      if (cancelled) return;
      if (peer.connectionState === "failed" || peer.connectionState === "disconnected") {
        setWebPreviewMode("fallback");
        setWebPreviewError(`WebRTC preview ${peer.connectionState}`);
      }
    };

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
        setWebPreviewMode("fallback");
        setWebPreviewError(message);
      }
    };

    void startPreview();

    return () => {
      cancelled = true;
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
    sessionId,
  ]);

  useEffect(() => {
    if (
      !isLocalPipelinePreview ||
      !isTestBusy ||
      isNative ||
      (webPreviewMode !== "fallback" && webPreviewMode !== "failed")
    ) {
      return;
    }

    let cancelled = false;
    let timeoutId: number | null = null;
    webPreviewRenderedGenerationRef.current = null;

    const poll = async () => {
      try {
        const framesResult = await testHarnessGetFrames({
          includeCaptured: false,
          includeRendered: true,
          lastRenderedGeneration:
            webPreviewRenderedGenerationRef.current ?? undefined,
        });
        if (cancelled) return;
        if (framesResult.ok) {
          const renderedFrame = framesResult.value[1] ?? framesResult.value[0];
          if (renderedFrame) {
            const generation = renderedFrame[3];
            if (typeof generation === "number") {
              webPreviewRenderedGenerationRef.current = generation;
            }
            setCapturedFrame((current) =>
              typeof generation === "number" && current?.[3] === generation
                ? current
                : renderedFrame
            );
          }
        }
      } finally {
        if (!cancelled) {
          timeoutId = window.setTimeout(poll, WEB_PREVIEW_FRAME_POLL_MS);
        }
      }
    };

    void poll();

    return () => {
      cancelled = true;
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
    };
  }, [isLocalPipelinePreview, isNative, isTestBusy, webPreviewMode]);

  useEffect(() => {
    if (isLocalPipelinePreview || !isTauriRuntime()) return;

    let cancelled = false;
    const poll = async () => {
      try {
        const [snapshot, probe] = await Promise.all([
          getSessionSnapshot(sessionId),
          getProbeSnapshot(sessionId),
        ]);
        if (cancelled) return;

        setSessionSnapshot(snapshot);
        setProbeSnapshot(probe);

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

    setCapture("dxgi");
    setEncoder("nvenc_h264");
    setDecoder("nvdec");
    setTransport("quic");
    setResolution("1920x1080");
    setFps("144");
    setBitrate("20");
    setRenderMode("d3d11_native");
  }, [capabilities, hostOs]);

  const handleStartRemoteReceiver = useCallback(async () => {
    setTestSettingsOpen(false);
    setLastError(null);
    setTestMessage("启动远程接收侧");
    setTestStatus("starting");
    setCurrentRunId(null);
    setMetrics(null);
    webPreviewRenderedGenerationRef.current = null;
    setCapturedFrame(null);

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
  }, [sessionId]);

  const handleStartTest = async () => {
    if (!isLocalPipelinePreview) {
      await handleStartRemoteReceiver();
      return;
    }

    setTestSettingsOpen(false);
    setLastError(null);
    setTestMessage("测试启动中");
    setTestStatus("starting");
    setCurrentRunId(null);
    setMetrics(null);
    webPreviewRenderedGenerationRef.current = null;
    setCapturedFrame(null);
    setWebPreviewMode("idle");
    setWebPreviewError(null);

    await testHarnessStop();

    let configForRun = testConfig;
    if (isNative) {
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
          : "Web preview"
      : renderModeLabel;

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
          <div className="hidden items-center gap-2 rounded-md bg-white/7 px-2 py-1 text-[11px] text-slate-300 md:flex">
            <Network className="h-3.5 w-3.5 text-emerald-300" />
            <span>0.3 ms</span>
            <span className="text-slate-600">/</span>
            <span>{formatTime(elapsed)}</span>
          </div>
          <div className="flex overflow-hidden rounded-md border border-white/10">
            <button
              className={`px-2.5 py-1 text-[11px] ${
                renderMode === "web"
                  ? "bg-white/14 text-white"
                  : "text-slate-400 hover:bg-white/8"
              }`}
              onClick={() => setRenderMode("web")}
            >
              Web preview
            </button>
            <button
              className={`px-2.5 py-1 text-[11px] ${
                renderMode === nativeRenderMode
                  ? "bg-cyan-500/25 text-cyan-100"
                  : nativeRendererAvailableForHost
                    ? "text-slate-400 hover:bg-white/8"
                    : "cursor-not-allowed text-slate-600"
              }`}
              onClick={switchToNativeRender}
              disabled={!nativeRendererAvailableForHost}
            >
              {nativeRenderLabel}
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
          <div className="flex max-h-[calc(100vh-2rem)] w-full max-w-3xl flex-col rounded-lg border border-white/10 bg-[#0f1724] shadow-2xl">
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

            <div className="flex items-center justify-between border-t border-white/10 px-4 py-3">
              <div className="text-[11px] text-slate-500">
                {metrics
                  ? `${metrics.capture_fps.toFixed(1)} FPS / ${metrics.frame_count} frames`
                  : "等待开始测试"}
              </div>
              <div className="flex items-center gap-2">
                <button
                  className="rounded-md border border-cyan-400/30 px-3 py-1.5 text-[11px] font-medium text-cyan-100 hover:bg-cyan-500/15"
                  onClick={applyLowLatencyProfile}
                >
                  Low latency
                </button>
                <button
                  className="rounded-md px-3 py-1.5 text-[11px] text-slate-300 hover:bg-white/10"
                  onClick={closeTestSettings}
                >
                  关闭
                </button>
                <button
                  className="inline-flex items-center gap-2 rounded-md bg-cyan-500 px-3 py-1.5 text-[11px] font-medium text-white hover:bg-cyan-400 disabled:opacity-50"
                  onClick={() => void handleStartTest()}
                  disabled={testStatus === "starting" || testStatus === "stopping"}
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
        {isLocalPipelinePreview && !isNative && !webPreviewUsesVideo && capturedFrame && (
          <img
            src={`data:image/png;base64,${capturedFrame[0]}`}
            alt="Rendered frame"
            className="absolute inset-0 h-full w-full object-contain"
          />
        )}
        {isLocalPipelinePreview && !isNative && !webPreviewUsesVideo && !capturedFrame && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="text-center">
              <PanelTop className="mx-auto mb-3 h-9 w-9 text-slate-500" />
              <div className="text-sm font-medium text-slate-300">
                {isTestBusy ? "Waiting for preview frames" : "Click start to show captured output"}
              </div>
              <div className="mt-1 text-xs text-slate-500">
                {webPreviewError ?? testDescription}
              </div>
            </div>
          </div>
        )}
        {isLocalPipelinePreview && !isNative && webPreviewMode === "connecting" && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="rounded-md border border-cyan-400/20 bg-black/55 px-4 py-3 text-center backdrop-blur">
              <Loader2 className="mx-auto mb-2 h-5 w-5 animate-spin text-cyan-300" />
              <div className="text-xs font-medium text-slate-200">Starting WebRTC video</div>
              <div className="mt-1 text-[11px] text-slate-500">{testDescription}</div>
            </div>
          </div>
        )}
        {!isLocalPipelinePreview && !hasRemoteFrames && (
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
            memory: {usesNativeSharedTexture ? "D3D11 shared" : nativeRendererType === "macos" ? "Metal upload" : "CPU preview"}
          </span>
          {isNative && nativeSurface?.attached && (
            <span className="hidden xl:inline">
              surface: {activeSurfaceId} / handle {nativeSurface.hwnd}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {testMessage && <span className="hidden max-w-[220px] truncate md:inline">{testMessage}</span>}
          {isLocalPipelinePreview && (
            <button
              className="inline-flex h-7 items-center gap-1.5 rounded-md border border-white/10 px-2 text-slate-300 hover:bg-white/10"
              onClick={openTestSettings}
            >
              <SlidersHorizontal className="h-3.5 w-3.5" />
              配置
            </button>
          )}
          <button
            className={`inline-flex h-7 items-center gap-1.5 rounded-md px-2 font-medium ${
              isLocalPipelinePreview && isTestBusy
                ? "bg-red-500/90 text-white hover:bg-red-400"
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
