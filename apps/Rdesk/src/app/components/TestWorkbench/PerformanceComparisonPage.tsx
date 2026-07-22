import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { BarChart3, Download, RefreshCw } from "lucide-react";
import {
  Bar,
  CartesianGrid,
  Cell,
  ComposedChart,
  Legend,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { testListRuns } from "../../adapters/tauri/commands";
import type { TestClassification, TestRun } from "../../adapters/tauri/types";
import {
  performanceRowFromRun,
  type PerformanceComparisonRow,
} from "../../services/testClassificationService";

type FilterState = {
  device: string;
  runScope: string;
  memoryPath: string;
  encodeAccel: string;
  decodeAccel: string;
  transportPath: string;
  renderPath: string;
  resolution: string;
  targetFps: string;
  bitrateMbps: string;
  status: string;
};

type FilterKey = keyof FilterState;
type FilterOption = {
  value: string;
  completed: number;
  failed: number;
  total: number;
};
type DetailedSummaryRow = {
  key: string;
  deviceLabel: string;
  runScope: string;
  memoryPath: string;
  encodeAccel: string;
  decodeAccel: string;
  transportPath: string;
  renderPath: string;
  resolution: string;
  targetFps: number | null;
  bitrateBucket: string;
  count: number;
  completed: number;
  failed: number;
  skipped: number;
  issues: number;
  fpsAvg: number | null;
  fpsMin: number | null;
  latencyP50Ms: number | null;
  latencyP95Ms: number | null;
  threeFrameBudgetMs: number | null;
  droppedFrames: number;
  dropRatePct: number | null;
  frameCount: number;
  cpuP95Percent: number | null;
  gpuP95Percent: number | null;
  memoryPeakMb: number | null;
  networkPeakMbps: number | null;
};

const ALL = "all";
const BITRATE_BUCKETS = [
  { value: "le_5", maxMbps: 5, label: "≤5M" },
  { value: "le_20", maxMbps: 20, label: "≤20M" },
  { value: "le_50", maxMbps: 50, label: "≤50M" },
  { value: "le_80", maxMbps: 80, label: "≤80M" },
  { value: "le_120", maxMbps: 120, label: "≤120M" },
  { value: "unlimited", maxMbps: null, label: "不限" },
] as const;
const FILTER_KEYS: FilterKey[] = [
  "device",
  "runScope",
  "memoryPath",
  "encodeAccel",
  "decodeAccel",
  "transportPath",
  "renderPath",
  "resolution",
  "targetFps",
  "bitrateMbps",
  "status",
];
const LOW_FPS_TARGET_RATIO = 0.8;
const LOW_FPS_FALLBACK = 10;
const CHART_COLORS = {
  fps: "#2563eb",
  fpsMin: "#0f766e",
  problem: "#dc2626",
  latencyP50: "#16a34a",
  latencyP95: "#dc2626",
  budget: "#9333ea",
  drop: "#ea580c",
  frames: "#0891b2",
  cpu: "#4f46e5",
  gpu: "#be123c",
  memory: "#64748b",
  network: "#7c3aed",
};

export function PerformanceComparisonPage() {
  const [runs, setRuns] = useState<TestRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filters, setFilters] = useState<FilterState>({
    device: ALL,
    runScope: ALL,
    memoryPath: ALL,
    encodeAccel: ALL,
    decodeAccel: ALL,
    transportPath: ALL,
    renderPath: ALL,
    resolution: ALL,
    targetFps: ALL,
    bitrateMbps: ALL,
    status: ALL,
  });

  const loadRuns = async () => {
    setLoading(true);
    setError(null);
    const result = await testListRuns({ limit: 200 });
    if (result.ok) {
      setRuns(result.value);
    } else {
      setError(result.error.message);
    }
    setLoading(false);
  };

  useEffect(() => {
    void loadRuns();
  }, []);

  const rows = useMemo(
    () => runs.map(performanceRowFromRun).sort((a, b) => b.startedAt - a.startedAt),
    [runs]
  );
  const filteredRows = useMemo(
    () => rows.filter((row) => matchesFilters(row, filters)),
    [filters, rows]
  );
  const groupedRows = useMemo(
    () => buildDetailedSummaryRows(filteredRows).slice(0, 60),
    [filteredRows]
  );
  const chartRows = useMemo(() => filteredRows.slice(0, 40).reverse(), [filteredRows]);
  const options = useMemo(() => buildFacetedFilterOptions(rows, filters), [filters, rows]);

  const completedCount = filteredRows.filter((row) => row.status === "completed").length;
  const failedCount = filteredRows.filter((row) => row.status === "failed").length;
  const skippedCount = filteredRows.filter((row) => row.status === "skipped").length;
  const lowFpsCount = filteredRows.filter(isLowFpsRow).length;
  const issueCount = filteredRows.filter(isProblemRow).length;
  const medianLatency = average(filteredRows.map((row) => row.latencyP95Ms));
  const averageFps = average(filteredRows.map((row) => row.fpsAvg));

  return (
    <div className="mx-auto max-w-7xl space-y-6 p-6">
      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-2xl font-bold text-foreground">
            <BarChart3 className="h-6 w-6" />
            性能对比
          </h1>
          <p className="text-sm text-muted-foreground">
            按设备、零拷贝路径、编解码加速、传输、渲染、配置和状态对比历史测试结果。
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => exportRowsAsCsv(filteredRows)}
            disabled={filteredRows.length === 0}
            className="inline-flex items-center gap-2 rounded border border-border bg-secondary px-3 py-2 text-sm text-secondary-foreground hover:bg-secondary/80 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Download className="h-4 w-4" />
            导出 CSV
          </button>
          <button
            type="button"
            onClick={() => void loadRuns()}
            className="inline-flex items-center gap-2 rounded border border-border bg-secondary px-3 py-2 text-sm text-secondary-foreground hover:bg-secondary/80"
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
      </div>

      <section className="rounded-lg border bg-card p-4">
        <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-5">
          <FilterSelect label="设备" value={filters.device} values={options.device} formatOption={deviceLabel} onChange={(value) => setFilters({ ...filters, device: value })} />
          <FilterSelect label="范围" value={filters.runScope} values={options.runScope} formatOption={(value) => classificationLabel("runScope", value)} onChange={(value) => setFilters({ ...filters, runScope: value })} />
          <FilterSelect label="内存路径" value={filters.memoryPath} values={options.memoryPath} formatOption={(value) => classificationLabel("memoryPath", value)} onChange={(value) => setFilters({ ...filters, memoryPath: value })} />
          <FilterSelect label="编码" value={filters.encodeAccel} values={options.encodeAccel} formatOption={(value) => classificationLabel("accel", value)} onChange={(value) => setFilters({ ...filters, encodeAccel: value })} />
          <FilterSelect label="解码" value={filters.decodeAccel} values={options.decodeAccel} formatOption={(value) => classificationLabel("accel", value)} onChange={(value) => setFilters({ ...filters, decodeAccel: value })} />
          <FilterSelect label="传输" value={filters.transportPath} values={options.transportPath} formatOption={(value) => classificationLabel("transport", value)} onChange={(value) => setFilters({ ...filters, transportPath: value })} />
          <FilterSelect label="渲染" value={filters.renderPath} values={options.renderPath} formatOption={(value) => classificationLabel("render", value)} onChange={(value) => setFilters({ ...filters, renderPath: value })} />
          <FilterSelect label="分辨率" value={filters.resolution} values={options.resolution} onChange={(value) => setFilters({ ...filters, resolution: value })} />
          <FilterSelect label="目标 FPS" value={filters.targetFps} values={options.targetFps} formatOption={(value) => value === "unknown" ? "未设置" : `${value} FPS`} onChange={(value) => setFilters({ ...filters, targetFps: value })} />
          <FilterSelect label="码率" value={filters.bitrateMbps} values={options.bitrateMbps} formatOption={bitrateBucketLabel} onChange={(value) => setFilters({ ...filters, bitrateMbps: value })} />
          <FilterSelect label="状态" value={filters.status} values={options.status} formatOption={statusLabel} onChange={(value) => setFilters({ ...filters, status: value })} />
        </div>
      </section>

      {error && (
        <div className="rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <section className="grid gap-4 md:grid-cols-5">
        <SummaryMetric label="测试数" value={filteredRows.length.toString()} detail={`${completedCount} 已完成 / ${failedCount} 失败 / ${skippedCount} 已跳过`} />
        <SummaryMetric label="问题" value={issueCount.toString()} detail={`${failedCount} 失败 / ${lowFpsCount} 低 FPS`} tone={issueCount > 0 ? "danger" : "default"} />
        <SummaryMetric label="平均 FPS" value={formatNumber(averageFps, " FPS")} detail="当前筛选测试的 capture_fps 均值" />
        <SummaryMetric label="平均 P95" value={formatNumber(medianLatency, " ms")} detail="当前筛选测试的端到端 P95 均值" />
        <SummaryMetric label="设备数" value={new Set(filteredRows.map((row) => row.deviceLabel)).size.toString()} detail="本机与跨设备标签数" />
      </section>

      <section className="rounded-lg border bg-card p-4 text-sm">
        <h2 className="mb-2 text-base font-semibold">分类说明</h2>
        <div className="grid gap-2 text-muted-foreground md:grid-cols-2">
          <div><span className="font-mono text-foreground">none</span> 表示该测试链路明确没有使用这个阶段，例如 capture-only、无解码器、无传输或无渲染。</div>
          <div><span className="font-mono text-foreground">unknown</span> 表示历史/导入数据缺少足够元数据，或当前逻辑无法安全推导该值。</div>
          <div className="md:col-span-2"><span className="font-mono text-red-600">红色</span> 表示失败或低 FPS。低 FPS 指低于目标 FPS 的 80%，没有目标 FPS 时低于 10 FPS。</div>
        </div>
      </section>

      {loading ? (
        <div className="flex h-64 items-center justify-center text-muted-foreground">
          正在加载性能历史...
        </div>
      ) : filteredRows.length === 0 ? (
        <div className="rounded-lg border bg-card p-8 text-center text-muted-foreground">
          没有符合当前筛选条件的测试记录。
        </div>
      ) : (
        <>
          <section className="grid gap-4 xl:grid-cols-2">
            <ChartPanel title="按测试对比 FPS">
              <ResponsiveContainer width="100%" height={280}>
                <ComposedChart data={chartRows}>
                  <CartesianGrid strokeDasharray="3 3" />
                  <XAxis dataKey="runId" tickFormatter={(_, index) => String(index + 1)} />
                  <YAxis />
                  <Tooltip content={<RunTooltip />} />
                  <Legend />
                  <Bar dataKey="fpsAvg" name="平均 FPS" fill={CHART_COLORS.fps}>
                    {chartRows.map((row) => (
                      <Cell key={`fps-${row.runId}`} fill={isProblemRow(row) ? CHART_COLORS.problem : CHART_COLORS.fps} />
                    ))}
                  </Bar>
                  <Bar dataKey="fpsMin" name="最低 FPS" fill={CHART_COLORS.fpsMin}>
                    {chartRows.map((row) => (
                      <Cell key={`fps-min-${row.runId}`} fill={isProblemRow(row) ? CHART_COLORS.problem : CHART_COLORS.fpsMin} />
                    ))}
                  </Bar>
                </ComposedChart>
              </ResponsiveContainer>
            </ChartPanel>

            <ChartPanel title="端到端延迟与 3 帧目标">
              <ResponsiveContainer width="100%" height={280}>
                <ComposedChart data={chartRows}>
                  <CartesianGrid strokeDasharray="3 3" />
                  <XAxis dataKey="runId" tickFormatter={(_, index) => String(index + 1)} />
                  <YAxis />
                  <Tooltip content={<RunTooltip />} />
                  <Legend />
                  <Bar dataKey="latencyP95Ms" name="P95 毫秒" fill={CHART_COLORS.latencyP95} />
                  <Bar dataKey="latencyP50Ms" name="P50/首帧 毫秒" fill={CHART_COLORS.latencyP50}>
                    {chartRows.map((row) => (
                      <Cell key={`latency-${row.runId}`} fill={isProblemRow(row) ? CHART_COLORS.problem : CHART_COLORS.latencyP50} />
                    ))}
                  </Bar>
                  <Line dataKey="threeFrameBudgetMs" name="3 帧目标" stroke={CHART_COLORS.budget} strokeWidth={2} dot={false} />
                </ComposedChart>
              </ResponsiveContainer>
            </ChartPanel>

            <ChartPanel title="掉帧与帧数">
              <ResponsiveContainer width="100%" height={280}>
                <ComposedChart data={chartRows}>
                  <CartesianGrid strokeDasharray="3 3" />
                  <XAxis dataKey="runId" tickFormatter={(_, index) => String(index + 1)} />
                  <YAxis yAxisId="left" />
                  <YAxis yAxisId="right" orientation="right" />
                  <Tooltip content={<RunTooltip />} />
                  <Legend />
                  <Bar yAxisId="left" dataKey="frameCount" name="帧数" fill={CHART_COLORS.frames} />
                  <Line yAxisId="right" dataKey="dropRatePct" name="掉帧率 %" stroke={CHART_COLORS.drop} strokeWidth={2} />
                </ComposedChart>
              </ResponsiveContainer>
            </ChartPanel>

            <ChartPanel title="资源占用">
              <ResponsiveContainer width="100%" height={280}>
                <ComposedChart data={chartRows}>
                  <CartesianGrid strokeDasharray="3 3" />
                  <XAxis dataKey="runId" tickFormatter={(_, index) => String(index + 1)} />
                  <YAxis />
                  <Tooltip content={<RunTooltip />} />
                  <Legend />
                  <Bar dataKey="cpuP95Percent" name="CPU P95 %" fill={CHART_COLORS.cpu} />
                  <Bar dataKey="gpuP95Percent" name="GPU P95 %" fill={CHART_COLORS.gpu} />
                  <Line dataKey="networkPeakMbps" name="网络峰值 Mbps" stroke={CHART_COLORS.network} strokeWidth={2} />
                </ComposedChart>
              </ResponsiveContainer>
            </ChartPanel>
          </section>

          <section className="rounded-lg border bg-card p-4">
            <h2 className="mb-3 text-lg font-semibold">精细汇总</h2>
            <div className="overflow-x-auto">
              <table className="w-full min-w-[1760px] border-separate border-spacing-0 text-xs">
                <thead className="bg-muted text-muted-foreground">
                  <tr>
                    <th className="sticky left-0 z-10 bg-muted px-2 py-2 text-left">设备</th>
                    <th className="px-2 py-2 text-left">范围</th>
                    <th className="px-2 py-2 text-left">内存路径</th>
                    <th className="px-2 py-2 text-left">编码</th>
                    <th className="px-2 py-2 text-left">解码</th>
                    <th className="px-2 py-2 text-left">传输</th>
                    <th className="px-2 py-2 text-left">渲染</th>
                    <th className="px-2 py-2 text-left">分辨率</th>
                    <th className="px-2 py-2 text-right">目标 FPS</th>
                    <th className="px-2 py-2 text-left">码率档位</th>
                    <th className="px-2 py-2 text-right">总数</th>
                    <th className="px-2 py-2 text-right">成功</th>
                    <th className="px-2 py-2 text-right">失败</th>
                    <th className="px-2 py-2 text-right">跳过</th>
                    <th className="px-2 py-2 text-right">问题</th>
                    <th className="px-2 py-2 text-right">均 FPS</th>
                    <th className="px-2 py-2 text-right">低 FPS</th>
                    <th className="px-2 py-2 text-right">P50 ms</th>
                    <th className="px-2 py-2 text-right">P95 ms</th>
                    <th className="px-2 py-2 text-right">3 帧 ms</th>
                    <th className="px-2 py-2 text-right">掉帧</th>
                    <th className="px-2 py-2 text-right">掉帧率 %</th>
                    <th className="px-2 py-2 text-right">帧数</th>
                    <th className="px-2 py-2 text-right">CPU P95 %</th>
                    <th className="px-2 py-2 text-right">GPU P95 %</th>
                    <th className="px-2 py-2 text-right">内存峰值 MB</th>
                    <th className="px-2 py-2 text-right">网络峰值 Mbps</th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {groupedRows.map((row) => (
                    <tr key={row.key} className={row.issues > 0 ? "bg-red-50 text-red-900" : undefined}>
                      <td className={`sticky left-0 z-10 max-w-[180px] truncate px-2 py-2 font-medium ${row.issues > 0 ? "bg-red-50" : "bg-card"}`} title={deviceLabel(row.deviceLabel)}>{deviceLabel(row.deviceLabel)}</td>
                      <td className="px-2 py-2">{classificationLabel("runScope", row.runScope)}</td>
                      <td className="px-2 py-2">{classificationLabel("memoryPath", row.memoryPath)}</td>
                      <td className="px-2 py-2">{classificationLabel("accel", row.encodeAccel)}</td>
                      <td className="px-2 py-2">{classificationLabel("accel", row.decodeAccel)}</td>
                      <td className="px-2 py-2">{classificationLabel("transport", row.transportPath)}</td>
                      <td className="px-2 py-2">{classificationLabel("render", row.renderPath)}</td>
                      <td className="px-2 py-2 font-mono">{row.resolution}</td>
                      <td className="px-2 py-2 text-right">{row.targetFps ?? "-"}</td>
                      <td className="px-2 py-2">{bitrateBucketLabel(row.bitrateBucket)}</td>
                      <td className="px-2 py-2 text-right font-medium">{row.count}</td>
                      <td className="px-2 py-2 text-right">{row.completed}</td>
                      <td className={`px-2 py-2 text-right ${row.failed > 0 ? "font-semibold text-red-700" : ""}`}>{row.failed}</td>
                      <td className="px-2 py-2 text-right">{row.skipped}</td>
                      <td className={`px-2 py-2 text-right ${row.issues > 0 ? "font-semibold text-red-700" : ""}`}>{row.issues}</td>
                      <td className={`px-2 py-2 text-right ${row.issues > 0 ? "font-semibold text-red-700" : ""}`}>{formatNumber(row.fpsAvg)}</td>
                      <td className={`px-2 py-2 text-right ${row.issues > 0 ? "font-semibold text-red-700" : ""}`}>{formatNumber(row.fpsMin)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.latencyP50Ms)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.latencyP95Ms)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.threeFrameBudgetMs)}</td>
                      <td className="px-2 py-2 text-right">{row.droppedFrames}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.dropRatePct)}</td>
                      <td className="px-2 py-2 text-right">{row.frameCount}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.cpuP95Percent)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.gpuP95Percent)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.memoryPeakMb)}</td>
                      <td className="px-2 py-2 text-right">{formatNumber(row.networkPeakMbps)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        </>
      )}
    </div>
  );
}

