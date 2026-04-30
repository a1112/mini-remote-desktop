import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";
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

describe("RemoteDisplayWindowPage", () => {
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

  it("does not start the local test harness for LAN remote sessions", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
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

    fireEvent.click(screen.getByRole("button", { name: /开始接收|开始测试/ }));

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
});
