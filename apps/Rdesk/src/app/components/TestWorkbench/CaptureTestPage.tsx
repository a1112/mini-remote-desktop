import { useState, useEffect } from "react";
import {
  Play,
  Square,
  Monitor,
  Activity,
  Gauge,
  Search,
  X,
  RefreshCw,
  ImageOff,
} from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type {
  Artifact,
  CaptureShareSourceTarget,
  EnvironmentSnapshot,
  TestConfig,
  WindowCaptureTarget,
} from "../../adapters/tauri/types";
import { capabilityAvailable, capabilityTag, unavailableText } from "./capabilityMeta";
import {
  shouldShowCapabilityOption,
  useShowUnavailableCapabilities,
} from "./useCapabilityVisibility";

type CaptureType = "dxgi" | "winrt" | "macos" | "linux" | "synthetic";
type CaptureScope = "screen" | "window_perf" | "window_probe";

interface CaptureOption {
  id: CaptureType;
  name: string;
  description: string;
}

const CAPTURE_OPTIONS: CaptureOption[] = [
  {
    id: "dxgi",
    name: "DXGI Desktop Duplication",
    description: "高性能桌面捕获，支持 Windows 8+",
  },
  {
    id: "winrt",
    name: "Windows Runtime Capture",
    description: "现代化屏幕捕获 API，支持窗口捕获",
  },
  {
    id: "macos",
    name: "macOS Capture",
    description: "macOS 屏幕捕获，需要 Screen Recording 权限",
  },
  {
    id: "linux",
    name: "Linux Capture",
    description: "Linux 屏幕捕获，优先 PipeWire/Portal 路径",
  },
  {
    id: "synthetic",
    name: "合成测试模式",
    description: "生成合成测试图案，用于基准测试",
  },
];

const CAPTURE_PERF_DURATION_MS = 30_000;

interface CaptureMetrics {
  is_running: boolean;
  capture_fps: number;
  frame_count: number;
  dropped_frames: number;
  resolution: [number, number];
  capture_latency_avg_ms: number;
  source_wait_latency_p95_ms: number;
  processing_latency_p95_ms: number;
  capture_latency_p95_ms: number;
  encode_latency_p95_ms: number;
  decode_latency_p95_ms: number;
  total_latency_p95_ms: number;
  error_message?: string | null;
}

function summarizeWindowProbeArtifacts(artifacts: Artifact[]): string | null {
  const artifact = artifacts.find((item) => item.kind === "structured_log");
  if (!artifact) return null;

  try {
    const parsed = JSON.parse(artifact.data) as {
      target_count?: number;
      selected_window?: {
        capture_item_created?: boolean;
        width?: number;
        height?: number;
        title?: string;
      };
      first_frame?: {
        captured?: boolean;
        width?: number;
        height?: number;
        byte_len?: number;
        pixel_format?: string;
      };
      media_probe?: {
        transport?: string;
        encoded_width?: number;
        encoded_height?: number;
        encoded_bytes?: number;
        transport_rtp_packet_count?: number;
        transport_payload_bytes?: number;
        decoded_frame_count?: number;
        rendered_frame_count?: number;
        render_backend?: string | null;
      };
    };
    const targetCount = parsed.target_count ?? 0;
    const selected = parsed.selected_window;
    const firstFrame = parsed.first_frame;
    const mediaProbe = parsed.media_probe;
    if (firstFrame?.captured && firstFrame.width && firstFrame.height) {
      const encodedSize =
        mediaProbe?.encoded_width && mediaProbe?.encoded_height
          ? `${mediaProbe.encoded_width}x${mediaProbe.encoded_height}, `
          : "";
      const transport =
        mediaProbe?.transport === "webrtc_rtp_loopback"
          ? ` over WebRTC RTP (${mediaProbe.transport_rtp_packet_count ?? 0} packets)`
          : mediaProbe?.transport
          ? ` over ${mediaProbe.transport}`
          : "";
      const mediaSummary = mediaProbe
        ? `; encoded ${encodedSize}${mediaProbe.encoded_bytes ?? 0} bytes${transport}, decoded ${
            mediaProbe.decoded_frame_count ?? 0
          }, rendered ${mediaProbe.rendered_frame_count ?? 0}${
            mediaProbe.render_backend ? ` via ${mediaProbe.render_backend}` : ""
          }`
        : "";
      return `Captured first frame ${firstFrame.width}x${firstFrame.height}, ${
        firstFrame.byte_len ?? 0
      } bytes (${firstFrame.pixel_format ?? "BGRA"}) from ${targetCount} targets${mediaSummary}`;
    }
    if (selected?.capture_item_created && selected.width && selected.height) {
      return `Found ${targetCount} targets; selected item ${selected.width}x${selected.height}`;
    }
    return `Found ${targetCount} window capture targets`;
  } catch {
    return null;
  }
}

function windowTargetKey(target: WindowCaptureTarget): string {
  return target.id ?? target.hwnd;
}

function shareSourceKey(source: CaptureShareSourceTarget): string {
  return source.id || source.native_id || source.hwnd || source.title;
}

