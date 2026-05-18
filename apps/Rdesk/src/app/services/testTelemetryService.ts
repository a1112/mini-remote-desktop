import type {
  Artifact,
  MetricSeries,
  TelemetryBundle,
  TelemetryLogEntry,
  TestStageEvent,
} from "../adapters/tauri/types";

export type MetricCategory =
  | "fps"
  | "bitrate"
  | "latency"
  | "drops_queue"
  | "profile_adaptive"
  | "transport"
  | "other";

export interface NormalizedMetric {
  key: string;
  label: string;
  unit: string;
  category: MetricCategory;
  source?: string;
  samples: Array<{
    timestamp: number;
    elapsedMs: number;
    value: number;
  }>;
}

export interface ChartGroup {
  unit: string;
  metrics: NormalizedMetric[];
  rows: Array<Record<string, number | string>>;
}

export interface NormalizedLogRow {
  key: string;
  timestamp: number;
  source: string;
  level: string;
  message: string;
}

export function metricCategory(series: MetricSeries): MetricCategory {
  const category = series.category;
  if (
    category === "fps" ||
    category === "bitrate" ||
    category === "latency" ||
    category === "drops_queue" ||
    category === "profile_adaptive" ||
    category === "transport"
  ) {
    return category;
  }

  const name = series.metric_name.toLowerCase();
  const unit = series.unit.toLowerCase();
  if (name.includes("fps") || unit === "fps") return "fps";
  if (name.includes("bitrate") || name.includes("bytes") || unit.includes("bps")) return "bitrate";
  if (name.includes("latency") || name.includes("time") || unit === "ms") return "latency";
  if (name.includes("drop") || name.includes("queue")) return "drops_queue";
  if (name.includes("profile") || name.includes("adaptive")) return "profile_adaptive";
  if (name.includes("transport") || name.includes("reassemble")) return "transport";
  return "other";
}

export function metricDisplayName(series: MetricSeries): string {
  if (series.display_name) return series.display_name;
  return series.metric_name
    .replace(/_ms$/, "")
    .split("_")
    .map((part) => {
      if (["fps", "p50", "p95", "p99"].includes(part.toLowerCase())) {
        return part.toUpperCase();
      }
      return part.charAt(0).toUpperCase() + part.slice(1);
    })
    .join(" ");
}

export function normalizeMetrics(
  metrics: Record<string, MetricSeries>,
  startedAt?: number
): NormalizedMetric[] {
  const fallbackStart = Object.values(metrics)
    .flatMap((series) => series.samples.map((sample) => sample.timestamp))
    .sort((a, b) => a - b)[0];
  const origin = startedAt ?? fallbackStart ?? 0;

  return Object.values(metrics)
    .map((series) => ({
      key: series.metric_name,
      label: metricDisplayName(series),
      unit: series.unit,
      category: metricCategory(series),
      source: series.source,
      samples: [...series.samples]
        .sort((a, b) => a.timestamp - b.timestamp)
        .map((sample) => ({
          timestamp: sample.timestamp,
          elapsedMs: sample.timestamp - origin,
          value: sample.value,
        })),
    }))
    .sort((a, b) => a.category.localeCompare(b.category) || a.label.localeCompare(b.label));
}

export function buildChartGroups(metrics: NormalizedMetric[]): ChartGroup[] {
  const groups = new Map<string, NormalizedMetric[]>();
  for (const metric of metrics) {
    if (!groups.has(metric.unit)) groups.set(metric.unit, []);
    groups.get(metric.unit)!.push(metric);
  }

  return Array.from(groups.entries()).map(([unit, unitMetrics]) => {
    const rowsByTime = new Map<number, Record<string, number | string>>();
    for (const metric of unitMetrics) {
      for (const sample of metric.samples) {
        const elapsedSec = Number((sample.elapsedMs / 1000).toFixed(2));
        const row = rowsByTime.get(sample.elapsedMs) ?? {
          elapsedMs: sample.elapsedMs,
          time: `${elapsedSec.toFixed(2)}s`,
        };
        row[metric.key] = Number(sample.value.toFixed(3));
        rowsByTime.set(sample.elapsedMs, row);
      }
    }

    return {
      unit,
      metrics: unitMetrics,
      rows: Array.from(rowsByTime.entries())
        .sort(([left], [right]) => left - right)
        .map(([, row]) => row),
    };
  });
}

export function normalizeLogRows(bundle: TelemetryBundle): NormalizedLogRow[] {
  const rows: NormalizedLogRow[] = [
    ...bundle.events.map(stageEventToLogRow),
    ...bundle.logs.map(telemetryLogToRow),
    ...bundle.artifacts.map(artifactToLogRow),
  ];
  return rows.sort((a, b) => a.timestamp - b.timestamp);
}

function stageEventToLogRow(event: TestStageEvent): NormalizedLogRow {
  return {
    key: `event:${event.stage}:${event.status}:${event.timestamp}`,
    timestamp: event.timestamp,
    source: "stage_event",
    level: event.error ? "error" : "info",
    message: `${event.stage} ${event.status}${event.error ? `: ${event.error}` : ""}`,
  };
}

function telemetryLogToRow(entry: TelemetryLogEntry): NormalizedLogRow {
  return {
    key: `log:${entry.source}:${entry.timestamp}:${entry.message}`,
    timestamp: entry.timestamp,
    source: entry.source,
    level: entry.level,
    message: entry.message,
  };
}

function artifactToLogRow(artifact: Artifact): NormalizedLogRow {
  return {
    key: `artifact:${artifact.artifact_id}`,
    timestamp: artifact.created_at,
    source: artifact.kind,
    level: "artifact",
    message: `${artifact.kind} ${artifact.metadata?.size_bytes ? `${artifact.metadata.size_bytes} bytes` : ""}`.trim(),
  };
}
