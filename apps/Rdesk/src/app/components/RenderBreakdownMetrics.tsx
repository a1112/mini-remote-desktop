import type { HarnessMetrics } from "../adapters/tauri/types";

function formatMs(value: number | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `${value.toFixed(2)} ms` : "-";
}

function formatCount(value: number | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `${value}` : "-";
}

function hasRenderBreakdown(metrics: HarnessMetrics): boolean {
  return [
    metrics.render_latency_p95_ms,
    metrics.render_submit_wait_latency_p95_ms,
    metrics.render_execute_latency_p95_ms,
    metrics.render_prepare_wait_latency_p95_ms,
    metrics.render_shared_resource_latency_p95_ms,
    metrics.render_draw_present_latency_p95_ms,
    metrics.render_present_gap_p95_ms,
    metrics.render_queue_replacements,
    metrics.render_stale_frame_drops,
  ].some((value) => typeof value === "number" && Number.isFinite(value));
}

export function RenderBreakdownMetrics({ metrics }: { metrics: HarnessMetrics }) {
  if (!hasRenderBreakdown(metrics)) {
    return null;
  }

  return (
    <section className="mb-6 bg-card rounded-lg border p-4">
      <h3 className="text-sm font-medium mb-3">Render Pipeline Breakdown</h3>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
        <MetricRow label="Render Upload P95:" value={formatMs(metrics.render_latency_p95_ms)} />
        <MetricRow
          label="Submit Wait P95:"
          value={formatMs(metrics.render_submit_wait_latency_p95_ms)}
        />
        <MetricRow
          label="Render Execute P95:"
          value={formatMs(metrics.render_execute_latency_p95_ms)}
        />
        <MetricRow
          label="Prepare Wait P95:"
          value={formatMs(metrics.render_prepare_wait_latency_p95_ms)}
        />
        <MetricRow
          label="Shared Resource P95:"
          value={formatMs(metrics.render_shared_resource_latency_p95_ms)}
        />
        <MetricRow
          label="Draw/Present P95:"
          value={formatMs(metrics.render_draw_present_latency_p95_ms)}
        />
        <MetricRow
          label="Present Gap P95:"
          value={formatMs(metrics.render_present_gap_p95_ms)}
        />
        <MetricRow
          label="Render Queue Replacements:"
          value={formatCount(metrics.render_queue_replacements)}
        />
        <MetricRow
          label="Render Stale Drops:"
          value={formatCount(metrics.render_stale_frame_drops)}
        />
      </div>
    </section>
  );
}

function MetricRow({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span className="text-muted-foreground">{label}</span>{" "}
      <span className="font-medium">{value}</span>
    </div>
  );
}
