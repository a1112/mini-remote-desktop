import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { TestTelemetryPanel } from "./TestTelemetryPanel";

describe("TestTelemetryPanel", () => {
  it("shows selectable metric curves and filters logs by source", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue({
      run: {
        run_id: "run-1",
        scenario_id: "e2e.local",
        status: "completed",
        started_at: 1000,
        finished_at: 3000,
        tags: ["manual"],
      },
      metrics: {
        capture_fps: {
          metric_name: "capture_fps",
          unit: "fps",
          samples: [{ timestamp: 1000, value: 144 }],
        },
        decode_latency_p95_ms: {
          metric_name: "decode_latency_p95_ms",
          unit: "ms",
          samples: [{ timestamp: 1000, value: 6.5 }],
        },
      },
      events: [{ stage: "capture", status: "started", timestamp: 1000 }],
      logs: [{ run_id: "run-1", timestamp: 1500, level: "info", source: "raw_log", message: "boot" }],
      artifacts: [],
      diagnostics: { corrupt_rows: 0, warnings: [] },
    });

    render(<TestTelemetryPanel runId="run-1" mode="inline" />);

    expect(await screen.findByText("e2e.local")).toBeInTheDocument();
    expect(screen.getByText("Capture FPS")).toBeInTheDocument();
    expect(screen.getByText("Decode Latency P95")).toBeInTheDocument();
    expect(await screen.findByText("单位：fps")).toBeInTheDocument();

    const sourceSelect = screen.getByLabelText("测试日志");
    await userEvent.selectOptions(sourceSelect, "raw_log");
    expect(screen.getByText("boot")).toBeInTheDocument();
    expect(screen.queryByText("capture started")).not.toBeInTheDocument();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_get_run_telemetry", {
        runId: "run-1",
        query: { max_points: 1200 },
      });
    });
  });

  it("renders an empty state when no metrics are available", async () => {
    getMockInvoke().mockResolvedValue({
      run: null,
      metrics: {},
      events: [],
      logs: [],
      artifacts: [],
      diagnostics: { corrupt_rows: 0, warnings: [] },
    });

    render(<TestTelemetryPanel runId="run-empty" />);

    const panel = await screen.findByLabelText("测试曲线");
    expect(within(panel).getByText("暂无指标样本")).toBeInTheDocument();
  });
});
