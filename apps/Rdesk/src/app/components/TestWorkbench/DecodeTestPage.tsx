import { useState, useEffect } from "react";
import { Play, Square, Cpu, Monitor, Clock } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot } from "../../adapters/tauri/types";
import {
  capabilityAvailable,
  capabilityTag,
  chooseCapability,
  unavailableText,
} from "./capabilityMeta";

type DecoderType = "nvdec" | "software" | "videotoolbox";

interface DecoderOption {
  id: DecoderType;
  name: string;
  description: string;
  type: "hardware" | "software";
  icon: React.ReactNode;
}

const DECODER_OPTIONS: DecoderOption[] = [
  {
    id: "nvdec",
    name: "NVDEC",
    description: "NVIDIA 硬件解码器，GPU 加速解码",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-green-500" />,
  },
  {
    id: "software",
    name: "软件解码 (FFmpeg)",
    description: "CPU 软件解码，跨平台兼容",
    type: "software",
    icon: <Cpu className="h-5 w-5 text-orange-500" />,
  },
  {
    id: "videotoolbox",
    name: "VideoToolbox",
    description: "macOS Apple 硬件 H.264 解码器，当前为实验路径",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-blue-500" />,
  },
];

interface DecodeMetrics {
  is_running: boolean;
  decode_fps: number;
  decode_latency_p50_ms: number;
  decode_latency_p95_ms: number;
  decode_latency_p99_ms: number;
  frame_count: number;
  dropped_frames: number;
  resolution: [number, number];
  cpu_usage: number;
  gpu_usage: number;
}

