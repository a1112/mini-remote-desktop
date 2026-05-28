import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router";
import {
  Activity,
  CheckCircle2,
  XCircle,
  Clock,
  Zap,
  Monitor,
  ArrowRight,
  Download,
  RefreshCw,
  RotateCcw,
  Wrench,
} from "lucide-react";
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import * as commands from "../../adapters/tauri/commands";
import type {
  TestScenario,
  TestRun,
  EnvironmentSnapshot,
  MetricSeries,
  FfmpegProbeResult,
} from "../../adapters/tauri/types";
import {
  buildCapabilitySnapshotFromIpc,
  buildCapabilitySnapshotFromEnvironment,
  evaluateProfileSupport,
  type CapabilityDomain,
  type CapabilityItem,
  type CapabilitySnapshot,
  type CapabilityStatus,
} from "../../services/capabilityMatrix";
import {
  shouldShowCapabilityStatus,
  useShowUnavailableCapabilities,
} from "./useCapabilityVisibility";
import {
  buildChartGroups,
  normalizeMetrics,
  type ChartGroup,
  type NormalizedMetric,
} from "../../services/testTelemetryService";

const CAPABILITY_DOMAIN_ORDER: CapabilityDomain[] = [
  "capture",
  "capture_source",
  "encode",
  "decode",
  "render",
  "memory",
  "transport",
  "control",
  "audio",
  "service",
  "security",
];

const REALTIME_CHART_COLORS = [
  "#2563eb",
  "#16a34a",
  "#dc2626",
  "#9333ea",
  "#ea580c",
  "#0891b2",
  "#4f46e5",
  "#be123c",
];

type FfmpegBusyAction = "probe" | "download" | "reset";

