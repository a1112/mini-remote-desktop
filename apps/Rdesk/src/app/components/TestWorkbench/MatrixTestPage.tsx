import { useEffect, useState } from "react";
import { Play, Grid3x3, CheckCircle2, XCircle, Clock, Loader2 } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type {
  EnvironmentSnapshot,
  TestConfig,
  TestRun,
  TestRunSummary,
} from "../../adapters/tauri/types";

interface MatrixDimension {
  id: string;
  name: string;
  options: MatrixOption[];
}

interface MatrixOption {
  id: string;
  name: string;
  enabled: boolean;
  defaultEnabledOn?: HostOs[];
}

type HostOs = "windows" | "macos" | "linux" | "other";

const MATRIX_DIMENSIONS: MatrixDimension[] = [
  {
    id: "capture",
    name: "捕获",
    options: [
      { id: "dxgi", name: "DXGI", enabled: true },
      { id: "winrt", name: "WinRT", enabled: false },
      { id: "macos", name: "macOS", enabled: false, defaultEnabledOn: ["macos"] },
      { id: "synthetic", name: "Synthetic", enabled: false },
    ],
  },
  {
    id: "encoder",
    name: "编码器",
    options: [
      { id: "nvenc_h264", name: "NVENC H.264", enabled: true },
      { id: "openh264", name: "OpenH264", enabled: true },
      { id: "nvenc_av1", name: "NVENC AV1", enabled: false },
      {
        id: "videotoolbox_h264",
        name: "VideoToolbox H.264",
        enabled: false,
        defaultEnabledOn: ["macos"],
      },
    ],
  },
  {
    id: "decoder",
    name: "解码器",
    options: [
      { id: "none", name: "None / encode only", enabled: false },
      { id: "nvdec", name: "NVDEC", enabled: true },
      { id: "software", name: "软件", enabled: true },
      {
        id: "videotoolbox",
        name: "VideoToolbox",
        enabled: false,
        defaultEnabledOn: ["macos"],
      },
    ],
  },
  {
    id: "transport",
    name: "传输层",
    options: [
      { id: "loopback", name: "Loopback", enabled: true },
      { id: "webrtc", name: "WebRTC RTP", enabled: false },
      { id: "quic", name: "QUIC Datagram", enabled: false },
    ],
  },
  {
    id: "renderer",
    name: "渲染",
    options: [
      { id: "renderer_none", name: "No display", enabled: true },
      { id: "d3d11", name: "DX11 popup", enabled: false },
      { id: "macos", name: "Metal", enabled: false },
    ],
  },
  {
    id: "memory",
    name: "Memory",
    options: [
      { id: "cpu", name: "CPU", enabled: true },
      { id: "d3d11_shared", name: "D3D11 shared texture", enabled: false },
    ],
  },
  {
    id: "resolution",
    name: "分辨率",
    options: [
      { id: "1280x720", name: "720p", enabled: true },
      { id: "1366x768", name: "768p", enabled: false },
      { id: "1600x900", name: "900p", enabled: false },
      { id: "1920x1080", name: "1080p", enabled: true },
      { id: "1920x1200", name: "1200p", enabled: false },
      { id: "2560x1440", name: "1440p", enabled: false },
      { id: "2560x1600", name: "1600p", enabled: false },
      { id: "3440x1440", name: "UWQHD", enabled: false },
      { id: "3840x2160", name: "4K", enabled: false },
    ],
  },
  {
    id: "fps",
    name: "帧率",
    options: [
      { id: "24", name: "24 FPS", enabled: false },
      { id: "30", name: "30 FPS", enabled: true },
      { id: "45", name: "45 FPS", enabled: false },
      { id: "60", name: "60 FPS", enabled: true },
      { id: "90", name: "90 FPS", enabled: false },
      { id: "120", name: "120 FPS", enabled: false },
      { id: "144", name: "144 FPS", enabled: false },
    ],
  },
  {
    id: "bitrate",
    name: "码率",
    options: [
      { id: "3000000", name: "3 Mbps", enabled: false },
      { id: "5000000", name: "5 Mbps", enabled: true },
      { id: "8000000", name: "8 Mbps", enabled: false },
      { id: "12000000", name: "12 Mbps", enabled: false },
      { id: "20000000", name: "20 Mbps", enabled: false },
    ],
  },
  {
    id: "duration",
    name: "时长",
    options: [
      { id: "3000", name: "3 秒", enabled: false },
      { id: "5000", name: "5 秒", enabled: true },
      { id: "10000", name: "10 秒", enabled: false },
      { id: "30000", name: "30 秒", enabled: false },
    ],
  },
];

