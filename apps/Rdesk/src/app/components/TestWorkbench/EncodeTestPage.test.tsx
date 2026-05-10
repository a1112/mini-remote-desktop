import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { EncodeTestPage } from "./EncodeTestPage";

describe("EncodeTestPage backend contract", () => {
  it("starts OpenH264 with CPU-backed synthetic capture and zero-copy disabled", async () => {
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
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-openh264");
      return Promise.resolve(null);
    });

    render(<EncodeTestPage />);

    fireEvent.click(await screen.findByRole("button", { name: /OpenH264/ }));
    expect(screen.getByText(/CPU-backed/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "encode.openh264",
        config: expect.objectContaining({
          capture_type: "synthetic",
          encoder_type: "openh264",
          decoder_type: "none",
          zero_copy: false,
        }),
      });
    });
  });

  it("shows encoded unit throughput instead of capture loop throughput", async () => {
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
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-openh264");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 48.5,
          encoded_fps: 14,
          encode_latency_p50_ms: 55,
          encode_latency_p95_ms: 67.19,
          frame_count: 120,
          encoded_units: 42,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      return Promise.resolve(null);
    });

    render(<EncodeTestPage />);

    fireEvent.click(await screen.findByRole("button", { name: /OpenH264/ }));
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("14.0 FPS")).toBeInTheDocument();
    expect(screen.getByText("编码帧数")).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.queryByText("48.5 FPS")).not.toBeInTheDocument();
    expect(screen.queryByText("120")).not.toBeInTheDocument();
  });

  it("starts Linux NVENC through the custom PipeWire capture path", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "NVIDIA",
          available_captures: ["linux", "synthetic"],
          available_encoders: ["none", "nvenc_h264", "nvenc_hevc", "openh264"],
          available_decoders: ["none", "software"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux-nvenc");
      return Promise.resolve(null);
    });

    render(<EncodeTestPage />);

    expect(await screen.findByText(/PipeWire\/Linux capture/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "custom",
        config: expect.objectContaining({
          capture_type: "linux",
          encoder_type: "nvenc_h264",
          decoder_type: "none",
          zero_copy: false,
        }),
      });
    });
  });
});
