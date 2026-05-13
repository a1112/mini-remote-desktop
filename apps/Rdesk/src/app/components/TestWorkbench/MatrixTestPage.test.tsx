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

function setCheckbox(checkbox: HTMLElement, checked: boolean) {
  const input = checkbox as HTMLInputElement;
  if (input.checked !== checked) {
    fireEvent.click(input);
  }
}

function setLabeledCheckbox(label: string | RegExp, checked: boolean) {
  setCheckbox(screen.getByLabelText(label), checked);
}

function resultRow() {
  return screen.getAllByRole("row")[1]!;
}

function mockMacCapabilities(command: string) {
  if (command === "test_get_capabilities") {
    return Promise.resolve({
      os_type: "macos",
      cpu_brand: "",
      cpu_cores: 8,
      memory_gb: 32,
      gpu_info: "",
      available_captures: ["macos", "synthetic"],
      available_encoders: ["videotoolbox_h264", "openh264"],
      available_decoders: ["software"],
      available_renderers: ["none", "macos"],
      available_memory_modes: ["cpu"],
    });
  }
  return undefined;
}

function mockLinuxCapabilities(command: string) {
  if (command === "test_get_capabilities") {
    return Promise.resolve({
      os_type: "linux",
      cpu_brand: "",
      cpu_cores: 12,
      memory_gb: 32,
      gpu_info: "Mesa",
      available_captures: ["linux", "synthetic"],
      available_encoders: ["openh264"],
      available_decoders: ["software"],
      available_renderers: ["none", "linux"],
      available_memory_modes: ["cpu"],
    });
  }
  return undefined;
}

function windowsCapabilities(overrides: Record<string, unknown> = {}) {
  return {
    os_type: "windows",
    cpu_brand: "",
    cpu_cores: 16,
    memory_gb: 32,
    gpu_info: "NVIDIA",
    available_captures: ["dxgi", "winrt", "synthetic"],
    available_encoders: ["nvenc_h264", "openh264"],
    available_decoders: ["nvdec", "software"],
    available_renderers: ["none", "d3d11"],
    available_memory_modes: ["cpu", "d3d11_shared"],
    ...overrides,
  };
}

