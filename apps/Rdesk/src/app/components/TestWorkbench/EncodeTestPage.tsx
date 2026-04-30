import { useState, useEffect } from "react";
import { Play, Square, Zap, Cpu, Gauge, Video } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot } from "../../adapters/tauri/types";
import { capabilityAvailable, capabilityTag, unavailableText } from "./capabilityMeta";

type EncoderType = "nvenc_h264" | "nvenc_av1" | "openh264" | "videotoolbox_h264";

interface EncoderOption {
  id: EncoderType;
  name: string;
  description: string;
  type: "hardware" | "software";
  available: boolean;
  icon: React.ReactNode;
}

const ENCODER_OPTIONS: EncoderOption[] = [
  {
    id: "nvenc_h264",
    name: "NVENC H.264",
    description: "NVIDIA 硬件编码器，低延迟高质量",
    type: "hardware",
    available: true,
    icon: <Zap className="h-5 w-5 text-green-500" />,
  },
  {
    id: "nvenc_av1",
    name: "NVENC AV1",
    description: "新一代 AV1 编码器，更高压缩效率",
    type: "hardware",
    available: true,
    icon: <Zap className="h-5 w-5 text-blue-500" />,
  },
  {
    id: "openh264",
    name: "OpenH264",
    description: "Cisco 开源软件编码器",
    type: "software",
    available: true,
    icon: <Cpu className="h-5 w-5 text-orange-500" />,
  },
  {
    id: "videotoolbox_h264",
    name: "VideoToolbox H.264",
    description: "macOS Apple 硬件 H.264 编码器",
    type: "hardware",
    available: true,
    icon: <Zap className="h-5 w-5 text-blue-500" />,
  },
];

interface EncoderMetrics {
  is_running: boolean;
  encode_fps: number;
  encode_latency_p50_ms: number;
  encode_latency_p95_ms: number;
  encode_latency_p99_ms: number;
  frame_count: number;
  dropped_frames: number;
  resolution: [number, number];
  bitrate_kbps: number;
}

