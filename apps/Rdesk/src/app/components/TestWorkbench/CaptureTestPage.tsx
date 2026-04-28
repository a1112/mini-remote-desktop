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
import type { Artifact, WindowCaptureTarget } from "../../adapters/tauri/types";

type CaptureType = "dxgi" | "winrt" | "synthetic";

interface CaptureOption {
  id: CaptureType;
  name: string;
  description: string;
  available: boolean;
}

const CAPTURE_OPTIONS: CaptureOption[] = [
  {
    id: "dxgi",
    name: "DXGI Desktop Duplication",
    description: "高性能桌面捕获，支持 Windows 8+",
    available: true,
  },
  {
    id: "winrt",
    name: "Windows Runtime Capture",
    description: "现代化屏幕捕获 API，支持窗口捕获",
    available: true,
  },
  {
    id: "synthetic",
    name: "合成测试模式",
    description: "生成合成测试图案，用于基准测试",
    available: true,
  },
];

interface CaptureMetrics {
  is_running: boolean;
  capture_fps: number;
  frame_count: number;
  dropped_frames: number;
  resolution: [number, number];
  avg_latency_ms: number;
  capture_latency_p95_ms: number;
  encode_latency_p95_ms: number;
  decode_latency_p95_ms: number;
  total_latency_p95_ms: number;
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

export function CaptureTestPage() {
  const [selectedCapture, setSelectedCapture] = useState<CaptureType>("dxgi");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<CaptureMetrics | null>(null);
  const [windowTargets, setWindowTargets] = useState<WindowCaptureTarget[]>([]);
  const [selectedWindowHwnd, setSelectedWindowHwnd] = useState<string | null>(null);
  const [windowTargetsLoading, setWindowTargetsLoading] = useState(false);
  const [windowTargetsError, setWindowTargetsError] = useState<string | null>(null);
  const [singleWindowProbeResult, setSingleWindowProbeResult] = useState<string | null>(null);
  const [windowPickerOpen, setWindowPickerOpen] = useState(false);
  const [windowPickerTargets, setWindowPickerTargets] = useState<WindowCaptureTarget[]>([]);
  const [windowPickerLoading, setWindowPickerLoading] = useState(false);
  const [windowPickerError, setWindowPickerError] = useState<string | null>(null);
  const [windowPickerQuery, setWindowPickerQuery] = useState("");

  const selectedOption = CAPTURE_OPTIONS.find((o) => o.id === selectedCapture);
  const selectedWindow =
    windowTargets.find((target) => target.hwnd === selectedWindowHwnd) ??
    windowPickerTargets.find((target) => target.hwnd === selectedWindowHwnd);

  const applyWindowTargets = (targets: WindowCaptureTarget[]) => {
    setWindowTargets(targets);
    setSelectedWindowHwnd((current) => {
      if (current && targets.some((target) => target.hwnd === current)) {
        return current;
      }
      return targets[0]?.hwnd ?? null;
    });
  };

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      // Get metrics from test harness
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        setMetrics({
          is_running: result.value.is_running,
          capture_fps: result.value.capture_fps,
          frame_count: result.value.frame_count,
          dropped_frames: result.value.dropped_frames,
          resolution: result.value.resolution,
          avg_latency_ms: result.value.total_latency_p50_ms,
          capture_latency_p95_ms: result.value.capture_latency_p95_ms,
          encode_latency_p95_ms: result.value.encode_latency_p95_ms,
          decode_latency_p95_ms: result.value.decode_latency_p95_ms,
          total_latency_p95_ms: result.value.total_latency_p95_ms,
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  useEffect(() => {
    if (selectedCapture !== "winrt") return;

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
  }, [selectedCapture]);

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

  const loadWindowPickerTargets = async () => {
    setWindowPickerLoading(true);
    setWindowPickerError(null);

    const result = await commands.testListWindowCaptureTargetsWithPreviews(24);
    if (result.ok) {
      setWindowPickerTargets(result.value);
      applyWindowTargets(result.value);
    } else {
      setWindowPickerTargets((current) => (current.length > 0 ? current : windowTargets));
      setWindowPickerError(result.error.message);
    }

    setWindowPickerLoading(false);
  };

  const openWindowPicker = () => {
    setWindowPickerOpen(true);
    setWindowPickerQuery("");
    setWindowPickerTargets(windowTargets);
    void loadWindowPickerTargets();
  };

  const handleStart = async () => {
    setIsRunning(true);
    setMetrics(null);
    setSingleWindowProbeResult(null);

    if (selectedCapture === "winrt") {
      const selectedWindow = windowTargets.find((target) => target.hwnd === selectedWindowHwnd);
      const result = await commands.testStartRun({
        scenarioId: "single_window.local",
        config: {
          capture_type: "winrt",
          input_source: "window",
          window_hwnd: selectedWindow?.hwnd,
          window_title: selectedWindow?.title,
          encoder_type: "openh264",
          decoder_type: "software",
          renderer_type: "d3d11",
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

    // Start with appropriate chain
    const chain = selectedCapture === "synthetic" ? "nvenc_only" : "capture_only";
    await commands.testHarnessStart(chain);
  };

  const handleStop = async () => {
    await commands.testHarnessStop();
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
          {CAPTURE_OPTIONS.map((option) => (
            <button
              key={option.id}
              onClick={() => setSelectedCapture(option.id)}
              disabled={isRunning || !option.available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedCapture === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!option.available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <h3 className="font-medium">{option.name}</h3>
              <p className="text-sm text-muted-foreground mt-1">{option.description}</p>
              {!option.available && (
                <span className="inline-block mt-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded">
                  即将推出
                </span>
              )}
            </button>
          ))}
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
              <dt className="text-muted-foreground">测试时长</dt>
              <dd className="font-medium">30 秒</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">分辨率</dt>
              <dd className="font-medium">
                {metrics?.resolution ? `${metrics.resolution[0]}x${metrics.resolution[1]}` : "自动"}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">目标帧率</dt>
              <dd className="font-medium">60 FPS</dd>
            </div>
          </dl>
        </div>
      )}

      {selectedCapture === "winrt" && (
        <div className="bg-card rounded-lg border p-4 mb-6">
          <div className="flex items-center justify-between mb-3">
            <div>
              <h3 className="font-medium">Single window capture</h3>
              <p className="text-sm text-muted-foreground">
                Enumerate foreground windows and pick a WinRT capture target.
              </p>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={refreshWindowTargets}
                disabled={windowTargetsLoading || isRunning}
                className="inline-flex items-center gap-2 text-sm px-3 py-1.5 rounded border hover:bg-muted disabled:opacity-50"
              >
                <RefreshCw className="h-4 w-4" />
                Refresh
              </button>
              <button
                onClick={openWindowPicker}
                disabled={windowTargetsLoading || isRunning}
                className="inline-flex items-center gap-2 text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                <Monitor className="h-4 w-4" />
                Choose window
              </button>
            </div>
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
                    <div className="text-muted-foreground">Class</div>
                    <div className="truncate font-mono">{selectedWindow.class_name}</div>
                  </div>
                  <div>
                    <div className="text-muted-foreground">HWND</div>
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
          {singleWindowProbeResult && (
            <div className="mt-3 text-sm text-muted-foreground">
              {singleWindowProbeResult}
            </div>
          )}
        </div>
      )}

      {windowPickerOpen && (
        <WindowPickerDialog
          targets={windowPickerTargets}
          selectedHwnd={selectedWindowHwnd}
          loading={windowPickerLoading}
          error={windowPickerError}
          query={windowPickerQuery}
          onQueryChange={setWindowPickerQuery}
          onRefresh={loadWindowPickerTargets}
          onClose={() => setWindowPickerOpen(false)}
          onSelect={(target) => {
            applyWindowTargets(
              windowPickerTargets.some((item) => item.hwnd === target.hwnd)
                ? windowPickerTargets
                : [target, ...windowPickerTargets]
            );
            setSelectedWindowHwnd(target.hwnd);
            setWindowPickerOpen(false);
          }}
        />
      )}

      {/* Control */}
      <div className="mb-6">
        {!isRunning ? (
          <button
            onClick={handleStart}
            disabled={selectedCapture === "winrt" && (!selectedWindowHwnd || windowTargetsLoading)}
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

      {/* Metrics */}
      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-5 gap-4 mb-6">
            <MetricCard
              icon={<Activity className="h-4 w-4" />}
              label="捕获帧率"
              value={`${metrics.capture_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.capture_fps)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="平均延迟"
              value={`${metrics.avg_latency_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.avg_latency_ms, 16, 33)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="Pipeline P95"
              value={`${metrics.total_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.total_latency_p95_ms, 16, 33)}
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
              <LatencyBar label="Capture" value={metrics.capture_latency_p95_ms} max={100} />
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

function WindowPickerDialog({
  targets,
  selectedHwnd,
  loading,
  error,
  query,
  onQueryChange,
  onRefresh,
  onClose,
  onSelect,
}: {
  targets: WindowCaptureTarget[];
  selectedHwnd: string | null;
  loading: boolean;
  error: string | null;
  query: string;
  onQueryChange: (query: string) => void;
  onRefresh: () => void;
  onClose: () => void;
  onSelect: (target: WindowCaptureTarget) => void;
}) {
  const normalizedQuery = query.trim().toLowerCase();
  const filteredTargets = normalizedQuery
    ? targets.filter((target) =>
        `${target.title} ${target.class_name} ${target.process_id}`
          .toLowerCase()
          .includes(normalizedQuery)
      )
    : targets;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-6"
      role="dialog"
      aria-modal="true"
      aria-labelledby="window-picker-title"
      onClick={onClose}
    >
      <div
        className="flex max-h-[86vh] w-full max-w-5xl flex-col overflow-hidden rounded-lg border bg-background shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b p-4">
          <div>
            <h2 id="window-picker-title" className="text-lg font-semibold">
              Window picker
            </h2>
            <p className="text-sm text-muted-foreground">
              Select a foreground window using live WinRT preview frames.
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
              aria-label="Close window picker"
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
              placeholder="Filter by title, class, or PID"
              aria-label="Filter windows"
            />
          </label>
          {error && <div className="mt-2 text-sm text-red-600">{error}</div>}
        </div>

        <div className="min-h-[280px] overflow-y-auto p-4">
          {loading && targets.length === 0 && (
            <div className="flex h-48 items-center justify-center text-sm text-muted-foreground">
              Loading window previews...
            </div>
          )}

          {!loading && filteredTargets.length === 0 && (
            <div className="flex h-48 items-center justify-center text-sm text-muted-foreground">
              No matching windows.
            </div>
          )}

          {filteredTargets.length > 0 && (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {filteredTargets.map((target) => {
                const selected = target.hwnd === selectedHwnd;
                return (
                  <button
                    key={target.hwnd}
                    onClick={() => onSelect(target)}
                    className={`rounded-lg border p-3 text-left transition hover:border-primary hover:bg-primary/5 ${
                      selected ? "border-primary bg-primary/10" : ""
                    }`}
                    aria-label={`Select ${target.title}`}
                  >
                    <WindowPreviewThumb target={target} />
                    <div className="mt-3 min-w-0">
                      <div className="truncate font-medium">{target.title}</div>
                      <div className="mt-1 text-xs text-muted-foreground">
                        {target.width}x{target.height} / PID {target.process_id}
                      </div>
                      <div className="mt-1 truncate font-mono text-xs text-muted-foreground">
                        {target.class_name}
                      </div>
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
