import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { TransportTestPage, crossDeviceConfigForPeer } from "./TransportTestPage";

describe("TransportTestPage execution targets", () => {
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

  it("uses service transport capability status instead of static transport defaults", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
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
      if (command === "ipc_capability_snapshot") {
        return Promise.resolve(
          serviceCapabilitySnapshot([
            capabilityItem("transport.quic", "transport", "available"),
            capabilityItem("transport.webrtc", "transport", "unsupported", "WebRTC media path disabled"),
            capabilityItem("capture.synthetic", "capture", "available"),
            capabilityItem("encode.openh264", "encode", "degraded"),
            capabilityItem("decode.software", "decode", "degraded"),
            capabilityItem("memory.cpu", "memory", "available"),
          ])
        );
      }
      return Promise.resolve(null);
    });

    render(<TransportTestPage />);

    expect(await screen.findByRole("button", { name: /QUIC/ })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /WebRTC/ })).not.toBeInTheDocument();
  });

  it("uses FFmpeg H.264 as the local decode fallback before software", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "",
          cpu_cores: 16,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["synthetic", "dxgi"],
          available_encoders: ["openh264", "nvenc_h264"],
          available_decoders: ["software", "ffmpeg_h264", "none"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "ipc_capability_snapshot") {
        return Promise.resolve(
          serviceCapabilitySnapshot([
            capabilityItem("transport.quic", "transport", "available"),
            capabilityItem("capture.synthetic", "capture", "available"),
            capabilityItem("encode.openh264", "encode", "degraded"),
            capabilityItem("decode.ffmpeg_h264", "decode", "available"),
            capabilityItem("decode.software", "decode", "degraded"),
            capabilityItem("memory.cpu", "memory", "available"),
          ])
        );
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-ffmpeg");
      }
      return Promise.resolve(null);
    });

    render(<TransportTestPage />);

    await screen.findByRole("button", { name: /QUIC/ });
    fireEvent.click(screen.getByRole("button", { name: "启动测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            decoder_type: "ffmpeg_h264",
          }),
        })
      );
    });
  });

  it("uses the local macOS VideoToolbox HEVC transport chain when available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "",
          cpu_cores: 8,
          memory_gb: 32,
          gpu_info: "Apple",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_hevc", "videotoolbox_h264", "openh264"],
          available_decoders: ["videotoolbox_hevc", "software", "none"],
          available_renderers: ["none", "macos"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "ipc_capability_snapshot") return Promise.resolve(null);
      if (command === "test_start_run") return Promise.resolve("run-macos-hevc");
      return Promise.resolve(null);
    });

    render(<TransportTestPage />);

    await screen.findByRole("button", { name: /QUIC/ });
    fireEvent.click(screen.getByRole("button", { name: "启动测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            capture_type: "macos",
            encoder_type: "videotoolbox_hevc",
            decoder_type: "videotoolbox",
            bitrate: 20_000_000,
          }),
        })
      );
    });
  });

  it("describes macOS cross-device transport records with VideoToolbox HEVC", () => {
    const peer = {
      device_id: "mac-agent",
      device_name: "Mac Agent",
      device_type: "desktop",
      ip: "192.168.1.52",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.52:21116",
      transports: [
        "quic",
        "quic_datagram",
        "quic_datagram_2k144",
        "quic_datagram_media_v3",
        "media_profile_control_v1",
      ],
      protocol_version: 1,
      media_protocol_version: 3,
      media_capabilities: [
        "macos_capture",
        "videotoolbox_hevc",
        "decode.videotoolbox_hevc",
        "media.hevc_main_420_8bit",
        "macos_native_render",
      ],
      age_ms: 40,
      p2p_available: true,
    };

    expect(
      crossDeviceConfigForPeer(
        peer,
        {
          width: 1280,
          height: 720,
          fps: 30,
          bitrate_mbps: 20,
          codec: "hevc",
          codec_profile: "main",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        },
        "quic"
      )
    ).toMatchObject({
      capture_type: "macos",
      encoder_type: "videotoolbox_hevc",
      decoder_type: "videotoolbox",
      renderer_type: "macos",
      zero_copy: false,
      transport_kind: "quic",
      resolution: [1280, 720],
      fps: 30,
      bitrate: 20_000_000,
    });
  });

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
        "quic_datagram_media_v3",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      media_protocol_version: 3,
      media_capabilities: [
        "pipewire_capture",
        "software_decode",
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
    let decodedFrames = 58;

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
      if (command === "ipc_list_remote_display_modes") {
        return Promise.resolve([
          {
            id: "display-1:1280x720@30",
            source_id: "display-1",
            width: 1280,
            height: 720,
            refresh_hz: 30,
            bit_depth: 32,
            is_current: true,
          },
        ]);
      }
      if (command === "ipc_set_remote_display_mode") {
        return Promise.resolve({
          session_id: args?.sessionId,
          requested: args?.mode,
          previous: null,
          active: args?.mode,
          status: "changed",
          reason: null,
          restore_required: true,
        });
      }
      if (command === "ipc_restore_remote_display_mode") {
        return Promise.resolve({
          session_id: args?.sessionId,
          requested: null,
          previous: null,
          active: null,
          status: "restored",
          reason: null,
          restore_required: false,
        });
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
        decodedFrames += 30;
        return Promise.resolve({
          session_id: args?.sessionId,
          frames_received: decodedFrames + 2,
          frames_decoded: decodedFrames,
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
          attached_surfaces: [
            {
              surface_id: "surface-1",
              backend: "linux",
              window_handle: null,
            },
          ],
          active_decoder: "software",
          active_renderer: "linux_native",
          active_codec: "h264",
          active_pixel_format: "cpu_rgb24",
          active_width: 1280,
          active_height: 720,
          active_fps: 30,
          active_bitrate_mbps: 5,
          queue_depth: 0,
          dropped_frames: 2,
          stage_metrics: [
            { stage: "decode", p50_ms: 0.8, p95_ms: 1.4 },
            { stage: "present", p50_ms: 4.0, p95_ms: 7.0 },
          ],
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({ status: "selected" });
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
            bitrate_mbps: 20,
            codec: "hevc",
            codec_profile: "main",
            bit_depth: 8,
            chroma_subsampling: "4:2:0",
            pixel_format: "nv12",
            hdr_enabled: false,
          },
        })
      );
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
    expect(await screen.findByText("118", {}, { timeout: 3_000 })).toBeInTheDocument();
  });
});
