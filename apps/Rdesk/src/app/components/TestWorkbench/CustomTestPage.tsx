import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { Play, Settings, Monitor, Cpu, Zap, Network } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot, TestConfig } from "../../adapters/tauri/types";
import { capabilityAvailable, capabilityTag, unavailableText } from "./capabilityMeta";
import {
  buildCapabilitySnapshotFromIpc,
  capabilityOptionState,
  environmentSnapshotFromCapabilitySnapshot,
  shouldShowCapabilityOptionForSnapshot,
  type CapabilitySnapshot,
} from "../../services/capabilityMatrix";
import {
  shouldShowCapabilityOption,
  useShowUnavailableCapabilities,
} from "./useCapabilityVisibility";

type CaptureId = NonNullable<TestConfig["capture_type"]>;
type EncoderId = NonNullable<TestConfig["encoder_type"]>;
type DecoderId = NonNullable<TestConfig["decoder_type"]>;
type TransportId = NonNullable<TestConfig["transport_kind"]>;
type RendererId = NonNullable<TestConfig["renderer_type"]>;

interface CaptureOption {
  id: CaptureId;
  name: string;
  description: string;
  icon: React.ReactNode;
}

interface EncoderOption {
  id: EncoderId;
  name: string;
  description: string;
  icon: React.ReactNode;
}

interface DecoderOption {
  id: DecoderId;
  name: string;
  description: string;
}

interface TransportOption {
  id: TransportId;
  name: string;
  description: string;
}

const CAPTURE_OPTIONS: CaptureOption[] = [
  {
    id: "dxgi",
    name: "DXGI",
    description: "DirectX Graphics Infrastructure - 高性能桌面捕获",
    icon: <Monitor className="h-5 w-5" />,
  },
  {
    id: "winrt",
    name: "WinRT",
    description: "Windows Runtime - 现代化屏幕捕获 API",
    icon: <Monitor className="h-5 w-5" />,
  },
  {
    id: "macos",
    name: "macOS",
    description: "macOS display capture - Screen Recording permission may be required",
    icon: <Monitor className="h-5 w-5" />,
  },
  {
    id: "linux",
    name: "Linux",
    description: "Linux display capture - PipeWire/Portal path",
    icon: <Monitor className="h-5 w-5" />,
  },
  {
    id: "synthetic",
    name: "Synthetic",
    description: "Synthetic frame generator - baseline pipeline input",
    icon: <Monitor className="h-5 w-5" />,
  },
];

const ENCODER_OPTIONS: EncoderOption[] = [
  {
    id: "none",
    name: "直连渲染",
    description: "跳过编码 - 采集帧直接进入渲染器",
    icon: <Monitor className="h-5 w-5" />,
  },
  {
    id: "nvenc_h264",
    name: "NVENC H.264",
    description: "NVIDIA 硬件编码器 - 低延迟高质量",
    icon: <Zap className="h-5 w-5" />,
  },
  {
    id: "nvenc_hevc",
    name: "NVENC HEVC Main",
    description: "NVIDIA HEVC Main - QUIC/loopback + NVDEC 全 GPU 链路",
    icon: <Zap className="h-5 w-5" />,
  },
  {
    id: "nvenc_hevc_main10",
    name: "NVENC HEVC Main10",
    description: "NVIDIA HEVC Main10 - 10-bit 编码能力与 NVDEC 对照",
    icon: <Zap className="h-5 w-5" />,
  },
  {
    id: "nvenc_av1",
    name: "NVENC AV1",
    description: "NVIDIA AV1 编码器 - 新一代压缩效率",
    icon: <Zap className="h-5 w-5" />,
  },
  {
    id: "openh264",
    name: "OpenH264",
    description: "软件编码器 - 跨平台兼容",
    icon: <Cpu className="h-5 w-5" />,
  },
  {
    id: "videotoolbox_h264",
    name: "VideoToolbox H.264",
    description: "macOS VideoToolbox - Apple 硬件 H.264 编码",
    icon: <Zap className="h-5 w-5" />,
  },
];

