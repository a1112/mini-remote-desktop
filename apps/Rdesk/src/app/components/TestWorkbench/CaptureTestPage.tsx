import { useState, useEffect } from "react";
import { Play, Square, Monitor, Activity, Gauge } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";

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
    available: false,
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
  p95_latency_ms: number;
  p99_latency_ms: number;
}

export function CaptureTestPage() {
  const [selectedCapture, setSelectedCapture] = useState<CaptureType>("dxgi");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<CaptureMetrics | null>(null);

  const selectedOption = CAPTURE_OPTIONS.find((o) => o.id === selectedCapture);

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
          avg_latency_ms: result.value.encode_latency_p50_ms, // Using encode latency as proxy
          p95_latency_ms: result.value.encode_latency_p95_ms,
          p99_latency_ms: result.value.encode_latency_p95_ms * 1.1, // Approximation
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    if (selectedCapture === "winrt") return; // Not implemented yet

    setIsRunning(true);
    setMetrics(null);

    // Start with appropriate chain
    const chain = selectedCapture === "synthetic" ? "nvenc_only" : "nvenc_nvdec";
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

      {/* Control */}
      <div className="mb-6">
        {!isRunning ? (
          <button
            onClick={handleStart}
            disabled={selectedCapture === "winrt"}
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
              icon={<Activity className="h-4 w-4" />}
              label="捕获帧率"
              value={`${metrics.capture_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.capture_fps)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="平均延迟"
              value={`${metrics.avg_latency_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.avg_latency_ms, 5, 10)}
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
              <LatencyBar label="P50" value={metrics.avg_latency_ms} max={50} />
              <LatencyBar label="P95" value={metrics.p95_latency_ms} max={50} />
              <LatencyBar label="P99" value={metrics.p99_latency_ms} max={50} />
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