export function OverviewPage() {
  const navigate = useNavigate();
  const [scenarios, setScenarios] = useState<TestScenario[]>([]);
  const [recentRuns, setRecentRuns] = useState<TestRun[]>([]);
  const [activeRun, setActiveRun] = useState<TestRun | null>(null);
  const [activeRunMetrics, setActiveRunMetrics] = useState<Record<string, MetricSeries>>({});
  const [activeMetricsError, setActiveMetricsError] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [ffmpegProbe, setFfmpegProbe] = useState<FfmpegProbeResult | null>(null);
  const [ffmpegBusyAction, setFfmpegBusyAction] = useState<FfmpegBusyAction | null>(null);
  const [ffmpegStatusMessage, setFfmpegStatusMessage] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [showUnavailable, setShowUnavailable] = useShowUnavailableCapabilities();

  useEffect(() => {
    loadOverviewData();
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function refreshRuns() {
      const runsResult = await commands.testListRuns({ limit: 5 });
      if (cancelled || !runsResult.ok) return;
      setRecentRuns(runsResult.value);
      setActiveRun(selectActiveRun(runsResult.value));
    }

    const intervalId = window.setInterval(() => {
      void refreshRuns();
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, []);

  useEffect(() => {
    if (!activeRun || !isActiveRun(activeRun)) {
      setActiveRunMetrics({});
      setActiveMetricsError(null);
      return;
    }

    let cancelled = false;
    const activeRunId = activeRun.run_id;

    async function refreshMetrics() {
      const metricsResult = await commands.testGetRunMetrics(activeRunId);
      if (cancelled) return;
      if (metricsResult.ok) {
        setActiveRunMetrics(metricsResult.value);
        setActiveMetricsError(null);
      } else {
        setActiveMetricsError(metricsResult.error.message);
      }
    }

    void refreshMetrics();
    const intervalId = window.setInterval(() => {
      void refreshMetrics();
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [activeRun?.run_id, activeRun?.status]);

  async function loadOverviewData() {
    setLoading(true);
    try {
      const [
        scenariosResult,
        runsResult,
        capsResult,
        serviceCapsResult,
        ffmpegProbeResult,
      ] = await Promise.all([
        commands.testListScenarios(),
        commands.testListRuns({ limit: 5 }),
        commands.testGetCapabilities(),
        commands.ipcCapabilitySnapshot(),
        commands.ffmpegProbe(),
      ]);

      if (scenariosResult.ok) setScenarios(scenariosResult.value);
      if (runsResult.ok) {
        setRecentRuns(runsResult.value);
        setActiveRun(selectActiveRun(runsResult.value));
      }
      if (capsResult.ok) setCapabilities(capsResult.value);
      if (serviceCapsResult.ok && serviceCapsResult.value) {
        setServiceCapabilitySnapshot(buildCapabilitySnapshotFromIpc(serviceCapsResult.value));
      }
      if (ffmpegProbeResult.ok) {
        setFfmpegProbe(ffmpegProbeResult.value);
        setFfmpegStatusMessage(null);
      } else {
        setFfmpegStatusMessage(ffmpegProbeResult.error.message);
      }
    } catch (error) {
      console.error("Failed to load overview data:", error);
    } finally {
      setLoading(false);
    }
  }

  async function refreshFfmpegStatus() {
    setFfmpegBusyAction("probe");
    try {
      const result = await commands.ffmpegProbe();
      if (result.ok) {
        setFfmpegProbe(result.value);
        setFfmpegStatusMessage("FFmpeg 状态已刷新");
      } else {
        setFfmpegStatusMessage(result.error.message);
      }
    } finally {
      setFfmpegBusyAction(null);
    }
  }

  async function downloadFfmpeg() {
    setFfmpegBusyAction("download");
    try {
      const result = await commands.ffmpegDownload();
      if (result.ok) {
        setFfmpegProbe(result.value.probe);
        setFfmpegStatusMessage(`FFmpeg 已安装到 ${result.value.install_dir}`);
      } else {
        setFfmpegStatusMessage(result.error.message);
      }
    } finally {
      setFfmpegBusyAction(null);
    }
  }

  async function resetFfmpegSettings() {
    setFfmpegBusyAction("reset");
    try {
      const result = await commands.ffmpegResetGoldenSettings();
      if (result.ok) {
        setFfmpegStatusMessage("FFmpeg 设置已重置");
        await refreshFfmpegStatus();
      } else {
        setFfmpegStatusMessage(result.error.message);
      }
    } finally {
      setFfmpegBusyAction(null);
    }
  }

  const successfulRuns = recentRuns.filter((r) => r.status === "completed").length;
  const failedRuns = recentRuns.filter((r) => r.status === "failed").length;
  const capabilitySnapshot =
    serviceCapabilitySnapshot ??
    (capabilities ? buildCapabilitySnapshotFromEnvironment(capabilities) : null);
  const capabilityGroups = capabilitySnapshot
    ? groupCapabilitiesByDomain(capabilitySnapshot.capabilities, showUnavailable)
    : [];
  const hiddenCapabilityCount = capabilitySnapshot
    ? capabilitySnapshot.capabilities.filter(
        (capability) => !shouldShowCapabilityStatus(capability.status, false)
      ).length
    : 0;
  const lan2k144Evaluation = capabilitySnapshot
    ? evaluateProfileSupport("lan.2k144", capabilitySnapshot)
    : null;
  const lan1600p165Evaluation = capabilitySnapshot
    ? evaluateProfileSupport("lan.1600p165", capabilitySnapshot)
    : null;

  return (
    <div className="p-6">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground">测试工作台总览</h1>
        <p className="text-muted-foreground">查看 CapTest 同口径链路、环境能力和最近运行结果</p>
      </div>

      {loading ? (
        <div className="flex items-center justify-center h-64">
          <div className="text-muted-foreground">加载中...</div>
        </div>
      ) : (
        <div className="space-y-6">
          {/* Environment Summary */}
          <section className="bg-card rounded-lg border p-6">
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <Monitor className="h-5 w-5" />
              环境摘要
            </h2>
            {capabilities && (
              <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                <div>
                  <span className="text-muted-foreground">CPU:</span>{" "}
                  {capabilities.cpu_brand}
                </div>
                <div>
                  <span className="text-muted-foreground">核心数:</span>{" "}
                  {capabilities.cpu_cores}
                </div>
                <div>
                  <span className="text-muted-foreground">内存:</span>{" "}
                  {capabilities.memory_gb} GB
                </div>
                <div>
                  <span className="text-muted-foreground">GPU:</span>{" "}
                  {capabilities.gpu_info}
                </div>
              </div>
            )}
          </section>

          <FfmpegToolingPanel
            probe={ffmpegProbe}
            busyAction={ffmpegBusyAction}
            statusMessage={ffmpegStatusMessage}
            onRefresh={refreshFfmpegStatus}
            onDownload={downloadFfmpeg}
            onReset={resetFfmpegSettings}
          />

          {/* Structured Capability Matrix */}
          {capabilitySnapshot && (
            <section className="bg-card rounded-lg border p-6">
              <div className="mb-4 flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
                <div>
                  <h2 className="text-lg font-semibold">结构化能力矩阵</h2>
                  <p className="text-sm text-muted-foreground">
                    默认只显示可用或可降级能力；勾选后显示所有平台能力。
                  </p>
                </div>
                <div className="flex flex-col gap-2 md:items-end">
                  <label className="flex cursor-pointer items-center gap-2 rounded-lg border bg-background/70 px-3 py-2 text-sm">
                    <input
                      type="checkbox"
                      checked={showUnavailable}
                      onChange={(event) => setShowUnavailable(event.target.checked)}
                      className="h-4 w-4 accent-primary"
                    />
                    <span>显示不可用能力</span>
                    {!showUnavailable && hiddenCapabilityCount > 0 && (
                      <span className="text-xs text-muted-foreground">
                        已隐藏 {hiddenCapabilityCount}
                      </span>
                    )}
                  </label>
                  <div className="rounded-lg border bg-background/70 px-3 py-2 text-sm">
                    <div className="text-xs text-muted-foreground">Profile readiness</div>
                    <div className="mt-1 flex flex-wrap items-center gap-2">
                      <span className="font-medium">lan.2k144</span>
                      <StatusBadge status={lan2k144Evaluation?.status ?? "blocked"} />
                      <span className="font-medium">lan.1600p165</span>
                      <StatusBadge status={lan1600p165Evaluation?.status ?? "blocked"} />
                    </div>
                  </div>
                </div>
              </div>

              {lan2k144Evaluation && lan2k144Evaluation.reasons.length > 0 && (
                <div className="mb-4 rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-3 text-xs text-yellow-700 dark:text-yellow-200">
                  {lan2k144Evaluation.reasons.join("; ")}
                </div>
              )}
              {lan1600p165Evaluation && lan1600p165Evaluation.reasons.length > 0 && (
                <div className="mb-4 rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-3 text-xs text-yellow-700 dark:text-yellow-200">
                  {lan1600p165Evaluation.reasons.join("; ")}
                </div>
              )}

              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                {capabilityGroups.map(({ domain, items }) => (
                  <div key={domain} className="rounded-lg border bg-background/60 p-3">
                    <div className="mb-2 flex items-center justify-between">
                      <h3 className="text-sm font-semibold">{domain}</h3>
                      <span className="text-xs text-muted-foreground">{items.length}</span>
                    </div>
                    <div className="space-y-2">
                      {items.slice(0, 4).map((item) => (
                        <div key={item.id} className="rounded border bg-card/60 px-2 py-1.5">
                          <div className="flex items-center justify-between gap-2">
                            <span className="truncate text-xs font-medium">{item.id}</span>
                            <StatusBadge status={item.status} />
                          </div>
                          {item.reason && (
                            <div className="mt-1 line-clamp-2 text-[11px] text-muted-foreground">
                              {item.reason}
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}

          <CurrentRunRealtimeCharts
            activeRun={activeRun}
            metrics={activeRunMetrics}
            error={activeMetricsError}
          />

          {/* Quick Stats */}
          <section className="grid grid-cols-1 md:grid-cols-4 gap-4">
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
                <Activity className="h-4 w-4" />
                <span>总测试场景</span>
              </div>
              <div className="text-2xl font-semibold">{scenarios.length}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
                <Clock className="h-4 w-4" />
                <span>最近运行</span>
              </div>
              <div className="text-2xl font-semibold">{recentRuns.length}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-green-500 text-sm mb-1">
                <CheckCircle2 className="h-4 w-4" />
                <span>成功</span>
              </div>
              <div className="text-2xl font-semibold">{successfulRuns}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-red-500 text-sm mb-1">
                <XCircle className="h-4 w-4" />
                <span>失败</span>
              </div>
              <div className="text-2xl font-semibold">{failedRuns}</div>
            </div>
          </section>

          {/* Quick Actions */}
          <section className="bg-card rounded-lg border p-6">
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <Zap className="h-5 w-5" />
              快速入口
            </h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <button
                type="button"
                onClick={() => navigate("/test/e2e")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">端到端本地测试</div>
                  <div className="text-sm text-muted-foreground">采集到渲染的直接性能基线</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
              <button
                type="button"
                onClick={() => navigate("/test/matrix")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">矩阵性能测试</div>
                  <div className="text-sm text-muted-foreground">DXGI/WGC、NVENC、NVDEC、QUIC 同口径组合</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
              <button
                type="button"
                onClick={() => navigate("/test/custom")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">自由组合测试</div>
                  <div className="text-sm text-muted-foreground">H.264/HEVC/Main10/AV1 单链路调试</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
            </div>
          </section>

          {/* Recent Runs */}
          {recentRuns.length > 0 && (
            <section className="bg-card rounded-lg border p-6">
              <h2 className="text-lg font-semibold mb-4">最近运行</h2>
              <div className="space-y-2">
                {recentRuns.map((run) => (
                  <div
                    key={run.run_id}
                    className="flex items-center justify-between rounded-lg border p-3 hover:bg-muted transition-colors"
                  >
                    <div className="flex items-center gap-3">
                      {run.status === "completed" ? (
                        <CheckCircle2 className="h-5 w-5 text-green-500" />
                      ) : run.status === "failed" ? (
                        <XCircle className="h-5 w-5 text-red-500" />
                      ) : (
                        <Activity className="h-5 w-5 text-yellow-500 animate-spin" />
                      )}
                      <div>
                        <div className="font-medium">{run.scenario_id}</div>
                        <div className="text-sm text-muted-foreground">
                          {new Date(run.started_at).toLocaleString()}
                        </div>
                      </div>
                    </div>
                    <div className="flex items-center gap-4">
                      {run.summary && (
                        <div className="text-sm text-muted-foreground">
                          {run.summary.frame_count} 帧 · {run.summary.dropped_frames} 丢帧
                        </div>
                      )}
                      <button
                        type="button"
                        onClick={() => navigate(`/test/run/${run.run_id}`)}
                        className="text-sm text-primary hover:underline"
                      >
                        查看详情
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>
      )}
    </div>
  );
}

function FfmpegToolingPanel({
  probe,
  busyAction,
  statusMessage,
  onRefresh,
  onDownload,
  onReset,
}: {
  probe: FfmpegProbeResult | null;
  busyAction: FfmpegBusyAction | null;
  statusMessage: string | null;
  onRefresh: () => void;
  onDownload: () => void;
  onReset: () => void;
}) {
  const status = !probe ? "skipped" : probe.available ? "available" : "driver_missing";
  const statusText = !probe ? "未探测" : probe.available ? "可用" : "不可用";

  return (
    <section className="bg-card rounded-lg border p-6">
      <div className="mb-4 flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h2 className="flex items-center gap-2 text-lg font-semibold">
            <Wrench className="h-5 w-5" />
            FFmpeg 可选工具
          </h2>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-sm">
            <StatusBadge status={status} />
            <span className="text-muted-foreground">{statusText}</span>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            aria-label="刷新 FFmpeg 状态"
            title="刷新 FFmpeg 状态"
            disabled={busyAction !== null}
            onClick={onRefresh}
            className="inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          >
            <RefreshCw className={["h-4 w-4", busyAction === "probe" ? "animate-spin" : ""].join(" ")} />
            <span>刷新</span>
          </button>
          <button
            type="button"
            aria-label="下载或更新 FFmpeg"
            title="下载或更新 FFmpeg"
            disabled={busyAction !== null}
            onClick={onDownload}
            className="inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Download className="h-4 w-4" />
            <span>{busyAction === "download" ? "处理中" : "下载/更新"}</span>
          </button>
          <button
            type="button"
            aria-label="重置 FFmpeg 设置"
            title="重置 FFmpeg 设置"
            disabled={busyAction !== null}
            onClick={onReset}
            className="inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm hover:bg-muted disabled:cursor-not-allowed disabled:opacity-60"
          >
            <RotateCcw className="h-4 w-4" />
            <span>重置</span>
          </button>
        </div>
      </div>

      <div className="grid gap-3 text-sm md:grid-cols-2">
        <div className="rounded-lg border bg-background/60 p-3">
          <div className="text-xs text-muted-foreground">版本</div>
          <div className="mt-1 break-words font-medium">
            {probe?.ffmpeg_version ?? "未探测"}
          </div>
        </div>
        <div className="rounded-lg border bg-background/60 p-3">
          <div className="text-xs text-muted-foreground">路径</div>
          <div className="mt-1 break-all font-mono text-xs">
            {probe?.ffmpeg_path ?? "未配置"}
          </div>
        </div>
      </div>

      {(probe?.reason || statusMessage) && (
        <div className="mt-3 rounded-lg border bg-background/60 p-3 text-sm text-muted-foreground">
          {statusMessage ?? probe?.reason}
        </div>
      )}
    </section>
  );
}

function CurrentRunRealtimeCharts({
  activeRun,
  metrics,
  error,
}: {
  activeRun: TestRun | null;
  metrics: Record<string, MetricSeries>;
  error: string | null;
}) {
  const normalizedMetrics = useMemo(
    () => normalizeMetrics(metrics, activeRun?.started_at),
    [metrics, activeRun?.started_at]
  );
  const fpsMetrics = normalizedMetrics
    .filter((metric) => metric.category === "fps" || metric.unit.toLowerCase() === "fps")
    .slice(0, 4);
  const latencyMetrics = normalizedMetrics
    .filter((metric) => isStageP95LatencyMetric(metric))
    .sort(compareStageLatencyMetric)
    .slice(0, 8);
  const primaryFps = latestMetricValue(fpsMetrics[0]);
  const primaryLatency =
    latestMetricValue(latencyMetrics.find((metric) => metric.key === "total_latency_p95_ms")) ??
    latestMetricValue(latencyMetrics[0]);
  const fpsChartGroup = buildChartGroups(fpsMetrics)[0];
  const latencyChartGroup = buildChartGroups(latencyMetrics)[0];

  return (
    <section className="bg-card rounded-lg border p-6">
      <div className="mb-4 flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">当前测试实时曲线</h2>
          <p className="text-sm text-muted-foreground">
            {activeRun ? activeRun.scenario_id : "暂无运行中的测试"}
          </p>
        </div>
        <div className="flex flex-wrap gap-2 text-sm">
          <RealtimeValuePill label="FPS" value={formatFps(primaryFps)} />
          <RealtimeValuePill label="Total P95" value={formatMs(primaryLatency)} />
        </div>
      </div>

      {error && (
        <div className="mb-4 rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-700 dark:text-red-200">
          {error}
        </div>
      )}

      {!activeRun ? (
        <div className="rounded-lg border border-dashed bg-background/60 p-8 text-center text-sm text-muted-foreground">
          启动测试后，这里会显示实时 FPS 和各阶段 P95 延迟。
        </div>
      ) : (
        <div className="grid gap-4 xl:grid-cols-2">
          <RealtimeChartCard
            title="FPS"
            emptyText="等待 FPS 样本"
            chartGroup={fpsChartGroup}
          />
          <RealtimeChartCard
            title="阶段 P95 延迟"
            emptyText="等待 P95 延迟样本"
            chartGroup={latencyChartGroup}
          />
        </div>
      )}
    </section>
  );
}

function RealtimeChartCard({
  title,
  emptyText,
  chartGroup,
}: {
  title: string;
  emptyText: string;
  chartGroup?: ChartGroup;
}) {
  return (
    <div className="rounded-lg border bg-background/60 p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">{title}</h3>
        {chartGroup && (
          <span className="text-xs text-muted-foreground">{chartGroup.unit}</span>
        )}
      </div>
      {!chartGroup || chartGroup.rows.length === 0 ? (
        <div className="flex h-48 items-center justify-center rounded border border-dashed text-sm text-muted-foreground">
          {emptyText}
        </div>
      ) : (
        <div className="h-48 min-w-0">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={chartGroup.rows} margin={{ top: 8, right: 16, bottom: 0, left: 0 }}>
              <CartesianGrid strokeDasharray="3 3" />
              <XAxis dataKey="time" tick={{ fontSize: 11 }} minTickGap={20} />
              <YAxis tick={{ fontSize: 11 }} width={44} />
              <Tooltip />
              <Legend wrapperStyle={{ fontSize: 11 }} />
              {chartGroup.metrics.map((metric, index) => (
                <Line
                  key={metric.key}
                  type="monotone"
                  dataKey={metric.key}
                  name={metric.label}
                  stroke={REALTIME_CHART_COLORS[index % REALTIME_CHART_COLORS.length]}
                  strokeWidth={2}
                  dot={false}
                  isAnimationActive={false}
                  connectNulls
                />
              ))}
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
    </div>
  );
}

function RealtimeValuePill({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border bg-background/70 px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="font-mono font-semibold">{value}</div>
    </div>
  );
}

function groupCapabilitiesByDomain(
  capabilities: CapabilityItem[],
  showUnavailable: boolean
): Array<{
  domain: CapabilityDomain;
  items: CapabilityItem[];
}> {
  return CAPABILITY_DOMAIN_ORDER.map((domain) => ({
    domain,
    items: capabilities.filter(
      (capability) =>
        capability.domain === domain &&
        shouldShowCapabilityStatus(capability.status, showUnavailable)
    ),
  })).filter((group) => group.items.length > 0);
}

function selectActiveRun(runs: TestRun[]): TestRun | null {
  return runs.find((run) => isActiveRun(run)) ?? null;
}

function isActiveRun(run: TestRun): boolean {
  return run.status === "queued" || run.status === "preparing" || run.status === "running";
}

function isStageP95LatencyMetric(metric: NormalizedMetric): boolean {
  const key = metric.key.toLowerCase();
  if (metric.unit.toLowerCase() !== "ms") return false;
  if (!key.includes("p95")) return false;
  return (
    key.includes("capture") ||
    key.includes("source_wait") ||
    key.includes("encode") ||
    key.includes("transport") ||
    key.includes("decode") ||
    key.includes("render") ||
    key.includes("present") ||
    key.includes("interactive") ||
    key.includes("total")
  );
}

function compareStageLatencyMetric(left: NormalizedMetric, right: NormalizedMetric): number {
  return stageLatencyRank(left.key) - stageLatencyRank(right.key) || left.label.localeCompare(right.label);
}

function stageLatencyRank(key: string): number {
  const normalized = key.toLowerCase();
  const order = [
    "capture",
    "source_wait",
    "encode",
    "transport",
    "decode",
    "render",
    "present",
    "interactive",
    "total",
  ];
  const index = order.findIndex((part) => normalized.includes(part));
  return index === -1 ? order.length : index;
}

function latestMetricValue(metric?: NormalizedMetric): number | null {
  if (!metric || metric.samples.length === 0) return null;
  return metric.samples[metric.samples.length - 1]!.value;
}

function formatFps(value: number | null): string {
  return value == null ? "-" : `${value.toFixed(1)} FPS`;
}

function formatMs(value: number | null): string {
  return value == null ? "-" : `${value.toFixed(2)} ms`;
}

function StatusBadge({ status }: { status: CapabilityStatus | "ready" | "blocked" | "skipped" }) {
  return (
    <span
      className={[
        "rounded-full px-2 py-0.5 text-[10px] font-semibold",
        statusClassName(status),
      ].join(" ")}
    >
      {status}
    </span>
  );
}

function statusClassName(status: CapabilityStatus | "ready" | "blocked" | "skipped"): string {
  switch (status) {
    case "available":
    case "usable":
    case "ready":
      return "bg-green-500/12 text-green-600 dark:text-green-300";
    case "degraded":
      return "bg-yellow-500/12 text-yellow-700 dark:text-yellow-300";
    case "blocked":
    case "permission_missing":
    case "driver_missing":
    case "hardware_missing":
      return "bg-red-500/12 text-red-600 dark:text-red-300";
    case "unimplemented":
    case "unsupported":
    case "skipped":
      return "bg-slate-500/12 text-slate-600 dark:text-slate-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}
