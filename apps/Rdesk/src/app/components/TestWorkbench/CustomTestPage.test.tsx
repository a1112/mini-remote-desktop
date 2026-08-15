import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { CustomTestPage } from "./CustomTestPage";

function capabilitySnapshot(capabilities: Array<{
  id: string;
  domain: string;
  label: string;
  status: string;
  reason?: string;
}>) {
  return {
    schema_version: 1,
    platform: "windows",
    service_version: "test",
    capabilities: capabilities.map((capability) => ({
      platform: "windows",
      ...capability,
    })),
    constraints: [],
    profiles: [],
    updated_at_ms: 1,
  };
}

describe("CustomTestPage platform capabilities", () => {
  it("uses service capability status instead of legacy custom test defaults", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "NVIDIA",
          available_captures: ["dxgi", "synthetic"],
          available_encoders: ["nvenc_h264", "openh264"],
          available_decoders: ["nvdec", "software", "none"],
          available_renderers: ["d3d11"],
          available_memory_modes: ["cpu", "d3d11_shared"],
        });
      }
      if (command === "ipc_capability_snapshot") {
        return Promise.resolve(
          capabilitySnapshot([
            {
              id: "capture.dxgi",
              domain: "capture",
              label: "DXGI",
              status: "driver_missing",
              reason: "DXGI probe failed",
            },
            {
              id: "capture.synthetic",
              domain: "capture",
              label: "Synthetic",
              status: "available",
            },
            {
              id: "encode.nvenc_h264",
              domain: "encode",
              label: "NVENC H.264",
              status: "driver_missing",
              reason: "NVENC probe failed",
            },
            {
              id: "encode.openh264",
              domain: "encode",
              label: "OpenH264",
              status: "degraded",
            },
            {
              id: "decode.nvdec",
              domain: "decode",
              label: "NVDEC",
              status: "driver_missing",
              reason: "NVDEC probe failed",
            },
            {
              id: "decode.software",
              domain: "decode",
              label: "Software",
              status: "degraded",
            },
            {
              id: "render.d3d11",
              domain: "render",
              label: "D3D11",
              status: "driver_missing",
              reason: "D3D11 probe failed",
            },
          ])
        );
      }
      return Promise.resolve(null);
    });

    render(
      <MemoryRouter>
        <CustomTestPage />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.queryByRole("radio", { name: /DXGI/ })).not.toBeInTheDocument();
    });
    expect(screen.queryByRole("radio", { name: /NVENC H\.264/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: /NVDEC/ })).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Synthetic/ })).toBeEnabled();
    expect(screen.getByRole("radio", { name: /OpenH264/ })).toBeEnabled();
  });

  it("allows HEVC custom tests to select WebRTC RTP transport", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "windows",
          cpu_brand: "test",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "NVIDIA",
          available_captures: ["dxgi", "synthetic"],
          available_encoders: ["nvenc_h264", "nvenc_hevc", "openh264"],
          available_decoders: ["nvdec", "software", "none"],
          available_renderers: ["d3d11", "none"],
          available_memory_modes: ["cpu"],
        });
      }
      return Promise.resolve(null);
    });

    render(
      <MemoryRouter>
        <CustomTestPage />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole("radio", { name: /NVENC HEVC Main/ }));
    const webrtc = screen.getByRole("radio", { name: /WebRTC RTP/ });

    expect(webrtc).toBeEnabled();
    expect(screen.queryByText("HEVC 未接入")).not.toBeInTheDocument();
  });

  it("blocks VideoToolbox HEVC custom runs without HEVC decode capability", async () => {
    const user = userEvent.setup();
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "test",
          cpu_cores: 12,
          memory_gb: 32,
          gpu_info: "Apple",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "videotoolbox_hevc", "openh264"],
          available_decoders: ["videotoolbox_h264", "software", "none"],
          available_renderers: ["macos", "none"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "test_start_run") return Promise.resolve("run-should-not-start");
      return Promise.resolve(null);
    });

    render(
      <MemoryRouter>
        <CustomTestPage />
      </MemoryRouter>
    );

    const encoder = await screen.findByRole("radio", { name: /VideoToolbox HEVC/ });
    await waitFor(() => {
      expect(screen.getByRole("radio", { name: /直连渲染/ })).toBeChecked();
      expect(screen.getByRole("radio", { name: /无解码/ })).toBeChecked();
    });
    await user.click(encoder);
    await waitFor(() => expect(encoder).toBeChecked());
    const decoder = screen
      .getAllByRole("radio")
      .find((radio) => (radio as HTMLInputElement).value === "videotoolbox");
    expect(decoder).toBeDefined();
    await user.click(decoder!);

    await waitFor(() => {
      expect(encoder).toBeChecked();
      expect(decoder).toBeChecked();
      expect(screen.getByRole("button", { name: /启动测试/ })).toBeDisabled();
      expect(
        screen.getByText("当前环境未暴露 VideoToolbox HEVC 解码能力。")
      ).toBeInTheDocument();
    });
    expect(mockInvoke).not.toHaveBeenCalledWith("test_start_run", expect.anything());
  });
});