function FilterSelect({
  label,
  value,
  values,
  formatOption = (option) => option,
  onChange,
}: {
  label: string;
  value: string;
  values: FilterOption[];
  formatOption?: (value: string) => string;
  onChange: (value: string) => void;
}) {
  const selected = values.find((option) => option.value === value) ?? values[0];
  const selectedLabel = selected
    ? selected.value === ALL
      ? "全部"
      : formatOption(selected.value)
    : "全部";

  return (
    <div className="relative block">
      <div className="mb-1 block text-xs font-medium text-muted-foreground">{label}</div>
      <details className="group">
        <summary className="flex min-h-9 cursor-pointer list-none items-center justify-between gap-2 rounded border border-border bg-background px-3 py-2 text-sm marker:hidden">
          <span className="truncate">{selectedLabel}</span>
          <span className="shrink-0 text-[11px] text-muted-foreground">
            {selected ? `${selected.completed} / ${selected.failed}` : ""}
          </span>
        </summary>
        <div className="absolute z-30 mt-1 max-h-72 w-full overflow-auto rounded border border-border bg-popover p-1 shadow-lg">
          {values.map((option) => {
            const optionLabel = option.value === ALL ? "全部" : formatOption(option.value);
            const active = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                onClick={(event) => {
                  onChange(option.value);
                  event.currentTarget.closest("details")?.removeAttribute("open");
                }}
                className={`flex w-full items-center justify-between gap-2 rounded px-2 py-2 text-left text-sm hover:bg-muted ${active ? "bg-muted font-medium" : ""}`}
              >
                <span className="min-w-0 truncate">{optionLabel}</span>
                <span className="shrink-0 rounded bg-background px-2 py-0.5 text-[11px] text-muted-foreground">
                  {option.completed} / {option.failed}
                </span>
              </button>
            );
          })}
        </div>
      </details>
    </div>
  );
}

