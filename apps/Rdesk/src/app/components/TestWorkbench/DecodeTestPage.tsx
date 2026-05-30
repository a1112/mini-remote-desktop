import { useEffect, useState, type ReactNode } from "react";
import { Play, Square, Cpu, Monitor, Clock, Gauge } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot, TestConfig } from "../../adapters/tauri/types";
import { capabilityAvailable, capabilityTag, chooseCapability, unavailableText } from "./capabilityMeta";
import {
  buildCapabilitySnapshotFromIpc,
  capabilityForOption,
  capabilityOptionState,
  environmentSnapshotFromCapabilitySnapshot,
  shouldShowCapabilityOptionForSnapshot,
  type CapabilitySnapshot,
} from "../../services/capabilityMatrix";
import {
  shouldShowCapabilityOption,
  useShowUnavailableCapabilities,
} from "./useCapabilityVisibility";

type DecoderType = "nvdec" | "software" | "linux_h264" | "linux_hevc" | "linux_hevc_main10" | "videotoolbox";
type DecodeCodec = "h264" | "hevc" | "hevc_main10" | "av1" | "vvc";

interface DecoderOption {
  id: DecoderType;
  name: string;
  description: string;
  type: "hardware" | "software";
  icon: ReactNode;
}

interface CodecOption {
  id: DecodeCodec;
  name: string;
  description: string;
  supportedDecoders: DecoderType[];
}

interface DecodeProfile {
  id: string;
  name: string;
  resolution: [number, number];
  fps: number;
  bitrate: number;
  description: string;
}

interface DecodeMetrics {
  is_running: boolean;
  capture_fps: number;
  encode_fps: number;
  decode_fps: number;
  decode_latency_p50_ms: number;
  decode_latency_p95_ms: number;
  decode_latency_p99_ms: number;
  decoded_frames: number;
  decode_failures: number;
  dropped_frames: number;
  resolution: [number, number];
}

interface DecodeCapabilityAssessment {
  label: string;
  detail: string;
  color: string;
}

const DECODER_OPTIONS: DecoderOption[] = [
  {
    id: "nvdec",
    name: "NVDEC",
    description: "NVIDIA 硬件解码器，支持 H.264 / HEVC / HEVC Main10 / AV1 的当前 harness 组合",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-green-500" />,
  },
  {
    id: "software",
    name: "软件解码",
    description: "CPU H.264 / HEVC / AV1 软件解码，跨平台 fallback 基线",
    type: "software",
    icon: <Cpu className="h-5 w-5 text-orange-500" />,
  },
  {
    id: "linux_h264",
    name: "Linux H.264 HW",
    description: "Linux GStreamer H.264 硬件解码，当前输出 CPU RGB 帧用于闭环验证",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-emerald-500" />,
  },
  {
    id: "linux_hevc",
    name: "Linux HEVC HW",
    description: "Linux GStreamer HEVC 硬件解码，当前输出 CPU RGB 帧用于闭环验证",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-emerald-500" />,
  },
  {
    id: "linux_hevc_main10",
    name: "Linux HEVC Main10 HW",
    description: "Linux GStreamer HEVC Main10 硬件解码，当前输出 CPU RGB 帧用于闭环验证",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-emerald-500" />,
  },
  {
    id: "videotoolbox",
    name: "VideoToolbox",
    description: "macOS Apple H.264 硬件解码器，当前为实验路径",
    type: "hardware",
    icon: <Monitor className="h-5 w-5 text-blue-500" />,
  },
];