const DECODER_OPTIONS: DecoderOption[] = [
  {
    id: "none",
    name: "无解码",
    description: "encode-only 或直接渲染链路",
  },
  {
    id: "nvdec",
    name: "NVDEC",
    description: "NVIDIA 硬件解码器 - GPU 加速",
  },
  {
    id: "software",
    name: "软件解码",
    description: "FFmpeg 软件解码 - CPU 解码",
  },
  {
    id: "linux_h264",
    name: "Linux H.264 HW",
    description: "Linux GStreamer H.264 硬件解码 - 当前输出 CPU RGB 帧",
  },
  {
    id: "linux_hevc",
    name: "Linux HEVC HW",
    description: "Linux GStreamer HEVC 硬件解码 - 当前输出 CPU RGB 帧",
  },
  {
    id: "linux_hevc_main10",
    name: "Linux HEVC Main10 HW",
    description: "Linux GStreamer HEVC Main10 硬件解码 - 当前输出 CPU RGB 帧",
  },
  {
    id: "videotoolbox",
    name: "VideoToolbox",
    description: "macOS VideoToolbox - Apple 硬件 H.264 解码",
  },
];

const RESOLUTIONS = [
  { id: "1280x720", name: "720p", width: 1280, height: 720 },
  { id: "1920x1080", name: "1080p", width: 1920, height: 1080 },
  { id: "2560x1440", name: "1440p", width: 2560, height: 1440 },
  { id: "3840x2160", name: "4K", width: 3840, height: 2160 },
];

const FPS_OPTIONS = [30, 60, 120, 144];

const BITRATE_OPTIONS = [
  { id: "1000", name: "1 Mbps", value: 1000000 },
  { id: "3000", name: "3 Mbps", value: 3000000 },
  { id: "5000", name: "5 Mbps", value: 5000000 },
  { id: "10000", name: "10 Mbps", value: 10000000 },
  { id: "20000", name: "20 Mbps", value: 20000000 },
];

const TRANSPORT_OPTIONS: TransportOption[] = [
  {
    id: "loopback",
    name: "Loopback",
    description: "本机链路基线，测编解码和渲染开销",
  },
  {
    id: "quic",
    name: "QUIC Datagram",
    description: "远程性能对齐链路，支持 H.264/HEVC/AV1",
  },
  {
    id: "webrtc",
    name: "WebRTC RTP",
    description: "浏览器 RTP 路径，支持 H.264/HEVC/AV1",
  },
];

function isHevcEncoder(encoder: EncoderId): boolean {
  return encoder === "nvenc_hevc" || encoder === "nvenc_hevc_main10";
}

function resolveRendererType(
  capture: CaptureId,
  encoder: EncoderId,
  decoder: DecoderId,
  capabilities: EnvironmentSnapshot | null
): RendererId {
  if (capture === "macos" || encoder === "videotoolbox_h264" || decoder === "videotoolbox") {
    return "macos";
  }
  if (capture === "linux") {
    return "linux";
  }
  if (capabilityAvailable(capabilities, "available_renderers", "d3d11")) {
    return "d3d11";
  }
  if (capabilityAvailable(capabilities, "available_renderers", "linux")) {
    return "linux";
  }
  if (capabilityAvailable(capabilities, "available_renderers", "macos")) {
    return "macos";
  }
  return "d3d11";
}