function SummaryMetric({
  label,
  value,
  detail,
  tone = "default",
}: {
  label: string;
  value: string;
  detail: string;
  tone?: "default" | "danger";
}) {
  return (
    <div className={`rounded-lg border p-4 ${tone === "danger" ? "border-red-300 bg-red-50 text-red-900" : "bg-card"}`}>
      <div className="text-xs font-medium uppercase text-muted-foreground">{label}</div>
      <div className="mt-1 text-2xl font-semibold">{value}</div>
      <div className="mt-1 text-xs text-muted-foreground">{detail}</div>
    </div>
  );
}

function ChartPanel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-lg border bg-card p-4">
      <h2 className="mb-3 text-lg font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function RunTooltip({ active, payload }: { active?: boolean; payload?: Array<{ payload: PerformanceComparisonRow; name: string; value: number | null }> }) {
  if (!active || !payload?.length) return null;
  const first = payload[0];
  if (!first) return null;
  const row = first.payload;
  return (
    <div className="max-w-sm rounded border bg-popover p-3 text-xs shadow">
      <div className="mb-1 font-semibold">{row.scenarioId}</div>
      <div className="mb-2 text-muted-foreground">{row.label}</div>
      <div>分辨率: {row.resolution} @ {row.targetFps ?? "-"} FPS / {row.bitrateMbps?.toFixed(1) ?? "-"} Mbps</div>
      <div className={isProblemRow(row) ? "font-semibold text-red-600" : undefined}>健康状态: {healthLabel(row)}</div>
      <div>状态: {statusLabel(row.status)}</div>
      {payload.map((item) => (
        <div key={item.name}>
          {item.name}: {formatNumber(typeof item.value === "number" ? item.value : null)}
        </div>
      ))}
    </div>
  );
}