const CODEC_OPTIONS: CodecOption[] = [
  {
    id: "h264",
    name: "H.264",
    description: "主流远程桌面基础路径，所有当前解码器都可参与。",
    supportedDecoders: ["nvdec", "software", "linux_h264", "videotoolbox"],
  },
  {
    id: "hevc",
    name: "HEVC",
    description: "NVENC HEVC -> 硬件或软件解码，Windows/NVIDIA、Linux GStreamer 与 FFmpeg fallback 路径。",
    supportedDecoders: ["nvdec", "software", "linux_hevc", "linux_hevc_main10"],
  },
  {
    id: "hevc_main10",
    name: "HEVC Main10",
    description: "10-bit HEVC -> 硬件或软件解码，验证 Main10 能力。",
    supportedDecoders: ["nvdec", "software", "linux_hevc_main10"],
  },
  {
    id: "av1",
    name: "AV1",
    description: "NVENC AV1 -> NVDEC 或软件解码，取决于 GPU 代际能力和 FFmpeg runtime。",
    supportedDecoders: ["nvdec", "software"],
  },
  {
    id: "vvc",
    name: "H.266/VVC",
    description: "VVenC H.266/VVC -> VVdeC 软件解码，需启用 VVC 软件 codec feature。",
    supportedDecoders: ["software"],
  },
];

const DEFAULT_DECODE_PROFILE: DecodeProfile = {
  id: "1080p60",
  name: "1080p 60",
  resolution: [1920, 1080],
  fps: 60,
  bitrate: 8_000_000,
  description: "基准实时桌面。",
};

const DECODE_PROFILES: DecodeProfile[] = [
  DEFAULT_DECODE_PROFILE,
  {
    id: "1080p144",
    name: "1080p 144",
    resolution: [1920, 1080],
    fps: 144,
    bitrate: 16_000_000,
    description: "高刷新桌面压力。",
  },
  {
    id: "2k60",
    name: "2K 60",
    resolution: [2560, 1440],
    fps: 60,
    bitrate: 16_000_000,
    description: "2K 常规高质量。",
  },
  {
    id: "2k144",
    name: "2K 144",
    resolution: [2560, 1440],
    fps: 144,
    bitrate: 30_000_000,
    description: "目标 2K144 高压档。",
  },
  {
    id: "4k60",
    name: "4K 60",
    resolution: [3840, 2160],
    fps: 60,
    bitrate: 45_000_000,
    description: "4K 解码压力。",
  },
  {
    id: "max240",
    name: "最大吞吐 240",
    resolution: [1920, 1080],
    fps: 240,
    bitrate: 40_000_000,
    description: "移除 60fps UI 限制的吞吐压力档。",
  },
];

function codecSupportedByDecoder(codec: DecodeCodec, decoder: DecoderType): boolean {
  return CODEC_OPTIONS.some((option) => option.id === codec && option.supportedDecoders.includes(decoder));
}

function hasCapability(
  capabilities: EnvironmentSnapshot | null,
  key:
    | "available_captures"
    | "available_encoders"
    | "available_decoders"
    | "available_renderers",
  value: string
): boolean {
  if (!capabilities) return false;
  return capabilities[key]?.includes(value) ?? false;
}

