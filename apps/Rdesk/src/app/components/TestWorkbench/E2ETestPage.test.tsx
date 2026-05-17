import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { E2ETestPage } from "./E2ETestPage";

describe("E2ETestPage LAN automation", () => {
  it("starts the Linux local end-to-end scenario with Linux capture and render", async () => {
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
          available_decoders: ["software"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux-e2e");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(
      <MemoryRouter>
        <E2ETestPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getAllByText("linux").length).toBeGreaterThanOrEqual(2);
    });
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "e2e.linux_local",
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "openh264",
            decoder_type: "software",
            renderer_type: "linux",
            render_display: true,
          }),
        })
      );
    });
  });

  it("uses the Linux NVENC to hardware decode path when available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "test",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["linux", "synthetic"],
          available_encoders: ["nvenc_h264", "openh264"],
          available_decoders: ["linux_h264", "software"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux-hw-e2e");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(
      <MemoryRouter>
        <E2ETestPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText("nvenc_h264")).toBeInTheDocument();
      expect(screen.getByText("linux_h264")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "e2e.linux_local",
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "nvenc_h264",
            decoder_type: "linux_h264",
            renderer_type: "linux",
            render_display: true,
            zero_copy: undefined,
          }),
        })
      );
    });
  });

  it("runs LAN remote display automation through IPC commands", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter>
        <E2ETestPage />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole("button", { name: /开始跨设备 E2E/ }));

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
    expect(screen.getByText(/QUIC datagram decoded 25/)).toBeInTheDocument();
    expect(screen.getAllByText(/全屏 shared \/ DISPLAY1 \/ 2560x1440/).length).toBeGreaterThan(0);
  });

  it("runs the cross-device discovery scenario without starting a LAN session", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter>
        <E2ETestPage />
      </MemoryRouter>
    );

    fireEvent.change(await screen.findByLabelText("跨设备场景"), {
      target: { value: "cross.e2e.discovery" },
    });
    fireEvent.click(await screen.findByRole("button", { name: /开始跨设备 E2E/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "automation_write_report",
        expect.objectContaining({
          report: expect.objectContaining({
            status: "completed",
            scenarioId: "cross.e2e.discovery",
            dataPlaneVerified: false,
            mediaVerified: false,
          }),
        })
      );
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === "ipc_start_lan_remote_session")).toBe(false);
    expect(await screen.findByText(/LAN E2E 完成/)).toBeInTheDocument();
  });

  it("autoruns LAN remote display automation from URL query parameters", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter
        initialEntries={[
          "/test/e2e?autorun=lan-e2e&targetDeviceId=agent-device&transport=quic&timeoutMs=2500&minDecodedFrames=2&minFps=5&width=1920&height=1080&fps=180&bitrateMbps=20",
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
          requestedProfile: {
            width: 1920,
            height: 1080,
            fps: 180,
            bitrate_mbps: 20,
            codec: "h264",
          },
        })
      );
    });
    expect(await screen.findByText(/LAN E2E 完成/)).toBeInTheDocument();
    expect(screen.getByText(/QUIC datagram decoded 25/)).toBeInTheDocument();
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "automation_write_report",
        expect.objectContaining({
          report: expect.objectContaining({
            status: "completed",
            scenarioId: "cross.e2e.remote_display_smoke",
          }),
        })
      );
    });
  });

  it("passes an exact autorun capture source id through to LAN automation", async () => {
    const mockInvoke = installSuccessfulLanAutomationMock();

    render(
      <MemoryRouter
        initialEntries={[
          "/test/e2e?autorun=lan-e2e&targetDeviceId=agent-device&transport=quic&captureSourceId=window-codex&captureSourceKind=display_shared",
        ]}
      >
        <E2ETestPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_select_remote_capture_source",
        expect.objectContaining({
          sessionId: expect.stringMatching(/^lan-e2e-agent-device-/),
          sourceId: "window-codex",
        })
      );
    });
  });
});