function matchesFilters(row: PerformanceComparisonRow, filters: FilterState) {
  return (
    matches(filters.device, row.deviceLabel) &&
    matches(filters.runScope, row.runScope) &&
    matches(filters.memoryPath, row.memoryPath) &&
    matches(filters.encodeAccel, row.encodeAccel) &&
    matches(filters.decodeAccel, row.decodeAccel) &&
    matches(filters.transportPath, row.transportPath) &&
    matches(filters.renderPath, row.renderPath) &&
    matches(filters.resolution, row.resolution) &&
    matches(filters.targetFps, row.targetFps == null ? "unknown" : String(row.targetFps)) &&
    matches(filters.bitrateMbps, rowValueForFilter(row, "bitrateMbps")) &&
    matches(filters.status, row.status)
  );
}

function buildFacetedFilterOptions(rows: PerformanceComparisonRow[], filters: FilterState): Record<FilterKey, FilterOption[]> {
  return Object.fromEntries(
    FILTER_KEYS.map((key) => [
      key,
      buildFilterOptionStats(
        key,
        rows.filter((row) => matchesFiltersExcept(row, filters, key))
      ),
    ])
  ) as Record<FilterKey, FilterOption[]>;
}

function buildFilterOptionStats(key: FilterKey, rows: PerformanceComparisonRow[]) {
  const values = key === "bitrateMbps"
    ? BITRATE_BUCKETS.map((bucket) => bucket.value)
    : unique(rows.map((row) => rowValueForFilter(row, key)));
  return [ALL, ...values].map((value) => {
    const optionRows = value === ALL ? rows : rows.filter((row) => rowValueForFilter(row, key) === value);
    return {
      value,
      completed: optionRows.filter((row) => row.status === "completed").length,
      failed: optionRows.filter((row) => row.status === "failed").length,
      total: optionRows.length,
    };
  });
}

