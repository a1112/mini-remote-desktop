import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  AlertTriangle,
  CheckSquare,
  RefreshCw,
  Square,
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
import { testGetRunTelemetry } from "../../adapters/tauri/commands";
import type { TelemetryBundle } from "../../adapters/tauri/types";
import {
  buildChartGroups,
  normalizeLogRows,
  normalizeMetrics,
  type MetricCategory,
} from "../../services/testTelemetryService";

type TelemetryPanelMode = "fullPage" | "modal" | "inline";

interface TestTelemetryPanelProps {
  runId: string;
  mode?: TelemetryPanelMode;
  className?: string;
}

const CATEGORY_LABELS: Record<MetricCategory, string> = {
  fps: "FPS",
  bitrate: "Bitrate",
  latency: "Latency",
  drops_queue: "Drops / Queue",
  profile_adaptive: "Profile / Adaptive",
  transport: "Transport",
  other: "Other",
};

const CHART_COLORS = [
  "#2563eb",
  "#16a34a",
  "#dc2626",
  "#9333ea",
  "#ea580c",
  "#0891b2",
  "#4f46e5",
  "#be123c",
];

export function TestTelemetryPanel({
  runId,
  mode = "fullPage",
  className = "",
}: TestTelemetryPanelProps) {
  const [bundle, setBundle] = useState<TelemetryBundle | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedMetrics, setSelectedMetrics] = useState<string[]>([]);
  const [logSource, setLogSource] = useState("all");

  const loadTelemetry = async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await testGetRunTelemetry(runId, { max_points: 1200 });
      if (result.ok) {
        setBundle(result.value);
      } else {
        setError(result.error.message);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    setBundle(null);
    setSelectedMetrics([]);
    void loadTelemetry();
  }, [runId]);

  const metrics = useMemo(
    () => normalizeMetrics(bundle?.metrics ?? {}, bundle?.run?.started_at),
    [bundle]
  );

  useEffect(() => {
    if (selectedMetrics.length > 0 || metrics.length === 0) return;
    const defaults = metrics
      .filter((metric) => metric.category === "fps" || metric.category === "latency")
      .slice(0, 6)
      .map((metric) => metric.key);
    setSelectedMetrics(defaults.length > 0 ? defaults : metrics.slice(0, 4).map((metric) => metric.key));
  }, [metrics, selectedMetrics.length]);

  const groupedMetrics = useMemo(() => {
    const groups = new Map<MetricCategory, typeof metrics>();
    for (const metric of metrics) {
      if (!groups.has(metric.category)) groups.set(metric.category, []);
      groups.get(metric.category)!.push(metric);
    }
    return Array.from(groups.entries());
  }, [metrics]);

  const selectedMetricSet = new Set(selectedMetrics);
  const chartGroups = buildChartGroups(metrics.filter((metric) => selectedMetricSet.has(metric.key)));
  const logs = useMemo(() => normalizeLogRows(bundle ?? emptyBundle()), [bundle]);
  const logSources = Array.from(new Set(logs.map((row) => row.source)));
  const visibleLogs = logSource === "all" ? logs : logs.filter((row) => row.source === logSource);

  const toggleMetric = (key: string) => {
    setSelectedMetrics((current) =>
      current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
    );
  };

  const compact = mode === "inline";

  return (
    <section
      className={`rounded-lg border border-gray-200 bg-white p-4 shadow-sm dark:border-gray-700 dark:bg-gray-800 ${className}`}
      aria-label="测试曲线"
    >
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <Activity className="h-5 w-5 text-blue-500" aria-hidden="true" />
            <h2 className="text-lg font-semibold">
              {bundle?.run?.scenario_id ?? "测试遥测"}
            </h2>
          </div>
          <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
            {runId}
            {bundle?.run?.status ? ` · ${bundle.run.status}` : ""}
          </p>
        </div>
        <button
          type="button"
          onClick={loadTelemetry}
          className="inline-flex items-center gap-2 rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700"
        >
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} aria-hidden="true" />
          刷新
        </button>
      </div>

      {error && (
        <div className="mb-4 rounded border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-900/20 dark:text-red-300">
          {error}
        </div>
      )}

      {bundle?.diagnostics?.corrupt_rows ? (
        <div className="mb-4 flex items-center gap-2 rounded border border-yellow-200 bg-yellow-50 p-3 text-sm text-yellow-800 dark:border-yellow-800 dark:bg-yellow-900/20 dark:text-yellow-200">
          <AlertTriangle className="h-4 w-4" aria-hidden="true" />
          遥测日志有 {bundle.diagnostics.corrupt_rows} 行损坏，已跳过。
        </div>
      ) : null}

      <div className={`grid gap-4 ${compact ? "grid-cols-1" : "lg:grid-cols-[minmax(0,1fr)_18rem]"}`}>
        <div className="min-w-0 space-y-4">
          <div className="rounded border border-gray-200 p-3 dark:border-gray-700">
            <div className="mb-2 text-sm font-medium">测试指标</div>
            {metrics.length === 0 ? (
              <p className="text-sm text-gray-500">暂无指标样本</p>
            ) : (
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                {groupedMetrics.map(([category, items]) => (
                  <div key={category}>
                    <div className="mb-1 text-xs font-medium uppercase tracking-wide text-gray-500">
                      {CATEGORY_LABELS[category]}
                    </div>
                    <div className="space-y-1">
                      {items.map((metric) => (
                        <label
                          key={metric.key}
                          className="flex cursor-pointer items-center gap-2 text-sm"
                        >
                          <input
                            type="checkbox"
                            className="sr-only"
                            checked={selectedMetricSet.has(metric.key)}
                            onChange={() => toggleMetric(metric.key)}
                          />
                          {selectedMetricSet.has(metric.key) ? (
                            <CheckSquare className="h-4 w-4 text-blue-500" aria-hidden="true" />
                          ) : (
                            <Square className="h-4 w-4 text-gray-400" aria-hidden="true" />
                          )}
                          <span className="truncate">{metric.label}</span>
                          <span className="text-xs text-gray-400">{metric.unit}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {chartGroups.length === 0 ? (
            <div className="flex h-48 items-center justify-center rounded border border-dashed border-gray-300 text-sm text-gray-500 dark:border-gray-700">
              选择指标后显示曲线
            </div>
          ) : (
            chartGroups.map((group) => (
              <div key={group.unit} className="rounded border border-gray-200 p-3 dark:border-gray-700">
                <div className="mb-2 text-sm font-medium">单位：{group.unit}</div>
                <div className={compact ? "h-56" : "h-72"}>
                  <ResponsiveContainer width="100%" height="100%">
                    <LineChart data={group.rows}>
                      <CartesianGrid strokeDasharray="3 3" />
                      <XAxis dataKey="time" minTickGap={24} />
                      <YAxis width={56} />
                      <Tooltip />
                      <Legend />
                      {group.metrics.map((metric, index) => (
                        <Line
                          key={metric.key}
                          type="monotone"
                          dataKey={metric.key}
                          name={metric.label}
                          dot={false}
                          stroke={CHART_COLORS[index % CHART_COLORS.length]}
                          strokeWidth={2}
                          isAnimationActive={false}
                          connectNulls
                        />
                      ))}
                    </LineChart>
                  </ResponsiveContainer>
                </div>
              </div>
            ))
          )}
        </div>

        <aside className="min-w-0 rounded border border-gray-200 p-3 dark:border-gray-700">
          <div className="mb-3 flex items-center justify-between gap-2">
            <label className="text-sm font-medium" htmlFor={`telemetry-log-source-${runId}`}>
              测试日志
            </label>
            <select
              id={`telemetry-log-source-${runId}`}
              value={logSource}
              onChange={(event) => setLogSource(event.target.value)}
              className="max-w-[12rem] rounded border px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
            >
              <option value="all">全部</option>
              {logSources.map((source) => (
                <option key={source} value={source}>
                  {source}
                </option>
              ))}
            </select>
          </div>
          <div className={compact ? "max-h-56 overflow-auto" : "max-h-[34rem] overflow-auto"}>
            {visibleLogs.length === 0 ? (
              <p className="text-sm text-gray-500">暂无日志</p>
            ) : (
              <ol className="space-y-2">
                {visibleLogs.map((row) => (
                  <li key={row.key} className="rounded bg-gray-50 p-2 text-xs dark:bg-gray-900/40">
                    <div className="mb-1 flex items-center justify-between gap-2 text-gray-500">
                      <span>{new Date(row.timestamp).toLocaleTimeString()}</span>
                      <span>{row.source}</span>
                    </div>
                    <div className={row.level === "error" ? "text-red-600 dark:text-red-300" : ""}>
                      {row.message}
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </div>
        </aside>
      </div>
    </section>
  );
}

function emptyBundle(): TelemetryBundle {
  return {
    run: null,
    metrics: {},
    events: [],
    logs: [],
    artifacts: [],
    diagnostics: { corrupt_rows: 0, warnings: [] },
  };
}
