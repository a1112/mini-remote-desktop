import { render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
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

vi.mock("recharts", () => {
  const Container = ({ children }: { children?: ReactNode }) => (
    <div data-testid="mock-recharts">{children}</div>
  );
  return {
    CartesianGrid: () => null,
    Legend: () => null,
    Line: () => null,
    LineChart: Container,
    ResponsiveContainer: Container,
    Tooltip: () => null,
    XAxis: () => null,
    YAxis: () => null,
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

    if (command === "ffmpeg_probe") {
      return Promise.resolve({
        available: true,
        ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
        ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
        ffmpeg_version: "ffmpeg version 8.1.1",
        ffprobe_version: "ffprobe version 8.1.1",
        reason: null,
      });
    }

    return Promise.resolve(null);
  });
}

function mockOverviewDataWithCapabilities(capabilities: Record<string, unknown>) {
  const mockInvoke = getMockInvoke();

  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_list_scenarios") {
      return Promise.resolve([]);
    }

    if (command === "test_list_runs") {
      return Promise.resolve([]);
    }

    if (command === "test_get_capabilities") {
      return Promise.resolve(capabilities);
    }

    if (command === "ffmpeg_probe") {
      return Promise.resolve({
        available: true,
        ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
        ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
        ffmpeg_version: "ffmpeg version 8.1.1",
        ffprobe_version: "ffprobe version 8.1.1",
        reason: null,
      });
    }

    return Promise.resolve(null);
  });
}

function mockOverviewDataWithActiveRun() {
  const mockInvoke = getMockInvoke();

  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_list_scenarios") {
      return Promise.resolve([]);
    }

    if (command === "test_list_runs") {
      return Promise.resolve([
        {
          run_id: "run-active",
          scenario_id: "matrix-live",
          run_mode: "matrix",
          status: "running",
          started_at: 1_000,
          config_snapshot: {},
          environment_snapshot: {
            cpu_brand: "Intel",
            cpu_cores: 16,
            memory_gb: 32,
            gpu_info: "NVIDIA RTX",
            available_encoders: ["nvenc_h264"],
            available_decoders: ["nvdec"],
          },
        },
      ]);
    }

    if (command === "test_get_run_metrics") {
      return Promise.resolve({
        capture_fps: {
          metric_name: "capture_fps",
          unit: "fps",
          samples: [
            { timestamp: 1_000, value: 120 },
            { timestamp: 2_000, value: 144 },
          ],
        },
        encode_latency_p95_ms: {
          metric_name: "encode_latency_p95_ms",
          unit: "ms",
          samples: [
            { timestamp: 1_000, value: 1.7 },
            { timestamp: 2_000, value: 2.4 },
          ],
        },
        decode_latency_p95_ms: {
          metric_name: "decode_latency_p95_ms",
          unit: "ms",
          samples: [
            { timestamp: 1_000, value: 0.9 },
            { timestamp: 2_000, value: 1.2 },
          ],
        },
        total_latency_p95_ms: {
          metric_name: "total_latency_p95_ms",
          unit: "ms",
          samples: [
            { timestamp: 1_000, value: 6.8 },
            { timestamp: 2_000, value: 7.4 },
          ],
        },
      });
    }

    if (command === "test_get_capabilities") {
      return Promise.resolve({
        os_type: "windows",
        cpu_brand: "Intel",
        cpu_cores: 16,
        memory_gb: 32,
        gpu_info: "NVIDIA RTX",
        available_captures: ["dxgi"],
        available_encoders: ["nvenc_h264"],
        available_decoders: ["nvdec"],
        available_renderers: ["d3d11"],
        available_memory_modes: ["d3d11_shared"],
      });
    }

    if (command === "ffmpeg_probe") {
      return Promise.resolve({
        available: true,
        ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
        ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
        ffmpeg_version: "ffmpeg version 8.1.1",
        ffprobe_version: "ffprobe version 8.1.1",
        reason: null,
      });
    }

    return Promise.resolve(null);
  });
}

