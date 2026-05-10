import { useState, useEffect, useRef } from "react";
import {
  Play,
  Square,
  Monitor,
  Zap,
  Activity,
  Clock,
  Video,
  ChevronDown,
  Settings,
} from "lucide-react";
import * as commands from "../adapters/tauri/commands";
import type { HarnessMetrics, TestChain, TestChainOption } from "../adapters/tauri/types";

const FRAME_UPDATE_INTERVAL_MS = 100;
const METRICS_UPDATE_INTERVAL_MS = 200;

const TEST_CHAINS: TestChainOption[] = [
  {
    value: "linux_openh264",
    label: "Linux 屏幕捕获 + OpenH264",
    description: "Linux 平台屏幕捕获与软件编码",
  },
  {
    value: "openh264",
    label: "OpenH264 软件编码",
    description: "CPU 软件编码测试",
  },
  {
    value: "capture_only",
    label: "仅捕获测试",
    description: "测试捕获功能，无编码",
  },
];

const TEST_PRESETS: TestChainOption[] = [
  {
    value: "linux_openh264",
    label: "Linux 屏幕捕获",
    description: "Linux 平台屏幕捕获 + OpenH264 编码",
  },
  {
    value: "openh264",
    label: "软件编码基准",
    description: "CPU 编码性能参考",
  },
  {
    value: "capture_only",
    label: "仅捕获测试",
    description: "测试屏幕捕获功能",
  },
];