function normalizeHostOs(osType?: string): HostOs {
  const normalized = osType?.toLowerCase() ?? "";
  if (normalized.includes("windows") || normalized === "win32") return "windows";
  if (normalized.includes("mac") || normalized === "darwin") return "macos";
  if (normalized.includes("linux")) return "linux";
  return "other";
}

function defaultCapturesForOs(os: HostOs): string[] {
  if (os === "windows") return ["dxgi", "winrt", "synthetic"];
  if (os === "macos") return ["macos", "synthetic"];
  return ["synthetic"];
}

function defaultEncodersForOs(os: HostOs): string[] {
  if (os === "windows") return ["nvenc_h264", "openh264", "nvenc_av1"];
  if (os === "macos") return ["videotoolbox_h264", "openh264"];
  return ["openh264"];
}

function defaultDecodersForOs(os: HostOs): string[] {
  if (os === "windows") return ["nvdec", "software", "none"];
  if (os === "macos") return ["software", "none"];
  return ["software", "none"];
}

function defaultRenderersForOs(os: HostOs): string[] {
  if (os === "windows") return ["none", "d3d11"];
  if (os === "macos") return ["none", "macos"];
  return ["none"];
}

function defaultMemoryModesForOs(os: HostOs): string[] {
  return os === "windows" ? ["cpu", "d3d11_shared"] : ["cpu"];
}

function optionEnabledForOs(option: MatrixOption, os: HostOs): boolean {
  return option.defaultEnabledOn ? option.defaultEnabledOn.includes(os) : option.enabled;
}

function createMatrixDimensions(capabilities?: EnvironmentSnapshot | null): MatrixDimension[] {
  const os = normalizeHostOs(capabilities?.os_type ?? "windows");
  const availableCaptures = capabilities?.available_captures ?? defaultCapturesForOs(os);
  const availableEncoders = capabilities?.available_encoders ?? defaultEncodersForOs(os);
  const availableDecoders = [
    "none",
    ...(capabilities?.available_decoders ?? defaultDecodersForOs(os)),
  ].filter((value, index, values) => values.indexOf(value) === index);
  const availableRenderers = capabilities?.available_renderers ?? defaultRenderersForOs(os);
  const availableMemoryModes =
    capabilities?.available_memory_modes ?? defaultMemoryModesForOs(os);

  const optionAvailable = (dimensionId: string, optionId: string): boolean => {
    switch (dimensionId) {
      case "capture":
        return availableCaptures.includes(optionId);
      case "encoder":
        return availableEncoders.includes(optionId);
      case "decoder":
        return availableDecoders.includes(optionId);
      case "renderer":
        return optionId === "renderer_none"
          ? availableRenderers.includes("none")
          : availableRenderers.includes(optionId);
      case "memory":
        return availableMemoryModes.includes(optionId);
      default:
        return true;
    }
  };

  return MATRIX_DIMENSIONS.map((dimension) => ({
    ...dimension,
    options: dimension.options
      .filter((option) => optionAvailable(dimension.id, option.id))
      .map((option) => ({
        ...option,
        enabled: optionEnabledForOs(option, os),
      })),
  })).filter((dimension) => dimension.options.length > 0);
}

interface MatrixTest {
  id: string;
  config: TestConfig;
  status: "pending" | "running" | "completed" | "failed" | "skipped";
  result?: TestRunSummary;
  duration?: number;
  skipReason?: string;
  failureReason?: string;
}

interface SelectedMatrixOption {
  dimensionId: string;
  option: MatrixOption;
}

interface MatrixAcceptanceResult {
  acceptable: boolean;
  reason?: string;
}

