import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../test/mocks/tauri";
import { RemoteDisplayWindowPage } from "./RemoteDisplayWindowPage";

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
}));

vi.mock("../utils/tauriWindow", () => ({
  withTauriWindow: (fn: (appWindow: {
    isMaximized: () => Promise<boolean>;
    minimize: () => Promise<void>;
    close: () => Promise<void>;
    startDragging: () => Promise<void>;
    toggleMaximize: () => Promise<void>;
  }) => Promise<unknown> | unknown) =>
    fn({
      isMaximized: () => Promise.resolve(false),
      minimize: () => Promise.resolve(undefined),
      close: () => Promise.resolve(undefined),
      startDragging: () => Promise.resolve(undefined),
      toggleMaximize: () => Promise.resolve(undefined),
    }),
}));

function renderRemoteDisplay(sessionId = "p2p-quic-123") {
  render(
    <MemoryRouter initialEntries={[`/display/${sessionId}?surface=surface-1`]}>
      <Routes>
        <Route path="/display/:id" element={<RemoteDisplayWindowPage />} />
      </Routes>
    </MemoryRouter>
  );
}

function mockRenderAreaRect() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 56,
    left: 0,
    top: 56,
    right: 1280,
    bottom: 776,
    width: 1280,
    height: 720,
    toJSON: () => ({}),
  } as DOMRect);
}

function mockResizeObserver() {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
  );
}

function windowsCapabilities() {
  return {
    os_type: "windows",
    cpu_brand: "Intel",
    cpu_cores: 16,
    memory_gb: 32,
    gpu_info: "NVIDIA",
    available_captures: ["dxgi", "winrt", "synthetic"],
    available_encoders: ["nvenc_h264", "nvenc_hevc", "nvenc_hevc_main10", "nvenc_av1", "openh264"],
    available_decoders: ["nvdec", "software"],
    available_renderers: ["d3d11"],
    available_memory_modes: ["cpu", "d3d11_shared"],
  };
}

describe("RemoteDisplayWindowPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    getMockInvoke().mockReset();
    mockRenderAreaRect();
    mockResizeObserver();
  });

  it("allows switching back to Metal after selecting Web preview on macOS", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "Apple",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "Apple GPU",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "openh264"],
          available_decoders: ["videotoolbox", "software"],
          available_renderers: ["macos"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "macos" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    const webButton = await screen.findByRole("button", { name: "Web preview" });
    fireEvent.click(webButton);

    const metalButton = await screen.findByRole("button", { name: "Metal native" });
    await waitFor(() => expect(metalButton).toBeEnabled());
    fireEvent.click(metalButton);

    await waitFor(() => {
      expect(screen.getByText("render: Metal native")).toBeInTheDocument();
    });
  });

  it("probes the D3D11 native surface before starting a local pipeline test", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return Promise.resolve(null);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-1",
          status: "running",
          summary: null,
        });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "configure_remote_display_native_surface",
        expect.objectContaining({ enabled: true })
      );
    });
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "present_test_harness_frame_on_native_surface",
        undefined
      );
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            renderer_type: "d3d11",
            render_display: true,
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("starts the local native pipeline with AV1 zero-copy when selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return Promise.resolve(null);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-1", status: "running", summary: null });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.change(screen.getByLabelText("ENC"), { target: { value: "nvenc_av1" } });
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            encoder_type: "nvenc_av1",
            decoder_type: "nvdec",
            renderer_type: "d3d11",
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("starts the local native pipeline with HEVC Main10 zero-copy when selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return Promise.resolve(null);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-1", status: "running", summary: null });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.change(screen.getByLabelText("ENC"), { target: { value: "nvenc_hevc_main10" } });
    fireEvent.change(screen.getByLabelText("NET"), { target: { value: "quic" } });
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            encoder_type: "nvenc_hevc_main10",
            decoder_type: "nvdec",
            transport_kind: "quic",
            renderer_type: "d3d11",
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("does not start the local test harness for LAN remote sessions", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          current_fps: null,
          bitrate_mbps: null,
          last_error: null,
        });
      }
      if (command === "ipc_start_receiver") {
        return Promise.resolve("p2p-quic-123");
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "Start remote receiver" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_start_receiver", {
        sessionId: "p2p-quic-123",
      });
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
  });

  it("applies remote media profile changes through IPC negotiation", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({
          requested: {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264",
          },
          selected: {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "h264",
          },
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 20,
          last_error: null,
        });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_update_media_profile", {
        sessionId: "p2p-quic-123",
        requestedProfile: {
          width: 1920,
          height: 1080,
          fps: 144,
          bitrate_mbps: 20,
          codec: "h264",
        },
      });
    });
  });

  it("routes the title bar close button through the remote display cleanup command", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByTitle("Close"));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("close_remote_display_window", {
        label: "render-p2p-quic-123-1",
      });
    });
  });
});