describe("OverviewPage", () => {
  beforeEach(() => {
    mockNavigate.mockReset();
    window.localStorage?.clear();
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

  it("shows structured capability domains, statuses, and high-refresh LAN readiness", async () => {
    mockOverviewData();

    render(<OverviewPage />);

    expect(await screen.findByText("结构化能力矩阵")).toBeInTheDocument();
    expect(screen.getByText("capture")).toBeInTheDocument();
    expect(screen.getAllByText("available").length).toBeGreaterThan(0);
    expect(screen.getAllByText("degraded").length).toBeGreaterThan(0);
    expect(screen.queryByText("unimplemented")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: /显示不可用能力/ }));
    expect(screen.getAllByText("unimplemented").length).toBeGreaterThan(0);
    expect(screen.getByText("lan.2k144")).toBeInTheDocument();
    expect(screen.getByText("lan.1600p165")).toBeInTheDocument();
    expect(screen.getAllByText("blocked").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/transport.media_profile_control_v1/).length).toBeGreaterThan(0);
  });

  it("uses the macOS H.264 2K144 profile when VideoToolbox HEVC is unavailable", async () => {
    mockOverviewDataWithCapabilities({
      os_type: "macos",
      cpu_brand: "Apple M",
      cpu_cores: 8,
      memory_gb: 32,
      gpu_info: "Apple GPU",
      available_captures: ["macos", "synthetic"],
      available_encoders: ["videotoolbox_h264", "openh264"],
      available_decoders: ["videotoolbox_h264", "software"],
      available_renderers: ["macos", "webview"],
      available_memory_modes: ["cpu"],
    });

    render(<OverviewPage />);

    expect(await screen.findByText("lan.macos.2k144")).toBeInTheDocument();
    expect(screen.queryByText("lan.macos.hevc.2k144")).not.toBeInTheDocument();
  });

  it("shows FFmpeg optional tooling status and actions", async () => {
    mockOverviewData();
    const mockInvoke = getMockInvoke();
    const user = userEvent.setup();

    render(<OverviewPage />);

    expect(await screen.findByText("FFmpeg 可选工具")).toBeInTheDocument();
    expect(screen.getByText("ffmpeg version 8.1.1")).toBeInTheDocument();
    expect(screen.getByText("C:\\ffmpeg\\bin\\ffmpeg.exe")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /刷新 FFmpeg 状态/ }));
    expect(mockInvoke).toHaveBeenCalledWith("ffmpeg_probe", undefined);

    mockInvoke.mockImplementation((command: string) => {
      if (command === "ffmpeg_download") {
        return Promise.resolve({
          install_dir: "C:\\ffmpeg",
          archive_sha256: "a".repeat(64),
          probe: {
            available: true,
            ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
            ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
            ffmpeg_version: "ffmpeg version 8.1.1",
            ffprobe_version: "ffprobe version 8.1.1",
            reason: null,
          },
        });
      }
      if (command === "ffmpeg_reset_golden_settings") {
        return Promise.resolve({});
      }
      if (command === "ffmpeg_probe") {
        return Promise.resolve({
          available: true,
          ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
          ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
          ffmpeg_version: "ffmpeg version 8.1.1",
          ffprobe_version: "ffprobe version 8.1.1",
          reason: null,
        });
      }
      return Promise.resolve(null);
    });

    await user.click(screen.getByRole("button", { name: /下载或更新 FFmpeg/ }));
    expect(mockInvoke).toHaveBeenCalledWith("ffmpeg_download", undefined);

    await user.click(screen.getByRole("button", { name: /重置 FFmpeg 设置/ }));
    expect(mockInvoke).toHaveBeenCalledWith("ffmpeg_reset_golden_settings", undefined);
  });

  it("shows realtime curves for the active test run", async () => {
    mockOverviewDataWithActiveRun();

    render(<OverviewPage />);

    expect(await screen.findByText("当前测试实时曲线")).toBeInTheDocument();
    expect(screen.getAllByText("matrix-live").length).toBeGreaterThan(0);
    expect(screen.getAllByText("FPS").length).toBeGreaterThan(0);
    expect(screen.getByText("阶段 P95 延迟")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("144.0 FPS")).toBeInTheDocument();
      expect(screen.getByText("7.40 ms")).toBeInTheDocument();
    });

    expect(getMockInvoke()).toHaveBeenCalledWith("test_get_run_metrics", { runId: "run-active" });
  });
});
