import { useState, useEffect, useCallback } from "react";
import { Play, Square, Network, Gauge, RefreshCw } from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type {
  EnvironmentSnapshot,
  LanPeerInfo,
  MediaProfile,
  TestConfig,
} from "../../adapters/tauri/types";
import {
  runLanE2EAutomation,
  type LanE2EAutomationCommands,
  type LanE2EAutomationReport,
} from "../../services/lanE2eAutomationService";
import { externalRunRecordFromLanE2EReport } from "../../services/lanE2eTelemetryService";
import {
  chooseCapability,
  chooseDecoderCapabilityForConfig,
} from "./capabilityMeta";
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

type TransportType = "quic" | "webrtc";
type TestProfile = "latency" | "throughput" | "stability";
type TransportRunScope = "local" | "cross-device";
type DecoderType = NonNullable<TestConfig["decoder_type"]>;

const LOCAL_LAN_TARGET_ID = "__local__";

const lanAutomationCommands: LanE2EAutomationCommands = {
  serviceBootstrapIfNeeded: commands.serviceBootstrapIfNeeded,
  serviceWaitForHealthy: (timeoutSecs = 10) =>
    commands.serviceWaitForHealthy(timeoutSecs),
  ipcRuntimeSnapshot: commands.ipcRuntimeSnapshot,
  getHardwareInfo: commands.getHardwareInfo,
  ipcRegisterDevice: commands.ipcRegisterDevice,
  ipcRefreshLanDiscovery: commands.ipcRefreshLanDiscovery,
  ipcStartLanRemoteSession: commands.ipcStartLanRemoteSession,
  ipcUpdateMediaProfile: commands.ipcUpdateMediaProfile,
  ipcListRemoteCaptureSources: commands.ipcListRemoteCaptureSources,
  ipcSelectRemoteCaptureSource: commands.ipcSelectRemoteCaptureSource,
  ipcListRemoteDisplayModes: commands.ipcListRemoteDisplayModes,
  ipcSetRemoteDisplayMode: commands.ipcSetRemoteDisplayMode,
  ipcRestoreRemoteDisplayMode: commands.ipcRestoreRemoteDisplayMode,
  ipcStartReceiver: commands.ipcStartReceiver,
  openRemoteDisplayWindow: commands.openRemoteDisplayWindow,
  ipcSessionSnapshot: commands.ipcSessionSnapshot,
  ipcProbeSnapshot: commands.ipcProbeSnapshot,
  ipcMediaPipelineSnapshot: commands.ipcMediaPipelineSnapshot,
  ipcStopSession: commands.ipcStopSession,
};

interface TransportOption {
  id: TransportType;
  name: string;
  description: string;
  available: boolean;
  icon: React.ReactNode;
  statusLabel?: string;
  unavailableReason?: string;
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
  frames_received: number;
  frames_decoded: number;
}