function evaluateMatrixRun(
  config: TestConfig,
  summary?: TestRunSummary
): MatrixAcceptanceResult {
  if (!summary || summary.frame_count <= 0 || summary.error_message) {
    return {
      acceptable: false,
      reason: summary?.error_message ?? "No frames were produced",
    };
  }

  const targetFps = Math.max(1, config.fps ?? 60);
  const minFps = minimumExpectedFps(config, targetFps);
  const captureFps = summary.capture_fps ?? 0;
  if (captureFps < minFps) {
    return {
      acceptable: false,
      reason: `Pipeline FPS ${captureFps.toFixed(1)} < ${minFps.toFixed(1)} expected`,
    };
  }

  const maxTotalP95Ms = maximumExpectedLatencyMs(config, targetFps);
  const totalLatencyP95 = summary.total_latency_p95 ?? Number.POSITIVE_INFINITY;
  if (totalLatencyP95 > maxTotalP95Ms) {
    const [slowestStage, slowestStageMs] = slowestPipelineStage(summary);
    return {
      acceptable: false,
      reason: `${slowestStage} P95 ${slowestStageMs.toFixed(
        2
      )} ms; total P95 ${totalLatencyP95.toFixed(2)} ms > ${maxTotalP95Ms.toFixed(2)} ms budget`,
    };
  }

  return { acceptable: true };
}

function minimumExpectedFps(config: TestConfig, targetFps: number): number {
  if (config.capture_type === "macos") {
    if (config.encoder_type === "openh264") {
      return targetFps * 0.3;
    }
    if (config.encoder_type === "videotoolbox_h264") {
      return targetFps * 0.35;
    }
  }
  if (config.encoder_type === "openh264") {
    return targetFps * 0.35;
  }
  if (config.decoder_type === "software") {
    return targetFps * 0.45;
  }
  return targetFps * 0.6;
}

function maximumExpectedLatencyMs(config: TestConfig, targetFps: number): number {
  const frameBudgetMs = 1000 / targetFps;
  if (config.encoder_type === "openh264") {
    return Math.max(120, frameBudgetMs * 8);
  }
  if (config.decoder_type === "software") {
    return Math.max(80, frameBudgetMs * 5);
  }
  return Math.max(100, frameBudgetMs * 4);
}

function slowestPipelineStage(summary: TestRunSummary): readonly [string, number] {
  const stages = [
    ["encode", summary.encode_latency_p95 ?? 0],
    ["transport", summary.transport_latency_p95 ?? 0],
    ["decode", summary.decode_latency_p95 ?? 0],
  ] as const;

  return stages.reduce((slowest, current) =>
    current[1] > slowest[1] ? current : slowest
  );
}

function unsupportedMatrixReason(config: TestConfig): string | null {
  if (config.zero_copy && config.capture_type !== "dxgi") {
    return "D3D11 shared texture path requires DXGI capture";
  }
  if (config.zero_copy && config.encoder_type !== "nvenc_h264") {
    return "D3D11 shared texture input currently requires NVENC H.264";
  }
  if (config.encoder_type === "nvenc_av1" && config.zero_copy) {
    return "NVENC AV1 D3D11 shared input path is not implemented yet";
  }
  if (config.encoder_type === "nvenc_av1" && config.decoder_type === "software") {
    return "NVENC AV1 currently requires NVDEC or encode-only matrix runs";
  }
  if (config.zero_copy && config.decoder_type !== "nvdec") {
    return "D3D11 shared texture path requires NVDEC";
  }
  if (config.zero_copy && (config.renderer_type !== "d3d11" || !config.render_display)) {
    return "D3D11 shared texture path requires DX11 popup renderer";
  }
  if (config.renderer_type === "d3d11" && config.capture_type === "macos") {
    return "DX11 popup renderer is Windows-only";
  }
  if (config.renderer_type === "macos" && config.zero_copy) {
    return "Metal renderer does not accept D3D11 shared texture input";
  }
  if (config.encoder_type === "videotoolbox_h264" && config.decoder_type === "nvdec") {
    return "VideoToolbox H.264 output should use VideoToolbox, software, or encode-only decode modes";
  }
  if (config.encoder_type === "nvenc_av1" && config.decoder_type === "videotoolbox") {
    return "VideoToolbox decoder path is H.264-only in this matrix";
  }
  return null;
}

