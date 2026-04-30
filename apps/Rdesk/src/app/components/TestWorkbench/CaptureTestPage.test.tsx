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

    await screen.findByText("Single window capture");
    fireEvent.click(screen.getByRole("button", { name: /Choose window/ }));

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