function installSuccessfulLanAutomationMock() {
  const mockInvoke = getMockInvoke();
  let activeProfile = {
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 64,
    codec: "h264",
  };
  mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
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
    if (command === "get_hardware_info") {
      return Promise.resolve({
        motherboard_serial: "MB-LOCAL-1234",
        hostname: "Controller PC",
        os_type: "windows",
        os_version: "Windows",
        cpu_info: {
          name: "CPU",
          vendor_id: "GenuineIntel",
          cores: 8,
        },
        total_memory_mb: 16384,
        gpu_info: [],
      });
    }
    if (command === "ipc_register_device") return Promise.resolve("lan-MBLOCAL1234");
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
            transports: [
              "quic",
              "quic_datagram",
              "quic_datagram_2k144",
              "quic_datagram_media_v2",
              "media_profile_control_v1",
            ],
            protocol_version: 1,
            service_build_id: "test-build",
            media_protocol_version: 2,
            media_capabilities: [
              "dxgi_capture",
              "nvenc_h264",
              "nvdec",
              "d3d11_native_render",
            ],
            age_ms: 25,
            p2p_available: true,
          },
        ],
      });
    }
    if (command === "ipc_start_lan_remote_session") {
      const requestedProfile = args?.requestedProfile as typeof activeProfile | undefined;
      if (requestedProfile) {
        activeProfile = requestedProfile;
      }
      return Promise.resolve("started");
    }
    if (command === "ipc_list_remote_capture_sources") {
      return Promise.resolve([
        {
          id: "display-shared",
          platform: "windows",
          source_kind: "display_shared",
          title: "DISPLAY1",
          class_name: "Monitor",
          width: 2560,
          height: 1440,
          process_id: 0,
          app_name: null,
        },
        {
          id: "window-codex",
          platform: "windows",
          source_kind: "window",
          title: "Codex",
          class_name: "Chrome_WidgetWin_1",
          width: 1280,
          height: 720,
          process_id: 100,
          app_name: "Codex",
        },
      ]);
    }
    if (command === "ipc_select_remote_capture_source") {
      const sourceId = args?.sourceId as string | undefined;
      const selectedSource =
        sourceId === "window-codex"
          ? {
              id: "window-codex",
              platform: "windows",
              source_kind: "window",
              title: "Codex",
              class_name: "Chrome_WidgetWin_1",
              width: 1280,
              height: 720,
              process_id: 100,
              app_name: "Codex",
            }
          : {
              id: "display-shared",
              platform: "windows",
              source_kind: "display_shared",
              title: "DISPLAY1",
              class_name: "Monitor",
              width: 2560,
              height: 1440,
              process_id: 0,
              app_name: null,
            };
      return Promise.resolve({
        session_id: "lan-e2e-agent-device-1000",
        source: selectedSource,
        status: "selected",
        reason: null,
      });
    }
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
        frames_received: 25,
        frames_decoded: 25,
        frames_dropped: 0,
        current_fps: activeProfile.fps,
        bitrate_mbps: activeProfile.bitrate_mbps,
        media_probe_valid: true,
        media_probe_format: "compressed_h264_test_pattern",
        media_probe_width: activeProfile.width,
        media_probe_height: activeProfile.height,
        media_probe_target_fps: activeProfile.fps,
        media_probe_target_bitrate_mbps: activeProfile.bitrate_mbps,
        media_probe_payload_bytes: 55555,
        last_media_sequence: 25,
        last_media_timestamp_us: 123456,
        last_media_payload_hash: "fnv1a64:abc123",
        last_error: null,
      });
    }
    if (command === "ipc_media_pipeline_snapshot") {
      return Promise.resolve({
        session_id: "lan-e2e-agent-device-1000",
        attached_surfaces: [],
        active_decoder: "nvdec",
        active_renderer: "d3d11",
        queue_depth: 1,
        dropped_frames: 0,
        stage_metrics: [
          { stage: "decode", p50_ms: 0.8, p95_ms: 1.2 },
          { stage: "render_present", p50_ms: 5, p95_ms: 7 },
        ],
      });
    }
    if (command === "ipc_stop_session") return Promise.resolve("stopped");
    if (command === "automation_write_report") return Promise.resolve(null);
    return Promise.resolve(null);
  });

  return mockInvoke;
}
