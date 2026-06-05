import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import {
  MatrixTestPage,
  crossDevicePeerSkipReason,
  formatMatrixMediaProfile,
  mediaProfileFromConfig,
} from "./MatrixTestPage";

beforeAll(() => {
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: class {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    },
  });
});

function selectSingleSupportedCombination() {
  setLabeledCheckbox("NVENC HEVC Main", false);
  setLabeledCheckbox("NVENC HEVC Main10", false);
  setLabeledCheckbox(/NVENC AV1/, false);
  setLabeledCheckbox("OpenH264", false);
  if (screen.queryByLabelText("NVENC H.264")) {
    setLabeledCheckbox("NVENC H.264", true);
  } else if (screen.queryByLabelText("OpenH264")) {
    setLabeledCheckbox("OpenH264", true);
  } else if (screen.queryByLabelText("NVENC HEVC Main")) {
    setLabeledCheckbox("NVENC HEVC Main", true);
  }
  setLabeledCheckbox("NVDEC", true);
  setLabeledCheckbox("软件", false);
  setLabeledCheckbox("720p", false);
  setLabeledCheckbox("1080p", true);
  setLabeledCheckbox("30 FPS", false);
  setLabeledCheckbox("60 FPS", true);
}

function setCheckbox(checkbox: HTMLElement, checked: boolean) {
  const input = checkbox as HTMLInputElement;
  if (input.checked !== checked) {
    fireEvent.click(input);
  }
}