function shareSourceToWindowTarget(source: CaptureShareSourceTarget): WindowCaptureTarget {
  return {
    id: source.id,
    platform: source.platform,
    source_kind: source.source_kind,
    hwnd: source.hwnd ?? source.native_id,
    title: source.title,
    class_name: source.class_name ?? source.app_name ?? source.platform,
    width: source.width,
    height: source.height,
    process_id: source.process_id ?? 0,
    app_name: source.app_name,
    bundle_identifier: source.bundle_identifier,
    window_layer: source.window_layer,
    preview_data_url: source.preview_data_url,
    preview_width: source.preview_width,
    preview_height: source.preview_height,
  };
}

function shareSourceCompatibleWithCapture(
  source: CaptureShareSourceTarget,
  capture: CaptureType
): boolean {
  if (capture === "synthetic") return false;
  if (capture === "dxgi") return source.platform === "windows" && source.source_kind === "screen";
  if (capture === "winrt") return source.platform === "windows";
  if (capture === "macos") return source.platform === "macos";
  if (capture === "linux") return source.platform === "linux";
  return false;
}

function shareSourceKindLabel(source: CaptureShareSourceTarget): string {
  if (source.source_kind === "window") return "Window";
  if (source.source_kind === "portal") return "System picker";
  return "Screen";
}

function supportsWindowCapture(capture: CaptureType): boolean {
  return capture === "winrt" || capture === "macos";
}

function windowCaptureApiName(capture: CaptureType): string {
  return capture === "macos" ? "ScreenCaptureKit" : "WinRT";
}

function capturePerformanceNote(capture: CaptureType, scope: CaptureScope): string | null {
  if (capture === "winrt" && scope === "screen") {
    return "WinRT 屏幕捕获通常受 DWM/显示刷新节拍影响，适合作为兼容与窗口捕获路径；2K/144Hz 高性能全屏采集优先使用 DXGI 零拷贝。";
  }
  if (capture === "winrt" && scope === "window_perf") {
    return "单窗口性能统计的是目标窗口新帧到达率；静态窗口或低刷新应用不会稳定产生 144 FPS，即使采集链路本身未限速。";
  }
  if (capture === "winrt" && scope === "window_probe") {
    return "单窗口验证只采集首帧并跑媒体探针，不代表持续采集 FPS。";
  }
  return null;
}

function captureModeLabel(scope: CaptureScope): string {
  if (scope === "window_perf") return "单窗口持续性能";
  if (scope === "window_probe") return "单窗口首帧验证";
  return "屏幕持续性能";
}