function capabilitySkipReason(config: TestConfig, message: string): string | null {
  if (/not supported on/i.test(message)) {
    return message;
  }
  if (
    config.encoder_type === "nvenc_av1" &&
    /NVENC AV1 unavailable|AV1 codec not supported|NVENC AV1 preset query failed/i.test(message)
  ) {
    return message;
  }
  return null;
}

const STATUS_LABELS: Record<MatrixTest["status"], string> = {
  pending: "待执行",
  running: "运行中",
  completed: "完成",
  failed: "失败",
  skipped: "跳过",
};

function formatMs(value: number | null | undefined): string {
  return value != null && Number.isFinite(value) ? `${value.toFixed(2)} ms` : "-";
}

interface MatrixTestPageProps {
  runDelayMs?: number;
}

function setOptionEnabled(
  dimensions: MatrixDimension[],
  dimensionId: string,
  optionId: string,
  enabled: boolean
): MatrixDimension[] {
  return dimensions.map((dim) =>
    dim.id === dimensionId
      ? {
          ...dim,
          options: dim.options.map((opt) =>
            opt.id === optionId ? { ...opt, enabled } : opt
          ),
        }
      : dim
  );
}

function isOptionEnabled(
  dimensions: MatrixDimension[],
  dimensionId: string,
  optionId: string
): boolean {
  return (
    dimensions
      .find((dim) => dim.id === dimensionId)
      ?.options.some((opt) => opt.id === optionId && opt.enabled) ?? false
  );
}

function matrixConfigKey(config: TestConfig): string {
  return JSON.stringify({
    capture_type: config.capture_type,
    encoder_type: config.encoder_type,
    decoder_type: config.decoder_type,
    transport_kind: config.transport_kind,
    renderer_type: config.renderer_type,
    render_display: config.render_display,
    zero_copy: config.zero_copy,
    resolution: config.resolution,
    fps: config.fps,
    bitrate: config.bitrate,
    duration_ms: config.duration_ms,
    warmup_ms: config.warmup_ms,
  });
}