function matchesFiltersExcept(row: PerformanceComparisonRow, filters: FilterState, except: FilterKey) {
  return FILTER_KEYS.every((key) => key === except || matches(filters[key], rowValueForFilter(row, key)));
}

function rowValueForFilter(row: PerformanceComparisonRow, key: FilterKey) {
  switch (key) {
    case "device":
      return row.deviceLabel;
    case "runScope":
      return row.runScope;
    case "memoryPath":
      return row.memoryPath;
    case "encodeAccel":
      return row.encodeAccel;
    case "decodeAccel":
      return row.decodeAccel;
    case "transportPath":
      return row.transportPath;
    case "renderPath":
      return row.renderPath;
    case "resolution":
      return row.resolution;
    case "targetFps":
      return row.targetFps == null ? "unknown" : String(row.targetFps);
    case "bitrateMbps":
      return bitrateBucketForMbps(row.bitrateMbps);
    case "status":
      return row.status;
  }
}

function matches(filterValue: string, rowValue: string) {
  return filterValue === ALL || filterValue === rowValue;
}

function buildDetailedSummaryRows(rows: PerformanceComparisonRow[]): DetailedSummaryRow[] {
  const groups = new Map<string, PerformanceComparisonRow[]>();
  for (const row of rows) {
    const key = [
      row.deviceLabel,
      row.runScope,
      row.memoryPath,
      row.encodeAccel,
      row.decodeAccel,
      row.transportPath,
      row.renderPath,
      row.resolution,
      row.targetFps ?? "unknown",
      bitrateBucketForMbps(row.bitrateMbps),
    ].join(" / ");
    groups.set(key, [...(groups.get(key) ?? []), row]);
  }

  return Array.from(groups.entries())
    .map(([key, groupRows]) => {
      const first = groupRows[0];
      if (!first) throw new Error("summary group must contain at least one row");
      return {
        key,
        deviceLabel: first.deviceLabel,
        runScope: first.runScope,
        memoryPath: first.memoryPath,
        encodeAccel: first.encodeAccel,
        decodeAccel: first.decodeAccel,
        transportPath: first.transportPath,
        renderPath: first.renderPath,
        resolution: first.resolution,
        targetFps: first.targetFps,
        bitrateBucket: bitrateBucketForMbps(first.bitrateMbps),
        count: groupRows.length,
        completed: groupRows.filter((row) => row.status === "completed").length,
        failed: groupRows.filter((row) => row.status === "failed").length,
        skipped: groupRows.filter((row) => row.status === "skipped").length,
        issues: groupRows.filter(isProblemRow).length,
        fpsAvg: average(groupRows.map((row) => row.fpsAvg)),
        fpsMin: min(groupRows.map((row) => row.fpsMin ?? row.fpsAvg)),
        latencyP50Ms: average(groupRows.map((row) => row.latencyP50Ms)),
        latencyP95Ms: average(groupRows.map((row) => row.latencyP95Ms)),
        threeFrameBudgetMs: average(groupRows.map((row) => row.threeFrameBudgetMs)),
        droppedFrames: sum(groupRows.map((row) => row.droppedFrames)),
        dropRatePct: average(groupRows.map((row) => row.dropRatePct)),
        frameCount: sum(groupRows.map((row) => row.frameCount)),
        cpuP95Percent: average(groupRows.map((row) => row.cpuP95Percent)),
        gpuP95Percent: average(groupRows.map((row) => row.gpuP95Percent)),
        memoryPeakMb: max(groupRows.map((row) => row.memoryPeakMb)),
        networkPeakMbps: max(groupRows.map((row) => row.networkPeakMbps)),
      };
    })
    .sort((a, b) =>
      b.issues - a.issues ||
      b.failed - a.failed ||
      b.count - a.count ||
      a.deviceLabel.localeCompare(b.deviceLabel) ||
      a.key.localeCompare(b.key)
    );
}

