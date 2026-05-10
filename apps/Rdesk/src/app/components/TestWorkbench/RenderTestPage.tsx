import { useCallback, useEffect, useRef, useState } from "react";
import { Play, Square, Monitor, Palette, Layers, ImageOff } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot, FrameData, MetricSeries, TestConfig } from "../../adapters/tauri/types";
import { capabilityAvailable, capabilityTag, chooseCapability, unavailableText } from "./capabilityMeta";

type RendererType = "d3d11" | "macos" | "linux" | "d3d12" | "opengl" | "webview";

interface RendererOption {
  id: RendererType;
  name: string;
  description: string;
}

const RENDERER_OPTIONS: RendererOption[] = [
  {
    id: "d3d11",
    name: "Direct3D 11",
    description: "Windows 标准渲染 API，兼容性最佳",
  },
  {
    id: "macos",
    name: "Metal",
    description: "macOS 原生 Metal 渲染器",
  },
  {
    id: "linux",
    name: "Linux",
    description: "Linux 原生窗口渲染器",
  },
  {
    id: "d3d12",
    name: "Direct3D 12",
    description: "低级 API，更低 CPU 开销",
  },
  {
    id: "opengl",
    name: "OpenGL",
    description: "跨平台渲染 API",
  },
  {
    id: "webview",
    name: "WebView",
    description: "嵌入实时画面区域的浏览器合成测试",
  },
];

const RENDER_TEST_POLL_MS = 500;

function isRendererAvailable(
  capabilities: EnvironmentSnapshot | null,
  renderer: RendererType
) {
  return capabilityAvailable(capabilities, "available_renderers", renderer);
}

function isIndependentRenderProbe(renderer: RendererType): boolean {
  return renderer === "d3d12" || renderer === "opengl";
}

function isWebViewRenderer(renderer: RendererType): boolean {
  return renderer === "webview";
}

function latestMetric(series: Record<string, MetricSeries> | null | undefined, name: string): number {
  const samples = series?.[name]?.samples ?? [];
  return samples[samples.length - 1]?.value ?? 0;
}

interface RenderMetrics {
  is_running: boolean;
  render_fps: number;
  frame_time_ms: number;
  gpu_frame_time_ms: number;
  capture_latency_p95_ms: number;
  transport_latency_p95_ms: number;
  decode_latency_p95_ms: number;
  draw_calls: number;
  triangles: number;
  textures: number;
  resolution: [number, number];
}

interface WebViewRenderSample {
  fps: number;
  averageFrameTimeMs: number;
  p95FrameTimeMs: number;
  frameCount: number;
}

