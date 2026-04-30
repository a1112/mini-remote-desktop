import { useState, useEffect } from "react";
import { Play, Square, Network, Gauge } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { EnvironmentSnapshot, TestConfig } from "../../adapters/tauri/types";
import { capabilityAvailable, chooseCapability } from "./capabilityMeta";

type TransportType = "quic" | "webrtc";
type TestProfile = "latency" | "throughput" | "stability";

interface TransportOption {
  id: TransportType;
  name: string;
  description: string;
  available: boolean;
  icon: React.ReactNode;
}

const TRANSPORT_OPTIONS: TransportOption[] = [
  {
    id: "quic",
    name: "QUIC",
    description: "基于 UDP 的低延迟传输协议",
    available: true,
    icon: <Network className="h-5 w-5 text-blue-500" />,
  },
  {
    id: "webrtc",
    name: "WebRTC",
    description: "实时通信传输协议",
    available: true,
    icon: <Network className="h-5 w-5 text-green-500" />,
  },
];

interface TransportMetrics {
  is_running: boolean;
  throughput_mbps: number;
  latency_ms: number;
  jitter_ms: number;
  packet_loss_percent: number;
  bytes_sent: number;
  bytes_received: number;
  connection_count: number;
}

export function TransportTestPage() {
  const [selectedTransport, setSelectedTransport] = useState<TransportType>("quic");
  const [testProfile, setTestProfile] = useState<TestProfile>("latency");
  const [selectedServer, setSelectedServer] = useState("localhost");
  const [isRunning, setIsRunning] = useState(false);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<TransportMetrics | null>(null);
  const [throughputHistory, setThroughputHistory] = useState<number[]>([]);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  const selectedOption = TRANSPORT_OPTIONS.find((o) => o.id === selectedTransport);

  const TEST_PROFILES = [
    { id: "latency", name: "延迟优先", desc: "优化低延迟传输" },
    { id: "throughput", name: "吞吐量", desc: "最大化数据传输" },
    { id: "stability", name: "稳定性", desc: "抗网络抖动" },
  ];

  const SERVER_OPTIONS = [
    { id: "localhost", name: "本地 (localhost)" },
    { id: "lan", name: "局域网服务器" },
    { id: "wan", name: "公网服务器" },
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
    if (!isRunning) return;

    const interval = setInterval(async () => {
      // Simulated metrics - in reality would come from transport layer
      const simulatedMetrics: TransportMetrics = {
        is_running: true,
        throughput_mbps: testProfile === "throughput" ? 800 + Math.random() * 200 : 100 + Math.random() * 50,
        latency_ms: testProfile === "latency" ? 5 + Math.random() * 5 : 20 + Math.random() * 30,
        jitter_ms: Math.random() * 5,
        packet_loss_percent: Math.random() * 0.1,
        bytes_sent: Math.floor(Math.random() * 10000000),
        bytes_received: Math.floor(Math.random() * 5000000),
        connection_count: 1,
      };

      setMetrics(simulatedMetrics);

      // Update throughput history
      setThroughputHistory((prev) => {
        const newHistory = [...prev, simulatedMetrics.throughput_mbps];
        return newHistory.slice(-30);
      });
    }, 500);

    return () => clearInterval(interval);
  }, [isRunning, testProfile]);

  const handleStart = async () => {
    if (!selectedOption?.available) return;

    setMetrics(null);
    setThroughputHistory([]);
    setStartError(null);
    setCurrentRunId(null);

    const capture = chooseCapability(
      ["macos", "dxgi", "synthetic"],
      capabilities,
      "available_captures",
      "synthetic"
    );
    const encoder = chooseCapability(
      capture === "macos" ? ["videotoolbox_h264", "openh264"] : ["nvenc_h264", "openh264"],
      capabilities,
      "available_encoders",
      "openh264"
    );
    const decoder = capabilityAvailable(capabilities, "available_decoders", "software", true)
      ? "software"
      : "none";
    const config: TestConfig = {
      capture_type: capture,
      encoder_type: encoder,
      decoder_type: decoder,
      transport_kind: selectedTransport,
      resolution: [1280, 720],
      fps: testProfile === "throughput" ? 60 : 30,
      bitrate: testProfile === "throughput" ? 20_000_000 : 5_000_000,
      duration_ms: 10_000,
      warmup_ms: 500,
      input_source: capture === "synthetic" ? "synthetic" : "screen",
    };

    const result = await commands.testStartRun({
      scenarioId: "custom",
      config,
    });
    if (result.ok) {
      setCurrentRunId(result.value);
      setIsRunning(true);
    } else {
      setStartError(result.error.message);
    }
  };

  const handleStop = async () => {
    if (currentRunId) {
      await commands.testStopRun(currentRunId);
    } else {
      await commands.testHarnessStop();
    }
    setIsRunning(false);
    setCurrentRunId(null);
  };

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
          <Network className="h-6 w-6" />
          传输测试
        </h1>
        <p className="text-muted-foreground">
          测试 QUIC/WebRTC 传输性能和延迟
        </p>
      </div>

      {/* Transport Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择传输协议</h2>
        <div className="grid md:grid-cols-2 gap-4">
          {TRANSPORT_OPTIONS.map((option) => (
            <button
              key={option.id}
              onClick={() => setSelectedTransport(option.id)}
              disabled={isRunning || !option.available}
              className={`p-4 rounded-lg border-2 text-left transition-all ${
                selectedTransport === option.id
                  ? "border-primary bg-primary/10"
                  : "border-transparent bg-muted/30 hover:bg-muted/50"
              } ${!option.available ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center gap-2 mb-2">
                {option.icon}
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

      {/* Test Profile */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h3 className="font-medium mb-4">测试配置</h3>
        <div className="grid md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">测试配置</label>
            <div className="space-y-2">
              {TEST_PROFILES.map((profile) => (
                <button
                  key={profile.id}
                  onClick={() => setTestProfile(profile.id as TestProfile)}
                  disabled={isRunning}
                  className={`w-full text-left px-3 py-2 rounded border text-sm ${
                    testProfile === profile.id
                      ? "bg-primary/10 border-primary"
                      : "bg-background hover:bg-muted"
                  }`}
                >
                  <div className="font-medium">{profile.name}</div>
                  <div className="text-xs text-muted-foreground">{profile.desc}</div>
                </button>
              ))}
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">目标服务器</label>
            <select
              value={selectedServer}
              onChange={(e) => setSelectedServer(e.target.value)}
              disabled={isRunning}
              className="w-full px-3 py-2 border rounded bg-background"
            >
              {SERVER_OPTIONS.map((server) => (
                <option key={server.id} value={server.id}>
                  {server.name}
                </option>
              ))}
            </select>

            <div className="mt-4 space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-muted-foreground">测试时长</span>
                <span>30 秒</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">并发连接</span>
                <span>1</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">数据包大小</span>
                <span>1400 bytes</span>
              </div>
            </div>
          </div>
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
      {startError && <p className="text-sm text-red-600 mb-6">{startError}</p>}

      {/* Metrics */}
      {metrics && (
        <>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
            <MetricCard
              icon={<Gauge className="h-4 w-4" />}
              label="吞吐量"
              value={`${metrics.throughput_mbps.toFixed(1)} Mbps`}
              color={getThroughputColor(metrics.throughput_mbps)}
            />
            <MetricCard
              icon={<Network className="h-4 w-4" />}
              label="延迟"
              value={`${metrics.latency_ms.toFixed(1)} ms`}
              color={getLatencyColor(metrics.latency_ms)}
            />
            <MetricCard
              label="抖动"
              value={`${metrics.jitter_ms.toFixed(2)} ms`}
            />
            <MetricCard
              label="丢包率"
              value={`${metrics.packet_loss_percent.toFixed(3)}%`}
              highlight={metrics.packet_loss_percent > 0.05}
            />
          </div>

          {/* Throughput Chart */}
          <div className="bg-card rounded-lg border p-4 mb-6">
            <h3 className="font-medium mb-4">吞吐量趋势</h3>
            <div className="h-32 flex items-end gap-1">
              {throughputHistory.map((value, idx) => (
                <div
                  key={idx}
                  className="flex-1 bg-blue-500 rounded-t transition-all"
                  style={{ height: `${(value / 1000) * 100}%` }}
                />
              ))}
              {throughputHistory.length === 0 && (
                <div className="w-full text-center text-muted-foreground text-sm">
                  等待数据...
                </div>
              )}
            </div>
          </div>

          {/* Detailed Stats */}
          <div className="bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-4">传输统计</h3>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">发送字节</p>
                <p className="font-mono">{formatBytes(metrics.bytes_sent)}</p>
              </div>
              <div>
                <p className="text-muted-foreground">接收字节</p>
                <p className="font-mono">{formatBytes(metrics.bytes_received)}</p>
              </div>
              <div>
                <p className="text-muted-foreground">连接数</p>
                <p className="font-mono">{metrics.connection_count}</p>
              </div>
              <div>
                <p className="text-muted-foreground">协议</p>
                <p className="font-mono uppercase">{selectedTransport}</p>
              </div>
            </div>
          </div>

          {/* Performance Grade */}
          <div className="mt-6 bg-card rounded-lg border p-4">
            <h3 className="font-medium mb-3">性能评级</h3>
            <PerformanceGrade metrics={metrics} />
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

function PerformanceGrade({ metrics }: { metrics: TransportMetrics }) {
  let grade: { letter: string; color: string; description: string };

  if (metrics.latency_ms < 10 && metrics.packet_loss_percent < 0.01) {
    grade = { letter: "A+", color: "text-green-500", description: "优秀 - 适合实时应用" };
  } else if (metrics.latency_ms < 50 && metrics.packet_loss_percent < 0.1) {
    grade = { letter: "A", color: "text-green-600", description: "良好 - 适合大多数应用" };
  } else if (metrics.latency_ms < 100 && metrics.packet_loss_percent < 0.5) {
    grade = { letter: "B", color: "text-yellow-600", description: "一般 - 可能影响实时体验" };
  } else if (metrics.latency_ms < 200) {
    grade = { letter: "C", color: "text-orange-500", description: "较差 - 不适合实时应用" };
  } else {
    grade = { letter: "D", color: "text-red-500", description: "差 - 需要优化网络" };
  }

  return (
    <div className="flex items-center gap-4">
      <div className={`text-5xl font-bold ${grade.color}`}>{grade.letter}</div>
      <div>
        <p className="font-medium">{grade.description}</p>
        <p className="text-sm text-muted-foreground">
          延迟: {metrics.latency_ms.toFixed(1)}ms | 丢包: {metrics.packet_loss_percent.toFixed(3)}%
        </p>
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes >= 1000000) {
    return `${(bytes / 1000000).toFixed(2)} MB`;
  }
  if (bytes >= 1000) {
    return `${(bytes / 1000).toFixed(2)} KB`;
  }
  return `${bytes} B`;
}

function getThroughputColor(mbps: number): string {
  if (mbps >= 500) return "text-green-500";
  if (mbps >= 100) return "text-yellow-500";
  return "text-red-500";
}

function getLatencyColor(ms: number): string {
  if (ms < 10) return "text-green-500";
  if (ms < 50) return "text-yellow-500";
  return "text-red-500";
}