export function TestPage() {
  const [isRunning, setIsRunning] = useState(false);
  const [selectedChain, setSelectedChain] = useState<TestChain>("linux_openh264");
  const [metrics, setMetrics] = useState<HarnessMetrics | null>(null);
  const [capturedFrame, setCapturedFrame] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showChainDropdown, setShowChainDropdown] = useState(false);

  const capturedCanvasRef = useRef<HTMLCanvasElement>(null);

  // Update metrics periodically
  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        setMetrics(result.value);
      }
    }, METRICS_UPDATE_INTERVAL_MS);

    return () => clearInterval(interval);
  }, [isRunning]);

  // Get captured frame
  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const framesResult = await commands.testHarnessGetFrames();
      if (framesResult.ok) {
        const [captured] = framesResult.value;
        if (captured) {
          setCapturedFrame(captured[0]);
        }
      }
    }, FRAME_UPDATE_INTERVAL_MS);

    return () => clearInterval(interval);
  }, [isRunning]);

  // Draw frame to canvas
  useEffect(() => {
    if (capturedFrame && capturedCanvasRef.current) {
      const canvas = capturedCanvasRef.current;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        const img = new Image();
        img.onload = () => {
          if (canvas.width !== img.width || canvas.height !== img.height) {
            canvas.width = img.width;
            canvas.height = img.height;
          }
          ctx.drawImage(img, 0, 0);
        };
        img.src = `data:image/png;base64,${capturedFrame}`;
      }
    }
  }, [capturedFrame]);

  const handleChainChange = async (chain: TestChain) => {
    if (isRunning) {
      setError("请先停止测试再切换链路");
      return;
    }
    setSelectedChain(chain);
    const result = await commands.testHarnessSetChain(chain);
    if (!result.ok) {
      setError(result.error.message);
    }
    setShowChainDropdown(false);
  };

  const handleStart = async () => {
    setError(null);
    setMetrics(null);
    const result = await commands.testHarnessStart();
    if (result.ok) {
      setIsRunning(true);
    } else {
      setError(result.error.message);
    }
  };

  const handleStop = async () => {
    const result = await commands.testHarnessStop();
    if (result.ok) {
      setIsRunning(false);
      setMetrics(null);
      setCapturedFrame(null);
    } else {
      setError(result.error.message);
    }
  };

  return (
    <div className="p-6 bg-background">
      <div className="max-w-7xl mx-auto">
        {/* Header */}
        <div className="mb-6">
          <h1 className="text-2xl font-bold text-foreground">端到端可视化测试</h1>
          <p className="text-muted-foreground mt-1">
            测试完整的捕获→编码→解码流程（后台线程自动处理）
          </p>
        </div>

        {/* Error Display */}
        {error && (
          <div className="mb-4 p-4 bg-destructive/10 border border-destructive/20 rounded-lg">
            <p className="text-destructive text-sm">{error}</p>
          </div>
        )}

        {/* Control Panel */}
        <div className="mb-6 flex flex-wrap items-center gap-4">
          {/* Chain Selection Dropdown */}
          <div className="relative">
            <button
              onClick={() => setShowChainDropdown(!showChainDropdown)}
              disabled={isRunning}
              className="flex items-center gap-2 px-4 py-2 bg-muted border rounded-lg hover:bg-muted/80 transition-colors disabled:opacity-50 disabled:cursor-not-allowed min-w-[240px]"
            >
              <Video className="w-4 h-4" />
              <span className="truncate">
                {TEST_CHAINS.find(c => c.value === selectedChain)?.label || "选择测试链路"}
              </span>
              <ChevronDown className="w-4 h-4 ml-auto" />
            </button>
            {showChainDropdown && (
              <div className="absolute top-full left-0 mt-1 w-72 bg-card border rounded-lg shadow-lg z-10">
                <div className="p-2 border-b text-xs text-muted-foreground px-3">
                  测试链路
                </div>
                {TEST_CHAINS.map((chain, idx) => (
                  <button
                    key={chain.value}
                    onClick={() => handleChainChange(chain.value)}
                    className={`w-full px-3 py-2 text-left hover:bg-muted/50 transition-colors ${
                      idx === 0 ? "first:rounded-t-lg" : ""
                    } ${idx === TEST_CHAINS.length - 1 ? "last:rounded-b-lg" : ""}`}
                  >
                    <div className="font-medium text-sm">{chain.label}</div>
                    {chain.description && (
                      <div className="text-xs text-muted-foreground mt-0.5">
                        {chain.description}
                      </div>
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Start/Stop Button */}
          {!isRunning ? (
            <button
              onClick={handleStart}
              className="flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors"
            >
              <Play className="w-4 h-4" />
              启动测试
            </button>
          ) : (
            <button
              onClick={handleStop}
              className="flex items-center gap-2 px-4 py-2 bg-destructive text-destructive-foreground rounded-lg hover:bg-destructive/90 transition-colors"
            >
              <Square className="w-4 h-4" />
              停止测试
            </button>
          )}
        </div>

        {/* Metrics Panel */}
        {metrics && (
          <div className="mb-6 grid grid-cols-2 md:grid-cols-4 gap-4">
            <MetricCard
              icon={<Activity className="w-4 h-4" />}
              label="Pipeline FPS"
              value={`${metrics.capture_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.capture_fps)}
            />
            <MetricCard
              icon={<Clock className="w-4 h-4" />}
              label="编码延迟 P95"
              value={`${metrics.encode_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.encode_latency_p95_ms, 10, 20)}
            />
            <MetricCard
              icon={<Zap className="w-4 h-4" />}
              label="解码延迟 P95"
              value={selectedChain === "nvenc_only" || selectedChain === "openh264"
                ? "N/A"
                : `${metrics.decode_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.decode_latency_p95_ms, 10, 30)}
            />
            <MetricCard
              icon={<Video className="w-4 h-4" />}
              label="总帧数"
              value={`${metrics.frame_count}`}
            />
          </div>
        )}

        {/* Detailed Metrics */}
        {metrics && (
          <div className="mb-6 p-4 bg-muted rounded-lg">
            <h3 className="text-sm font-medium mb-2">详细指标</h3>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
              <div>
                <span className="text-muted-foreground">分辨率:</span>{" "}
                {metrics.resolution[0]}x{metrics.resolution[1]}
              </div>
              <div>
                <span className="text-muted-foreground">丢帧数:</span>{" "}
                {metrics.dropped_frames}
              </div>
              <div>
                <span className="text-muted-foreground">编码 P50:</span>{" "}
                {metrics.encode_latency_p50_ms.toFixed(2)} ms
              </div>
              {metrics.decode_latency_p50_ms > 0 && (
                <div>
                  <span className="text-muted-foreground">解码 P50:</span>{" "}
                  {metrics.decode_latency_p50_ms.toFixed(2)} ms
                </div>
              )}
            </div>
          </div>
        )}

        {/* Frame Display */}
        <div className="bg-card rounded-lg p-4">
          <h3 className="text-sm font-medium mb-2 flex items-center gap-2">
            <Monitor className="w-4 h-4" />
            捕获画面（实时）
          </h3>
          <div className="aspect-video bg-black rounded flex items-center justify-center">
            {capturedFrame ? (
              <canvas
                ref={capturedCanvasRef}
                className="max-w-full max-h-full"
              />
            ) : (
              <p className="text-muted-foreground text-sm">
                {isRunning ? "正在捕获..." : "等待启动..."}
              </p>
            )}
          </div>
        </div>
      </div>
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