export function TransportTestPage() {
  const [selectedTransport, setSelectedTransport] = useState<TransportType>("quic");
  const [testProfile, setTestProfile] = useState<TestProfile>("latency");
  const [selectedServer, setSelectedServer] = useState("localhost");
  const [runScope, setRunScope] = useState<TransportRunScope>("local");
  const [lanPeers, setLanPeers] = useState<LanPeerInfo[]>([]);
  const [selectedLanTargetId, setSelectedLanTargetId] =
    useState(LOCAL_LAN_TARGET_ID);
  const [isRefreshingLanPeers, setIsRefreshingLanPeers] = useState(false);
  const [isRunning, setIsRunning] = useState(false);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<TransportMetrics | null>(null);
  const [throughputHistory, setThroughputHistory] = useState<number[]>([]);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [showUnavailable] = useShowUnavailableCapabilities();

  const visibleTransportOptions = TRANSPORT_OPTIONS.map((option) => {
    const capability = capabilityForOption(serviceCapabilitySnapshot, "transport", option.id);
    const state = capabilityOptionState(serviceCapabilitySnapshot, "transport", option.id);
    return {
      ...option,
      available: serviceCapabilitySnapshot ? state === "selectable" : option.available,
      statusLabel: capability?.status,
      unavailableReason: capability?.reason ?? capability?.detail,
    };
  }).filter((option) =>
    serviceCapabilitySnapshot
      ? shouldShowCapabilityOptionForSnapshot(
          serviceCapabilitySnapshot,
          "transport",
          option.id,
          showUnavailable
        )
      : shouldShowCapabilityOption(option.available, showUnavailable)
  );
  const selectedOption = visibleTransportOptions.find((o) => o.id === selectedTransport);

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

    void commands.testGetCapabilities().then((environmentResult) => {
      if (environmentResult.ok) {
        applyLegacyEnvironment(environmentResult.value);
      }
    });

    void commands.ipcCapabilitySnapshot().then((serviceResult) => {
      if (serviceResult.ok && serviceResult.value) {
        applyServiceSnapshot(buildCapabilitySnapshotFromIpc(serviceResult.value));
      } else if (!cancelled) {
        setServiceCapabilitySnapshot(null);
      }
    });

    return () => {
      cancelled = true;
    };
  }, []);

  const refreshLanPeers = useCallback(async () => {
    setIsRefreshingLanPeers(true);
    const result = await commands.ipcRefreshLanDiscovery();
    if (result.ok) {
      const peers = result.value.peers ?? [];
      setLanPeers(peers);
      setSelectedLanTargetId((current) =>
        current === LOCAL_LAN_TARGET_ID ||
        peers.some((peer) => peer.device_id === current)
          ? current
          : LOCAL_LAN_TARGET_ID
      );
    } else {
      setStartError(`刷新 LAN 发现失败：${result.error.message}`);
    }
    setIsRefreshingLanPeers(false);
  }, []);

  useEffect(() => {
    if (runScope !== "cross-device") return;
    void refreshLanPeers();
  }, [refreshLanPeers, runScope]);

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
        frames_received: 0,
        frames_decoded: 0,
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

    if (runScope === "cross-device" && selectedLanTargetId !== LOCAL_LAN_TARGET_ID) {
      const selectedPeer = lanPeers.find(
        (peer) => peer.device_id === selectedLanTargetId
      );
      if (!selectedPeer) {
        setStartError("未找到选中的跨设备目标，请刷新发现设备后重试。");
        return;
      }

      setIsRunning(true);
      const requestedProfile = mediaProfileForTestProfile(testProfile);
      const crossDeviceConfig = crossDeviceConfigForPeer(
        selectedPeer,
        requestedProfile,
        selectedTransport
      );
      const report = await runLanE2EAutomation(lanAutomationCommands, {
        scenarioId: "cross.e2e.remote_display_smoke",
        targetDeviceId: selectedPeer.device_id,
        transportKind: selectedTransport,
        requestedProfile,
        displayModePolicy: "temporary",
        timeoutMs: 15_000,
        sampleIntervalMs: 500,
        minSampleDurationMs: 500,
        minDecodedFrames: 1,
        minFps: 1,
        createSessionId: () =>
          `transport-lan-${sanitizeSessionPart(selectedPeer.device_id)}-${Date.now()}`,
      });
      setMetrics(transportMetricsFromLanReport(report, requestedProfile));
      setThroughputHistory([
        report.probeSnapshot?.bitrate_mbps ?? requestedProfile.bitrate_mbps,
      ]);
      void commands.testRecordExternalRun(
        externalRunRecordFromLanE2EReport(report, crossDeviceConfig, {
          environment: capabilities,
          peer: selectedPeer,
          runMode: "manual",
          runIdPrefix: "transport-lan",
        })
      );
      setIsRunning(false);

      if (report.status !== "completed") {
        setStartError(
          report.errorMessage ?? report.failureReason ?? "跨设备传输测试失败"
        );
      }
      return;
    }

    const hostOs = normalizeHostOs(capabilities?.os_type);
    const capture = chooseCapability(
      captureCandidatesForHost(hostOs),
      capabilities,
      "available_captures",
      "synthetic"
    );
    const encoder = chooseCapability(
      capture === "macos"
        ? ["videotoolbox_hevc", "videotoolbox_h264", "openh264"]
        : capture === "linux"
          ? ["nvenc_hevc", "nvenc_h264", "openh264"]
          : ["nvenc_hevc", "nvenc_h264", "openh264"],
      capabilities,
      "available_encoders",
      "openh264"
    );
    const decoderCandidates: DecoderType[] =
      capture === "linux"
        ? ["linux_h264", "software", "none"]
        : capture === "macos"
          ? ["videotoolbox", "software", "none"]
          : encoder === "nvenc_hevc"
            ? ["nvdec", "ffmpeg_hevc", "software", "none"]
            : ["nvdec", "ffmpeg_h264", "software", "none"];
    const decoder = chooseDecoderCapabilityForConfig(
      decoderCandidates,
      capabilities,
      encoder,
      "none"
    );
    const config: TestConfig = {
      capture_type: capture,
      encoder_type: encoder,
      decoder_type: decoder,
      transport_kind: selectedTransport,
      resolution: [1280, 720],
      fps: testProfile === "throughput" ? 60 : 30,
      bitrate:
        isHevcEncoder(encoder) || testProfile === "throughput"
          ? 20_000_000
          : 5_000_000,
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

      {/* Execution Target */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">执行目标</h2>
        <div className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
          <label className="block">
            <span className="mb-2 block text-sm font-medium">执行范围</span>
            <select
              aria-label="执行范围"
              value={runScope}
              disabled={isRunning}
              onChange={(event) => {
                setRunScope(event.target.value as TransportRunScope);
                setStartError(null);
              }}
              className="w-full rounded border border-border bg-background px-3 py-2 text-sm"
            >
              <option value="local">本机</option>
              <option value="cross-device">跨设备</option>
            </select>
          </label>

          {runScope === "cross-device" && (
            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
              <label className="block">
                <span className="mb-2 block text-sm font-medium">跨设备目标设备</span>
                <select
                  aria-label="跨设备目标设备"
                  value={selectedLanTargetId}
                  disabled={isRunning}
                  onChange={(event) => setSelectedLanTargetId(event.target.value)}
                  className="w-full rounded border border-border bg-background px-3 py-2 text-sm"
                >
                  <option value={LOCAL_LAN_TARGET_ID}>本机</option>
                  {lanPeers.map((peer) => (
                    <option key={peer.device_id} value={peer.device_id}>
                      {peer.device_name} ({peer.ip})
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                onClick={() => void refreshLanPeers()}
                disabled={isRunning || isRefreshingLanPeers}
                className="flex items-center justify-center gap-2 rounded border border-border bg-secondary px-3 py-2 text-sm text-secondary-foreground hover:bg-secondary/80 disabled:opacity-50"
              >
                <RefreshCw
                  className={`h-4 w-4 ${isRefreshingLanPeers ? "animate-spin" : ""}`}
                />
                刷新发现设备
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Transport Selection */}
      <div className="bg-card rounded-lg border p-4 mb-6">
        <h2 className="text-lg font-semibold mb-4">选择传输协议</h2>
        <div className="grid md:grid-cols-2 gap-4">
          {visibleTransportOptions.map((option) => (
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
                <span
                  className="inline-block mt-2 text-xs bg-yellow-100 text-yellow-800 px-2 py-0.5 rounded"
                  title={option.unavailableReason ?? option.statusLabel}
                >
                  {option.statusLabel ?? "不可用"}
                </span>
              )}
              {option.available && option.statusLabel && option.statusLabel !== "available" && (
                <span
                  className="inline-block mt-2 text-xs bg-amber-100 text-amber-800 px-2 py-0.5 rounded"
                  title={option.unavailableReason ?? option.statusLabel}
                >
                  {option.statusLabel === "supported" ? "待探测" : option.statusLabel}
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
                <p className="text-muted-foreground">解码帧</p>
                <p className="font-mono">{metrics.frames_decoded}</p>
              </div>
              <div>
                <p className="text-muted-foreground">接收帧</p>
                <p className="font-mono">{metrics.frames_received}</p>
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

function mediaProfileForTestProfile(testProfile: TestProfile): MediaProfile {
  return {
    width: 1280,
    height: 720,
    fps: testProfile === "throughput" ? 60 : 30,
    bitrate_mbps: 20,
    codec: "hevc",
    codec_profile: "main",
    bit_depth: 8,
    chroma_subsampling: "4:2:0",
    pixel_format: "nv12",
    hdr_enabled: false,
  };
}

function normalizeHostOs(osType?: string): "windows" | "macos" | "linux" | "other" {
  const normalized = osType?.toLowerCase() ?? "";
  if (normalized.includes("windows") || normalized === "win32") return "windows";
  if (normalized.includes("mac") || normalized === "darwin") return "macos";
  if (normalized.includes("linux")) return "linux";
  return "other";
}

function captureCandidatesForHost(
  hostOs: ReturnType<typeof normalizeHostOs>
): NonNullable<TestConfig["capture_type"]>[] {
  if (hostOs === "macos") return ["macos", "synthetic", "linux", "dxgi"];
  if (hostOs === "linux") return ["linux", "synthetic", "macos", "dxgi"];
  return ["synthetic", "dxgi", "macos", "linux"];
}

function isHevcEncoder(encoder?: TestConfig["encoder_type"]): boolean {
  return encoder === "nvenc_hevc" || encoder === "videotoolbox_hevc";
}

function peerHasMediaCapabilities(peer: LanPeerInfo, capabilityGroups: string[][]): boolean {
  const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
    capability.toLowerCase()
  );
  return capabilityGroups.every((aliases) =>
    aliases.some((capability) => mediaCapabilities.includes(capability))
  );
}

function peerSupportsMacosVideoToolboxProfile(
  peer: LanPeerInfo,
  requestedProfile: MediaProfile
): boolean {
  const codec = requestedProfile.codec?.toLowerCase() ?? "h264";
  const codecEncoder =
    codec === "hevc"
      ? ["videotoolbox_hevc", "encode.videotoolbox_hevc"]
      : ["videotoolbox_h264", "encode.videotoolbox_h264"];
  const codecDecoder =
    codec === "hevc"
      ? ["decode.videotoolbox_hevc"]
      : ["decode.videotoolbox_h264"];
  const capabilityGroups = [
    ["macos_capture", "capture.macos"],
    codecEncoder,
    codecDecoder,
    ["macos_native_render", "render.macos"],
  ];
  if (codec === "hevc") {
    capabilityGroups.push(["media.hevc_main_420_8bit"]);
  }
  return peerHasMediaCapabilities(peer, capabilityGroups);
}

export function crossDeviceConfigForPeer(
  peer: LanPeerInfo,
  requestedProfile: MediaProfile,
  transportKind: TransportType
): TestConfig {
  const codec = requestedProfile.codec?.toLowerCase() ?? "h264";
  const common = {
    render_display: true,
    transport_kind: transportKind,
    resolution: [requestedProfile.width, requestedProfile.height] as [number, number],
    fps: requestedProfile.fps,
    bitrate: requestedProfile.bitrate_mbps * 1_000_000,
    duration_ms: 15_000,
    warmup_ms: 500,
  };
  if (peerSupportsMacosVideoToolboxProfile(peer, requestedProfile)) {
    return {
      ...common,
      capture_type: "macos",
      encoder_type: codec === "hevc" ? "videotoolbox_hevc" : "videotoolbox_h264",
      decoder_type: "videotoolbox",
      renderer_type: "macos",
      zero_copy: false,
    };
  }
  return {
    ...common,
    capture_type: "dxgi",
    encoder_type: codec === "hevc" ? "nvenc_hevc" : "nvenc_h264",
    decoder_type: "nvdec",
    renderer_type: "d3d11",
    zero_copy: true,
  };
}

function transportMetricsFromLanReport(
  report: LanE2EAutomationReport,
  requestedProfile: MediaProfile
): TransportMetrics {
  const probe = report.probeSnapshot;
  const framesReceived = probe?.frames_received ?? 0;
  const framesDropped = probe?.frames_dropped ?? 0;
  const packetLossPercent =
    framesReceived > 0 ? (framesDropped / framesReceived) * 100 : 0;
  const throughputMbps = probe?.bitrate_mbps ?? requestedProfile.bitrate_mbps;
  const sampleSeconds = Math.max(report.sampleDurationMs / 1000, 1);
  const estimatedBytes = Math.round((throughputMbps * 1_000_000 * sampleSeconds) / 8);

  return {
    is_running: false,
    throughput_mbps: throughputMbps,
    latency_ms: 0,
    jitter_ms: 0,
    packet_loss_percent: packetLossPercent,
    bytes_sent: estimatedBytes,
    bytes_received: estimatedBytes,
    connection_count: report.peer ? 1 : 0,
    frames_received: framesReceived,
    frames_decoded: probe?.frames_decoded ?? 0,
  };
}

function sanitizeSessionPart(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-");
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