export function RenderTestPage() {
  const [selectedRenderer, setSelectedRenderer] = useState<RendererType>("d3d11");
  const [selectedMode, setSelectedMode] = useState<"video" | "animation" | "static">("video");
  const [selectedResolution, setSelectedResolution] = useState("1920x1080");
  const [isRunning, setIsRunning] = useState(false);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [currentRunUsesProbe, setCurrentRunUsesProbe] = useState(false);
  const [currentRunUsesWebView, setCurrentRunUsesWebView] = useState(false);
  const [webViewStartedAt, setWebViewStartedAt] = useState<number | null>(null);
  const [webViewDurationMs, setWebViewDurationMs] = useState(0);
  const [metrics, setMetrics] = useState<RenderMetrics | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [previewFrame, setPreviewFrame] = useState<FrameData | null>(null);
  const lastCapturedGenerationRef = useRef<number | undefined>(undefined);
  const lastRenderedGenerationRef = useRef<number | undefined>(undefined);
  const frameRequestInFlightRef = useRef(false);

  const selectedOption = RENDERER_OPTIONS.find((o) => o.id === selectedRenderer);
  const selectedAvailable = selectedOption
    ? isRendererAvailable(capabilities, selectedOption.id)
    : false;

  const RENDER_MODES = [
    { id: "video", name: "视频流", desc: "连续帧渲染" },
    { id: "animation", name: "动画测试", desc: "高帧率动画" },
    { id: "static", name: "静态画面", desc: "单帧渲染" },
  ];

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
    if (!capabilities || isRendererAvailable(capabilities, selectedRenderer)) return;
    const nextRenderer = RENDERER_OPTIONS.find((option) =>
      isRendererAvailable(capabilities, option.id)
    );
    if (nextRenderer) setSelectedRenderer(nextRenderer.id);
  }, [capabilities, selectedRenderer]);

  useEffect(() => {
    if (!isRunning) return;

    if (currentRunUsesWebView && webViewStartedAt !== null) {
      const elapsedMs = Math.max(Date.now() - webViewStartedAt, 0);
      const remainingMs = Math.max(webViewDurationMs - elapsedMs, 0);
      const timeout = window.setTimeout(() => {
        setIsRunning(false);
        setCurrentRunUsesWebView(false);
      }, remainingMs);

      return () => window.clearTimeout(timeout);
    }

    const interval = setInterval(async () => {
      if (currentRunUsesProbe && currentRunId) {
        const metricsResult = await commands.testGetRunMetrics(currentRunId);
        if (metricsResult.ok) {
          const series = metricsResult.value;
          const fps = latestMetric(series, "render_fps");
          const p95 = latestMetric(series, "render_frame_time_p95_ms");
          setMetrics({
            is_running: true,
            render_fps: fps,
            frame_time_ms: latestMetric(series, "render_frame_time_ms"),
            gpu_frame_time_ms: p95,
            capture_latency_p95_ms: 0,
            transport_latency_p95_ms: 0,
            decode_latency_p95_ms: 0,
            draw_calls: latestMetric(series, "draw_calls"),
            triangles: latestMetric(series, "triangles"),
            textures: latestMetric(series, "textures"),
            resolution: [
              latestMetric(series, "render_width"),
              latestMetric(series, "render_height"),
            ],
          });
        }
      } else {
        const result = await commands.testHarnessGetMetrics();
        if (result.ok && result.value) {
          const fps = result.value.capture_fps;
          setMetrics({
            is_running: result.value.is_running,
            render_fps: fps,
            frame_time_ms: fps > 0 ? 1000 / fps : 0,
            gpu_frame_time_ms: result.value.total_latency_p95_ms,
            capture_latency_p95_ms: result.value.capture_latency_p95_ms,
            transport_latency_p95_ms: result.value.transport_latency_p95_ms,
            decode_latency_p95_ms: result.value.decode_latency_p95_ms,
            draw_calls: result.value.frame_count,
            triangles: result.value.frame_count * 2,
            textures: result.value.frame_count > 0 ? 1 : 0,
            resolution: result.value.resolution,
          });
        }

        if (!frameRequestInFlightRef.current) {
          frameRequestInFlightRef.current = true;
          const framesResult = await commands.testHarnessGetFrames({
            includeCaptured: false,
            includeRendered: true,
            lastCapturedGeneration: lastCapturedGenerationRef.current,
            lastRenderedGeneration: lastRenderedGenerationRef.current,
          });
          frameRequestInFlightRef.current = false;

          if (framesResult.ok) {
            const [capturedFrame, renderedFrame] = framesResult.value;
            const nextFrame = renderedFrame ?? capturedFrame ?? null;
            if (renderedFrame) lastRenderedGenerationRef.current = renderedFrame[3];
            if (capturedFrame) lastCapturedGenerationRef.current = capturedFrame[3];
            if (nextFrame) setPreviewFrame(nextFrame);
          }
        }
      }

      if (currentRunId) {
        const runResult = await commands.testGetRun(currentRunId);
        if (
          runResult.ok &&
          runResult.value &&
          runResult.value.status !== "queued" &&
          runResult.value.status !== "preparing" &&
          runResult.value.status !== "running"
        ) {
          setIsRunning(false);
        }
      }
    }, RENDER_TEST_POLL_MS);

    return () => clearInterval(interval);
  }, [
    currentRunId,
    currentRunUsesProbe,
    currentRunUsesWebView,
    isRunning,
    selectedMode,
    selectedResolution,
    webViewDurationMs,
    webViewStartedAt,
  ]);

  const handleWebViewRenderSample = useCallback((sample: WebViewRenderSample) => {
    setMetrics({
      is_running: true,
      render_fps: sample.fps,
      frame_time_ms: sample.averageFrameTimeMs,
      gpu_frame_time_ms: sample.p95FrameTimeMs,
      capture_latency_p95_ms: 0,
      transport_latency_p95_ms: 0,
      decode_latency_p95_ms: 0,
      draw_calls: sample.frameCount,
      triangles: sample.frameCount * 2,
      textures: 4,
      resolution: selectedResolution.split("x").map(Number) as [number, number],
    });
  }, [selectedResolution]);

  const handleStart = async () => {
    if (!selectedOption || !selectedAvailable) {
      setStartError("当前平台未暴露所选渲染器能力。");
      return;
    }

    setMetrics(null);
    setPreviewFrame(null);
    lastCapturedGenerationRef.current = undefined;
    lastRenderedGenerationRef.current = undefined;
    frameRequestInFlightRef.current = false;
    setStartError(null);
    setCurrentRunId(null);
    setCurrentRunUsesProbe(false);
    setCurrentRunUsesWebView(false);
    setWebViewStartedAt(null);
    setWebViewDurationMs(0);

    const [width, height] = selectedResolution.split("x").map(Number) as [number, number];
    const fps = selectedMode === "static" ? 30 : selectedMode === "animation" ? 144 : 60;
    const durationMs =
      selectedMode === "static" ? 3_000 : selectedMode === "animation" ? 15_000 : 30_000;
    const usesProbe = isIndependentRenderProbe(selectedRenderer);
    const usesWebView = isWebViewRenderer(selectedRenderer);

    if (usesWebView) {
      setMetrics({
        is_running: true,
        render_fps: 0,
        frame_time_ms: 0,
        gpu_frame_time_ms: 0,
        capture_latency_p95_ms: 0,
        transport_latency_p95_ms: 0,
        decode_latency_p95_ms: 0,
        draw_calls: 0,
        triangles: 0,
        textures: 4,
        resolution: [width, height],
      });
      setCurrentRunUsesWebView(true);
      setWebViewStartedAt(Date.now());
      setWebViewDurationMs(durationMs);
      setIsRunning(true);
      return;
    }

    const scenarioId = usesProbe ? "render.probe" : "custom";
    const directCaptureCandidates: Array<NonNullable<TestConfig["capture_type"]>> =
      selectedRenderer === "macos"
        ? ["macos", "synthetic"]
        : selectedRenderer === "linux"
          ? ["linux", "synthetic"]
          : ["dxgi", "synthetic"];
    const syntheticCaptureCandidates: Array<NonNullable<TestConfig["capture_type"]>> = [
      "synthetic",
      ...directCaptureCandidates,
    ];
    const capture = chooseCapability(
      selectedMode === "video" ? directCaptureCandidates : syntheticCaptureCandidates,
      capabilities,
      "available_captures",
      "synthetic"
    );
    const config: TestConfig = {
      capture_type: usesProbe ? "synthetic" : capture,
      encoder_type: "none",
      decoder_type: "none",
      renderer_type: selectedRenderer,
      render_display: true,
      transport_kind: "loopback",
      resolution: [width, height],
      fps,
      bitrate: 8_000_000,
      duration_ms: durationMs,
      warmup_ms: 500,
      input_source: usesProbe || capture === "synthetic" ? "synthetic" : "screen",
      output_validation: true,
      visual_preview: true,
    };

    const result = await commands.testStartRun({
      scenarioId,
      config,
    });

    if (result.ok) {
      setCurrentRunId(result.value);
      setCurrentRunUsesProbe(usesProbe);
      setIsRunning(true);
    } else {
      setStartError(result.error.message);
    }
  };

  const handleStop = async () => {
    if (currentRunUsesWebView) {
      // WebView rendering is local to this page and does not own a backend run.
    } else if (currentRunId) {
      await commands.testStopRun(currentRunId);
    } else {
      await commands.testHarnessStop();
    }
    setIsRunning(false);
    setCurrentRunId(null);
    setCurrentRunUsesProbe(false);
    setCurrentRunUsesWebView(false);
    setWebViewStartedAt(null);
    setWebViewDurationMs(0);
  };

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Palette className="h-6 w-6" />
          渲染测试
        </h1>
        <p className="text-muted-foreground">
          测试当前平台可用的原生渲染器性能和帧率
        </p>
      </div>

      {/* Renderer Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择渲染器</h2>
        <div className="grid md:grid-cols-3 gap-4">
          {RENDERER_OPTIONS.map((option) => {
            const available = isRendererAvailable(capabilities, option.id);
            const disabledLabel = unavailableText(capabilities, "available_renderers", option.id);
            return (
            <button
              key={option.id}
              onClick={() => setSelectedRenderer(option.id)}
              disabled={isRunning || !available}
              aria-pressed={selectedRenderer === option.id}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedRenderer === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center gap-2 mb-2">
                <Monitor className="h-5 w-5 text-blue-500" />
                <span className="font-medium">{option.name}</span>
              </div>
              <p className="text-sm text-muted-foreground">{option.description}</p>
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

      {/* Render Mode Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">渲染模式</h3>
        <div className="grid md:grid-cols-3 gap-3">
          {RENDER_MODES.map((mode) => (
            <button
              key={mode.id}
              onClick={() => setSelectedMode(mode.id as any)}
              disabled={isRunning}
              className={`px-4 py-3 rounded-lg border text-sm text-left transition-all ${
                selectedMode === mode.id
                  ? "bg-primary text-primary-foreground border-primary"
                  : "bg-background hover:bg-muted"
              }`}
            >
              <div className="font-medium">{mode.name}</div>
              <div className="text-xs opacity-70">{mode.desc}</div>
            </button>
          ))}
        </div>
      </div>

      {/* Resolution Options */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">输出分辨率</h3>
        <div className="flex gap-2 flex-wrap">
          {["1280x720", "1920x1080", "2560x1440", "3840x2160"].map((res) => (
            <button
              key={res}
              onClick={() => setSelectedResolution(res)}
              disabled={isRunning}
              className={`px-3 py-1 rounded border text-sm ${
                selectedResolution === res
                  ? "bg-primary text-primary-foreground border-primary"
                  : "bg-background hover:bg-muted"
              }`}
            >
              {res === "1280x720"
                ? "720p"
                : res === "1920x1080"
                ? "1080p"
                : res === "2560x1440"
                ? "1440p"
                : "4K"}
            </button>
          ))}
        </div>
      </div>

      {/* Control */}
      <div className="mb-6">
        {!isRunning ? (
          <button
            onClick={handleStart}
            disabled={!selectedAvailable}
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
      {startError && (
        <p className="text-sm text-red-600 mb-6">{startError}</p>
      )}

      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">实时画面</h3>
        <div className="aspect-video rounded bg-black flex items-center justify-center overflow-hidden">
          {currentRunUsesWebView && isRunning ? (
            <WebViewRenderPreview mode={selectedMode} onSample={handleWebViewRenderSample} />
          ) : previewFrame ? (
            <img
              src={`data:image/png;base64,${previewFrame[0]}`}
              alt="Render preview"
              className="h-full w-full object-contain"
            />
          ) : (
            <div className="text-center text-sm text-muted-foreground">
              <ImageOff className="mx-auto mb-2 h-8 w-8" />
              {isRunning ? "等待渲染帧..." : "启动测试后显示渲染输入帧"}
            </div>
          )}
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          {isWebViewRenderer(selectedRenderer)
            ? "WebView 测试直接嵌套在实时画面区域，覆盖浏览器合成、CSS 动画和页面绘制路径。"
            : isIndependentRenderProbe(selectedRenderer)
            ? "D3D12/OpenGL 当前执行独立渲染 probe；启动后会弹出原生渲染窗口，指标来自后端可见窗口渲染循环。"
            : "预览来自测试 harness 的最新渲染输入帧；原生渲染器仍按所选后端执行上传/呈现链路。"}
        </p>
      </div>

      {/* Metrics */}
      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
            <MetricCard
              icon={<Monitor className="h-4 w-4" />}
              label="渲染帧率"
              value={`${metrics.render_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.render_fps)}
            />
            <MetricCard
              icon={<Layers className="h-4 w-4" />}
              label="帧时间"
              value={`${metrics.frame_time_ms.toFixed(2)} ms`}
            />
            <MetricCard
              label="绘制调用"
              value={metrics.draw_calls.toLocaleString()}
            />
            <MetricCard
              label="三角形"
              value={`${(metrics.triangles / 1000).toFixed(1)}K`}
            />
          </div>

          {/* Frame Time Graph */}
          <div className="bg-card rounded-lg border p-4 mb-6">
            <h3 className="font-medium mb-4">帧时间分布</h3>
            <div className="space-y-2">
              <FrameTimeBar label="16.6ms (60 FPS)" value={metrics.frame_time_ms} target={16.6} />
              <FrameTimeBar label="8.3ms (120 FPS)" value={metrics.frame_time_ms} target={8.3} />
              <FrameTimeBar label="Pipeline P95" value={metrics.gpu_frame_time_ms} target={16.6} />
            </div>
          </div>

          {/* Detailed Stats */}
          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">详细统计</h3>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">分辨率</p>
                <p className="font-mono">{metrics.resolution.join(" x ")}</p>
              </div>
              <div>
                <p className="text-muted-foreground">纹理数量</p>
                <p className="font-mono">{metrics.textures}</p>
              </div>
              <div>
                <p className="text-muted-foreground">平均帧时间</p>
                <p className="font-mono">{metrics.frame_time_ms.toFixed(2)} ms</p>
              </div>
              <div>
                <p className="text-muted-foreground">采集 P95</p>
                <p className="font-mono">{metrics.capture_latency_p95_ms.toFixed(2)} ms</p>
              </div>
              <div>
                <p className="text-muted-foreground">传输 P95</p>
                <p className="font-mono">{metrics.transport_latency_p95_ms.toFixed(2)} ms</p>
              </div>
              <div>
                <p className="text-muted-foreground">解码 P95</p>
                <p className="font-mono">{metrics.decode_latency_p95_ms.toFixed(2)} ms</p>
              </div>
              <div>
                <p className="text-muted-foreground">Pipeline P95</p>
                <p className="font-mono">{metrics.gpu_frame_time_ms.toFixed(2)} ms</p>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function WebViewRenderPreview({
  mode,
  onSample,
}: {
  mode: "video" | "animation" | "static";
  onSample: (sample: WebViewRenderSample) => void;
}) {
  const animate = mode !== "static";

  useEffect(() => {
    let animationFrame = 0;
    let lastFrameAt: number | null = null;
    let lastEmitAt: number | null = null;
    let frameCount = 0;
    const frameTimes: number[] = [];

    const tick = (now: number) => {
      if (lastFrameAt !== null) {
        const delta = now - lastFrameAt;
        if (delta > 0 && delta < 1000) {
          frameTimes.push(delta);
          if (frameTimes.length > 240) frameTimes.shift();
        }
      }
      lastFrameAt = now;
      frameCount += 1;

      if (lastEmitAt === null || now - lastEmitAt >= 250) {
        lastEmitAt = now;
        const averageFrameTimeMs = average(frameTimes);
        const p95FrameTimeMs = percentile(frameTimes, 0.95);
        onSample({
          fps: averageFrameTimeMs > 0 ? 1000 / averageFrameTimeMs : 0,
          averageFrameTimeMs,
          p95FrameTimeMs,
          frameCount,
        });
      }

      animationFrame = window.requestAnimationFrame(tick);
    };

    animationFrame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(animationFrame);
  }, [onSample]);

  return (
    <div className="relative h-full w-full overflow-hidden bg-slate-950 text-cyan-100">
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_30%_20%,rgba(34,211,238,0.35),transparent_32%),radial-gradient(circle_at_75%_65%,rgba(59,130,246,0.32),transparent_34%),linear-gradient(135deg,#020617,#0f172a_48%,#111827)]" />
      <div className={`absolute inset-x-0 top-1/2 h-px bg-cyan-300/70 ${animate ? "animate-pulse" : ""}`} />
      <div className="absolute inset-8 rounded-2xl border border-cyan-300/30 bg-black/30 shadow-[0_0_40px_rgba(34,211,238,0.25)] backdrop-blur-sm" />
      <div className="absolute left-10 top-8 text-xs uppercase tracking-[0.35em] text-cyan-200/70">
        WebView 实时动画
      </div>
      <div className="absolute left-10 top-16 text-3xl font-semibold text-white">
        Browser Compositor Probe
      </div>
      <div
        className={`absolute bottom-10 left-10 h-20 w-20 rounded-full bg-cyan-300 shadow-[0_0_48px_rgba(34,211,238,0.65)] ${
          animate ? "animate-bounce" : ""
        }`}
      />
      <div className="absolute bottom-10 right-10 grid grid-cols-4 gap-2">
        {Array.from({ length: 16 }).map((_, index) => (
          <span
            key={index}
            className={`h-4 w-4 rounded-sm bg-cyan-200/70 ${animate ? "animate-pulse" : ""}`}
            style={{ animationDelay: `${index * 60}ms` }}
          />
        ))}
      </div>
    </div>
  );
}

function average(values: number[]): number {
  if (values.length === 0) return 0;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function percentile(values: number[], quantile: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(sorted.length * quantile) - 1)
  );
  return sorted[index] ?? 0;
}

function MetricCard({
  icon,
  label,
  value,
  color = "text-foreground",
}: {
  icon?: React.ReactNode;
  label: string;
  value: string | number;
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

function FrameTimeBar({
  label,
  value,
  target,
}: {
  label: string;
  value: number;
  target: number;
}) {
  const ratio = value / target;
  const percent = Math.min(ratio * 100, 100);
  const color = ratio <= 1 ? "bg-green-500" : ratio <= 1.5 ? "bg-yellow-500" : "bg-red-500";

  return (
    <div>
      <div className="flex justify-between text-sm mb-1">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono">{value.toFixed(2)} ms</span>
      </div>
      <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden relative">
        <div className="absolute h-full bg-white/30" style={{ left: "100%", width: "2px" }} />
        <div
          className={`h-full ${color} transition-all duration-300`}
          style={{ width: `${percent}%` }}
        />
      </div>
    </div>
  );
}

function getFpsColor(fps: number): string {
  if (fps >= 55) return "text-green-500";
  if (fps >= 30) return "text-yellow-500";
  return "text-red-500";
}
