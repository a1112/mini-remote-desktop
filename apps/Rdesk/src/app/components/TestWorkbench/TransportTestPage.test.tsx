import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { TransportTestPage } from "./TransportTestPage";

describe("TransportTestPage execution targets", () => {
  it("runs a cross-device transport test against the selected discovered peer", async () => {
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
        "quic_datagram_media_v2",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      media_protocol_version: 2,
      media_capabilities: [
        "dxgi_capture",
        "nvenc_h264",
        "nvdec",
        "d3d11_native_render",
      ],
      age_ms: 80,
      p2p_available: true,
    };
    const source = {
      id: "display-1",
      platform: "linux",
      source_kind: "display",
      title: "Linux Display",
      class_name: "display",
      width: 1280,
      height: 720,
      process_id: 0,
    };

    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "",
          cpu_cores: 16,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["synthetic", "dxgi"],
          available_encoders: ["openh264", "nvenc_h264"],
          available_decoders: ["software", "nvdec", "none"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
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
          current_fps: 30,
          bitrate_mbps: 5,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1280,
          media_probe_height: 720,
          media_probe_target_fps: 30,
          media_probe_target_bitrate_mbps: 5,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          attached_surfaces: [],
          active_decoder: "nvdec",
          active_renderer: "d3d11_native",
          queue_depth: 0,
          dropped_frames: 2,
          stage_metrics: [
            { stage: "decode", p50_ms: 0.8, p95_ms: 1.4 },
            { stage: "present", p50_ms: 4.0, p95_ms: 7.0 },
          ],
        });
      }
      if (command === "ipc_stop_session") return Promise.resolve(args?.sessionId);
      return Promise.resolve(null);
    });

    render(<TransportTestPage />);

    expect(screen.getByLabelText("执行范围")).toHaveValue("local");
    fireEvent.change(screen.getByLabelText("执行范围"), {
      target: { value: "cross-device" },
    });

    await screen.findByRole("option", { name: "Linux Agent (192.168.1.50)" });
    fireEvent.change(screen.getByLabelText("跨设备目标设备"), {
      target: { value: "linux-agent" },
    });
    fireEvent.click(screen.getByRole("button", { name: "启动测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_start_lan_remote_session",
        expect.objectContaining({
          targetDeviceId: "linux-agent",
          transportKind: "quic",
          requestedProfile: {
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_mbps: 5,
            codec: "h264",
          },
        })
      );
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
    expect(await screen.findByText("88")).toBeInTheDocument();
  });
});