export function CustomTestPage() {
  const navigate = useNavigate();
  const [selectedCapture, setSelectedCapture] = useState<CaptureId>("dxgi");
  const [selectedEncoder, setSelectedEncoder] = useState<EncoderId>("nvenc_hevc");
  const [selectedDecoder, setSelectedDecoder] = useState<DecoderId>("nvdec");
  const [selectedTransport, setSelectedTransport] = useState<TransportId>("quic");
  const [selectedResolution, setSelectedResolution] = useState("1920x1080");
  const [selectedFps, setSelectedFps] = useState(60);
  const [selectedBitrate, setSelectedBitrate] = useState("20000");
  const [starting, setStarting] = useState(false);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [showUnavailable] = useShowUnavailableCapabilities();

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

  const isCaptureAvailable = (capture: CaptureId) =>
    serviceCapabilitySnapshot
      ? capabilityOptionState(serviceCapabilitySnapshot, "capture", capture) !== "disabled"
      : capabilityAvailable(capabilities, "available_captures", capture, capture === "synthetic");
  const isEncoderAvailable = (encoder: EncoderId) =>
    encoder === "none" ||
    (serviceCapabilitySnapshot
      ? capabilityOptionState(serviceCapabilitySnapshot, "encoder", encoder) !== "disabled"
      : capabilityAvailable(capabilities, "available_encoders", encoder, encoder === "openh264"));
  const isDecoderAvailable = (decoder: DecoderId) =>
    decoder === "none" ||
    (serviceCapabilitySnapshot
      ? capabilityOptionState(serviceCapabilitySnapshot, "decoder", decoder) !== "disabled"
      : capabilityAvailable(capabilities, "available_decoders", decoder, decoder === "software"));
  const selectedRenderer = resolveRendererType(
    selectedCapture,
    selectedEncoder,
    selectedDecoder,
    capabilities
  );
  const isRendererAvailable = capabilityAvailable(
    capabilities,
    "available_renderers",
    selectedRenderer
  );
  const visibleCaptureOptions = CAPTURE_OPTIONS.filter((option) =>
    serviceCapabilitySnapshot
      ? shouldShowCapabilityOptionForSnapshot(
          serviceCapabilitySnapshot,
          "capture",
          option.id,
          showUnavailable
        )
      : !capabilities || shouldShowCapabilityOption(isCaptureAvailable(option.id), showUnavailable)
  );
  const visibleEncoderOptions = ENCODER_OPTIONS.filter((option) =>
    serviceCapabilitySnapshot
      ? shouldShowCapabilityOptionForSnapshot(
          serviceCapabilitySnapshot,
          "encoder",
          option.id,
          showUnavailable
        )
      : !capabilities || shouldShowCapabilityOption(isEncoderAvailable(option.id), showUnavailable)
  );
  const visibleDecoderOptions = DECODER_OPTIONS.filter((option) =>
    serviceCapabilitySnapshot
      ? shouldShowCapabilityOptionForSnapshot(
          serviceCapabilitySnapshot,
          "decoder",
          option.id,
          showUnavailable
        )
      : !capabilities || shouldShowCapabilityOption(isDecoderAvailable(option.id), showUnavailable)
  );

  useEffect(() => {
    if (!capabilities) return;

    if (!isCaptureAvailable(selectedCapture)) {
      const nextCapture = CAPTURE_OPTIONS.find((option) => isCaptureAvailable(option.id));
      if (nextCapture) setSelectedCapture(nextCapture.id);
    }
    if (!isEncoderAvailable(selectedEncoder)) {
      const nextEncoder = ENCODER_OPTIONS.find((option) => isEncoderAvailable(option.id));
      if (nextEncoder) setSelectedEncoder(nextEncoder.id);
    }
    if (selectedEncoder === "none" && selectedDecoder !== "none") {
      setSelectedDecoder("none");
    }
    if (!isDecoderAvailable(selectedDecoder)) {
      const nextDecoder = DECODER_OPTIONS.find((option) => isDecoderAvailable(option.id));
      if (nextDecoder) setSelectedDecoder(nextDecoder.id);
    }
  }, [
    capabilities,
    selectedCapture,
    selectedDecoder,
    selectedEncoder,
    selectedTransport,
    serviceCapabilitySnapshot,
  ]);

  const blockedReason = () => {
    if (!isCaptureAvailable(selectedCapture)) {
      return "当前平台未暴露所选采集能力。";
    }
    if (!isEncoderAvailable(selectedEncoder)) {
      if (selectedEncoder === "nvenc_av1") {
        return "当前 GPU/驱动未暴露 NVENC AV1 编码能力。RTX 30 系通常支持 AV1 解码，但不支持 AV1 NVENC 编码。";
      }
      if (isHevcEncoder(selectedEncoder)) {
        return "当前 GPU/驱动未暴露 NVENC HEVC 编码能力。";
      }
      return "当前环境未暴露所选编码器能力。";
    }
    if (!isDecoderAvailable(selectedDecoder)) {
      return selectedDecoder === "videotoolbox"
        ? "VideoToolbox 解码当前为实验路径，需显式启用后才可测试。"
        : "当前平台未暴露所选解码能力。";
    }
    if (!isRendererAvailable) {
      return "当前平台未暴露所选渲染能力。";
    }
    if (selectedEncoder === "none" && selectedDecoder !== "none") {
      return "直接渲染链路不经过码流，请选择无解码。";
    }
    if (selectedEncoder === "nvenc_av1" && selectedDecoder === "linux_h264") {
      return "Linux H.264 硬解当前只接入 H.264，不能解码 NVENC AV1 输出。";
    }
    if (
      selectedEncoder === "nvenc_av1" &&
      (selectedDecoder === "linux_hevc" || selectedDecoder === "linux_hevc_main10")
    ) {
      return "Linux HEVC 硬解不能解码 NVENC AV1 输出。";
    }
    if (isHevcEncoder(selectedEncoder) && selectedDecoder === "linux_h264") {
      return "Linux H.264 硬解当前只接入 H.264，不能解码 NVENC HEVC 输出。";
    }
    if (selectedEncoder === "nvenc_hevc_main10" && selectedDecoder === "linux_hevc") {
      return "NVENC HEVC Main10 请使用 Linux HEVC Main10 硬解路径。";
    }
    if (
      (selectedEncoder === "nvenc_h264" ||
        selectedEncoder === "openh264" ||
        selectedEncoder === "videotoolbox_h264") &&
      (selectedDecoder === "linux_hevc" || selectedDecoder === "linux_hevc_main10")
    ) {
      return "Linux HEVC 硬解不能解码 H.264 输出。";
    }
    if (selectedEncoder === "videotoolbox_h264" && selectedDecoder === "nvdec") {
      return "VideoToolbox H.264 是 macOS 原生路径，请选择 VideoToolbox、软件解码或 encode-only。";
    }
    if (
      (selectedEncoder === "nvenc_av1" || isHevcEncoder(selectedEncoder)) &&
      selectedDecoder === "videotoolbox"
    ) {
      return "VideoToolbox H.264 解码器不能解码 NVENC AV1/HEVC 输出。";
    }
    return null;
  };

  const handleStart = async () => {
    const reason = blockedReason();
    if (reason) {
      setStartError(reason);
      return;
    }

    setStarting(true);
    setStartError(null);
    const config: TestConfig = {
      capture_type: selectedCapture,
      encoder_type: selectedEncoder,
      decoder_type: selectedDecoder,
      transport_kind: selectedTransport,
      renderer_type: selectedRenderer,
      render_display: true,
      zero_copy:
        selectedRenderer === "d3d11" &&
        selectedCapture !== "synthetic" &&
        (selectedEncoder === "none" || selectedEncoder.startsWith("nvenc"))
          ? true
          : undefined,
      resolution: (() => {
        const resolution = RESOLUTIONS.find((r) => r.id === selectedResolution)!;
        return [resolution.width, resolution.height] as [number, number];
      })(),
      fps: selectedFps,
      bitrate: Number(selectedBitrate) * 1000,
      duration_ms: 30000,
      warmup_ms: 2000,
    };

    const result = await commands.testStartRun({
      scenarioId: "custom",
      config,
    });

    if (result.ok) {
      navigate(`/test/run/${result.value}`);
    } else {
      setStartError(result.error.message);
    }

    setStarting(false);
  };

  const canStart = () => {
    return blockedReason() === null;
  };

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground">自由组合测试</h1>
        <p className="text-muted-foreground">
          自定义测试配置，自由组合管道组件
        </p>
      </div>

      {/* Pipeline Visualization */}
      <div className="bg-card rounded-lg border p-6 mb-6">
        <h2 className="text-sm font-medium mb-4 flex items-center gap-2">
          <Settings className="h-4 w-4" />
          管道配置
        </h2>
        <div className="flex items-center justify-center gap-2 md:gap-4">
          {/* Capture */}
          <PipelineStage
            label="捕获"
            value={CAPTURE_OPTIONS.find((c) => c.id === selectedCapture)?.name || "-"}
            icon={<Monitor className="h-5 w-5" />}
          />
          <Arrow />
          {/* Encoder */}
          <PipelineStage
            label="编码"
            value={ENCODER_OPTIONS.find((e) => e.id === selectedEncoder)?.name || "-"}
            icon={
              ENCODER_OPTIONS.find((e) => e.id === selectedEncoder)?.icon || <Zap className="h-5 w-5" />
            }
          />
          <Arrow />
          {/* Transport */}
          <PipelineStage
            label="传输"
            value={TRANSPORT_OPTIONS.find((t) => t.id === selectedTransport)?.name || "-"}
            icon={<Network className="h-5 w-5" />}
          />
          <Arrow />
          {/* Decoder */}
          <PipelineStage
            label="解码"
            value={DECODER_OPTIONS.find((d) => d.id === selectedDecoder)?.name || "-"}
            icon={<Cpu className="h-5 w-5" />}
          />
        </div>
      </div>

      {/* Component Selection */}
      <div className="grid md:grid-cols-2 xl:grid-cols-4 gap-6 mb-6">
        {/* Capture Selection */}
        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-3">捕获源</h3>
          <div className="space-y-2">
            {visibleCaptureOptions.map((option) => {
              const available = isCaptureAvailable(option.id);
              const disabledLabel = unavailableText(capabilities, "available_captures", option.id);
              return (
              <label
                key={option.id}
                className={`flex items-start gap-3 p-3 rounded cursor-pointer border transition-colors ${
                  selectedCapture === option.id
                    ? "bg-primary/10 border-primary"
                    : "bg-background hover:bg-muted"
                } ${!available ? "opacity-50 cursor-not-allowed" : ""}`}
              >
                <input
                  type="radio"
                  name="capture"
                  value={option.id}
                  checked={selectedCapture === option.id}
                  onChange={(e) => setSelectedCapture(e.target.value as CaptureId)}
                  className="mt-1"
                  disabled={!available}
                />
                <div className="flex-1">
                  <div className="flex items-center gap-2 font-medium text-sm">
                    {option.icon}
                    {option.name}
                    <span className="text-xs bg-muted px-1 rounded">
                      {capabilityTag(option.id)}
                    </span>
                    {disabledLabel && (
                      <span className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded">
                        {disabledLabel}
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-1">{option.description}</p>
                </div>
              </label>
              );
            })}
          </div>
        </div>

        {/* Encoder Selection */}
        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-3">编码器</h3>
          <div className="space-y-2">
            {visibleEncoderOptions.map((option) => {
              const available = isEncoderAvailable(option.id);
              const isAv1Unavailable = option.id === "nvenc_av1" && !available;

              return (
              <label
                key={option.id}
                className={`flex items-start gap-3 p-3 rounded cursor-pointer border transition-colors ${
                  selectedEncoder === option.id
                    ? "bg-primary/10 border-primary"
                    : "bg-background hover:bg-muted"
                } ${!available ? "opacity-50" : ""}`}
              >
                <input
                  type="radio"
                  name="encoder"
                  value={option.id}
                  checked={selectedEncoder === option.id}
                  onChange={(e) => setSelectedEncoder(e.target.value as EncoderId)}
                  className="mt-1"
                  disabled={!available}
                />
                <div className="flex-1">
                  <div className="flex items-center gap-2 font-medium text-sm">
                    {option.icon}
                    {option.name}
                    <span className="text-xs bg-muted px-1 rounded">
                      {capabilityTag(option.id)}
                    </span>
                    {isAv1Unavailable && (
                      <span className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded">
                        GPU 不支持
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-1">{option.description}</p>
                  {isAv1Unavailable && (
                    <p className="text-xs text-yellow-700 mt-1">
                      当前机器没有 AV1 NVENC 编码能力
                    </p>
                  )}
                </div>
              </label>
              );
            })}
          </div>
        </div>

        {/* Decoder Selection */}
        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-3">解码器</h3>
          <div className="space-y-2">
            {visibleDecoderOptions.map((option) => {
              const available = isDecoderAvailable(option.id);
              const disabledLabel =
                option.id === "none"
                  ? null
                  : unavailableText(capabilities, "available_decoders", option.id);
              return (
              <label
                key={option.id}
                className={`flex items-start gap-3 p-3 rounded cursor-pointer border transition-colors ${
                  selectedDecoder === option.id
                    ? "bg-primary/10 border-primary"
                    : "bg-background hover:bg-muted"
                } ${!available ? "opacity-50 cursor-not-allowed" : ""}`}
              >
                <input
                  type="radio"
                  name="decoder"
                  value={option.id}
                  checked={selectedDecoder === option.id}
                  onChange={(e) => setSelectedDecoder(e.target.value as DecoderId)}
                  className="mt-1"
                  disabled={!available}
                />
                <div className="flex-1">
                  <div className="flex items-center gap-2 font-medium text-sm">
                    {option.name}
                    <span className="text-xs bg-muted px-1 rounded">
                      {capabilityTag(option.id)}
                    </span>
                    {disabledLabel && (
                      <span className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded">
                        {disabledLabel}
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-1">{option.description}</p>
                </div>
              </label>
              );
            })}
          </div>
        </div>

        {/* Transport Selection */}
        <div className="bg-card rounded-lg border p-4">
          <h3 className="font-medium mb-3">传输层</h3>
          <div className="space-y-2">
            {TRANSPORT_OPTIONS.map((option) => (
              <label
                key={option.id}
                className={`flex items-start gap-3 p-3 rounded cursor-pointer border transition-colors ${
                  selectedTransport === option.id
                    ? "bg-primary/10 border-primary"
                    : "bg-background hover:bg-muted"
                }`}
              >
                <input
                  type="radio"
                  name="transport"
                  value={option.id}
                  checked={selectedTransport === option.id}
                  onChange={(e) => setSelectedTransport(e.target.value as TransportId)}
                  className="mt-1"
                />
                <div className="flex-1">
                  <div className="flex items-center gap-2 font-medium text-sm">
                    {option.name}
                    <span className="text-xs bg-muted px-1 rounded">
                      {capabilityTag(option.id)}
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground mt-1">{option.description}</p>
                </div>
              </label>
            ))}
          </div>
        </div>
      </div>

      {/* Parameters */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">参数设置</h3>
        <div className="grid md:grid-cols-3 gap-4">
          {/* Resolution */}
          <div>
            <label className="block text-sm font-medium mb-2">分辨率</label>
            <select
              value={selectedResolution}
              onChange={(e) => setSelectedResolution(e.target.value)}
              className="w-full px-3 py-2 border rounded bg-background"
            >
              {RESOLUTIONS.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name} ({r.width}x{r.height})
                </option>
              ))}
            </select>
          </div>

          {/* FPS */}
          <div>
            <label className="block text-sm font-medium mb-2">帧率</label>
            <select
              value={selectedFps}
              onChange={(e) => setSelectedFps(Number(e.target.value))}
              className="w-full px-3 py-2 border rounded bg-background"
            >
              {FPS_OPTIONS.map((fps) => (
                <option key={fps} value={fps}>
                  {fps} FPS
                </option>
              ))}
            </select>
          </div>

          {/* Bitrate */}
          <div>
            <label className="block text-sm font-medium mb-2">码率</label>
            <select
              value={selectedBitrate}
              onChange={(e) => setSelectedBitrate(e.target.value)}
              className="w-full px-3 py-2 border rounded bg-background"
            >
              {BITRATE_OPTIONS.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name}
                </option>
              ))}
            </select>
          </div>
        </div>
      </div>

      {/* Start Button */}
      <div className="flex justify-center">
        <button
          onClick={handleStart}
          disabled={starting || !canStart()}
          className="flex items-center gap-2 px-6 py-3 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Play className="h-5 w-5" />
          {starting ? "启动中..." : "启动测试"}
        </button>
      </div>

      {!canStart() && (
        <p className="text-center text-sm text-yellow-600 mt-4">
          {blockedReason()}
        </p>
      )}
      {startError && (
        <p className="text-center text-sm text-red-600 mt-4">
          {startError}
        </p>
      )}
    </div>
  );
}

function PipelineStage({
  label,
  value,
  icon,
}: {
  label: string;
  value: string;
  icon: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-center">
      <div className="bg-primary/10 p-3 rounded-lg border border-primary">
        {icon}
      </div>
      <p className="text-xs text-muted-foreground mt-2">{label}</p>
      <p className="font-medium text-sm">{value}</p>
    </div>
  );
}

function Arrow() {
  return (
    <div className="flex items-center justify-center text-muted-foreground">
      <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M5 12h14M12 5l7 7-7 7" />
      </svg>
    </div>
  );
}
