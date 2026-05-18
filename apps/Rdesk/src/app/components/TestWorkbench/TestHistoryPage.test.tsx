import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { TestHistoryPage } from "./TestHistoryPage";

const mockNavigate = vi.hoisted(() => vi.fn());

vi.mock("react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router")>();
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  };
});

function mockHistoryCommands() {
  const mockInvoke = getMockInvoke();
  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_list_runs") {
      return Promise.resolve([
        {
          run_id: "run-1",
          scenario_id: "e2e.local",
          run_mode: "manual",
          status: "completed",
          started_at: 1000,
          finished_at: 3000,
          config_snapshot: {},
          environment_snapshot: {
            os_type: "windows",
            cpu_brand: "Intel",
            cpu_cores: 16,
            memory_gb: 32,
            gpu_info: "NVIDIA",
            available_captures: [],
            available_encoders: [],
            available_decoders: [],
            available_renderers: [],
            available_memory_modes: [],
          },
          summary: {
            total_duration_ms: 2000,
            capture_fps: 144,
            dropped_frames: 0,
            frame_count: 288,
          },
        },
      ]);
    }

    if (command === "test_get_run_telemetry") {
      return Promise.resolve({
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
        },
        events: [{ stage: "capture", status: "started", timestamp: 1000 }],
        logs: [],
        artifacts: [],
        diagnostics: { corrupt_rows: 0, warnings: [] },
      });
    }

    return Promise.resolve(null);
  });
  return mockInvoke;
}

describe("TestHistoryPage telemetry actions", () => {
  beforeEach(() => {
    mockNavigate.mockReset();
  });

  it("opens telemetry in a modal without navigating away", async () => {
    mockHistoryCommands();
    const user = userEvent.setup();

    render(<TestHistoryPage />);

    await screen.findByText("e2e.local");
    await user.click(screen.getByRole("button", { name: /曲线/ }));

    const dialog = await screen.findByRole("dialog", { name: "测试曲线" });
    expect(within(dialog).getByText(/run-1/)).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("expands telemetry inline under the selected history row", async () => {
    mockHistoryCommands();
    const user = userEvent.setup();

    render(<TestHistoryPage />);

    await screen.findByText("e2e.local");
    await user.click(screen.getByRole("button", { name: /展开/ }));

    expect(await screen.findByLabelText("测试曲线")).toBeInTheDocument();
    expect(screen.getByText("Capture FPS")).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});