function bitrateBucketForMbps(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value) || value <= 0) return "unlimited";
  for (const bucket of BITRATE_BUCKETS) {
    if (bucket.maxMbps != null && value <= bucket.maxMbps) return bucket.value;
  }
  return "unlimited";
}

function bitrateBucketLabel(value: string) {
  return BITRATE_BUCKETS.find((bucket) => bucket.value === value)?.label ?? value;
}

function classificationLabel(
  kind: "runScope" | "memoryPath" | "accel" | "transport" | "render",
  value: string
) {
  const labels: Record<typeof kind, Record<string, string>> = {
    runScope: {
      local: "本机测试",
      cross_device: "跨设备测试",
      unknown: "未知范围",
    },
    memoryPath: {
      zero_copy_d3d11_shared: "D3D11 零拷贝",
      cpu_copy: "CPU 拷贝",
      webrtc_media_stream: "WebRTC MediaStream",
      unknown: "未知内存路径",
      none: "未使用内存路径",
    },
    accel: {
      hardware: "硬件",
      software: "软件",
      browser: "浏览器",
      none: "未使用",
      unknown: "未知",
    },
    transport: {
      none: "无传输",
      webrtc: "WebRTC",
      quic: "QUIC",
      loopback: "本机回环",
      unknown: "未知传输",
    },
    render: {
      native_d3d11: "原生 D3D11",
      native_d3d12: "原生 D3D12",
      native_opengl: "原生 OpenGL",
      native_macos: "原生 macOS",
      native_linux: "原生 Linux",
      browser_video: "浏览器 video",
      webcodecs: "WebCodecs",
      none: "无渲染",
      unknown: "未知渲染",
    },
  };
  return labels[kind][value] ?? value;
}