export function MatrixTestPage({ runDelayMs = 7000 }: MatrixTestPageProps = {}) {
  const [dimensions, setDimensions] = useState<MatrixDimension[]>(() =>
    createMatrixDimensions(null)
  );
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [tests, setTests] = useState<MatrixTest[]>([]);
  const [currentTestIndex, setCurrentTestIndex] = useState(0);
  const [completedCount, setCompletedCount] = useState(0);
  const [failedCount, setFailedCount] = useState(0);
  const [skippedCount, setSkippedCount] = useState(0);

  useEffect(() => {
    let cancelled = false;

    async function loadCapabilities() {
      const result = await commands.testGetCapabilities();
      if (!cancelled && result.ok && result.value) {
        setCapabilities(result.value);
        setDimensions(createMatrixDimensions(result.value));
      }
    }

    void loadCapabilities();

    return () => {
      cancelled = true;
    };
  }, []);

  const toggleOption = (dimensionId: string, optionId: string) => {
    setDimensions((current) => {
      let next = current.map((dim) =>
        dim.id === dimensionId
          ? {
              ...dim,
              options: dim.options.map((opt) =>
                opt.id === optionId ? { ...opt, enabled: !opt.enabled } : opt
              ),
            }
          : dim
      );

      if (
        dimensionId === "memory" &&
        optionId === "d3d11_shared" &&
        isOptionEnabled(next, "memory", "d3d11_shared")
      ) {
        next = setOptionEnabled(next, "renderer", "d3d11", true);
      }

      if (
        dimensionId === "renderer" &&
        optionId === "d3d11" &&
        !isOptionEnabled(next, "renderer", "d3d11")
      ) {
        next = setOptionEnabled(next, "memory", "d3d11_shared", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId !== "renderer_none" &&
        isOptionEnabled(next, "renderer", optionId)
      ) {
        next = setOptionEnabled(next, "renderer", "renderer_none", false);
      }

      if (
        dimensionId === "renderer" &&
        optionId === "renderer_none" &&
        isOptionEnabled(next, "renderer", "renderer_none")
      ) {
        next = setOptionEnabled(next, "renderer", "d3d11", false);
        next = setOptionEnabled(next, "renderer", "macos", false);
        next = setOptionEnabled(next, "memory", "d3d11_shared", false);
      }

      return next;
    });
  };

  const generateMatrix = () => {
    const enabledOptions = dimensions
      .map((dim) =>
        dim.options
          .filter((o) => o.enabled)
          .map((option) => ({
            dimensionId: dim.id,
            option,
          }))
      )
      .filter((opts) => opts.length > 0);

    if (enabledOptions.length === 0) return [];

    const combinations: MatrixTest[] = [];
    const seenConfigs = new Set<string>();
    const generate = (index: number, current: SelectedMatrixOption[]) => {
      if (index >= enabledOptions.length) {
        const config = buildConfig(current);
        if (unsupportedMatrixReason(config)) {
          return;
        }

        const configKey = matrixConfigKey(config);
        if (seenConfigs.has(configKey)) {
          return;
        }
        seenConfigs.add(configKey);

        combinations.push({
          id: `matrix_${combinations.length}`,
          config,
          status: "pending",
        });
        return;
      }

      const options = enabledOptions[index];
      if (!options) return;

      for (const option of options) {
        generate(index + 1, [...current, option]);
      }
    };

    generate(0, []);
    return combinations;
  };

  const buildConfig = (options: SelectedMatrixOption[]): TestConfig => {
    const config: TestConfig = {};
    options.forEach(({ dimensionId, option }) => {
      switch (dimensionId) {
        case "capture":
          config.capture_type = option.id as TestConfig["capture_type"];
          break;
        case "encoder":
          config.encoder_type = option.id as TestConfig["encoder_type"];
          break;
        case "decoder":
          config.decoder_type = option.id as TestConfig["decoder_type"];
          break;
        case "transport":
          config.transport_kind = option.id as TestConfig["transport_kind"];
          break;
        case "renderer":
          if (option.id === "d3d11") {
            config.renderer_type = "d3d11";
            config.render_display = true;
          } else if (option.id === "macos") {
            config.renderer_type = "macos";
            config.render_display = true;
          } else {
            config.render_display = false;
          }
          break;
        case "memory":
          config.zero_copy = option.id === "d3d11_shared";
          break;
        case "resolution": {
          const [w, h] = option.id.split("x").map(Number);
          if (w && h) {
            config.resolution = [w, h];
          }
          break;
        }
        case "fps":
          config.fps = Number(option.id);
          break;
        case "bitrate":
          config.bitrate = Number(option.id);
          break;
        case "duration":
          config.duration_ms = Number(option.id);
          break;
      }
    });

    config.transport_kind ??= "loopback";
    config.render_display ??= false;
    config.zero_copy ??= false;
    config.bitrate ??= 5000000;
    config.duration_ms ??= 5000; // Short duration for matrix tests
    config.warmup_ms = 1000;

    return config;
  };

  const waitForRunCompletion = async (runId: string, config: TestConfig): Promise<TestRun | null> => {
    const timeoutMs = Math.max(
      runDelayMs,
      (config.duration_ms ?? 5000) + (config.warmup_ms ?? 0) + 3000
    );
    const startedAt = Date.now();
    let lastRun: TestRun | null = null;

    while (Date.now() - startedAt <= timeoutMs) {
      const runResult = await commands.testGetRun(runId);
      if (!runResult.ok) {
        throw new Error(runResult.error.message);
      }
      if (!runResult.value) {
        return null;
      }

      lastRun = runResult.value;
      if (
        runResult.value.status === "completed" ||
        runResult.value.status === "failed" ||
        runResult.value.status === "cancelled"
      ) {
        return runResult.value;
      }

      await new Promise((resolve) => setTimeout(resolve, 250));
    }

    return lastRun;
  };

  const handleStart = async () => {
    const matrixTests = generateMatrix();
    if (matrixTests.length === 0) return;

    setTests(matrixTests);
    setIsRunning(true);
    setCurrentTestIndex(0);
    setCompletedCount(0);
    setFailedCount(0);
    setSkippedCount(0);

    // Run tests sequentially (for now - could be parallelized)
    for (let i = 0; i < matrixTests.length; i++) {
      setCurrentTestIndex(i);

      const test = matrixTests[i];
      if (!test) continue;

      const staticSkipReason = unsupportedMatrixReason(test.config);
      if (staticSkipReason) {
        setSkippedCount((count) => count + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i ? { ...t, status: "skipped" as const, skipReason: staticSkipReason } : t
          )
        );
        continue;
      }

      setTests((prev) =>
        prev.map((t, idx) =>
          idx === i ? { ...t, status: "running" as const } : t
        )
      );

      const startTime = Date.now();
      const markFailed = (
        duration = Date.now() - startTime,
        result?: TestRunSummary,
        failureReason?: string
      ) => {
        setFailedCount((f) => f + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i
              ? {
                  ...t,
                  status: "failed" as const,
                  result,
                  duration,
                  failureReason,
                }
              : t
          )
        );
      };
      const markSkipped = (skipReason: string, duration = Date.now() - startTime) => {
        setSkippedCount((count) => count + 1);
        setTests((prev) =>
          prev.map((t, idx) =>
            idx === i ? { ...t, status: "skipped" as const, skipReason, duration } : t
          )
        );
      };

      let activeRunId: string | null = null;
      try {
        const result = await commands.testStartRun({
          scenarioId: "matrix",
          config: test.config,
        });

        if (!result.ok) {
          const skipReason = capabilitySkipReason(test.config, result.error.message);
          if (skipReason) {
            markSkipped(skipReason);
            continue;
          }
          markFailed();
          continue;
        }

        activeRunId = result.value;
        const run = await waitForRunCompletion(activeRunId, test.config);
        if (!run) {
          markFailed();
          await commands.testStopRun(activeRunId);
          continue;
        }

        const duration = Date.now() - startTime;

        const acceptance = evaluateMatrixRun(test.config, run.summary);
        if (run.status === "completed" && acceptance.acceptable) {
          setCompletedCount((c) => c + 1);
          setTests((prev) =>
            prev.map((t, idx) =>
              idx === i
                ? {
                    ...t,
                    status: "completed" as const,
                    result: run.summary,
                    duration,
              }
                : t
            )
          );
        } else {
          markFailed(
            duration,
            run.summary,
            run.summary?.error_message ?? acceptance.reason ?? run.status
          );
        }

        await commands.testStopRun(activeRunId);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const skipReason = capabilitySkipReason(test.config, message);
        if (skipReason) {
          markSkipped(skipReason);
          continue;
        }
        markFailed();
        if (activeRunId) {
          await commands.testStopRun(activeRunId);
        }
      }
    }

    setIsRunning(false);
  };

  const totalTests = generateMatrix().length;
  const finishedCount = completedCount + failedCount + skippedCount;
  const progress = totalTests > 0 ? (finishedCount / totalTests) * 100 : 0;
  const platformLabel = capabilities?.os_type ?? "windows";

  const getStatusIcon = (status: MatrixTest["status"]) => {
    switch (status) {
      case "pending":
        return <div className="w-4 h-4 rounded-full border-2 border-gray-300" />;
      case "running":
        return <Loader2 className="h-4 w-4 text-blue-500 animate-spin" />;
      case "completed":
        return <CheckCircle2 className="h-4 w-4 text-green-500" />;
      case "failed":
        return <XCircle className="h-4 w-4 text-red-500" />;
      case "skipped":
        return <Clock className="h-4 w-4 text-gray-400" />;
    }
  };

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Grid3x3 className="h-6 w-6" />
          矩阵测试
        </h1>
        <p className="text-muted-foreground">
          批量参数组合测试，当前平台矩阵：{platformLabel}
        </p>
      </div>

      {/* Dimension Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择测试维度</h2>
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {dimensions.map((dim) => (
            <div key={dim.id}>
              <h3 className="font-medium text-sm mb-2">{dim.name}</h3>
              <div className="space-y-1">
                {dim.options.map((opt) => (
                  <label
                    key={opt.id}
                    className="flex items-center gap-2 p-2 rounded hover:bg-muted cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      checked={opt.enabled}
                      onChange={() => toggleOption(dim.id, opt.id)}
                      disabled={isRunning}
                      className="rounded"
                    />
                    <span className="text-sm">{opt.name}</span>
                    {opt.id === "nvenc_av1" && (
                      <span className="text-xs bg-yellow-100 text-yellow-800 px-1 rounded">
                        encode only
                      </span>
                    )}
                  </label>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Summary */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">测试概览</h2>
          {!isRunning ? (
            <button
              onClick={handleStart}
              disabled={totalTests === 0}
              className="flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50"
            >
              <Play className="h-4 w-4" />
              启动矩阵测试 ({totalTests} 个组合)
            </button>
          ) : (
            <div className="flex items-center gap-4">
              <span className="text-sm text-muted-foreground">
                {currentTestIndex + 1} / {totalTests}
              </span>
              <div className="w-32 h-2 bg-gray-200 rounded-full overflow-hidden">
                <div
                  className="h-full bg-blue-500 transition-all"
                  style={{ width: `${progress}%` }}
                />
              </div>
            </div>
          )}
        </div>

        {tests.length > 0 && (
          <div className="grid grid-cols-4 gap-4 text-center">
            <div>
              <p className="text-2xl font-bold">{totalTests}</p>
              <p className="text-xs text-muted-foreground">总计</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-green-600">{completedCount}</p>
              <p className="text-xs text-muted-foreground">成功</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-red-600">{failedCount}</p>
              <p className="text-xs text-muted-foreground">失败</p>
            </div>
            <div>
              <p className="text-2xl font-bold text-gray-400">
                {skippedCount}
              </p>
              <p className="text-xs text-muted-foreground">跳过</p>
            </div>
          </div>
        )}
      </div>

      {/* Test Results Grid */}
      {tests.length > 0 && (
        <div className="bg-card rounded-lg border overflow-x-auto">
          <table className="w-full min-w-[1660px]">
            <thead className="bg-muted">
              <tr>
                <th className="px-4 py-2 text-left text-sm font-medium">状态</th>
                <th className="px-4 py-2 text-left text-sm font-medium">捕获</th>
                <th className="px-4 py-2 text-left text-sm font-medium">编码器</th>
                <th className="px-4 py-2 text-left text-sm font-medium">解码器</th>
                <th className="px-4 py-2 text-left text-sm font-medium">传输</th>
                <th className="px-4 py-2 text-left text-sm font-medium">渲染</th>
                <th className="px-4 py-2 text-left text-sm font-medium">分辨率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">帧率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">码率</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Pipeline FPS</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Memory</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Encode P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Transport P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">Decode P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">延迟 P95</th>
                <th className="px-4 py-2 text-left text-sm font-medium">时长</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {tests.map((test) => (
                <tr
                  key={test.id}
                  className={
                    test.status === "running"
                      ? "bg-blue-50 dark:bg-blue-900/10"
                      : ""
                  }
                >
                  <td className="px-4 py-2 flex items-center gap-2">
                    {getStatusIcon(test.status)}
                    <div className="min-w-0">
                      <span className="text-xs text-muted-foreground">
                        {STATUS_LABELS[test.status]}
                      </span>
                      {test.skipReason && (
                        <div
                          className="max-w-[220px] truncate text-[11px] text-muted-foreground"
                          title={test.skipReason}
                        >
                          {test.skipReason}
                        </div>
                      )}
                      {test.failureReason && (
                        <div
                          className="max-w-[260px] truncate text-[11px] text-destructive"
                          title={test.failureReason}
                        >
                          {test.failureReason}
                        </div>
                      )}
                    </div>
                  </td>
                  <td className="px-4 py-2 text-sm">{test.config.capture_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.encoder_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.decoder_type}</td>
                  <td className="px-4 py-2 text-sm">{test.config.transport_kind}</td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.render_display ? test.config.renderer_type ?? "native" : "none"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.resolution?.join("x")}
                  </td>
                  <td className="px-4 py-2 text-sm">{test.config.fps}</td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.bitrate ? `${(test.config.bitrate / 1000000).toFixed(0)} Mbps` : "-"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.result?.capture_fps?.toFixed(1) ?? "-"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.config.zero_copy ? "d3d11_shared" : "cpu"}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.encode_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.transport_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.decode_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {formatMs(test.result?.total_latency_p95)}
                  </td>
                  <td className="px-4 py-2 text-sm">
                    {test.duration ? `${(test.duration / 1000).toFixed(1)}s` : "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
