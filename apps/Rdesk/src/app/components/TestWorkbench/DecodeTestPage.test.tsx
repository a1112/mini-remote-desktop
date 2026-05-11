import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { DecodeTestPage } from "./DecodeTestPage";

function mockCapabilities() {
  return {
    os_type: "windows",
    cpu_brand: "test",
    cpu_cores: 16,
    memory_gb: 32,
    gpu_info: "NVIDIA",
    available_captures: ["dxgi", "synthetic"],
    available_encoders: ["none", "nvenc_h264", "openh264"],
    available_decoders: ["none", "software", "nvdec"],
    available_renderers: ["none", "d3d11"],
    available_memory_modes: ["cpu", "d3d11_shared"],
  };
}

describe("DecodeTestPage backend contract", () => {
  it("uses a Linux-compatible rendererless software decode path on Linux", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "test",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "Mesa",
          available_captures: ["synthetic", "linux"],
          available_encoders: ["openh264"],
          available_decoders: ["software"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux-software");
      return Promise.resolve(null);
    });

    render(<DecodeTestPage />);

    const startButton = await screen.findByRole("button", { name: /启动测试/ });
    await waitFor(() => expect(startButton).not.toBeDisabled());
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "synthetic",
            encoder_type: "openh264",
            decoder_type: "software",
            render_display: false,
          }),
        })
      );
    });

    const startCall = mockInvoke.mock.calls.find(([command]) => command === "test_start_run");
    expect(startCall?.[1]).toMatchObject({
      scenarioId: "custom",
      config: expect.objectContaining({
        capture_type: "synthetic",
        encoder_type: "openh264",
        decoder_type: "software",
        render_display: false,
      }),
    });
    expect((startCall?.[1] as { config?: { renderer_type?: string } } | undefined)?.config?.renderer_type).toBeUndefined();
  });

  it("prefers Linux hardware decode and NVENC when both are available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "linux",
          cpu_brand: "test",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "NVIDIA",
          available_captures: ["synthetic", "linux"],
          available_encoders: ["nvenc_h264", "openh264"],
          available_decoders: ["linux_h264", "software"],
          available_renderers: ["linux"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-linux-hw-decode");
      return Promise.resolve(null);
    });

    render(<DecodeTestPage />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^选择解码器 Linux H\.264 HW$/ })).toHaveClass(
        "border-primary"
      )
    );
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "synthetic",
            encoder_type: "nvenc_h264",
            decoder_type: "linux_h264",
            render_display: false,
          }),
        })
      );
    });
  });

  it("starts NVDEC with an explicit 2K 144Hz decode profile", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") return Promise.resolve(mockCapabilities());
      if (command === "test_start_run") return Promise.resolve("run-nvdec-2k144");
      return Promise.resolve(null);
    });

    render(<DecodeTestPage />);

    fireEvent.click(await screen.findByRole("button", { name: /^选择解码器 NVDEC$/ }));
    fireEvent.click(screen.getByRole("button", { name: /2K 144/ }));
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "custom",
        config: expect.objectContaining({
          capture_type: "dxgi",
          encoder_type: "nvenc_h264",
          decoder_type: "nvdec",
          resolution: [2560, 1440],
          fps: 144,
          zero_copy: true,
          visual_preview: false,
        }),
      });
    });
  });

  it("shows decoded throughput and decoded frames instead of capture loop counters", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") return Promise.resolve(mockCapabilities());
      if (command === "test_start_run") return Promise.resolve("run-software-decode");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 60,
          encoded_fps: 38,
          decoded_fps: 37,
          decode_latency_p50_ms: 8,
          decode_latency_p95_ms: 17,
          frame_count: 120,
          decoded_frames: 74,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      return Promise.resolve(null);
    });

    render(<DecodeTestPage />);

    fireEvent.click(await screen.findByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("37.0 FPS")).toBeInTheDocument();
    expect(screen.getByText("60.0 FPS")).toBeInTheDocument();
    expect(screen.getByText("38.0 FPS")).toBeInTheDocument();
    expect(screen.getAllByText("解码帧数").length).toBeGreaterThanOrEqual(2);
    expect(screen.getAllByText("74").length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText("120")).not.toBeInTheDocument();
  });

  it("classifies low FPS with tiny decode latency as upstream limited", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") return Promise.resolve(mockCapabilities());
      if (command === "test_start_run") return Promise.resolve("run-upstream-limited");
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 24.1,
          encoded_fps: 23.9,
          decoded_fps: 23.7,
          decode_latency_p50_ms: 0.08,
          decode_latency_p95_ms: 0.15,
          frame_count: 650,
          decoded_frames: 642,
          decode_failures: 0,
          dropped_frames: 0,
          resolution: [1920, 1080],
        });
      }
      return Promise.resolve(null);
    });

    render(<DecodeTestPage />);

    fireEvent.click(await screen.findByRole("button", { name: /启动测试/ }));

    expect(await screen.findByText("解码器余量充足，当前受上游限制")).toBeInTheDocument();
    expect(screen.getByText("23.7 FPS")).toBeInTheDocument();
    expect(screen.getAllByText("642").length).toBeGreaterThanOrEqual(2);
  });
});
