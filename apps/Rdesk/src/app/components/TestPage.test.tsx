import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../test/mocks/tauri";
import { TestPage } from "./TestPage";

describe("TestPage render diagnostics", () => {
  it("shows the render pipeline execution breakdown from harness metrics", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_harness_start") return Promise.resolve(null);
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 144,
          encode_latency_p50_ms: 1.1,
          encode_latency_p95_ms: 2.2,
          decode_latency_p50_ms: 1.3,
          decode_latency_p95_ms: 2.4,
          render_latency_p95_ms: 0.71,
          render_submit_wait_latency_p95_ms: 0.12,
          render_execute_latency_p95_ms: 0.56,
          render_prepare_wait_latency_p95_ms: 0.01,
          render_shared_resource_latency_p95_ms: 0.02,
          render_draw_present_latency_p95_ms: 0.53,
          render_present_gap_p95_ms: 8.9,
          render_queue_replacements: 2,
          render_stale_frame_drops: 3,
          frame_count: 360,
          dropped_frames: 0,
          resolution: [2560, 1440],
        });
      }
      return Promise.resolve(null);
    });

    render(<TestPage />);

    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("Render Pipeline Breakdown")).toBeInTheDocument();
    expect(screen.getByText("Render Execute P95:")).toBeInTheDocument();
    expect(screen.getByText("0.56 ms")).toBeInTheDocument();
    expect(screen.getByText("Draw/Present P95:")).toBeInTheDocument();
    expect(screen.getByText("0.53 ms")).toBeInTheDocument();
    expect(screen.getByText("Render Queue Replacements:")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    expect(screen.getByText("Render Stale Drops:")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });
});
