import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { RenderTestPage } from "./RenderTestPage";

describe("RenderTestPage platform capabilities", () => {
  function mockWindowsRenderCapabilities() {
    return {
      os_type: "windows",
      cpu_brand: "test",
      cpu_cores: 16,
      memory_gb: 32,
      gpu_info: "NVIDIA",
      available_captures: ["dxgi", "winrt", "synthetic"],
      available_encoders: ["none", "openh264"],
      available_decoders: ["none", "software"],
      available_renderers: ["none", "d3d11", "d3d12", "opengl", "webview"],
      available_memory_modes: ["cpu", "d3d11_shared"],
    };
  }

  function capabilityItem(id: string, domain: string, status: string, reason?: string) {
    return {
      id,
      domain,
      label: id,
      status,
      platform: "windows",
      reason: reason ?? null,
      detail: null,
      requires: [],
      conflicts_with: [],
      depends_on: [],
      fallback_ids: [],
      last_probe_time_ms: null,
    };
  }

  function serviceCapabilitySnapshot(capabilities: ReturnType<typeof capabilityItem>[]) {
    return {
      schema_version: 1,
      platform: "windows",
      service_version: "test",
      capabilities,
      constraints: [],
      profiles: [],
      recent_profile_results: [],
      updated_at_ms: 1,
    };
  }

  it("uses service renderer capability status instead of legacy renderer defaults", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(mockWindowsRenderCapabilities());
      }
      if (command === "ipc_capability_snapshot") {
        return Promise.resolve(
          serviceCapabilitySnapshot([
            capabilityItem("render.d3d11", "render", "available"),
            capabilityItem("render.d3d12_native", "render", "unimplemented", "probe-only"),
            capabilityItem("render.opengl", "render", "unknown", "probe pending"),
            capabilityItem("render.webview", "render", "degraded", "diagnostic fallback"),
          ])
        );
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    expect(await screen.findByRole("button", { name: /Direct3D 11/ })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /Direct3D 12/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /OpenGL/ })).not.toBeInTheDocument();
  });

  it("starts Direct3D 12 through the independent render probe without downgrading to D3D11", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(mockWindowsRenderCapabilities());
      }
      if (command === "test_start_run") return Promise.resolve("run-d3d12");
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const d3d12Button = await screen.findByRole("button", { name: /Direct3D 12/ });
    await waitFor(() => expect(d3d12Button).toBeEnabled());

    fireEvent.click(d3d12Button);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "render.probe",
          config: expect.objectContaining({
            renderer_type: "d3d12",
            capture_type: "synthetic",
            encoder_type: "none",
            decoder_type: "none",
            render_display: true,
          }),
        })
      )
    );
  });

  it("starts OpenGL through the independent render probe", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(mockWindowsRenderCapabilities());
      }
      if (command === "test_start_run") return Promise.resolve("run-opengl");
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const openglButton = await screen.findByRole("button", { name: /OpenGL/ });
    await waitFor(() => expect(openglButton).toBeEnabled());

    fireEvent.click(openglButton);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "render.probe",
          config: expect.objectContaining({
            renderer_type: "opengl",
            capture_type: "synthetic",
            encoder_type: "none",
            decoder_type: "none",
            render_display: true,
          }),
        })
      )
    );
  });

  it("runs WebView rendering inside the realtime preview without backend probe", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(mockWindowsRenderCapabilities());
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const webviewButton = await screen.findByRole("button", { name: /WebView/ });
    await waitFor(() => expect(webviewButton).toBeEnabled());

    fireEvent.click(webviewButton);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("WebView 实时动画")).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
  });

  it("waits for measured animation frames before reporting WebView FPS", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(mockWindowsRenderCapabilities());
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const webviewButton = await screen.findByRole("button", { name: /WebView/ });
    fireEvent.click(webviewButton);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("0.0 FPS")).toBeInTheDocument();
    expect(screen.getByText("WebView 实时动画")).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
  });

  it("enables Metal and disables Windows-only renderers on macOS", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "Apple GPU",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "openh264"],
          available_decoders: ["software", "none"],
          available_renderers: ["macos"],
          available_memory_modes: ["cpu"],
        });
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const metalButton = screen.getByRole("button", { name: /Metal/ });
    await waitFor(() => expect(metalButton).toBeEnabled());

    await waitFor(() => expect(metalButton).toHaveAttribute("aria-pressed", "true"));
    expect(screen.queryByRole("button", { name: /Direct3D 11/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Direct3D 12/ })).not.toBeInTheDocument();
    expect(screen.getByText("实时画面")).toBeInTheDocument();
    expect(screen.getByText("启动测试后显示渲染输入帧")).toBeInTheDocument();
  });

  it("uses Linux capture and the Linux renderer when running on Linux", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "test",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "Mesa",
          available_captures: ["linux", "synthetic"],
          available_encoders: ["openh264"],
          available_decoders: ["software", "none"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 48,
          capture_latency_p50_ms: 12,
          capture_latency_p95_ms: 18,
          encode_latency_p50_ms: 0,
          encode_latency_p95_ms: 0,
          transport_latency_p50_ms: 0,
          transport_latency_p95_ms: 0,
          decode_latency_p50_ms: 0,
          decode_latency_p95_ms: 0,
          total_latency_p50_ms: 14,
          total_latency_p95_ms: 21,
          frame_count: 45,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      if (command === "test_harness_get_frames") {
        return Promise.resolve([null, ["AAAA", 2, 2, 7]]);
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    const linuxButton = await screen.findByRole("button", { name: /Linux/ });
    await waitFor(() => expect(linuxButton).toBeEnabled());
    fireEvent.click(linuxButton);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "none",
            decoder_type: "none",
            renderer_type: "linux",
            render_display: true,
            input_source: "screen",
          }),
        })
      );
    });
  });

  it("uses direct macOS capture to Metal render without encode/decode", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "Apple GPU",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "openh264"],
          available_decoders: ["software", "none"],
          available_renderers: ["macos"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-1");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 55,
          capture_latency_p50_ms: 10,
          capture_latency_p95_ms: 18,
          encode_latency_p50_ms: 0,
          encode_latency_p95_ms: 0,
          transport_latency_p50_ms: 0,
          transport_latency_p95_ms: 0,
          decode_latency_p50_ms: 0,
          decode_latency_p95_ms: 0,
          total_latency_p50_ms: 15,
          total_latency_p95_ms: 20,
          frame_count: 30,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      if (command === "test_harness_get_frames") return Promise.resolve([null, null]);
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Metal/ })).toBeEnabled()
    );
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "macos",
            decoder_type: "none",
            encoder_type: "none",
            input_source: "screen",
            renderer_type: "macos",
            render_display: true,
          }),
        })
      )
    );
  });

  it("displays rendered preview frames while the native Metal run is active", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "Apple GPU",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "openh264"],
          available_decoders: ["software", "none"],
          available_renderers: ["macos"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-metal");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 60,
          capture_latency_p50_ms: 8,
          capture_latency_p95_ms: 12,
          encode_latency_p50_ms: 0,
          encode_latency_p95_ms: 0,
          transport_latency_p50_ms: 0,
          transport_latency_p95_ms: 0,
          decode_latency_p50_ms: 0,
          decode_latency_p95_ms: 0,
          total_latency_p50_ms: 11,
          total_latency_p95_ms: 16,
          frame_count: 60,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      if (command === "test_harness_get_frames") {
        return Promise.resolve([null, ["AAAA", 2, 2, 7]]);
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-metal",
          scenario_id: "custom",
          run_mode: "manual",
          status: "running",
          started_at: Date.now(),
          finished_at: null,
          config_snapshot: {},
          environment_snapshot: {},
          summary: null,
        });
      }
      return Promise.resolve(null);
    });

    render(<RenderTestPage />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Metal/ })).toBeEnabled()
    );
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    const image = await screen.findByRole("img", { name: "Render preview" });
    expect(image).toHaveAttribute("src", "data:image/png;base64,AAAA");
  });
});