describe("MatrixTestPage failure handling", () => {
  it("exposes HEVC encoders when Windows capabilities report NVENC HEVC support", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "",
          cpu_cores: 16,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["nvenc_h264", "nvenc_hevc", "nvenc_hevc_main10", "nvenc_av1", "openh264"],
          available_decoders: ["nvdec", "software"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("NVENC HEVC Main")).toBeInTheDocument();
    expect(screen.getByLabelText("NVENC HEVC Main10")).toBeInTheDocument();
  });

  it("passes HEVC Main10 D3D11 shared texture matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "",
          cpu_cores: 16,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["dxgi", "synthetic"],
          available_encoders: ["nvenc_h264", "nvenc_hevc_main10", "openh264"],
          available_decoders: ["nvdec", "software"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("NVENC HEVC Main10");
    fireEvent.click(screen.getByLabelText("OpenH264"));
    fireEvent.click(screen.getByLabelText("NVENC H.264"));
    fireEvent.click(screen.getByLabelText("NVENC HEVC Main10"));
    fireEvent.click(screen.getByLabelText("软件"));
    fireEvent.click(screen.getByLabelText("CPU"));
    fireEvent.click(screen.getByLabelText("D3D11 shared texture"));
    fireEvent.click(screen.getByLabelText("Loopback"));
    fireEvent.click(screen.getByLabelText("QUIC Datagram"));
    fireEvent.click(screen.getByLabelText("720p"));
    fireEvent.click(screen.getByLabelText("30 FPS"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "nvenc_hevc_main10",
            decoder_type: "nvdec",
            transport_kind: "quic",
            renderer_type: "d3d11",
            render_display: true,
            zero_copy: true,
            visual_preview: false,
          }),
        })
      );
    });
  });

  it("loads macOS-specific matrix dimensions from environment capabilities", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacCapabilities(command);
      if (macCapabilities) return macCapabilities;
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    await waitFor(() => {
      expect(screen.getByLabelText("macOS")).toBeInTheDocument();
      expect(screen.getByLabelText("VideoToolbox H.264")).toBeInTheDocument();
      expect(screen.getByLabelText("Metal")).toBeInTheDocument();
    });

    expect(screen.queryByLabelText("DXGI")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("NVENC H.264")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("DX11 popup")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("D3D11 shared texture")).not.toBeInTheDocument();
  });

  it("loads Linux-specific matrix dimensions from environment capabilities", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const linuxCapabilities = mockLinuxCapabilities(command);
      if (linuxCapabilities) return linuxCapabilities;
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    await waitFor(() => {
      expect(screen.getAllByLabelText("Linux")).toHaveLength(2);
      expect(screen.getByLabelText("OpenH264")).toBeInTheDocument();
      expect(screen.getByLabelText("软件")).toBeInTheDocument();
    });

    expect(screen.queryByLabelText("DXGI")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("NVENC H.264")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("DX11 popup")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("D3D11 shared texture")).not.toBeInTheDocument();
  });

  it("prefers Linux hardware decode over software fallback in the default matrix", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["synthetic"],
          available_encoders: ["nvenc_h264", "openh264"],
          available_decoders: ["linux_h264", "software"],
          available_renderers: ["none"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-linux-hw");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("Linux H.264 HW");
    expect(screen.getByLabelText("Linux H.264 HW")).toBeChecked();
    expect(screen.getByLabelText("软件")).not.toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "nvenc_h264",
            decoder_type: "linux_h264",
          }),
        })
      );
    });
  });

  it("runs a local UI debug matrix with synthetic capture on Linux", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const linuxCapabilities = mockLinuxCapabilities(command);
      if (linuxCapabilities) return linuxCapabilities;
      if (command === "test_start_run") {
        return Promise.resolve("run-local-ui");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    fireEvent.click(await screen.findByRole("button", { name: /本地 UI 调试矩阵/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "synthetic",
            encoder_type: "openh264",
            transport_kind: "loopback",
            duration_ms: 3000,
          }),
        })
      );
    });
  });

  it("accepts the macOS OpenH264 CPU fallback performance tier", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacCapabilities(command);
      if (macCapabilities) return macCapabilities;
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
            capture_fps: 19,
            total_latency_p95: 58,
            dropped_frames: 0,
            frame_count: 95,
          },
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("macOS");
    fireEvent.click(screen.getByLabelText("VideoToolbox H.264"));
    fireEvent.click(screen.getByLabelText("720p"));
    fireEvent.click(screen.getByLabelText("30 FPS"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("完成")).toBeInTheDocument();
      expect(within(resultRow()).getByText("19.0")).toBeInTheDocument();
    });
  });

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

  it("lets an in-flight matrix run be stopped from the UI", async () => {
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
          status: "running",
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
          summary: undefined,
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={10_000} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke.mock.calls.some(([command]) => command === "test_get_run")).toBe(true);
    });

    fireEvent.click(screen.getByRole("button", { name: /停止/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-1" });
    });
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
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

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

  it("passes the macOS Metal renderer flag and disables no-display rows", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacCapabilities(command);
      if (macCapabilities) return macCapabilities;
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("Metal");
    fireEvent.click(screen.getByLabelText("OpenH264"));
    fireEvent.click(screen.getByLabelText("720p"));
    fireEvent.click(screen.getByLabelText("30 FPS"));
    fireEvent.click(screen.getByLabelText("Metal"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            renderer_type: "macos",
            render_display: true,
          }),
        })
      );
    });

    const startCalls = mockInvoke.mock.calls.filter(([command]) => command === "test_start_run");
    expect(startCalls).toHaveLength(1);
    expect(startCalls[0]?.[1]).toEqual(
      expect.objectContaining({
        config: expect.not.objectContaining({
          render_display: false,
        }),
      })
    );
  });

  it("passes the Linux renderer flag for Linux matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const linuxCapabilities = mockLinuxCapabilities(command);
      if (linuxCapabilities) return linuxCapabilities;
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    const linuxOptions = await screen.findAllByLabelText("Linux");
    setCheckbox(linuxOptions[0]!, true);
    setCheckbox(linuxOptions[1]!, true);
    setLabeledCheckbox("Synthetic", false);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "openh264",
            decoder_type: "software",
            renderer_type: "linux",
            render_display: true,
            zero_copy: false,
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

  it("skips OpenH264 D3D11 shared texture rows without calling the backend", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("D3D11 shared texture");
    setLabeledCheckbox("NVENC H.264", false);
    setLabeledCheckbox("OpenH264", true);
    setLabeledCheckbox("NVDEC", false);
    setLabeledCheckbox("软件", true);
    setLabeledCheckbox("720p", true);
    setLabeledCheckbox("1080p", false);
    setLabeledCheckbox("30 FPS", true);
    setLabeledCheckbox("60 FPS", false);
    setLabeledCheckbox("CPU", false);
    setLabeledCheckbox("D3D11 shared texture", true);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("跳过")).toBeInTheDocument();
      expect(
        within(resultRow()).getByText(/OpenH264 requires CPU-backed input/)
      ).toBeInTheDocument();
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === "test_start_run")).toBe(false);
  });

  it("skips DX12 native renderer rows as unimplemented without calling the backend", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_renderers: ["none", "d3d12"],
            available_memory_modes: ["cpu"],
          })
        );
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("DX12 native");
    fireEvent.click(screen.getByLabelText("OpenH264"));
    fireEvent.click(screen.getByLabelText("软件"));
    fireEvent.click(screen.getByLabelText("1080p"));
    fireEvent.click(screen.getByLabelText("60 FPS"));
    fireEvent.click(screen.getByLabelText("No display"));
    fireEvent.click(screen.getByLabelText("DX12 native"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("跳过")).toBeInTheDocument();
      expect(
        within(resultRow()).getByText(/D3D12 native renderer is probe-only/)
      ).toBeInTheDocument();
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === "test_start_run")).toBe(false);
  });

  it("allows WinRT D3D11 shared texture matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "",
          cpu_cores: 16,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["nvenc_h264", "openh264"],
          available_decoders: ["nvdec", "software"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    await screen.findByLabelText("WinRT");
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("DXGI"));
    fireEvent.click(screen.getByLabelText("WinRT"));
    fireEvent.click(screen.getByLabelText("CPU"));
    fireEvent.click(screen.getByLabelText("D3D11 shared texture"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "winrt",
            renderer_type: "d3d11",
            render_display: true,
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("allows NVENC AV1 with D3D11 shared texture matrix runs", async () => {
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
    fireEvent.click(screen.getByLabelText("NVENC H.264"));
    fireEvent.click(screen.getByLabelText(/NVENC AV1/));
    fireEvent.click(screen.getByLabelText("CPU"));
    fireEvent.click(screen.getByLabelText("D3D11 shared texture"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "nvenc_av1",
            decoder_type: "nvdec",
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

  it("runs a cross-device LAN matrix against the selected discovered peer", async () => {
    const mockInvoke = getMockInvoke();
    const peer = {
      device_id: "linux-agent",
      device_name: "Linux Agent",
      device_type: "desktop",
      ip: "192.168.1.50",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.50:21116",
      transports: [
        "quic",
        "quic_datagram",
        "quic_datagram_2k144",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      age_ms: 120,
      p2p_available: true,
    };
    const source = {
      id: "display-1",
      platform: "linux",
      source_kind: "display",
      title: "Linux Display",
      class_name: "display",
      width: 1920,
      height: 1080,
      process_id: 0,
    };

    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") return Promise.resolve(windowsCapabilities());
      if (command === "ipc_capability_snapshot") return Promise.resolve(null);
      if (command === "ipc_refresh_lan_discovery") {
        return Promise.resolve({
          enabled: true,
          running: true,
          discovery_port: 21116,
          instance_id: "controller",
          last_probe_ms: 10,
          peers: [peer],
        });
      }
      if (command === "service_bootstrap_if_needed") return Promise.resolve(false);
      if (command === "service_wait_for_healthy") return Promise.resolve(true);
      if (command === "ipc_runtime_snapshot") {
        return Promise.resolve({
          sessions: [],
          device_id: "controller-device",
          is_registered: true,
        });
      }
      if (command === "ipc_start_lan_remote_session") {
        return Promise.resolve(args?.sessionId);
      }
      if (command === "ipc_list_remote_capture_sources") return Promise.resolve([source]);
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({ session_id: args?.sessionId, source, status: "selected" });
      }
      if (command === "ipc_start_receiver") return Promise.resolve(args?.sessionId);
      if (command === "open_remote_display_window") {
        return Promise.resolve({
          label: "remote-display",
          session_id: args?.sessionId,
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: true,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          frames_received: 90,
          frames_decoded: 88,
          frames_dropped: 2,
          current_fps: 60,
          bitrate_mbps: 8,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 8,
        });
      }
      if (command === "ipc_stop_session") return Promise.resolve(args?.sessionId);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    expect(screen.getByLabelText("执行范围")).toHaveValue("local");
    fireEvent.change(screen.getByLabelText("执行范围"), {
      target: { value: "cross-device" },
    });

    await screen.findByRole("option", { name: "Linux Agent (192.168.1.50)" });
    fireEvent.change(screen.getByLabelText("跨设备目标设备"), {
      target: { value: "linux-agent" },
    });
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("5 Mbps"));
    fireEvent.click(screen.getByLabelText("8 Mbps"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_start_lan_remote_session",
        expect.objectContaining({
          targetDeviceId: "linux-agent",
          transportKind: "quic",
          requestedProfile: {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 8,
            codec: "h264",
          },
        })
      );
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
  });

  it("skips cross-device transport rows when the selected peer does not support them", async () => {
    const mockInvoke = getMockInvoke();
    const peer = {
      device_id: "linux-agent",
      device_name: "Linux Agent",
      device_type: "desktop",
      ip: "192.168.1.50",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.50:21116",
      transports: [
        "quic",
        "quic_datagram",
        "quic_datagram_2k144",
        "media_profile_control_v1",
      ],
      protocol_version: 1,
      age_ms: 120,
      p2p_available: true,
    };

    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") return Promise.resolve(windowsCapabilities());
      if (command === "ipc_capability_snapshot") return Promise.resolve(null);
      if (command === "ipc_refresh_lan_discovery") {
        return Promise.resolve({
          enabled: true,
          running: true,
          discovery_port: 21116,
          instance_id: "controller",
          last_probe_ms: 10,
          peers: [peer],
        });
      }
      if (command === "service_bootstrap_if_needed") return Promise.resolve(false);
      if (command === "service_wait_for_healthy") return Promise.resolve(true);
      if (command === "ipc_runtime_snapshot") {
        return Promise.resolve({
          sessions: [],
          device_id: "controller-device",
          is_registered: true,
        });
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    fireEvent.change(screen.getByLabelText("执行范围"), {
      target: { value: "cross-device" },
    });

    await screen.findByRole("option", { name: "Linux Agent (192.168.1.50)" });
    fireEvent.change(screen.getByLabelText("跨设备目标设备"), {
      target: { value: "linux-agent" },
    });
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByLabelText("Loopback"));
    fireEvent.click(screen.getByLabelText("WebRTC RTP"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await screen.findByText(/LAN peer does not support webrtc/);
    expect(resultRow()).toHaveTextContent("跳过");
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "ipc_start_lan_remote_session",
      expect.anything()
    );
  });

  it("skips cross-device media profile mismatch rows instead of failing the matrix", async () => {
    const mockInvoke = getMockInvoke();
    const peer = {
      device_id: "linux-agent",
      device_name: "Linux Agent",
      device_type: "desktop",
      ip: "192.168.1.50",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.50:21116",
      transports: [
        "quic",
        "quic_datagram",
        "quic_datagram_2k144",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      age_ms: 120,
      p2p_available: true,
    };
    const source = {
      id: "display-1",
      platform: "linux",
      source_kind: "display",
      title: "Linux Display",
      class_name: "display",
      width: 1728,
      height: 1080,
      process_id: 0,
    };

    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") return Promise.resolve(windowsCapabilities());
      if (command === "ipc_capability_snapshot") return Promise.resolve(null);
      if (command === "ipc_refresh_lan_discovery") {
        return Promise.resolve({
          enabled: true,
          running: true,
          discovery_port: 21116,
          instance_id: "controller",
          last_probe_ms: 10,
          peers: [peer],
        });
      }
      if (command === "service_bootstrap_if_needed") return Promise.resolve(false);
      if (command === "service_wait_for_healthy") return Promise.resolve(true);
      if (command === "ipc_runtime_snapshot") {
        return Promise.resolve({
          sessions: [],
          device_id: "controller-device",
          is_registered: true,
        });
      }
      if (command === "ipc_start_lan_remote_session") return Promise.resolve(args?.sessionId);
      if (command === "ipc_list_remote_capture_sources") return Promise.resolve([source]);
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({ session_id: args?.sessionId, source, status: "selected" });
      }
      if (command === "ipc_start_receiver") return Promise.resolve(args?.sessionId);
      if (command === "open_remote_display_window") {
        return Promise.resolve({
          label: "remote-display",
          session_id: args?.sessionId,
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: true,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          frames_received: 90,
          frames_decoded: 88,
          frames_dropped: 2,
          current_fps: 60,
          bitrate_mbps: 5,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1728,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 5,
        });
      }
      if (command === "ipc_stop_session") return Promise.resolve(args?.sessionId);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    fireEvent.change(screen.getByLabelText("执行范围"), {
      target: { value: "cross-device" },
    });

    await screen.findByRole("option", { name: "Linux Agent (192.168.1.50)" });
    fireEvent.change(screen.getByLabelText("跨设备目标设备"), {
      target: { value: "linux-agent" },
    });
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await screen.findByText(/Runtime media profile mismatch/);
    expect(resultRow()).toHaveTextContent("跳过");
  });
});