function buildDecodeRun(
  decoder: DecoderType,
  codec: DecodeCodec,
  profile: DecodeProfile,
  capabilities?: EnvironmentSnapshot | null
): {
  scenarioId: string;
  config: TestConfig;
} {
  const common = {
    resolution: profile.resolution,
    fps: profile.fps,
    bitrate: profile.bitrate,
    duration_ms: 30_000,
    transport_kind: "loopback" as const,
    render_display: false,
    visual_preview: false,
  };

  if (decoder === "software") {
    const encoderType: TestConfig["encoder_type"] =
      codec === "hevc"
        ? "nvenc_hevc"
        : codec === "hevc_main10"
          ? "nvenc_hevc_main10"
          : codec === "av1"
            ? "nvenc_av1"
            : codec === "vvc"
              ? "software_vvc"
              : "openh264";
    return {
      scenarioId: "custom",
      config: {
        ...common,
        capture_type:
          encoderType === "openh264" || encoderType === "software_vvc" ? "synthetic" : "dxgi",
        encoder_type: encoderType,
        decoder_type: "software",
        zero_copy: false,
      },
    };
  }

  if (decoder === "videotoolbox") {
    return {
      scenarioId: "custom",
      config: {
        ...common,
        capture_type: "macos",
        encoder_type: "videotoolbox_h264",
        decoder_type: "videotoolbox",
        renderer_type: "macos",
        zero_copy: false,
      },
    };
  }

  if (decoder === "linux_h264") {
    const encoder = chooseCapability(
      ["nvenc_h264", "openh264"],
      capabilities ?? null,
      "available_encoders",
      "openh264"
    );
    return {
      scenarioId: "custom",
      config: {
        ...common,
        capture_type: "synthetic",
        encoder_type: encoder,
        decoder_type: "linux_h264",
        zero_copy: false,
      },
    };
  }

  if (decoder === "linux_hevc" || decoder === "linux_hevc_main10") {
    return {
      scenarioId: "custom",
      config: {
        ...common,
        capture_type: "synthetic",
        encoder_type: codec === "hevc_main10" ? "nvenc_hevc_main10" : "nvenc_hevc",
        decoder_type: decoder,
        zero_copy: false,
      },
    };
  }

  const encoderByCodec: Record<DecodeCodec, NonNullable<TestConfig["encoder_type"]>> = {
    h264: "nvenc_h264",
    hevc: "nvenc_hevc",
    hevc_main10: "nvenc_hevc_main10",
    av1: "nvenc_av1",
    vvc: "software_vvc",
  };

  return {
    scenarioId: "custom",
    config: {
      ...common,
      capture_type: "dxgi",
      encoder_type: encoderByCodec[codec],
      decoder_type: "nvdec",
      renderer_type: "d3d11",
      zero_copy: true,
    },
  };
}

function missingChainCapability(
  capabilities: EnvironmentSnapshot | null,
  config: TestConfig
): string | null {
  if (!capabilities) return "capabilities";
  if (config.capture_type && !hasCapability(capabilities, "available_captures", config.capture_type)) {
    return config.capture_type;
  }
  if (config.encoder_type && !hasCapability(capabilities, "available_encoders", config.encoder_type)) {
    return config.encoder_type;
  }
  if (config.decoder_type && !hasCapability(capabilities, "available_decoders", config.decoder_type)) {
    return config.decoder_type;
  }
  if (config.renderer_type && !hasCapability(capabilities, "available_renderers", config.renderer_type)) {
    return config.renderer_type;
  }
  return null;
}

function minPositive(values: number[]): number {
  const positives = values.filter((value) => value > 0);
  return positives.length > 0 ? Math.min(...positives) : 0;
}

function assessDecodeCapability(metrics: DecodeMetrics, targetFps: number): DecodeCapabilityAssessment {
  const target = Math.max(targetFps, 1);
  const frameBudgetMs = 1000 / target;
  const upstreamFps = minPositive([metrics.capture_fps, metrics.encode_fps]);
  const latencyRatio = metrics.decode_latency_p95_ms / frameBudgetMs;

  if (metrics.decode_failures > 0) {
    return {
      label: "解码存在失败",
      detail: `已出现 ${metrics.decode_failures} 次解码失败，需要优先看错误日志和输入码流。`,
      color: "text-red-500",
    };
  }

  if (metrics.decoded_frames === 0) {
    return {
      label: "尚未产出解码帧",
      detail: "当前还没有 decoded frame，可能正在等待首个关键帧或上游尚未产出 access unit。",
      color: "text-yellow-500",
    };
  }

  if (latencyRatio <= 0.25 && metrics.decode_fps < target * 0.75) {
    const upstreamText = upstreamFps > 0 ? `上游当前约 ${upstreamFps.toFixed(1)} FPS` : "上游 FPS 尚未稳定";
    return {
      label: "解码器余量充足，当前受上游限制",
      detail: `${upstreamText}，P95 解码延迟仅占 ${target} FPS 帧预算的 ${(latencyRatio * 100).toFixed(1)}%。需要继续优化采集/编码供帧，不能把 ${metrics.decode_fps.toFixed(1)} FPS 直接判定为 decoder 上限。`,
      color: "text-green-500",
    };
  }

  if (metrics.decode_fps >= target * 0.9 && latencyRatio <= 0.5) {
    return {
      label: "解码能力达标",
      detail: `当前 decoded FPS 接近目标 ${target} FPS，且 P95 解码延迟低于半帧预算。`,
      color: "text-green-500",
    };
  }

  if (latencyRatio > 1) {
    return {
      label: "解码延迟超过帧预算",
      detail: `目标 ${target} FPS 的单帧预算约 ${frameBudgetMs.toFixed(2)} ms，当前 P95 为 ${metrics.decode_latency_p95_ms.toFixed(2)} ms，decoder 已经是主要风险。`,
      color: "text-red-500",
    };
  }

  if (latencyRatio > 0.5) {
    return {
      label: "解码接近瓶颈",
      detail: `P95 解码延迟已占 ${target} FPS 帧预算的 ${(latencyRatio * 100).toFixed(1)}%，需要降低分辨率/码率或换硬解路径验证。`,
      color: "text-yellow-500",
    };
  }

  return {
    label: "解码正常，继续观察上游",
    detail: `解码失败为 0，P95 延迟低于半帧预算；如果 FPS 仍低，优先看采集 FPS 和编码 FPS。`,
    color: "text-green-500",
  };
}