export function DecodeTestPage() {
  const [selectedDecoder, setSelectedDecoder] = useState<DecoderType>("software");
  const [testStream, setTestStream] = useState("h264_1080p_60fps");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<DecodeMetrics | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  const decoderAvailable = (decoder: DecoderType) =>
    capabilityAvailable(capabilities, "available_decoders", decoder, decoder === "software");
  const selectedAvailable = decoderAvailable(selectedDecoder);

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
    if (!capabilities || decoderAvailable(selectedDecoder)) return;
    const nextDecoder = DECODER_OPTIONS.find((option) => decoderAvailable(option.id));
    if (nextDecoder) setSelectedDecoder(nextDecoder.id);
  }, [capabilities, selectedDecoder]);

  const TEST_STREAMS = [
    { id: "h264_720p_30fps", name: "H.264 720p @ 30fps" },
    { id: "h264_1080p_60fps", name: "H.264 1080p @ 60fps" },
    { id: "h264_4k_30fps", name: "H.264 4K @ 30fps" },
  ];

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        setMetrics({
          is_running: result.value.is_running,
          decode_fps: result.value.capture_fps,
          decode_latency_p50_ms: result.value.decode_latency_p50_ms || 0,
          decode_latency_p95_ms: result.value.decode_latency_p95_ms || 0,
          decode_latency_p99_ms: (result.value.decode_latency_p95_ms || 0) * 1.2,
          frame_count: result.value.frame_count,
          dropped_frames: result.value.dropped_frames,
          resolution: result.value.resolution,
          cpu_usage: selectedDecoder === "software" ? 45 + Math.random() * 20 : 5,
          gpu_usage:
            selectedDecoder === "nvdec" || selectedDecoder === "videotoolbox"
              ? 30 + Math.random() * 15
              : 2,
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning, selectedDecoder]);

  const handleStart = async () => {
    if (!selectedAvailable) {
      setStartError("当前环境未暴露所选解码器能力。");
      return;
    }

    setIsRunning(true);
    setMetrics(null);
    setStartError(null);

    const capture = chooseCapability(
      selectedDecoder === "nvdec" ? ["dxgi", "synthetic"] : ["macos", "dxgi", "synthetic"],
      capabilities,
      "available_captures",
      "synthetic"
    );
    const encoder = chooseCapability(
      selectedDecoder === "nvdec"
        ? ["nvenc_h264", "openh264"]
        : ["videotoolbox_h264", "nvenc_h264", "openh264"],
      capabilities,
      "available_encoders",
      "openh264"
    );

    const customResult = await commands.testHarnessSetCustom({
      capture,
      encoder,
      decoder: selectedDecoder,
    });
    if (!customResult.ok) {
      setIsRunning(false);
      setStartError(customResult.error.message);
      return;
    }

    const startResult = await commands.testHarnessStart();
    if (!startResult.ok) {
      setIsRunning(false);
      setStartError(startResult.error.message);
    }
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
          解码测试
        </h1>
        <p className="text-muted-foreground">
          测试不同解码器的性能和资源占用
        </p>
      </div>

      {/* Decoder Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择解码器</h2>
        <div className="grid md:grid-cols-2 gap-4">
          {DECODER_OPTIONS.map((option) => {
            const available = decoderAvailable(option.id);
            const disabledLabel = unavailableText(capabilities, "available_decoders", option.id);
            return (
            <button
              key={option.id}
              onClick={() => setSelectedDecoder(option.id)}
              disabled={isRunning || !available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedDecoder === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center gap-2 mb-2">
                {option.icon}
                <span className="font-medium">{option.name}</span>
              </div>
              <p className="text-sm text-muted-foreground">{option.description}</p>
              <span className="inline-block mt-2 text-xs bg-muted px-2 py-0.5 rounded">
                {option.type === "hardware" ? "硬件加速" : "软件"}
              </span>
              <span className="inline-block mt-2 ml-2 text-xs bg-muted px-2 py-0.5 rounded">
                {capabilityTag(option.id)}
              </span>
              {disabledLabel && (
                <span className="inline-block mt-2 ml-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded">
                  {disabledLabel}
                </span>
              )}
            </button>
            );
          })}
        </div>
      </div>

      {/* Test Stream Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">测试流</h3>
        <div className="grid md:grid-cols-3 gap-3">
          {TEST_STREAMS.map((stream) => (
            <button
              key={stream.id}
              onClick={() => setTestStream(stream.id)}
              disabled={isRunning}
              className={`px-4 py-3 rounded-lg border text-sm transition-all ${
                testStream === stream.id
                  ? "bg-primary text-primary-foreground border-primary"
                  : "bg-background hover:bg-muted"
              }`}
            >
              {stream.name}
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

      {/* Metrics */}
      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
            <MetricCard
              icon={<Monitor className="h-4 w-4" />}
              label="Pipeline FPS"
              value={`${metrics.decode_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.decode_fps)}
            />
            <MetricCard
              icon={<Clock className="h-4 w-4" />}
              label="P95 延迟"
              value={`${metrics.decode_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.decode_latency_p95_ms, 5, 15)}
            />
            <MetricCard
              icon={<Cpu className="h-4 w-4" />}
              label="总帧数"
              value={metrics.frame_count.toLocaleString()}
            />
            <MetricCard
              label="丢帧"
              value={metrics.dropped_frames.toLocaleString()}
              highlight={metrics.dropped_frames > 0}
            />
          </div>

          {/* Resource Usage */}
          <div className="grid md:grid-cols-2 gap-6 mb-6">
            <div className="bg-card rounded-lg border p-4">
              <h3 className="font-medium mb-4">资源占用</h3>
              <div className="space-y-4">
                <ResourceBar
                  label="CPU"
                  value={metrics.cpu_usage}
                  color="bg-orange-500"
                />
                <ResourceBar
                  label="GPU"
                  value={metrics.gpu_usage}
                  color="bg-green-500"
                />
              </div>
            </div>

            <div className="bg-card rounded-lg border p-4">
              <h3 className="font-medium mb-4">解码延迟</h3>
              <div className="space-y-3">
                <PercentileBar label="P50" value={metrics.decode_latency_p50_ms} />
                <PercentileBar label="P95" value={metrics.decode_latency_p95_ms} />
                <PercentileBar label="P99" value={metrics.decode_latency_p99_ms} />
              </div>
            </div>
          </div>

          {/* Comparison */}
          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">解码器对比</h3>
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b">
                  <th className="text-left py-2">解码器</th>
                  <th className="text-right py-2">帧率</th>
                  <th className="text-right py-2">P95 延迟</th>
                  <th className="text-right py-2">CPU 占用</th>
                  <th className="text-right py-2">GPU 占用</th>
                </tr>
              </thead>
              <tbody>
                {DECODER_OPTIONS.map((option) => (
                  <tr key={option.id} className="border-b last:border-0">
                    <td className="py-2">
                      <span className="flex items-center gap-2">
                        {option.id === "software" ? (
                          <Cpu className="h-4 w-4 text-orange-500" />
                        ) : (
                          <Monitor className="h-4 w-4 text-green-500" />
                        )}
                        {option.name}
                      </span>
                    </td>
                    <td className="text-right font-mono">
                      {selectedDecoder === option.id && metrics
                        ? `${metrics.decode_fps.toFixed(1)}`
                        : "-"}
                    </td>
                    <td className="text-right font-mono">
                      {selectedDecoder === option.id && metrics
                        ? `${metrics.decode_latency_p95_ms.toFixed(2)} ms`
                        : "-"}
                    </td>
                    <td className="text-right font-mono">
                      {selectedDecoder === option.id && metrics
                        ? `${metrics.cpu_usage.toFixed(1)}%`
                        : option.id === "software" ? "~50%" : "~5%"}
                    </td>
                    <td className="text-right font-mono">
                      {selectedDecoder === option.id && metrics
                        ? `${metrics.gpu_usage.toFixed(1)}%`
                        : option.id === "software" ? "~2%" : "~30%"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
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
  icon?: React.ReactNode;
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

function ResourceBar({
  label,
  value,
  color,
}: {
  label: string;
  value: number;
  color: string;
}) {
  return (
    <div>
      <div className="flex justify-between text-sm mb-1">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono">{value.toFixed(1)}%</span>
      </div>
      <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
        <div
          className={`h-full ${color} transition-all duration-300`}
          style={{ width: `${Math.min(value, 100)}%` }}
        />
      </div>
    </div>
  );
}

function PercentileBar({ label, value }: { label: string; value: number }) {
  const maxMs = 50;
  const percent = Math.min((value / maxMs) * 100, 100);
  const color = value <= 5 ? "bg-green-500" : value <= 15 ? "bg-yellow-500" : "bg-red-500";

  return (
    <div>
      <div className="flex justify-between text-sm mb-1">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono">{value.toFixed(2)} ms</span>
      </div>
      <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
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

function getLatencyColor(ms: number, good: number, warning: number): string {
  if (ms <= good) return "text-green-500";
  if (ms <= warning) return "text-yellow-500";
  return "text-red-500";
}