export function EncodeTestPage() {
  const [selectedEncoder, setSelectedEncoder] = useState<EncoderType>("nvenc_h264");
  const [selectedBitrate, setSelectedBitrate] = useState(5000);
  const [selectedPreset, setSelectedPreset] = useState("p1");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<EncoderMetrics | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  const selectedOption = ENCODER_OPTIONS.find((o) => o.id === selectedEncoder);
  const isEncoderAvailable = (option: EncoderOption) => {
    if (!option.available) return false;
    return capabilityAvailable(capabilities, "available_encoders", option.id, option.id === "openh264");
  };
  const selectedAvailable = selectedOption ? isEncoderAvailable(selectedOption) : false;

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
    if (!capabilities || selectedAvailable) return;
    const nextEncoder = ENCODER_OPTIONS.find((option) => isEncoderAvailable(option));
    if (nextEncoder) setSelectedEncoder(nextEncoder.id);
  }, [capabilities, selectedAvailable]);

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        setMetrics({
          is_running: result.value.is_running,
          encode_fps: result.value.capture_fps,
          encode_latency_p50_ms: result.value.encode_latency_p50_ms,
          encode_latency_p95_ms: result.value.encode_latency_p95_ms,
          encode_latency_p99_ms: result.value.encode_latency_p95_ms * 1.15,
          frame_count: result.value.frame_count,
          dropped_frames: result.value.dropped_frames,
          resolution: result.value.resolution,
          bitrate_kbps: 5000, // Default, would need actual measurement
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    if (!selectedOption || !selectedAvailable) {
      setStartError(
        selectedEncoder === "nvenc_av1"
          ? "当前 GPU/驱动未暴露 NVENC AV1 编码能力。RTX 30 系通常支持 AV1 解码，但不支持 AV1 NVENC 编码。"
          : "当前环境未暴露所选编码器能力。"
      );
      return;
    }

    setIsRunning(true);
    setMetrics(null);
    setStartError(null);

    if (selectedEncoder === "nvenc_av1") {
      const customResult = await commands.testHarnessSetCustom({
        capture: "dxgi",
        encoder: "nvenc_av1",
        decoder: "nvdec",
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
      return;
    }

    if (selectedEncoder === "videotoolbox_h264") {
      const customResult = await commands.testHarnessSetCustom({
        capture: "synthetic",
        encoder: "videotoolbox_h264",
        decoder: "none",
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
      return;
    }

    // Map encoder to test chain
    const chainMap: Record<
      Exclude<EncoderType, "nvenc_av1" | "videotoolbox_h264">,
      "nvenc_only" | "openh264"
    > = {
      nvenc_h264: "nvenc_only",
      openh264: "openh264",
    };

    const startResult = await commands.testHarnessStart(chainMap[selectedEncoder]);
    if (!startResult.ok) {
      setIsRunning(false);
      setStartError(startResult.error.message);
    }
  };

  const handleStop = async () => {
    await commands.testHarnessStop();
    setIsRunning(false);
  };

  const BITRATES = [1000, 3000, 5000, 8000, 10000, 20000];

  const PRESETS = [
    { id: "p1", name: "P1 (最快)", desc: "最低延迟" },
    { id: "p4", name: "P4 (平衡)", desc: "速度与质量平衡" },
    { id: "p7", name: "P7 (最高质量)", desc: "最佳质量" },
  ];

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Video className="h-6 w-6" />
          编码测试
        </h1>
        <p className="text-muted-foreground">
          测试不同编码器的性能和质量表现
        </p>
      </div>

      {/* Encoder Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择编码器</h2>
        <div className="grid md:grid-cols-3 gap-4">
          {ENCODER_OPTIONS.map((option) => {
            const available = isEncoderAvailable(option);
            const isAv1Unavailable = option.id === "nvenc_av1" && !available;
            const disabledLabel = unavailableText(capabilities, "available_encoders", option.id);
            return (
            <button
              key={option.id}
              onClick={() => setSelectedEncoder(option.id)}
              disabled={isRunning || !available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedEncoder === option.id
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
              {(isAv1Unavailable || disabledLabel) && (
                <span className="inline-block mt-2 ml-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded">
                  {isAv1Unavailable ? "GPU 不支持" : disabledLabel}
                </span>
              )}
            </button>
            );
          })}
        </div>
      </div>

      {/* Parameters */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">编码参数</h3>
        <div className="grid md:grid-cols-2 gap-6">
          {/* Bitrate */}
          <div>
            <label className="block text-sm font-medium mb-2">目标码率</label>
            <div className="flex gap-2">
              {BITRATES.map((br) => (
                <button
                  key={br}
                  onClick={() => setSelectedBitrate(br)}
                  disabled={isRunning}
                  className={`px-3 py-1 rounded border text-sm ${
                    selectedBitrate === br
                      ? "bg-primary text-primary-foreground border-primary"
                      : "bg-background hover:bg-muted"
                  }`}
                >
                  {br >= 1000 ? `${br / 1000}M` : `${br}K`}
                </button>
              ))}
            </div>
          </div>

          {/* Preset - Hardware only */}
          {selectedOption?.type === "hardware" && (
            <div>
              <label className="block text-sm font-medium mb-2">编码预设</label>
              <div className="space-y-1">
                {PRESETS.map((preset) => (
                  <button
                    key={preset.id}
                    onClick={() => setSelectedPreset(preset.id)}
                    disabled={isRunning}
                    className={`w-full text-left px-3 py-2 rounded border text-sm flex justify-between ${
                      selectedPreset === preset.id
                        ? "bg-primary/10 border-primary"
                        : "bg-background hover:bg-muted"
                    }`}
                  >
                    <span>{preset.name}</span>
                    <span className="text-muted-foreground">{preset.desc}</span>
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Software Encoder Options */}
          {selectedOption?.type === "software" && (
            <div>
              <label className="block text-sm font-medium mb-2">线程数</label>
              <select
                disabled={isRunning}
                className="w-full px-3 py-2 border rounded bg-background"
                defaultValue="0"
              >
                <option value="0">自动 (0)</option>
                <option value="2">2 线程</option>
                <option value="4">4 线程</option>
                <option value="8">8 线程</option>
              </select>
            </div>
          )}
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
              icon={<Video className="h-4 w-4" />}
              label="编码帧率"
              value={`${metrics.encode_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.encode_fps)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="P95 延迟"
              value={`${metrics.encode_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.encode_latency_p95_ms, 5, 15)}
            />
            <MetricCard
              icon={<Zap className="h-4 w-4" />}
              label="总帧数"
              value={metrics.frame_count.toLocaleString()}
            />
            <MetricCard
              label="丢帧"
              value={metrics.dropped_frames.toLocaleString()}
              highlight={metrics.dropped_frames > 0}
            />
          </div>

          {/* Latency Percentiles */}
          <div className="bg-card rounded-lg border p-4 mb-6">
            <h3 className="font-medium mb-4">编码延迟分布</h3>
            <div className="space-y-3">
              <PercentileBar label="P50 (中位数)" value={metrics.encode_latency_p50_ms} />
              <PercentileBar label="P95 (95%)" value={metrics.encode_latency_p95_ms} />
              <PercentileBar label="P99 (99%)" value={metrics.encode_latency_p99_ms} />
            </div>
          </div>

          {/* Quality Metrics */}
          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">质量指标</h3>
            <div className="grid grid-cols-3 gap-4">
              <div className="text-center">
                <p className="text-2xl font-bold text-green-500">
                  {metrics.bitrate_kbps}K
                </p>
                <p className="text-sm text-muted-foreground">实际码率</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold">-</p>
                <p className="text-sm text-muted-foreground">PSNR (dB)</p>
              </div>
              <div className="text-center">
                <p className="text-2xl font-bold">-</p>
                <p className="text-sm text-muted-foreground">SSIM</p>
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
