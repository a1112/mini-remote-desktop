import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { CaptureTestPage } from "./CaptureTestPage";

const baseTargets = [
  {
    hwnd: "0x100",
    title: "Browser",
    class_name: "Chrome_WidgetWin_1",
    width: 1280,
    height: 720,
    process_id: 100,
  },
  {
    hwnd: "0x200",
    title: "Editor",
    class_name: "ApplicationFrameWindow",
    width: 1600,
    height: 900,
    process_id: 200,
  },
];

const previewTargets = baseTargets.map((target) => ({
  ...target,
  preview_data_url:
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lHbX9QAAAABJRU5ErkJggg==",
  preview_width: 1,
  preview_height: 1,
}));

describe("CaptureTestPage window picker", () => {
  it("starts DXGI desktop capture through the unthrottled zero-copy run path", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "test",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["none", "openh264"],
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 144,
          frame_count: 10,
          dropped_frames: 0,
          resolution: [2560, 1440],
          capture_latency_avg_ms: 6,
          total_latency_p50_ms: 6,
          capture_latency_p95_ms: 7,
          encode_latency_p95_ms: 0,
          decode_latency_p95_ms: 0,
          total_latency_p95_ms: 7,
        });
      }
      return Promise.resolve(null);
    });

    render(<CaptureTestPage />);

    const startButton = await screen.findByRole("button", { name: /启动测试/ });
    await waitFor(() => expect(startButton).not.toBeDisabled());
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "capture.dxgi",
        config: expect.objectContaining({
          capture_type: "dxgi",
          encoder_type: "none",
          decoder_type: "none",
          duration_ms: 30_000,
          input_source: "screen",
          zero_copy: true,
          visual_preview: false,
        }),
      });
    });
  });

  it("shows interactive P95 separately from source wait P95", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "test",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["none", "openh264"],
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 51.5,
          frame_count: 30,
          dropped_frames: 0,
          resolution: [1280, 720],
          capture_latency_avg_ms: 19.42,
          capture_latency_p95_ms: 24.2,
          source_wait_latency_p95_ms: 24.2,
          interactive_latency_avg_ms: 2.1,
          interactive_latency_p50_ms: 1.7,
          interactive_latency_p95_ms: 3.4,
          encode_latency_p95_ms: 0,
          decode_latency_p95_ms: 0,
          total_latency_p95_ms: 24.2,
        });
      }
      return Promise.resolve(null);
    });

    render(<CaptureTestPage />);

    const startButton = await screen.findByRole("button", { name: /启动测试/ });
    await waitFor(() => expect(startButton).not.toBeDisabled());
    fireEvent.click(startButton);

    expect(await screen.findByText("感知 P95")).toBeInTheDocument();
    expect(screen.getByText("源等待 P95")).toBeInTheDocument();
    expect(screen.getAllByText("3.40 ms").length).toBeGreaterThan(0);
    expect(screen.getAllByText("24.20 ms").length).toBeGreaterThan(0);
  });

  it("starts WinRT screen capture as a performance run unless window mode is selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "test",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["none", "openh264"],
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-winrt");
      }
      return Promise.resolve(null);
    });

    render(<CaptureTestPage />);

    const winrtButton = screen.getByRole("button", { name: /Windows Runtime Capture/ });
    await waitFor(() => expect(winrtButton).not.toBeDisabled());
    fireEvent.click(winrtButton);
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "capture.winrt",
        config: expect.objectContaining({
          capture_type: "winrt",
          encoder_type: "none",
          decoder_type: "none",
          duration_ms: 30_000,
          input_source: "screen",
          zero_copy: true,
          visual_preview: false,
        }),
      });
    });
    expect(mockInvoke).not.toHaveBeenCalledWith("test_list_window_capture_targets");
  });

  it("starts selected WinRT window capture as a continuous performance run", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "test",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["none", "openh264"],
          available_decoders: ["none", "software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "test_list_window_capture_targets") {
        return Promise.resolve(baseTargets);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-window-perf");
      }
      return Promise.resolve(null);
    });

    render(<CaptureTestPage />);

    const winrtButton = screen.getByRole("button", { name: /Windows Runtime Capture/ });
    await waitFor(() => expect(winrtButton).not.toBeDisabled());
    fireEvent.click(winrtButton);
    fireEvent.click(screen.getByRole("button", { name: /单窗口性能/ }));

    await screen.findByText("Browser");
    fireEvent.click(screen.getByRole("button", { name: /启动测试/ }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_start_run", {
        scenarioId: "custom",
        config: expect.objectContaining({
          capture_type: "winrt",
          encoder_type: "none",
          decoder_type: "none",
          duration_ms: 30_000,
          input_source: "window",
          window_hwnd: "0x100",
          window_title: "Browser",
          zero_copy: true,
          visual_preview: false,
        }),
      });
    });
  });

  it("opens an Alt-Tab style picker and selects a WinRT window target", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "test",
          available_captures: ["dxgi", "winrt", "synthetic"],
          available_encoders: ["openh264"],
          available_decoders: ["software"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_list_window_capture_targets") {
        return Promise.resolve(baseTargets);
      }
      if (command === "test_list_window_capture_targets_with_previews") {
        return Promise.resolve(previewTargets);
      }
      return Promise.resolve(null);
    });

    render(<CaptureTestPage />);
    const winrtButton = screen.getByRole("button", { name: /Windows Runtime Capture/ });
    await waitFor(() => expect(winrtButton).not.toBeDisabled());
    fireEvent.click(winrtButton);
    fireEvent.click(screen.getByRole("button", { name: /单窗口验证/ }));

    await screen.findByText("Browser");
    const chooseWindowButton = screen.getByRole("button", { name: /Choose window/ });
    await waitFor(() => expect(chooseWindowButton).not.toBeDisabled());
    fireEvent.click(chooseWindowButton);

    const dialog = await screen.findByRole("dialog", { name: /Window picker/ });
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_list_window_capture_targets_with_previews",
        { limit: 24 }
      );
    });

    expect(within(dialog).getByText("Browser")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: /Select Editor/ }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(screen.getByText("Editor")).toBeInTheDocument();
    expect(screen.getByText(/1600x900/)).toBeInTheDocument();
  });
});