function setLabeledCheckbox(label: string | RegExp, checked: boolean) {
  const checkbox = screen.queryByLabelText(label);
  if (checkbox) setCheckbox(checkbox, checked);
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

function mockMacHevcCapabilities(command: string) {
  if (command === "test_get_capabilities") {
    return Promise.resolve({
      os_type: "macos",
      cpu_brand: "",
      cpu_cores: 8,
      memory_gb: 32,
      gpu_info: "",
      available_captures: ["macos", "synthetic"],
      available_encoders: ["videotoolbox_hevc", "videotoolbox_h264", "openh264"],
      available_decoders: ["videotoolbox_hevc", "software"],
      available_renderers: ["none", "macos"],
      available_memory_modes: ["cpu"],
    });
  }
  return undefined;
}

function mockMacHevcWithH264DecodeOnlyCapabilities(command: string) {
  if (command === "test_get_capabilities") {
    return Promise.resolve({
      os_type: "macos",
      cpu_brand: "",
      cpu_cores: 8,
      memory_gb: 32,
      gpu_info: "",
      available_captures: ["macos", "synthetic"],
      available_encoders: ["videotoolbox_hevc", "videotoolbox_h264", "openh264"],
      available_decoders: ["videotoolbox_h264", "software"],
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

describe("MatrixTestPage failure handling", () => {
  it("builds HEVC Main and Main10 media profiles from HEVC matrix encoders", () => {
    expect(
      mediaProfileFromConfig({
        encoder_type: "nvenc_hevc",
        resolution: [2560, 1440],
        fps: 144,
        bitrate: 80_000_000,
      })
    ).toEqual({
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 80,
      codec: "hevc",
      codec_profile: "main",
      bit_depth: 8,
      chroma_subsampling: "4:2:0",
      pixel_format: "nv12",
      hdr_enabled: false,
    });

    expect(
      mediaProfileFromConfig({
        encoder_type: "nvenc_hevc_main10",
        resolution: [3840, 2160],
        fps: 120,
        bitrate: 120_000_000,
      })
    ).toEqual({
      width: 3840,
      height: 2160,
      fps: 120,
      bitrate_mbps: 120,
      codec: "hevc",
      codec_profile: "main10",
      bit_depth: 10,
      chroma_subsampling: "4:2:0",
      pixel_format: "p010",
      hdr_enabled: false,
    });

    expect(
      mediaProfileFromConfig({
        encoder_type: "videotoolbox_hevc",
        resolution: [2560, 1440],
        fps: 144,
        bitrate: 40_000_000,
      })
    ).toEqual({
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 40,
      codec: "hevc",
      codec_profile: "main",
      bit_depth: 8,
      chroma_subsampling: "4:2:0",
      pixel_format: "nv12",
      hdr_enabled: false,
    });
  });

  it("requires HEVC peer media capabilities for HEVC cross-device matrix profiles", () => {
    const peer = {
      device_id: "windows-peer",
      device_name: "Windows Peer",
      device_type: "desktop",
      ip: "192.168.1.51",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.51:21116",
      transports: [
        "quic",
        "quic_datagram",
        "quic_datagram_2k144",
        "quic_datagram_media_v3",
        "media_profile_control_v1",
      ],
      protocol_version: 1,
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [
        "dxgi_capture",
        "nvenc_h264",
        "nvdec",
        "d3d11_native_render",
      ],
      age_ms: 20,
      p2p_available: true,
    };

    expect(
      crossDevicePeerSkipReason(peer, "quic", {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc",
        codec_profile: "main",
        bit_depth: 8,
        chroma_subsampling: "4:2:0",
        pixel_format: "nv12",
        hdr_enabled: false,
      })
    ).toMatch(/nvenc_hevc.*nvdec_hevc.*media\.hevc_main_420_8bit/);
  });

  it("accepts macOS VideoToolbox media capabilities for HEVC cross-device matrix profiles", () => {
    const peer = {
      device_id: "mac-peer",
      device_name: "Mac Peer",
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
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [
        "quic_datagram_media_v3",
        "macos_capture",
        "videotoolbox_hevc",
        "decode.videotoolbox_hevc",
        "media.hevc_main_420_8bit",
        "macos_native_render",
      ],
      age_ms: 20,
      p2p_available: true,
    };

    expect(
      crossDevicePeerSkipReason(peer, "quic", {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 40,
        codec: "hevc",
        codec_profile: "main",
        bit_depth: 8,
        chroma_subsampling: "4:2:0",
        pixel_format: "nv12",
        hdr_enabled: false,
      })
    ).toBeNull();
  });

  it("rejects generic VideoToolbox decode for HEVC cross-device matrix profiles", () => {
    const peer = {
      device_id: "mac-peer",
      device_name: "Mac Peer",
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
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [
        "quic_datagram_media_v3",
        "macos_capture",
        "videotoolbox_hevc",
        "decode.videotoolbox",
        "media.hevc_main_420_8bit",
        "macos_native_render",
      ],
      age_ms: 20,
      p2p_available: true,
    };

    const skipReason = crossDevicePeerSkipReason(peer, "quic", {
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 40,
      codec: "hevc",
      codec_profile: "main",
      bit_depth: 8,
      chroma_subsampling: "4:2:0",
      pixel_format: "nv12",
      hdr_enabled: false,
    });

    expect(skipReason).toContain("decode.videotoolbox_hevc");
  });

  it("formats matrix media profiles with HEVC codec and chroma metadata", () => {
    expect(
      formatMatrixMediaProfile({
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 80,
        codec: "hevc",
        codec_profile: "main",
        bit_depth: 8,
        chroma_subsampling: "4:2:0",
        pixel_format: "nv12",
        hdr_enabled: false,
      })
    ).toBe("hevc/main/8-bit/4:2:0/nv12 2560x1440@144/80Mbps");
  });

  it("uses service capability status instead of legacy environment defaults", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["nvenc_h264", "openh264"],
            available_decoders: ["nvdec", "software"],
            available_renderers: ["none", "d3d11"],
            available_memory_modes: ["cpu", "d3d11_shared"],
          })
        );
      }
      if (command === "ipc_capability_snapshot") {
        return Promise.resolve(
          serviceCapabilitySnapshot([
            capabilityItem("capture.synthetic", "capture", "available"),
            capabilityItem("encode.nvenc_h264", "encode", "hardware_missing", "NVENC probe failed"),
            capabilityItem("encode.openh264", "encode", "degraded", "software fallback"),
            capabilityItem("decode.software", "decode", "degraded", "software fallback"),
            capabilityItem("render.webview", "render", "degraded", "diagnostic fallback"),
            capabilityItem("memory.cpu", "memory", "available"),
            capabilityItem("transport.loopback", "transport", "available"),
          ])
        );
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("Synthetic")).toBeInTheDocument();
    expect(screen.queryByLabelText("NVENC H.264")).not.toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /OpenH264/ })).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: /软件/ })).not.toBeChecked();
  });

  it("exposes 180 and 249 FPS high-refresh matrix options", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("180 FPS")).toBeInTheDocument();
    expect(screen.getByLabelText("249 FPS")).toBeInTheDocument();
  });

  it("removes non-target resolution presets from the matrix", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation(() => Promise.resolve(null));

    render(<MatrixTestPage />);

    expect(screen.queryByLabelText("768p")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("900p")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("1200p")).not.toBeInTheDocument();
  });

  it("disables loopback when the execution scope is cross-device", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation(() => Promise.resolve(null));

    render(<MatrixTestPage runDelayMs={0} />);

    fireEvent.change(screen.getByLabelText("执行范围"), {
      target: { value: "cross-device" },
    });

    await waitFor(() => {
      const loopback = screen.getByLabelText("Loopback") as HTMLInputElement;
      expect(loopback).toBeDisabled();
      expect(loopback).not.toBeChecked();
    });
    expect(screen.getByText("仅本机")).toBeInTheDocument();
    expect(screen.getByText(/Loopback 仅支持本机进程内测试/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /启动矩阵测试/ })).toBeDisabled();
  });

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

  it("exposes FFmpeg decoders from capabilities and sends FFmpeg H.264 matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["openh264"],
            available_decoders: ["software", "ffmpeg_h264", "ffmpeg_hevc"],
            available_renderers: ["none"],
            available_memory_modes: ["cpu"],
          })
        );
      }
      if (command === "test_start_run") return Promise.resolve("run-1");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    expect(await screen.findByLabelText("FFmpeg H.264")).toBeInTheDocument();
    expect(screen.getByLabelText("FFmpeg HEVC")).toBeInTheDocument();
    setLabeledCheckbox("软件", false);
    setLabeledCheckbox("FFmpeg H.264", true);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "openh264",
            decoder_type: "ffmpeg_h264",
          }),
        })
      );
    });
  });

  it("prefers FFmpeg over software decode by default when NVDEC is unavailable", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["openh264"],
            available_decoders: ["software", "ffmpeg_h264"],
            available_renderers: ["none"],
            available_memory_modes: ["cpu"],
          })
        );
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("FFmpeg H.264")).toBeChecked();
    expect(screen.getByLabelText("软件")).not.toBeChecked();
  });

  it("prefers NVDEC over software and FFmpeg decode by default when NVDEC is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["nvenc_h264", "openh264"],
            available_decoders: ["nvdec", "software", "ffmpeg_h264"],
            available_renderers: ["none", "d3d11"],
            available_memory_modes: ["cpu"],
          })
        );
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("NVDEC")).toBeChecked();
    expect(screen.getByLabelText("FFmpeg H.264")).not.toBeChecked();
    expect(screen.getByLabelText("软件")).not.toBeChecked();
  });

  it("prefers software decode by default when macOS HEVC encode lacks a matching VideoToolbox decoder", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacHevcWithH264DecodeOnlyCapabilities(command);
      if (macCapabilities) return macCapabilities;
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    expect(await screen.findByLabelText("VideoToolbox HEVC")).toBeChecked();
    expect(screen.getByLabelText("VideoToolbox")).not.toBeChecked();
    expect(screen.getByLabelText("软件")).toBeChecked();
  });

  it("skips macOS HEVC VideoToolbox decode rows when only the H.264 decoder is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacHevcWithH264DecodeOnlyCapabilities(command);
      if (macCapabilities) return macCapabilities;
      if (command === "test_start_run") return Promise.resolve("run-should-not-start");
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    setCheckbox(await screen.findByLabelText("VideoToolbox"), true);
    setLabeledCheckbox("软件", false);
    setLabeledCheckbox("720p", false);
    setLabeledCheckbox("30 FPS", false);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(
        screen.getByText("VideoToolbox HEVC decoder is not available for this macOS capability snapshot")
      ).toBeInTheDocument();
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === "test_start_run")).toBe(false);
  });

  it("allows HEVC Main matrix runs over WebRTC RTP when decoder is compatible", async () => {
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
          available_encoders: ["nvenc_h264", "nvenc_hevc", "openh264"],
          available_decoders: ["nvdec", "software"],
          available_renderers: ["none", "d3d11"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-1");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("NVENC HEVC Main");
    setLabeledCheckbox("OpenH264", false);
    setLabeledCheckbox("NVENC H.264", false);
    setLabeledCheckbox("NVENC HEVC Main", true);
    setLabeledCheckbox("软件", false);
    setLabeledCheckbox("Loopback", false);
    setLabeledCheckbox("WebRTC RTP", true);
    setLabeledCheckbox("720p", false);
    setLabeledCheckbox("30 FPS", false);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "nvenc_hevc",
            decoder_type: "nvdec",
            transport_kind: "webrtc",
          }),
        })
      );
    });
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

  it("includes color mode and pipeline in D3D11 shared matrix runs", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["nvenc_h264", "nvenc_hevc_main10"],
            available_decoders: ["nvdec"],
            available_renderers: ["none", "d3d11"],
            available_memory_modes: ["cpu", "d3d11_shared"],
          })
        );
      }
      if (command === "test_start_run") return Promise.resolve("run-color");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("Monochrome");
    setLabeledCheckbox("NVENC H.264", false);
    setLabeledCheckbox("NVENC HEVC Main10", true);
    setLabeledCheckbox("Full color", false);
    setLabeledCheckbox("Monochrome", true);
    setLabeledCheckbox("SDR 8-bit", false);
    setLabeledCheckbox("HDR Main10", true);
    setLabeledCheckbox("CPU", false);
    setLabeledCheckbox("D3D11 shared texture", true);
    setLabeledCheckbox("Loopback", false);
    setLabeledCheckbox("QUIC Datagram", true);
    setLabeledCheckbox("720p", false);
    setLabeledCheckbox("30 FPS", false);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            encoder_type: "nvenc_hevc_main10",
            decoder_type: "nvdec",
            renderer_type: "d3d11",
            render_display: true,
            zero_copy: true,
            color_mode: "monochrome",
            color_pipeline: "hdr_main10",
          }),
        })
      );
    });
  });

  it("skips non-full color modes when D3D11 shared memory is not selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(
          windowsCapabilities({
            available_encoders: ["nvenc_h264"],
            available_decoders: ["nvdec"],
            available_renderers: ["none", "d3d11"],
            available_memory_modes: ["cpu", "d3d11_shared"],
          })
        );
      }
      if (command === "test_start_run") return Promise.resolve("run-color-cpu");
      if (command === "test_get_run") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);

    await screen.findByLabelText("Grayscale");
    setLabeledCheckbox("Full color", false);
    setLabeledCheckbox("Grayscale", true);
    setLabeledCheckbox("D3D11 shared texture", false);
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      const rowText = resultRow().textContent ?? "";
      expect(rowText).toContain("GPU color modes require D3D11 shared texture memory");
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.objectContaining({
        config: expect.objectContaining({ color_mode: "grayscale" }),
      })
    );
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

  it("defaults macOS matrix runs to VideoToolbox HEVC encode and decode when available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      const macCapabilities = mockMacHevcCapabilities(command);
      if (macCapabilities) return macCapabilities;
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);

    await screen.findByLabelText("VideoToolbox HEVC");
    expect(screen.getByLabelText("VideoToolbox HEVC")).toBeChecked();
    expect(screen.getByLabelText("VideoToolbox H.264")).not.toBeChecked();
    expect(screen.getByLabelText("OpenH264")).not.toBeChecked();
    expect(screen.getByLabelText("VideoToolbox")).toBeChecked();
    expect(screen.getByLabelText("软件")).not.toBeChecked();
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
    fireEvent.click(screen.getByLabelText("OpenH264"));
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
        "quic_datagram_media_v2",
        "quic_datagram_media_v3",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [
        "quic_datagram_media_v3",
        "dxgi_capture",
        "nvenc_h264",
        "nvdec",
        "d3d11_native_render",
      ],
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
    fireEvent.click(screen.getByLabelText("QUIC Datagram"));
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

  it("passes dynamic resolution into cross-device adaptive matrix runs", async () => {
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
        "quic_datagram_media_v3",
        "media_profile_control_v1",
        "capture_source_control_v1",
      ],
      protocol_version: 1,
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [
        "quic_datagram_media_v3",
        "dxgi_capture",
        "nvenc_h264",
        "nvdec",
        "d3d11_native_render",
      ],
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
      if (command === "ipc_start_lan_remote_session") return Promise.resolve(args?.sessionId);
      if (command === "ipc_list_remote_capture_sources") return Promise.resolve([source]);
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({ session_id: args?.sessionId, source, status: "selected" });
      }
      if (command === "ipc_list_remote_display_modes") return Promise.resolve([]);
      if (command === "ipc_configure_media_adaptation") {
        const currentProfile = {
          width: 1920,
          height: 1080,
          fps: 60,
          bitrate_mbps: 8,
          codec: "h264",
        };
        return Promise.resolve({
          enabled: true,
          state: "steady",
          ladder_index: 0,
          current_profile: currentProfile,
          target_profile: currentProfile,
          last_reason: null,
          last_change_ms: 0,
          observed_fps: 60,
          drop_ratio: 0,
          queue_depth: 0,
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
          media_probe_payload_bytes: 1024,
          last_media_sequence: 90,
          last_media_timestamp_us: 1_000_000,
          last_media_payload_hash: "fnv1a64:test",
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") return Promise.resolve(null);
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
    fireEvent.click(screen.getByLabelText("QUIC Datagram"));
    fireEvent.click(screen.getByLabelText("关键帧阶梯"));
    fireEvent.click(screen.getByLabelText("降采样"));
    fireEvent.click(screen.getByLabelText("5 Mbps"));
    fireEvent.click(screen.getByLabelText("8 Mbps"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "ipc_configure_media_adaptation",
        expect.objectContaining({
          config: expect.objectContaining({
            enabled: true,
            dynamic_resolution_enabled: true,
          }),
        })
      );
    });
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
        "quic_datagram_media_v2",
        "media_profile_control_v1",
        "capture_source_control_v1",
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
      if (command === "ipc_list_remote_display_modes") {
        return Promise.resolve([
          {
            id: "display-1:1728x1080@60",
            source_id: "display-1",
            width: 1728,
            height: 1080,
            refresh_hz: 60,
            bit_depth: 8,
            is_current: true,
          },
        ]);
      }
      if (command === "ipc_set_remote_display_mode") {
        return Promise.resolve({
          session_id: args?.sessionId,
          requested: args?.mode,
          previous: args?.mode,
          active: args?.mode,
          status: "changed",
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
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: args?.sessionId,
          attached_surfaces: [
            {
              surface_id: "surface-1",
              backend: "d3d11",
              window_handle: 1,
            },
          ],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          stage_metrics: [],
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
    fireEvent.click(screen.getByLabelText("QUIC Datagram"));
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await screen.findByText(/Runtime media profile downgraded/, undefined, {
      timeout: 3000,
    });
    expect(resultRow()).toHaveTextContent("跳过");
  });
});
