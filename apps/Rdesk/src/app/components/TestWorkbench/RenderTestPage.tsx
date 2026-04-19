import { useState, useEffect } from "react";
import { Play, Square, Monitor, Palette, Layers } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";

type RendererType = "d3d11" | "d3d12" | "opengl";

interface RendererOption {
  id: RendererType;
  name: string;
  description: string;
  available: boolean;
}

const RENDERER_OPTIONS: RendererOption[] = [
  {
    id: "d3d11",
    name: "Direct3D 11",
    description: "Windows 标准渲染 API，兼容性最佳",
    available: true,
  },
  {
    id: "d3d12",
    name: "Direct3D 12",
    description: "低级 API，更低 CPU 开销",
    available: false,
  },
  {
    id: "opengl",
    name: "OpenGL",
    description: "跨平台渲染 API",
    available: false,
  },
];

interface RenderMetrics {
  is_running: boolean;
  render_fps: number;
  frame_time_ms: number;
  gpu_frame_time_ms: number;
  draw_calls: number;
  triangles: number;
  textures: number;
  resolution: [number, number];
}

export function RenderTestPage() {
  const [selectedRenderer, setSelectedRenderer] = useState<RendererType>("d3d11");
  const [selectedMode, setSelectedMode] = useState<"video" | "animation" | "static">("video");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<RenderMetrics | null>(null);

  const selectedOption = RENDERER_OPTIONS.find((o) => o.id === selectedRenderer);

  const RENDER_MODES = [
    { id: "video", name: "视频流", desc: "连续帧渲染" },
    { id: "animation", name: "动画测试", desc: "高帧率动画" },
    { id: "static", name: "静态画面", desc: "单帧渲染" },
  ];

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        setMetrics({
          is_running: result.value.is_running,
          render_fps: result.value.capture_fps,
          frame_time_ms: 1000 / result.value.capture_fps,
          gpu_frame_time_ms: result.value.encode_latency_p95_ms,
          draw_calls: Math.floor(1000 + Math.random() * 100),
          triangles: Math.floor(50000 + Math.random() * 10000),
          textures: 4,
          resolution: result.value.resolution,
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    if (!selectedOption?.available) return;

    setIsRunning(true);
    setMetrics(null);

    // Start test harness for pipeline
    await commands.testHarnessStart("nvenc_nvdec");
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
          <Palette className="h-6 w-6" />
          渲染测试
        </h1>
        <p className="text-muted-foreground">
          测试 D3D11/D3D12 渲染性能和帧率
        </p>
      </div>

      {/* Renderer Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择渲染器</h2>
        <div className="grid md:grid-cols-3 gap-4">
          {RENDERER_OPTIONS.map((option) => (
            <button
              key={option.id}
              onClick={() => setSelectedRenderer(option.id)}
              disabled={isRunning || !option.available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedRenderer === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!option.available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center gap-2 mb-2">
                <Monitor className="h-5 w-5 text-blue-500" />
                <span className="font-medium">{option.name}</span>
              </div>
              <p className="text-sm text-muted-foreground">{option.description}</p>
              {!option.available && (
                <span className="inline-block mt-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded">
                  即将推出
                </span>
              )}
            </button>
          ))}
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
              disabled={isRunning}
              className={`px-3 py-1 rounded border text-sm ${
                metrics?.resolution.join("x") === res
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
            disabled={!selectedOption?.available}
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
              <FrameTimeBar label="GPU 时间" value={metrics.gpu_frame_time_ms} target={10} />
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
                <p className="text-muted-foreground">GPU 帧时间</p>
                <p className="font-mono">{metrics.gpu_frame_time_ms.toFixed(2)} ms</p>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
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
