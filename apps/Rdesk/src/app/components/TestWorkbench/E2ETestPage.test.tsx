import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { E2ETestPage } from "./E2ETestPage";

describe("E2ETestPage LAN automation", () => {
  it("runs LAN remote display automation through IPC commands", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter>
        <E2ETestPage />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole("button", { name: /开始 LAN E2E/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_start_lan_remote_session",
        expect.objectContaining({
          targetDeviceId: "agent-device",
          transportKind: "quic",
        })
      );
    });
    expect(mockInvoke).toHaveBeenCalledWith("ipc_start_receiver", expect.any(Object));
    expect(mockInvoke).toHaveBeenCalledWith(
      "open_remote_display_window",
      expect.objectContaining({
        sessionId: expect.stringMatching(/^lan-e2e-agent-device-/),
      })
    );
    expect(await screen.findByText(/LAN E2E 完成/)).toBeInTheDocument();
    expect(screen.getAllByText(/Agent PC/).length).toBeGreaterThan(0);
    expect(screen.getByText(/decoded 4/)).toBeInTheDocument();
  });

  it("autoruns LAN remote display automation from URL query parameters", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter
        initialEntries={[
          "/test/e2e?autorun=lan-e2e&targetDeviceId=agent-device&transport=quic&timeoutMs=2500&minDecodedFrames=2&minFps=5",
        ]}
      >
        <E2ETestPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_start_lan_remote_session",
        expect.objectContaining({
          targetDeviceId: "agent-device",
          transportKind: "quic",
        })
      );
    });
    expect(await screen.findByText(/LAN E2E 完成/)).toBeInTheDocument();
    expect(screen.getByText(/decoded 4/)).toBeInTheDocument();
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "automation_write_report",
        expect.objectContaining({
          report: expect.objectContaining({
            status: "completed",
            scenarioId: "lan.e2e.remote_display",
          }),
        })
      );
    });
  });
});

function installSuccessfulLanAutomationMock() {
  const mockInvoke = getMockInvoke();
  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_get_capabilities") {
      return Promise.resolve({
        os_type: "windows",
        cpu_brand: "test",
        cpu_cores: 8,
        memory_gb: 16,
        gpu_info: "NVIDIA",
        available_captures: ["dxgi", "winrt", "synthetic"],
        available_encoders: ["nvenc_h264", "openh264"],
        available_decoders: ["nvdec", "software"],
        available_renderers: ["d3d11"],
        available_memory_modes: ["cpu", "d3d11_shared"],
      });
    }
    if (command === "service_bootstrap_if_needed") return Promise.resolve(true);
    if (command === "service_wait_for_healthy") return Promise.resolve(true);
    if (command === "ipc_runtime_snapshot") {
      return Promise.resolve({
        device_id: "controller-device",
        is_registered: true,
        sessions: [],
      });
    }
    if (command === "ipc_refresh_lan_discovery") {
      return Promise.resolve({
        enabled: true,
        running: true,
        discovery_port: 37777,
        instance_id: "controller-instance",
        peers: [
          {
            device_id: "agent-device",
            device_name: "Agent PC",
            device_type: "desktop",
            ip: "192.168.1.24",
            discovery_port: 37777,
            p2p_control_addr: "192.168.1.24:37778",
            transports: ["quic"],
            protocol_version: 1,
            age_ms: 25,
            p2p_available: true,
          },
        ],
      });
    }
    if (command === "ipc_start_lan_remote_session") return Promise.resolve("started");
    if (command === "ipc_start_receiver") return Promise.resolve("receiver-started");
    if (command === "open_remote_display_window") {
      return Promise.resolve({
        label: "remote-display-1",
        session_id: "lan-e2e-agent-device-1000",
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
        session_id: "lan-e2e-agent-device-1000",
        role: "controller",
        state: "streaming",
        transport_kind: "quic",
        sender_active: false,
        receiver_active: true,
      });
    }
    if (command === "ipc_probe_snapshot") {
      return Promise.resolve({
        session_id: "lan-e2e-agent-device-1000",
        frames_received: 5,
        frames_decoded: 4,
        frames_dropped: 0,
        current_fps: 20,
        bitrate_mbps: 8,
        last_error: null,
      });
    }
    if (command === "ipc_stop_session") return Promise.resolve("stopped");
    if (command === "automation_write_report") return Promise.resolve(null);
    return Promise.resolve(null);
  });

  return mockInvoke;
}
