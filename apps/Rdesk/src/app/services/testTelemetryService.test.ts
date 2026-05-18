import { describe, expect, it } from "vitest";
import type { TelemetryBundle } from "../adapters/tauri/types";
import { buildChartGroups, normalizeLogRows, normalizeMetrics } from "./testTelemetryService";

describe("testTelemetryService", () => {
  it("normalizes metric categories and elapsed timestamps", () => {
    const metrics = normalizeMetrics(
      {
        capture_fps: {
          metric_name: "capture_fps",
          unit: "fps",
          samples: [
            { timestamp: 1500, value: 60 },
            { timestamp: 1000, value: 30 },
          ],
        },
        decode_latency_p95_ms: {
          metric_name: "decode_latency_p95_ms",
          unit: "ms",
          samples: [{ timestamp: 1500, value: 4.25 }],
        },
      },
      1000
    );

    const captureFps = metrics.find((metric) => metric.key === "capture_fps");
    expect(captureFps?.category).toBe("fps");
    expect(captureFps?.samples[0]?.elapsedMs).toBe(0);
    expect(metrics.find((metric) => metric.key === "decode_latency_p95_ms")?.category).toBe("latency");
  });

  it("builds one chart group per unit", () => {
    const metrics = normalizeMetrics(
      {
        capture_fps: {
          metric_name: "capture_fps",
          unit: "fps",
          samples: [{ timestamp: 1000, value: 144 }],
        },
        total_latency_p95_ms: {
          metric_name: "total_latency_p95_ms",
          unit: "ms",
          samples: [{ timestamp: 1000, value: 8.5 }],
        },
      },
      1000
    );

    const groups = buildChartGroups(metrics);
    expect(groups.map((group) => group.unit).sort()).toEqual(["fps", "ms"]);
    expect(groups.find((group) => group.unit === "fps")?.rows[0]?.capture_fps).toBe(144);
  });

  it("merges stage events, logs, and artifacts by time", () => {
    const bundle: TelemetryBundle = {
      run: null,
      metrics: {},
      events: [
        { stage: "capture", status: "started", timestamp: 2000 },
      ],
      logs: [
        { run_id: "run-1", timestamp: 1000, level: "info", source: "raw_log", message: "boot" },
      ],
      artifacts: [
        {
          artifact_id: "artifact-1",
          kind: "summary_json",
          run_id: "run-1",
          created_at: 3000,
          data: "{}",
        },
      ],
      diagnostics: { corrupt_rows: 0, warnings: [] },
    };

    expect(normalizeLogRows(bundle).map((row) => row.source)).toEqual([
      "raw_log",
      "stage_event",
      "summary_json",
    ]);
  });
});
