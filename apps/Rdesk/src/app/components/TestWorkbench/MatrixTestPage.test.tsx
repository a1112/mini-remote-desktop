import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { MatrixTestPage } from "./MatrixTestPage";

function selectSingleSupportedCombination() {
  fireEvent.click(screen.getByLabelText("OpenH264"));
  fireEvent.click(screen.getByLabelText("软件"));
  fireEvent.click(screen.getByLabelText("720p"));
  fireEvent.click(screen.getByLabelText("30 FPS"));
}

function resultRow() {
  return screen.getAllByRole("row")[1]!;
}

describe("MatrixTestPage failure handling", () => {
  it("marks a row failed when test_start_run rejects", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.reject(new Error("unsupported scenario"));
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
  });

  it("marks a row failed and stops the run when test_get_run rejects", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.reject(new Error("run missing"));
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
    expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-1" });
  });

  it("marks a row failed and stops the run when test_get_run returns null", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
    expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-1" });
  });

  it("marks a completed run failed when performance is below the matrix threshold", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-1",
          scenario_id: "matrix",
          run_mode: "matrix",
          status: "completed",
          started_at: Date.now(),
          config_snapshot: {},
          environment_snapshot: {
            cpu_brand: "",
            cpu_cores: 8,
            memory_gb: 32,
            gpu_info: "",
            available_encoders: [],
            available_decoders: [],
          },
          summary: {
            total_duration_ms: 5000,
            capture_fps: 10,
            total_latency_p95: 250,
            dropped_frames: 0,
            frame_count: 50,
          },
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button"));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
      expect(within(resultRow()).getByText("10.0")).toBeInTheDocument();
    });
  });

  it("accepts the slower software decoder performance tier", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-1",
          scenario_id: "matrix",
          run_mode: "matrix",
          status: "completed",
          started_at: Date.now(),
          config_snapshot: {},
          environment_snapshot: {
            cpu_brand: "",
            cpu_cores: 8,
            memory_gb: 32,
            gpu_info: "",
            available_encoders: [],
            available_decoders: [],
          },
          summary: {
            total_duration_ms: 5000,
            capture_fps: 29,
            total_latency_p95: 40,
            dropped_frames: 0,
            frame_count: 145,
          },
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("NVDEC"));
    fireEvent.click(screen.getByLabelText("软件"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("完成")).toBeInTheDocument();
      expect(within(resultRow()).getByText("29.0")).toBeInTheDocument();
    });
  });

  it("passes the DX11 renderer flag when render display is enabled", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("No display"));
    fireEvent.click(screen.getByLabelText("DX11 popup"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            renderer_type: "d3d11",
            render_display: true,
          }),
        })
      );
    });
  });

  it("auto-enables DX11 popup for D3D11 shared texture matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("CPU"));
    fireEvent.click(screen.getByLabelText("D3D11 shared texture"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            renderer_type: "d3d11",
            render_display: true,
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("passes selected transport matrix value", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("Loopback"));
    fireEvent.click(screen.getByLabelText("QUIC Datagram"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            transport_kind: "quic",
          }),
        })
      );
    });
  });

  it("passes selected bitrate and duration matrix values", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("5 Mbps"));
    fireEvent.click(screen.getByLabelText("8 Mbps"));
    fireEvent.click(screen.getByLabelText("5 秒"));
    fireEvent.click(screen.getByLabelText("10 秒"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            bitrate: 8000000,
            duration_ms: 10000,
          }),
        })
      );
    });
  });
});