export function CaptureTestPage() {
  const [selectedCapture, setSelectedCapture] = useState<CaptureType>("dxgi");
  const [captureScope, setCaptureScope] = useState<CaptureScope>("screen");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<CaptureMetrics | null>(null);
  const [windowTargets, setWindowTargets] = useState<WindowCaptureTarget[]>([]);
  const [selectedWindowHwnd, setSelectedWindowHwnd] = useState<string | null>(null);
  const [windowTargetsLoading, setWindowTargetsLoading] = useState(false);
  const [windowTargetsError, setWindowTargetsError] = useState<string | null>(null);
  const [singleWindowProbeResult, setSingleWindowProbeResult] = useState<string | null>(null);
  const [shareSources, setShareSources] = useState<CaptureShareSourceTarget[]>([]);
  const [selectedShareSourceId, setSelectedShareSourceId] = useState<string | null>(null);
  const [sourcePickerOpen, setSourcePickerOpen] = useState(false);
  const [sourcePickerSources, setSourcePickerSources] = useState<CaptureShareSourceTarget[]>([]);
  const [sourcePickerLoading, setSourcePickerLoading] = useState(false);
  const [sourcePickerError, setSourcePickerError] = useState<string | null>(null);
  const [sourcePickerQuery, setSourcePickerQuery] = useState("");
  const [startAfterSourcePick, setStartAfterSourcePick] = useState(false);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [showUnavailable] = useShowUnavailableCapabilities();

  const selectedOption = CAPTURE_OPTIONS.find((o) => o.id === selectedCapture);
  const selectedWindowCapture = supportsWindowCapture(selectedCapture);
  const isWindowPerfMode = selectedWindowCapture && captureScope === "window_perf";
  const isWindowProbeMode = selectedWindowCapture && captureScope === "window_probe";
  const isWindowMode = isWindowPerfMode || isWindowProbeMode;
  const captureAvailable = (capture: CaptureType) =>
    capabilityAvailable(capabilities, "available_captures", capture, capture === "synthetic");
  const visibleCaptureOptions = CAPTURE_OPTIONS.filter((option) =>
    !capabilities || shouldShowCapabilityOption(captureAvailable(option.id), showUnavailable)
  );
  const compatibleShareSources = shareSources.filter((source) =>
    shareSourceCompatibleWithCapture(source, selectedCapture)
  );
  const sourcePickerCompatibleSources = sourcePickerSources.filter((source) =>
    shareSourceCompatibleWithCapture(source, selectedCapture)
  );
  const visibleSourcePickerSources =
    sourcePickerCompatibleSources.length > 0 ? sourcePickerCompatibleSources : compatibleShareSources;
  const selectedShareSource =
    compatibleShareSources.find((source) => shareSourceKey(source) === selectedShareSourceId) ??
    sourcePickerSources.find((source) => shareSourceKey(source) === selectedShareSourceId);
  const selectedWindowFromShareSource =
    selectedShareSource?.source_kind === "window"
      ? shareSourceToWindowTarget(selectedShareSource)
      : null;
  const selectedWindow =
    selectedWindowFromShareSource ??
    windowTargets.find((target) => windowTargetKey(target) === selectedWindowHwnd) ??
    sourcePickerSources
      .filter((source) => source.source_kind === "window")
      .map(shareSourceToWindowTarget)
      .find((target) => windowTargetKey(target) === selectedWindowHwnd);
  const performanceNote = capturePerformanceNote(selectedCapture, captureScope);
  const waitingForPortalFrame =
    isRunning &&
    selectedShareSource?.requires_system_picker &&
    !startError &&
    (metrics?.frame_count ?? 0) === 0;

  const applyWindowTargets = (targets: WindowCaptureTarget[]) => {
    setWindowTargets(targets);
    setSelectedWindowHwnd((current) => {
      if (current && targets.some((target) => windowTargetKey(target) === current)) {
        return current;
      }
      return targets[0] ? windowTargetKey(targets[0]) : null;
    });
  };

  const applyShareSource = (source: CaptureShareSourceTarget) => {
    setSelectedShareSourceId(shareSourceKey(source));
    if (source.source_kind === "window") {
      const target = shareSourceToWindowTarget(source);
      applyWindowTargets([target, ...windowTargets.filter((item) => windowTargetKey(item) !== windowTargetKey(target))]);
      setSelectedWindowHwnd(windowTargetKey(target));
      if (captureScope === "screen") setCaptureScope("window_perf");
      return;
    }

    setSelectedWindowHwnd(null);
    if (captureScope !== "screen") setCaptureScope("screen");
  };

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

  useEffect(() => {
    let cancelled = false;

    commands.testListCaptureShareSources().then((result) => {
      if (cancelled) return;
      if (result.ok && Array.isArray(result.value)) setShareSources(result.value);
      else setShareSources([]);
    });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!capabilities || captureAvailable(selectedCapture)) return;
    const nextCapture = CAPTURE_OPTIONS.find((option) => captureAvailable(option.id));
    if (nextCapture) setSelectedCapture(nextCapture.id);
  }, [capabilities, selectedCapture]);

  useEffect(() => {
    if (!selectedShareSourceId) return;
    const stillCompatible = compatibleShareSources.some(
      (source) => shareSourceKey(source) === selectedShareSourceId
    );
    if (!stillCompatible) {
      setSelectedShareSourceId(null);
      setSelectedWindowHwnd(null);
    }
  }, [compatibleShareSources, selectedShareSourceId]);

  useEffect(() => {
    if (!selectedWindowCapture && captureScope !== "screen") {
      setCaptureScope("screen");
    }
  }, [captureScope, selectedWindowCapture]);

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const [result, runResult] = await Promise.all([
        commands.testHarnessGetMetrics(),
        activeRunId ? commands.testGetRun(activeRunId) : Promise.resolve(null),
      ]);
      if (result.ok) {
        if (!result.value.is_running) {
          setIsRunning(false);
          setActiveRunId(null);
        }
        if (result.value.error_message) {
          setStartError(result.value.error_message);
        }
        const sourceWaitLatencyP95 =
          result.value.source_wait_latency_p95_ms ?? result.value.capture_latency_p95_ms;
        const interactiveLatencyP95 =
          result.value.interactive_latency_p95_ms ?? result.value.total_latency_p95_ms;
        setMetrics({
          is_running: result.value.is_running,
          capture_fps: result.value.capture_fps,
          frame_count: result.value.frame_count,
          dropped_frames: result.value.dropped_frames,
          resolution: result.value.resolution,
          capture_latency_avg_ms: result.value.capture_latency_avg_ms,
          source_wait_latency_p95_ms: sourceWaitLatencyP95,
          processing_latency_p95_ms: interactiveLatencyP95,
          capture_latency_p95_ms: result.value.capture_latency_p95_ms,
          encode_latency_p95_ms: result.value.encode_latency_p95_ms,
          decode_latency_p95_ms: result.value.decode_latency_p95_ms,
          total_latency_p95_ms: result.value.total_latency_p95_ms,
          error_message: result.value.error_message ?? null,
        });
      }

      if (runResult?.ok && runResult.value) {
        const run = runResult.value;
        if (run.status !== "running" && run.status !== "preparing" && run.status !== "queued") {
          setIsRunning(false);
          setActiveRunId(null);
          const errorMessage =
            run.summary?.error_message ??
            (run.status === "failed" ? "采集测试失败" : null);
          if (errorMessage) setStartError(errorMessage);
        }
      } else if (runResult && !runResult.ok) {
        setStartError(runResult.error.message);
      }
    }, 200);

    return () => clearInterval(interval);
  }, [activeRunId, isRunning]);

  useEffect(() => {
    if (!isWindowMode) return;

    let cancelled = false;
    setWindowTargetsLoading(true);
    setWindowTargetsError(null);

    commands.testListWindowCaptureTargets()
      .then((result) => {
        if (cancelled) return;
        if (result.ok) {
          applyWindowTargets(result.value);
        } else {
          setWindowTargets([]);
          setSelectedWindowHwnd(null);
          setWindowTargetsError(result.error.message);
        }
      })
      .finally(() => {
        if (!cancelled) setWindowTargetsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [isWindowMode]);

  const refreshWindowTargets = async () => {
    setWindowTargetsLoading(true);
    setWindowTargetsError(null);

    const result = await commands.testListWindowCaptureTargets();
    if (result.ok) {
      applyWindowTargets(result.value);
    } else {
      setWindowTargets([]);
      setSelectedWindowHwnd(null);
      setWindowTargetsError(result.error.message);
    }

    setWindowTargetsLoading(false);
  };

  const loadSourcePickerSources = async () => {
    setSourcePickerLoading(true);
    setSourcePickerError(null);

    const result = await commands.testListCaptureShareSourcesWithPreviews(24);
    if (result.ok && Array.isArray(result.value)) {
      setSourcePickerSources(result.value);
      setShareSources(result.value);
      const windowSources = result.value
        .filter((source) => source.source_kind === "window")
        .map(shareSourceToWindowTarget);
      if (windowSources.length > 0) applyWindowTargets(windowSources);
    } else {
      setSourcePickerSources((current) => (current.length > 0 ? current : shareSources));
      setSourcePickerError(result.ok ? "Invalid share source response" : result.error.message);
    }

    setSourcePickerLoading(false);
  };

  const openSourcePicker = (startAfterPick = false) => {
    setSourcePickerOpen(true);
    setStartAfterSourcePick(startAfterPick);
    setSourcePickerQuery("");
    setSourcePickerSources(shareSources);
    void loadSourcePickerSources();
  };

  const startCaptureRun = async (sourceOverride?: CaptureShareSourceTarget | null) => {
    if (!captureAvailable(selectedCapture)) {
      setStartError("当前平台未暴露所选采集能力。");
      return;
    }

    const sourceForRun = sourceOverride ?? selectedShareSource;
    const runWindowProbeMode = isWindowProbeMode && sourceForRun?.source_kind === "window";
    const runWindowPerfMode =
      isWindowPerfMode || (sourceForRun?.source_kind === "window" && !runWindowProbeMode);
    const runWindowMode = runWindowProbeMode || runWindowPerfMode;
    const selectedWindowForRun =
      sourceForRun?.source_kind === "window"
        ? shareSourceToWindowTarget(sourceForRun)
        : selectedWindow;
    const sourceConfig: Pick<TestConfig, "source_id" | "source_kind" | "display_id"> =
      sourceForRun
        ? {
            source_id: shareSourceKey(sourceForRun),
            source_kind: sourceForRun.source_kind,
            display_id:
              sourceForRun.source_kind === "screen" || sourceForRun.source_kind === "portal"
                ? sourceForRun.native_id
                : undefined,
          }
        : {};

    setIsRunning(true);
    setMetrics(null);
    setStartError(null);
    setSingleWindowProbeResult(null);

    if (runWindowMode && !selectedWindowForRun) {
      setStartError("请先选择一个可捕获窗口。");
      setIsRunning(false);
      return;
    }

    if (runWindowProbeMode) {
      const isMacos = selectedCapture === "macos";
      const result = await commands.testStartRun({
        scenarioId: "single_window.local",
        config: {
          capture_type: selectedCapture,
          input_source: "window",
          window_hwnd: selectedWindowForRun?.hwnd,
          window_title: selectedWindowForRun?.title,
          ...sourceConfig,
          encoder_type: "openh264",
          decoder_type: "software",
          renderer_type: isMacos ? "macos" : "d3d11",
          transport_kind: "webrtc",
          duration_ms: 1000,
        },
      });

      if (result.ok) {
        const run = await commands.testGetRun(result.value);
        const artifacts = await commands.testGetRunArtifacts(result.value);
        const artifactSummary = artifacts.ok
          ? summarizeWindowProbeArtifacts(artifacts.value)
          : null;
        if (run.ok && run.value?.summary) {
          if (run.value.status === "failed" && run.value.summary.error_message) {
            setSingleWindowProbeResult(run.value.summary.error_message);
          } else {
            setSingleWindowProbeResult(
              artifactSummary ??
                `Found ${run.value.summary.frame_count} window capture targets`
            );
          }
        } else {
          setSingleWindowProbeResult(`Probe run created: ${result.value}`);
        }
      } else {
        setSingleWindowProbeResult(result.error.message);
      }

      setIsRunning(false);
      return;
    }

    if (runWindowPerfMode) {
      const config: TestConfig = {
        capture_type: selectedCapture,
        encoder_type: "none",
        decoder_type: "none",
        duration_ms: CAPTURE_PERF_DURATION_MS,
        input_source: "window",
        window_hwnd: selectedWindowForRun?.hwnd,
        window_title: selectedWindowForRun?.title,
        ...sourceConfig,
        zero_copy: selectedCapture === "winrt",
        visual_preview: false,
      };
      const result = await commands.testStartRun({
        scenarioId: "custom",
        config,
      });

      if (result.ok) {
        setActiveRunId(result.value);
      } else {
        setStartError(result.error.message);
        setIsRunning(false);
      }
      return;
    }

    const config: TestConfig = {
      capture_type: selectedCapture,
      encoder_type: "none",
      decoder_type: "none",
      duration_ms: CAPTURE_PERF_DURATION_MS,
      input_source: "screen",
      ...sourceConfig,
      zero_copy: selectedCapture === "dxgi" || selectedCapture === "winrt",
      visual_preview: false,
    };
    const scenarioId =
      selectedCapture === "dxgi"
        ? "capture.dxgi"
        : selectedCapture === "winrt"
        ? "capture.winrt"
        : selectedCapture === "macos"
        ? "capture.macos"
        : selectedCapture === "linux"
        ? "capture.linux"
        : "custom";
    const result = await commands.testStartRun({
      scenarioId,
      config,
    });

    if (result.ok) {
      setActiveRunId(result.value);
    } else {
      setStartError(result.error.message);
      setIsRunning(false);
    }
  };

  const handleStart = async () => {
    if (!captureAvailable(selectedCapture)) {
      setStartError("当前平台未暴露所选采集能力。");
      return;
    }

    if (selectedCapture !== "synthetic" && !selectedShareSource && !(isWindowMode && selectedWindow)) {
      setStartError(null);
      openSourcePicker(true);
      return;
    }

    await startCaptureRun(selectedShareSource);
  };

  const handleStop = async () => {
    if (activeRunId) {
      await commands.testStopRun(activeRunId);
      setActiveRunId(null);
    } else {
      await commands.testHarnessStop();
    }
    setIsRunning(false);
  };

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Monitor className="h-6 w-6" />
          采集测试
        </h1>
        <p className="text-muted-foreground">
          测试不同捕获源的性能和稳定性
        </p>
      </div>

      {/* Capture Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择捕获源</h2>
        <div className="grid md:grid-cols-3 gap-4">
          {visibleCaptureOptions.map((option) => {
            const available = captureAvailable(option.id);
            const disabledLabel = unavailableText(capabilities, "available_captures", option.id);
            return (
            <button
              key={option.id}
              onClick={() => {
                setSelectedCapture(option.id);
                setCaptureScope("screen");
                setSelectedShareSourceId(null);
                setSelectedWindowHwnd(null);
              }}
              disabled={isRunning || !available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedCapture === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <h3 className="font-medium">{option.name}</h3>
              <p className="text-sm text-muted-foreground mt-1">{option.description}</p>
              <span className="inline-block mt-2 text-xs bg-muted px-2 py-0.5 rounded">
                {capabilityTag(option.id)}
              </span>
              {disabledLabel && (
                <span className="inline-block mt-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded">
                  {disabledLabel}
                </span>
              )}
            </button>
            );
          })}
        </div>
      </div>

      {/* Selected Option Details */}
      {selectedOption && (
        <div className="bg-card rounded-lg border p-4 mb-6">
          <h3 className="font-medium mb-2">测试配置</h3>
          <dl className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <dt className="text-muted-foreground">捕获源</dt>
              <dd className="font-medium">{selectedOption.name}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">共享源</dt>
              <dd className="font-medium">
                {selectedShareSource
                  ? `${shareSourceKindLabel(selectedShareSource)} / ${selectedShareSource.title}`
                  : selectedCapture === "synthetic"
                  ? "合成帧"
                  : "未选择"}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">测试模式</dt>
              <dd className="font-medium">{captureModeLabel(captureScope)} / 无 FPS 限制</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">分辨率</dt>
              <dd className="font-medium">
                {metrics?.resolution ? `${metrics.resolution[0]}x${metrics.resolution[1]}` : "自动"}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">内存路径</dt>
              <dd className="font-medium">
                {selectedCapture === "dxgi" || selectedCapture === "winrt" ? "D3D11 零拷贝" : "CPU"}
              </dd>
            </div>
          </dl>
          {performanceNote && (
            <div className="mt-4 rounded border border-yellow-200 bg-yellow-50 px-3 py-2 text-sm text-yellow-900 dark:border-yellow-900/40 dark:bg-yellow-950/30 dark:text-yellow-100">
              {performanceNote}
            </div>
          )}
        </div>
      )}

      {selectedCapture !== "synthetic" && captureAvailable(selectedCapture) && (
        <div className="bg-card rounded-lg border p-4 mb-6">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="font-medium">共享源</h3>
              <p className="text-sm text-muted-foreground">
                {selectedShareSource
                  ? selectedShareSource.subtitle
                  : "选择屏幕、窗口或平台授权入口。"}
              </p>
            </div>
            <button
              onClick={() => openSourcePicker(false)}
              disabled={isRunning}
              className="inline-flex items-center gap-2 rounded border px-3 py-1.5 text-sm hover:bg-muted disabled:opacity-50"
            >
              <Monitor className="h-4 w-4" />
              选择共享源
            </button>
          </div>
          {selectedShareSource && (
            <div className="mt-4 grid gap-4 md:grid-cols-[220px_1fr]">
              <ShareSourcePreviewThumb source={selectedShareSource} />
              <div className="min-w-0 space-y-2">
                <div>
                  <div className="truncate text-base font-medium">{selectedShareSource.title}</div>
                  <div className="text-sm text-muted-foreground">
                    {shareSourceKindLabel(selectedShareSource)}
                    {selectedShareSource.width > 0 && selectedShareSource.height > 0
                      ? ` / ${selectedShareSource.width}x${selectedShareSource.height}`
                      : selectedShareSource.requires_system_picker
                      ? " / 等待系统选择"
                      : ""}
                    {selectedShareSource.requires_system_picker ? " / requires OS approval" : ""}
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-3 text-sm">
                  <div>
                    <div className="text-muted-foreground">Platform</div>
                    <div className="truncate font-mono">{selectedShareSource.platform}</div>
                  </div>
                  <div>
                    <div className="text-muted-foreground">Source ID</div>
                    <div className="truncate font-mono">{selectedShareSource.native_id}</div>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {selectedWindowCapture && (
        <div className="bg-card rounded-lg border p-4 mb-6">
          <div className="flex items-center justify-between mb-3">
            <div>
              <h3 className="font-medium">采集范围</h3>
              <p className="text-sm text-muted-foreground">
                屏幕和单窗口性能都会持续采集；单窗口验证只检查选中窗口的首帧和媒体链路。
              </p>
            </div>
            <div className="flex items-center gap-2 rounded-lg border bg-muted/30 p-1">
              <button
                onClick={() => setCaptureScope("screen")}
                disabled={isRunning}
                className={`text-sm px-3 py-1.5 rounded disabled:opacity-50 ${
                  captureScope === "screen" ? "bg-background shadow-sm" : "hover:bg-muted"
                }`}
              >
                屏幕性能
              </button>
              <button
                onClick={() => setCaptureScope("window_perf")}
                disabled={isRunning}
                className={`text-sm px-3 py-1.5 rounded disabled:opacity-50 ${
                  captureScope === "window_perf" ? "bg-background shadow-sm" : "hover:bg-muted"
                }`}
              >
                单窗口性能
              </button>
              <button
                onClick={() => setCaptureScope("window_probe")}
                disabled={isRunning}
                className={`text-sm px-3 py-1.5 rounded disabled:opacity-50 ${
                  captureScope === "window_probe" ? "bg-background shadow-sm" : "hover:bg-muted"
                }`}
              >
                单窗口验证
              </button>
            </div>
          </div>

          {isWindowMode ? (
            <>
              <div className="mb-3 flex items-center gap-2">
                <button
                  onClick={refreshWindowTargets}
                  disabled={windowTargetsLoading || isRunning}
                  className="inline-flex items-center gap-2 text-sm px-3 py-1.5 rounded border hover:bg-muted disabled:opacity-50"
                >
                  <RefreshCw className="h-4 w-4" />
                  Refresh
                </button>
                <button
                  onClick={() => openSourcePicker(false)}
                  disabled={windowTargetsLoading || isRunning}
                  className="inline-flex items-center gap-2 text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                >
                  <Monitor className="h-4 w-4" />
                  Choose window
                </button>
              </div>

              {windowTargetsLoading && (
                <p className="text-sm text-muted-foreground">Loading windows...</p>
              )}
              {windowTargetsError && (
                <p className="text-sm text-red-600">{windowTargetsError}</p>
              )}
              {!windowTargetsLoading && !windowTargetsError && selectedWindow && (
                <div className="grid gap-4 md:grid-cols-[220px_1fr]">
                  <WindowPreviewThumb target={selectedWindow} />
                  <div className="min-w-0 space-y-2">
                    <div>
                      <div className="truncate text-base font-medium">{selectedWindow.title}</div>
                      <div className="text-sm text-muted-foreground">
                        {selectedWindow.width}x{selectedWindow.height} / PID{" "}
                        {selectedWindow.process_id}
                      </div>
                    </div>
                    <div className="grid grid-cols-2 gap-3 text-sm">
                      <div>
                        <div className="text-muted-foreground">
                          {selectedCapture === "macos" ? "Bundle" : "Class"}
                        </div>
                        <div className="truncate font-mono">{selectedWindow.class_name}</div>
                      </div>
                      <div>
                        <div className="text-muted-foreground">
                          {selectedCapture === "macos" ? "Window ID" : "HWND"}
                        </div>
                        <div className="truncate font-mono">{selectedWindow.hwnd}</div>
                      </div>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {windowTargets.length} windows available. Open the picker to refresh screenshots.
                    </div>
                  </div>
                </div>
              )}
              {!windowTargetsLoading && !windowTargetsError && !selectedWindow && (
                <div className="rounded border border-dashed p-4 text-sm text-muted-foreground">
                  No capturable window selected.
                </div>
              )}
              {isWindowProbeMode && singleWindowProbeResult && (
                <div className="mt-3 text-sm text-muted-foreground">
                  {singleWindowProbeResult}
                </div>
              )}
            </>
          ) : (
            <div className="rounded border border-dashed p-4 text-sm text-muted-foreground">
              {windowCaptureApiName(selectedCapture)} 将按屏幕采集性能测试运行，不会枚举或验证单窗口。
            </div>
          )}
        </div>
      )}

      {sourcePickerOpen && (
        <ShareSourcePickerDialog
          sources={visibleSourcePickerSources}
          selectedSourceId={selectedShareSourceId}
          captureApiName={windowCaptureApiName(selectedCapture)}
          loading={sourcePickerLoading}
          error={sourcePickerError}
          query={sourcePickerQuery}
          onQueryChange={setSourcePickerQuery}
          onRefresh={loadSourcePickerSources}
          onClose={() => {
            setSourcePickerOpen(false);
            setStartAfterSourcePick(false);
          }}
          onSelect={(source) => {
            applyShareSource(source);
            setSourcePickerOpen(false);
            if (startAfterSourcePick) {
              setStartAfterSourcePick(false);
              void startCaptureRun(source);
            }
          }}
        />
      )}

      {/* Control */}
      <div className="mb-6">
        {!isRunning ? (
          <button
            onClick={handleStart}
            disabled={!captureAvailable(selectedCapture)}
            className="flex items-center gap-2 px-6 py-3 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            <Play className="h-5 w-5" />
            启动测试
          </button>
        ) : (
          <button
            onClick={handleStop}
            className="flex items-center gap-2 px-6 py-3 bg-destructive text-destructive-foreground rounded-lg hover:bg-destructive/90 transition-colors"
          >
            <Square className="h-5 w-5" />
            停止测试
          </button>
        )}
      </div>
      {startError && <p className="text-sm text-red-600 mb-6">{startError}</p>}
      {waitingForPortalFrame && (
        <div className="mb-6 rounded border border-cyan-200 bg-cyan-50 px-3 py-2 text-sm text-cyan-900 dark:border-cyan-900/40 dark:bg-cyan-950/30 dark:text-cyan-100">
          正在等待系统屏幕共享授权或第一帧。请在系统弹窗中选择屏幕/窗口并点击共享；授权完成前 FPS 和分辨率会保持 0。
        </div>
      )}

      {/* Metrics */}
      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-6 gap-4 mb-6">
            <MetricCard
              icon={<Activity className="h-4 w-4" />}
              label="捕获帧率"
              value={`${metrics.capture_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.capture_fps)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="采集平均"
              value={`${metrics.capture_latency_avg_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.capture_latency_avg_ms, 16, 33)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="采集 P95"
              value={`${metrics.capture_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.capture_latency_p95_ms, 16, 33)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="源等待 P95"
              value={`${metrics.source_wait_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.source_wait_latency_p95_ms, 16, 33)}
            />
            <MetricCard
              icon={<Monitor className="h-4 w-4" />}
              label="总帧数"
              value={metrics.frame_count.toLocaleString()}
            />
            <MetricCard
              icon={<Activity className="h-4 w-4" />}
              label="丢帧"
              value={metrics.dropped_frames.toLocaleString()}
              highlight={metrics.dropped_frames > 0}
            />
          </div>

          {/* Latency Distribution */}
          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">延迟分布</h3>
            <div className="space-y-2">
              <LatencyBar label="Source wait" value={metrics.source_wait_latency_p95_ms} max={100} />
              <LatencyBar label="Processing" value={metrics.processing_latency_p95_ms} max={100} />
              <LatencyBar label="Encode" value={metrics.encode_latency_p95_ms} max={100} />
              <LatencyBar label="Decode" value={metrics.decode_latency_p95_ms} max={100} />
              <LatencyBar label="Total" value={metrics.total_latency_p95_ms} max={100} />
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function ShareSourcePickerDialog({
  sources,
  selectedSourceId,
  captureApiName,
  loading,
  error,
  query,
  onQueryChange,
  onRefresh,
  onClose,
  onSelect,
}: {
  sources: CaptureShareSourceTarget[];
  selectedSourceId: string | null;
  captureApiName: string;
  loading: boolean;
  error: string | null;
  query: string;
  onQueryChange: (query: string) => void;
  onRefresh: () => void;
  onClose: () => void;
  onSelect: (source: CaptureShareSourceTarget) => void;
}) {
  const normalizedQuery = query.trim().toLowerCase();
  const filteredSources = normalizedQuery
    ? sources.filter((source) =>
        `${source.title} ${source.subtitle} ${source.class_name ?? ""} ${source.app_name ?? ""} ${
          source.bundle_identifier ?? ""
        } ${source.process_id ?? ""} ${source.native_id}`
          .toLowerCase()
          .includes(normalizedQuery)
      )
    : sources;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-6"
      role="dialog"
      aria-modal="true"
      aria-labelledby="share-source-picker-title"
      onClick={onClose}
    >
      <div
        className="flex max-h-[86vh] w-full max-w-5xl flex-col overflow-hidden rounded-lg border bg-background shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b p-4">
          <div>
            <h2 id="share-source-picker-title" className="text-lg font-semibold">
              Share source picker
            </h2>
            <p className="text-sm text-muted-foreground">
              Select a screen, window, or platform permission entry for {captureApiName}.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={onRefresh}
              disabled={loading}
              className="inline-flex items-center gap-2 rounded border px-3 py-1.5 text-sm hover:bg-muted disabled:opacity-50"
            >
              <RefreshCw className="h-4 w-4" />
              Refresh
            </button>
            <button
              onClick={onClose}
              className="inline-flex h-9 w-9 items-center justify-center rounded border hover:bg-muted"
              aria-label="Close share source picker"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        <div className="border-b p-4">
          <label className="relative block">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              value={query}
              onChange={(event) => onQueryChange(event.target.value)}
              className="w-full rounded border bg-background py-2 pl-9 pr-3 text-sm"
              placeholder="Filter by title, app, source, or PID"
              aria-label="Filter share sources"
            />
          </label>
          {error && <div className="mt-2 text-sm text-red-600">{error}</div>}
        </div>

        <div className="min-h-[280px] overflow-y-auto p-4">
          {loading && sources.length === 0 && (
            <div className="flex h-48 items-center justify-center text-sm text-muted-foreground">
              Loading share sources...
            </div>
          )}

          {!loading && filteredSources.length === 0 && (
            <div className="flex h-48 items-center justify-center text-sm text-muted-foreground">
              No matching sources.
            </div>
          )}

          {filteredSources.length > 0 && (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {filteredSources.map((source) => {
                const selected = shareSourceKey(source) === selectedSourceId;
                return (
                  <button
                    key={shareSourceKey(source)}
                    onClick={() => onSelect(source)}
                    className={`rounded-lg border p-3 text-left transition hover:border-primary hover:bg-primary/5 ${
                      selected ? "border-primary bg-primary/10" : ""
                    }`}
                    aria-label={`Select ${source.title}`}
                  >
                    <ShareSourcePreviewThumb source={source} />
                    <div className="mt-3 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate font-medium">{source.title}</span>
                        <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                          {shareSourceKindLabel(source)}
                        </span>
                      </div>
                      <div className="mt-1 text-xs text-muted-foreground">
                        {source.subtitle}
                      </div>
                      {source.class_name && (
                        <div className="mt-1 truncate font-mono text-xs text-muted-foreground">
                          {source.class_name}
                        </div>
                      )}
                      {source.app_name && source.app_name !== source.title && (
                        <div className="mt-1 truncate text-xs text-muted-foreground">
                          {source.app_name}
                        </div>
                      )}
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ShareSourcePreviewThumb({ source }: { source: CaptureShareSourceTarget }) {
  return (
    <div className="flex aspect-video w-full items-center justify-center overflow-hidden rounded border bg-muted">
      {source.preview_data_url ? (
        <img
          src={source.preview_data_url}
          alt=""
          className="h-full w-full object-contain"
          draggable={false}
        />
      ) : (
        <div className="flex flex-col items-center gap-2 text-muted-foreground">
          {source.source_kind === "screen" || source.source_kind === "portal" ? (
            <Monitor className="h-5 w-5" />
          ) : (
            <ImageOff className="h-5 w-5" />
          )}
          <span className="text-xs">{shareSourceKindLabel(source)}</span>
        </div>
      )}
    </div>
  );
}

function WindowPreviewThumb({ target }: { target: WindowCaptureTarget }) {
  return (
    <div className="flex aspect-video w-full items-center justify-center overflow-hidden rounded border bg-muted">
      {target.preview_data_url ? (
        <img
          src={target.preview_data_url}
          alt=""
          className="h-full w-full object-contain"
          draggable={false}
        />
      ) : (
        <div className="flex flex-col items-center gap-2 text-muted-foreground">
          <ImageOff className="h-5 w-5" />
          <span className="text-xs">No preview</span>
        </div>
      )}
    </div>
  );
}

function MetricCard({
  icon,
  label,
  value,
  color = "text-foreground",
  highlight = false,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
  color?: string;
  highlight?: boolean;
}) {
  return (
    <div
      className={`bg-card rounded-lg p-4 border ${
        highlight ? "border-red-500 bg-red-50 dark:bg-red-900/10" : ""
      }`}
    >
      <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
        {icon}
        <span>{label}</span>
      </div>
      <div className={`text-xl font-semibold ${color}`}>{value}</div>
    </div>
  );
}

function LatencyBar({
  label,
  value,
  max,
}: {
  label: string;
  value: number;
  max: number;
}) {
  const percent = Math.min((value / max) * 100, 100);
  const color = value <= 5 ? "bg-green-500" : value <= 15 ? "bg-yellow-500" : "bg-red-500";

  return (
    <div className="flex items-center gap-3">
      <span className="w-12 text-sm text-muted-foreground">{label}</span>
      <div className="flex-1 h-6 bg-gray-200 dark:bg-gray-700 rounded overflow-hidden">
        <div
          className={`h-full ${color} transition-all duration-300`}
          style={{ width: `${percent}%` }}
        />
      </div>
      <span className="w-20 text-sm text-right font-mono">{value.toFixed(2)} ms</span>
    </div>
  );
}

function getFpsColor(fps: number): string {
  if (fps >= 55) return "text-green-500";
  if (fps >= 30) return "text-yellow-500";
  return "text-red-500";
}

function getLatencyColor(ms: number, good: number, warning: number): string {
  if (ms <= good) return "text-green-500";
  if (ms <= warning) return "text-yellow-500";
  return "text-red-500";
}