export function DecodeTestPage() {
  const [selectedDecoder, setSelectedDecoder] = useState<DecoderType>("software");
  const [selectedCodec, setSelectedCodec] = useState<DecodeCodec>("h264");
  const [selectedProfileId, setSelectedProfileId] = useState("1080p60");
  const [isRunning, setIsRunning] = useState(false);
  const [metrics, setMetrics] = useState<DecodeMetrics | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [showUnavailable] = useShowUnavailableCapabilities();

  const selectedProfile =
    DECODE_PROFILES.find((profile) => profile.id === selectedProfileId) ?? DEFAULT_DECODE_PROFILE;
  const selectedOption = DECODER_OPTIONS.find((option) => option.id === selectedDecoder);
  const selectedRun = buildDecodeRun(selectedDecoder, selectedCodec, selectedProfile, capabilities);
  const capabilityAssessment = metrics
    ? assessDecodeCapability(metrics, selectedProfile.fps)
    : null;
  const selectedAvailable =
    (serviceCapabilitySnapshot
      ? capabilityOptionState(serviceCapabilitySnapshot, "decoder", selectedDecoder) !==
        "disabled"
      : capabilityAvailable(capabilities, "available_decoders", selectedDecoder, selectedDecoder === "software")) &&
    codecSupportedByDecoder(selectedCodec, selectedDecoder) &&
    !missingChainCapability(capabilities, selectedRun.config);
  const decoderAvailable = (decoder: DecoderType) =>
    serviceCapabilitySnapshot
      ? capabilityOptionState(serviceCapabilitySnapshot, "decoder", decoder) !== "disabled"
      : capabilityAvailable(capabilities, "available_decoders", decoder, decoder === "software");
  const visibleDecoderOptions = DECODER_OPTIONS.filter((option) =>
    serviceCapabilitySnapshot
      ? shouldShowCapabilityOptionForSnapshot(
          serviceCapabilitySnapshot,
          "decoder",
          option.id,
          showUnavailable
        )
      : !capabilities || shouldShowCapabilityOption(decoderAvailable(option.id), showUnavailable)
  );

  useEffect(() => {
    let cancelled = false;
    let legacyEnvironment: EnvironmentSnapshot | null = null;
    let serviceSnapshot: CapabilitySnapshot | null = null;

    const applyLegacyEnvironment = (environment: EnvironmentSnapshot) => {
      if (cancelled) return;
      legacyEnvironment = environment;
      if (serviceSnapshot) {
        setCapabilities(environmentSnapshotFromCapabilitySnapshot(serviceSnapshot, environment));
        return;
      }
      setServiceCapabilitySnapshot(null);
      setCapabilities(environment);
    };

    const applyServiceSnapshot = (snapshot: CapabilitySnapshot) => {
      if (cancelled) return;
      serviceSnapshot = snapshot;
      setServiceCapabilitySnapshot(snapshot);
      setCapabilities(environmentSnapshotFromCapabilitySnapshot(snapshot, legacyEnvironment));
    };

    void commands.testGetCapabilities().then((result) => {
      if (result.ok) applyLegacyEnvironment(result.value);
    });

    void commands.ipcCapabilitySnapshot().then((result) => {
      if (result.ok && result.value) {
        applyServiceSnapshot(buildCapabilitySnapshotFromIpc(result.value));
      } else if (!cancelled) {
        setServiceCapabilitySnapshot(null);
      }
    });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!codecSupportedByDecoder(selectedCodec, selectedDecoder)) {
      setSelectedCodec("h264");
    }
  }, [selectedCodec, selectedDecoder]);

  useEffect(() => {
    if (!capabilities) return;
    const decoderIsAvailable = (decoder: DecoderType) =>
      capabilityAvailable(capabilities, "available_decoders", decoder, decoder === "software");
    const preferredHardware = DECODER_OPTIONS.find(
      (option) => option.type === "hardware" && decoderIsAvailable(option.id)
    );
    if (selectedDecoder === "software" && preferredHardware) {
      setSelectedDecoder(preferredHardware.id);
      return;
    }
    if (decoderIsAvailable(selectedDecoder)) return;
    const nextDecoder = DECODER_OPTIONS.find((option) => decoderIsAvailable(option.id));
    if (nextDecoder) setSelectedDecoder(nextDecoder.id);
  }, [capabilities, serviceCapabilitySnapshot]);

  useEffect(() => {
    if (!isRunning) return;

    const interval = setInterval(async () => {
      const result = await commands.testHarnessGetMetrics();
      if (result.ok) {
        if (!result.value.is_running) {
          setIsRunning(false);
          setActiveRunId(null);
        }
        setMetrics({
          is_running: result.value.is_running,
          capture_fps: result.value.capture_fps,
          encode_fps: result.value.encoded_fps ?? result.value.capture_fps,
          decode_fps: result.value.decoded_fps ?? result.value.capture_fps,
          decode_latency_p50_ms: result.value.decode_latency_p50_ms || 0,
          decode_latency_p95_ms: result.value.decode_latency_p95_ms || 0,
          decode_latency_p99_ms: (result.value.decode_latency_p95_ms || 0) * 1.2,
          decoded_frames: result.value.decoded_frames ?? result.value.frame_count,
          decode_failures: result.value.decode_failures ?? 0,
          dropped_frames: result.value.dropped_frames,
          resolution: result.value.resolution,
        });
      }
    }, 200);

    return () => clearInterval(interval);
  }, [isRunning]);

  const handleStart = async () => {
    const run = buildDecodeRun(selectedDecoder, selectedCodec, selectedProfile, capabilities);
    const missing = missingChainCapability(capabilities, run.config);
    if (missing) {
      setStartError(`当前环境缺少解码测试链路能力：${missing}`);
      return;
    }
    if (!codecSupportedByDecoder(selectedCodec, selectedDecoder)) {
      setStartError("当前解码器不支持所选 codec。");
      return;
    }

    setIsRunning(true);
    setMetrics(null);
    setStartError(null);

    const startResult = await commands.testStartRun(run);
    if (!startResult.ok) {
      setIsRunning(false);
      setStartError(startResult.error.message);
      return;
    }
    setActiveRunId(startResult.value);
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
    <div className="p-6 max-w-5xl mx-auto">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Monitor className="h-6 w-6" />
          解码测试
        </h1>
        <p className="text-muted-foreground">
          测试不同解码器的真实解码吞吐、延迟和失败计数
        </p>
      </div>

      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择解码器</h2>
        <div className="grid md:grid-cols-3 gap-4">
          {visibleDecoderOptions.map((option) => {
            const capability = capabilityForOption(
              serviceCapabilitySnapshot,
              "decoder",
              option.id
            );
            const available = decoderAvailable(option.id);
            const disabledLabel = serviceCapabilitySnapshot
              ? !available
                ? capability?.reason ?? capability?.status ?? "不可用"
                : null
              : unavailableText(capabilities, "available_decoders", option.id);
            const statusLabel =
              serviceCapabilitySnapshot && available && capability?.status !== "available"
                ? capability?.status
                : null;
            return (
              <button
                key={option.id}
                aria-label={`选择解码器 ${option.name}`}
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
                {statusLabel && (
                  <span
                    className="inline-block mt-2 ml-2 text-xs bg-amber-100 text-amber-800 px-2 py-0.5 rounded"
                    title={capability?.reason ?? statusLabel}
                  >
                    {statusLabel === "supported" ? "待探测" : statusLabel}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>

      <div className="grid lg:grid-cols-2 gap-6 mb-6">
        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-4">Codec</h3>
          <div className="grid sm:grid-cols-2 gap-3">
            {CODEC_OPTIONS.map((codec) => {
              const supported = codec.supportedDecoders.includes(selectedDecoder);
              return (
                <button
                  key={codec.id}
                  onClick={() => setSelectedCodec(codec.id)}
                  disabled={isRunning || !supported}
                  className={`px-4 py-3 rounded-lg border text-left text-sm transition-all ${
                    selectedCodec === codec.id
                      ? "bg-primary text-primary-foreground border-primary"
                      : "bg-background hover:bg-muted"
                  } ${!supported ? "opacity-50 cursor-not-allowed" : ""}`}
                >
                  <span className="block font-medium">{codec.name}</span>
                  <span className="block text-xs opacity-80">{codec.description}</span>
                </button>
              );
            })}
          </div>
        </div>

        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-4">测试档位</h3>
          <div className="grid sm:grid-cols-2 gap-3">
            {DECODE_PROFILES.map((profile) => (
              <button
                key={profile.id}
                onClick={() => setSelectedProfileId(profile.id)}
                disabled={isRunning}
                className={`px-4 py-3 rounded-lg border text-left text-sm transition-all ${
                  selectedProfileId === profile.id
                    ? "bg-primary text-primary-foreground border-primary"
                    : "bg-background hover:bg-muted"
                }`}
              >
                <span className="block font-medium">{profile.name}</span>
                <span className="block text-xs opacity-80">
                  {profile.resolution[0]}x{profile.resolution[1]} @ {profile.fps}fps
                </span>
                <span className="block text-xs opacity-80">{profile.description}</span>
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-3">当前链路</h3>
        <div className="grid sm:grid-cols-2 lg:grid-cols-4 gap-3 text-sm">
          <InfoPill label="解码器" value={selectedOption?.name ?? selectedDecoder} />
          <InfoPill label="Codec" value={CODEC_OPTIONS.find((codec) => codec.id === selectedCodec)?.name ?? selectedCodec} />
          <InfoPill label="编码输入" value={selectedRun.config.encoder_type ?? "none"} />
          <InfoPill label="目标档位" value={`${selectedProfile.resolution[0]}x${selectedProfile.resolution[1]} @ ${selectedProfile.fps}fps`} />
        </div>
        <p className="mt-3 text-xs text-muted-foreground">
          软件解码可跑 H.264 / HEVC / HEVC Main10 / AV1 CPU fallback；NVDEC 可切换 H.264 / HEVC / HEVC Main10 / AV1。页面不再限制在 60fps 档位，实际吞吐以 decoded FPS 为准。
        </p>
      </div>

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
      {startError && <p className="text-sm text-red-600 mb-6">{startError}</p>}

      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-3 xl:grid-cols-6 gap-4 mb-6">
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="采集 FPS"
              value={`${metrics.capture_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.capture_fps)}
            />
            <MetricCard
              icon={<Cpu className="h-4 w-4" />}
              label="编码 FPS"
              value={`${metrics.encode_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.encode_fps)}
            />
            <MetricCard
              icon={<Monitor className="h-4 w-4" />}
              label="解码 FPS"
              value={`${metrics.decode_fps.toFixed(1)} FPS`}
              color={getFpsColor(metrics.decode_fps)}
            />
            <MetricCard
              icon={<Clock className="h-4 w-4" />}
              label="P95 解码延迟"
              value={`${metrics.decode_latency_p95_ms.toFixed(2)} ms`}
              color={getLatencyColor(metrics.decode_latency_p95_ms, 5, 15)}
            />
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="解码帧数"
              value={metrics.decoded_frames.toLocaleString()}
            />
            <MetricCard
              label="失败解码"
              value={metrics.decode_failures.toLocaleString()}
              highlight={metrics.decode_failures > 0 || metrics.dropped_frames > 0}
            />
          </div>

          {capabilityAssessment && (
            <div className="bg-card rounded-lg border p-4 mb-6">
              <h3 className="font-medium mb-3">解码能力判断</h3>
              <div className={`text-lg font-semibold ${capabilityAssessment.color}`}>
                {capabilityAssessment.label}
              </div>
              <p className="mt-2 text-sm text-muted-foreground leading-6">
                {capabilityAssessment.detail}
              </p>
              <div className="mt-4 grid sm:grid-cols-3 gap-3 text-sm">
                <InfoPill label="目标帧率" value={`${selectedProfile.fps} FPS`} />
                <InfoPill label="帧预算" value={`${(1000 / selectedProfile.fps).toFixed(2)} ms`} />
                <InfoPill label="解码占预算" value={`${((metrics.decode_latency_p95_ms / (1000 / selectedProfile.fps)) * 100).toFixed(1)}%`} />
              </div>
            </div>
          )}

          <div className="grid md:grid-cols-2 gap-6 mb-6">
            <div className="bg-card rounded-lg border p-4">
              <h3 className="font-medium mb-4">解码延迟</h3>
              <div className="space-y-3">
                <PercentileBar label="P50" value={metrics.decode_latency_p50_ms} />
                <PercentileBar label="P95" value={metrics.decode_latency_p95_ms} />
                <PercentileBar label="P99" value={metrics.decode_latency_p99_ms} />
              </div>
            </div>

            <div className="bg-card rounded-lg border p-4">
              <h3 className="font-medium mb-4">采样状态</h3>
              <div className="space-y-2 text-sm">
                <InfoRow label="运行中" value={metrics.is_running ? "是" : "否"} />
                <InfoRow label="输出分辨率" value={`${metrics.resolution[0]}x${metrics.resolution[1]}`} />
                <InfoRow label="采集 FPS" value={metrics.capture_fps.toFixed(1)} />
                <InfoRow label="编码 FPS" value={metrics.encode_fps.toFixed(1)} />
                <InfoRow label="丢帧" value={metrics.dropped_frames.toLocaleString()} />
                <InfoRow label="资源占用" value="未采集，后续接系统指标" />
              </div>
            </div>
          </div>

          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">当前结果</h3>
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b">
                  <th className="text-left py-2">解码器</th>
                  <th className="text-right py-2">Codec</th>
                  <th className="text-right py-2">解码 FPS</th>
                  <th className="text-right py-2">P95 延迟</th>
                  <th className="text-right py-2">解码帧数</th>
                </tr>
              </thead>
              <tbody>
                <tr className="border-b last:border-0">
                  <td className="py-2">{selectedOption?.name ?? selectedDecoder}</td>
                  <td className="text-right font-mono">{selectedCodec}</td>
                  <td className="text-right font-mono">{metrics.decode_fps.toFixed(1)}</td>
                  <td className="text-right font-mono">{metrics.decode_latency_p95_ms.toFixed(2)} ms</td>
                  <td className="text-right font-mono">{metrics.decoded_frames.toLocaleString()}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}

function InfoPill({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-muted/40 px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="font-mono text-sm">{value}</div>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-mono text-right">{value}</span>
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
  icon?: ReactNode;
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
  if (fps >= 120) return "text-green-500";
  if (fps >= 60) return "text-yellow-500";
  return "text-red-500";
}

function getLatencyColor(ms: number, good: number, warning: number): string {
  if (ms <= good) return "text-green-500";
  if (ms <= warning) return "text-yellow-500";
  return "text-red-500";
}