function deviceLabel(value: string) {
  if (value === "local") return "本机";
  return value;
}

function isProblemRow(row: PerformanceComparisonRow) {
  return row.status === "failed" || isLowFpsRow(row);
}

function isLowFpsRow(row: PerformanceComparisonRow) {
  if (row.fpsAvg == null || !Number.isFinite(row.fpsAvg)) return false;
  if (row.targetFps != null && Number.isFinite(row.targetFps) && row.targetFps > 0) {
    return row.fpsAvg < row.targetFps * LOW_FPS_TARGET_RATIO;
  }
  return row.fpsAvg > 0 && row.fpsAvg < LOW_FPS_FALLBACK;
}

function healthLabel(row: PerformanceComparisonRow) {
  if (row.status === "failed" && isLowFpsRow(row)) return "失败 + 低 FPS";
  if (row.status === "failed") return "失败";
  if (isLowFpsRow(row)) return "低 FPS";
  return "正常";
}

function statusLabel(status: string) {
  switch (status) {
    case "queued":
      return "已排队";
    case "preparing":
      return "准备中";
    case "running":
      return "运行中";
    case "completed":
      return "已完成";
    case "failed":
      return "失败";
    case "skipped":
      return "已跳过";
    case "cancelled":
      return "已取消";
    default:
      return status;
  }
}

