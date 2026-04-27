import { useState, useEffect } from "react";
import { useNavigate } from "react-router";
import { Play, Square, Monitor, Clock, Zap, Activity, Video } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { TestConfig, HarnessMetrics } from "../../adapters/tauri/types";

const DEFAULT_CONFIG: TestConfig = {
  capture_type: "dxgi",
  encoder_type: "nvenc_h264",
  decoder_type: "nvdec",
  renderer_type: "d3d11",
  resolution: [1920, 1080],
  fps: 60,
  bitrate: 5000000,
  duration_ms: 10000,
  warmup_ms: 2000,
};

export function E2ETestPage() {
  const navigate = useNavigate();
  const [isRunning, setIsRunning] = useState(false);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<HarnessMetrics | null>(null);
  const [capturedFrame, setCapturedFrame] = useState<string | null>(null);

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
    const result = await commands.testStartRun({
      scenarioId: "e2e.local",
      config: DEFAULT_CONFIG,
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
            <span className="text-muted-foreground">采集:</span> DXGI
          </div>
          <div>
            <span className="text-muted-foreground">编码:</span> NVENC H.264
          </div>
          <div>
            <span className="text-muted-foreground">解码:</span> NVDEC
          </div>
          <div>
            <span className="text-muted-foreground">渲染:</span> D3D11
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
  icon: React.ReactNode;
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
