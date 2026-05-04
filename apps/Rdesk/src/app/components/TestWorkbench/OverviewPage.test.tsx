import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { OverviewPage } from "./OverviewPage";

const mockNavigate = vi.hoisted(() => vi.fn());

vi.mock("react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router")>();
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  };
});

function mockOverviewData() {
  const mockInvoke = getMockInvoke();

  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_list_scenarios") {
      return Promise.resolve([]);
    }

    if (command === "test_list_runs") {
      return Promise.resolve([
        {
          run_id: "run-1",
          scenario_id: "matrix",
          run_mode: "matrix",
          status: "completed",
          started_at: Date.now(),
          config_snapshot: {},
          environment_snapshot: {
            cpu_brand: "Apple M",
            cpu_cores: 8,
            memory_gb: 32,
            gpu_info: "Apple GPU",
            available_encoders: [],
            available_decoders: [],
          },
          summary: {
            total_duration_ms: 1000,
            capture_fps: 30,
            frame_count: 30,
            dropped_frames: 0,
          },
        },
      ]);
    }

    if (command === "test_get_capabilities") {
      return Promise.resolve({
        os_type: "windows",
        cpu_brand: "Intel",
        cpu_cores: 16,
        memory_gb: 32,
        gpu_info: "NVIDIA RTX",
        available_captures: ["dxgi", "winrt", "synthetic"],
        available_encoders: ["nvenc_h264", "openh264"],
        available_decoders: ["nvdec", "software"],
        available_renderers: ["d3d11", "webview"],
        available_memory_modes: ["cpu", "d3d11_shared"],
      });
    }

    return Promise.resolve(null);
  });
}

describe("OverviewPage", () => {
  beforeEach(() => {
    mockNavigate.mockReset();
  });

  it("navigates from quick entry buttons and recent run detail", async () => {
    mockOverviewData();
    const user = userEvent.setup();

    render(<OverviewPage />);

    await screen.findByRole("button", { name: /端到端本地测试/ });

    await user.click(screen.getByRole("button", { name: /端到端本地测试/ }));
    expect(mockNavigate).toHaveBeenLastCalledWith("/test/e2e");

    await user.click(screen.getByRole("button", { name: /矩阵性能测试/ }));
    expect(mockNavigate).toHaveBeenLastCalledWith("/test/matrix");

    await user.click(screen.getByRole("button", { name: /自由组合测试/ }));
    expect(mockNavigate).toHaveBeenLastCalledWith("/test/custom");

    await waitFor(() => {
      expect(screen.getByText("matrix")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "查看详情" }));
    expect(mockNavigate).toHaveBeenLastCalledWith("/test/run/run-1");
  });

  it("shows structured capability domains, statuses, and 2K144 readiness", async () => {
    mockOverviewData();

    render(<OverviewPage />);

    expect(await screen.findByText("结构化能力矩阵")).toBeInTheDocument();
    expect(screen.getByText("capture")).toBeInTheDocument();
    expect(screen.getAllByText("available").length).toBeGreaterThan(0);
    expect(screen.getAllByText("degraded").length).toBeGreaterThan(0);
    expect(screen.getAllByText("unimplemented").length).toBeGreaterThan(0);
    expect(screen.getByText("lan.2k144")).toBeInTheDocument();
    expect(screen.getByText("blocked")).toBeInTheDocument();
    expect(screen.getAllByText(/transport.media_profile_control_v1/).length).toBeGreaterThan(0);
  });
});