function unique(values: string[]) {
  return Array.from(new Set(values)).sort((a, b) => a.localeCompare(b));
}

function exportRowsAsCsv(rows: PerformanceComparisonRow[]) {
  if (rows.length === 0) return;
  const headers: Array<[string, (row: PerformanceComparisonRow) => string | number | null | undefined]> = [
    ["run_id", (row) => row.runId],
    ["scenario_id", (row) => row.scenarioId],
    ["status", (row) => row.status],
    ["started_at", (row) => new Date(row.startedAt).toISOString()],
    ["device", (row) => row.deviceLabel],
    ["run_scope", (row) => row.runScope],
    ["memory_path", (row) => row.memoryPath],
    ["encode_accel", (row) => row.encodeAccel],
    ["decode_accel", (row) => row.decodeAccel],
    ["transport_path", (row) => row.transportPath],
    ["render_path", (row) => row.renderPath],
    ["resolution", (row) => row.resolution],
    ["target_fps", (row) => row.targetFps],
    ["bitrate_mbps", (row) => row.bitrateMbps],
    ["fps_avg", (row) => row.fpsAvg],
    ["fps_min", (row) => row.fpsMin],
    ["latency_p50_ms", (row) => row.latencyP50Ms],
    ["latency_p95_ms", (row) => row.latencyP95Ms],
    ["three_frame_budget_ms", (row) => row.threeFrameBudgetMs],
    ["dropped_frames", (row) => row.droppedFrames],
    ["drop_rate_pct", (row) => row.dropRatePct],
    ["frame_count", (row) => row.frameCount],
    ["cpu_p95_percent", (row) => row.cpuP95Percent],
    ["gpu_p95_percent", (row) => row.gpuP95Percent],
    ["memory_peak_mb", (row) => row.memoryPeakMb],
    ["network_peak_mbps", (row) => row.networkPeakMbps],
    ["label", (row) => row.label],
  ];
  const csv = [
    headers.map(([header]) => csvCell(header)).join(","),
    ...rows.map((row) =>
      headers.map(([, getter]) => csvCell(getter(row))).join(",")
    ),
  ].join("\r\n");
  const blob = new Blob([`\uFEFF${csv}`], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `mrd-performance-${new Date().toISOString().replace(/[:.]/g, "-")}.csv`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

function csvCell(value: string | number | null | undefined) {
  if (value == null) return "";
  const text = String(value);
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

function average(values: Array<number | null>) {
  const finite = values.filter((value): value is number => value != null && Number.isFinite(value));
  if (finite.length === 0) return null;
  return finite.reduce((sum, value) => sum + value, 0) / finite.length;
}

function min(values: Array<number | null>) {
  const finite = values.filter((value): value is number => value != null && Number.isFinite(value));
  if (finite.length === 0) return null;
  return Math.min(...finite);
}

function max(values: Array<number | null>) {
  const finite = values.filter((value): value is number => value != null && Number.isFinite(value));
  if (finite.length === 0) return null;
  return Math.max(...finite);
}

function sum(values: Array<number | null | undefined>): number {
  let total = 0;
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) total += value;
  }
  return total;
}

function formatNumber(value: number | null | undefined, suffix = "") {
  if (value == null || !Number.isFinite(value)) return "-";
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)}${suffix}`;
}
