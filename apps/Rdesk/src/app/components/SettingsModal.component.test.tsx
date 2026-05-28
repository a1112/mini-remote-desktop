/**
 * Component test example for SettingsModal
 *
 * This demonstrates how to test:
 * - Component behavior (not static HTML)
 * - User interactions
 * - Error handling
 * - Lifecycle (mounting, effects)
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { getMockInvoke } from "../../test/mocks/tauri";
import { SettingsModal } from "./SettingsModal";

// Mock theme context
vi.mock("./ThemeContext", () => ({
  useTheme: () => ({
    isDark: false,
    theme: "light",
    setTheme: vi.fn(),
  }),
}));

// Mock IpcSessionCard
vi.mock("./IpcSessionCard", () => ({
  IpcSessionCard: () => (
    <div data-testid="ipc-session-card">
      <div>IPC Session Control</div>
    </div>
  ),
}));

describe("SettingsModal - Component Behavior", () => {
  const defaultProps = {
    open: true,
    onClose: vi.fn(),
  };
  const runningStatus = {
    service_pid: 12345,
    ui_pid: 54321,
    tray_available: true,
    autostart_enabled: true,
    active_session_count: 0,
    last_error: null,
  };

  const ffmpegProbe = {
    available: true,
    ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
    ffprobe_path: "C:\\ffmpeg\\bin\\ffprobe.exe",
    ffmpeg_version: "ffmpeg version 8.1.1",
    ffprobe_version: "ffprobe version 8.1.1",
    reason: null,
  };

  const mockSettingsCommands = (mockInvoke: ReturnType<typeof getMockInvoke>) => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "shell_get_status") return Promise.resolve(runningStatus);
      if (cmd === "decode_policy") return Promise.resolve({ decode_policy: "auto" });
      if (cmd === "set_decode_policy") return Promise.resolve({ decode_policy: "nvdec" });
      if (cmd === "ffmpeg_probe") return Promise.resolve(ffmpegProbe);
      if (cmd === "ffmpeg_download") {
        return Promise.resolve({
          install_dir: "C:\\ffmpeg",
          archive_sha256: "a".repeat(64),
          probe: ffmpegProbe,
        });
      }
      if (cmd === "ffmpeg_reset_golden_settings") {
        return Promise.resolve({
          decode_policy: "auto",
          ffmpeg: {
            enabled: true,
            channel: "release-essentials",
            install_dir: "C:\\ffmpeg",
            ffmpeg_path: null,
            ffprobe_path: null,
            download: {
              archive_url: "https://example.test/ffmpeg.zip",
              sha256_url: "https://example.test/ffmpeg.zip.sha256",
              require_sha256: true,
            },
          },
        });
      }
      return Promise.resolve(true);
    });
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should render without crashing", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });
  });

  it("should fetch service status on mount", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Service status should be fetched
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("shell_get_status", undefined);
    });
  });

  it("should show media decode settings with FFmpeg status", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section
    await userEvent.click(screen.getByText("网络"));

    expect(await screen.findByText("媒体解码")).toBeInTheDocument();
    expect(screen.getByText("FFmpeg 可选工具")).toBeInTheDocument();
    expect(screen.getByText("ffmpeg version 8.1.1")).toBeInTheDocument();
    expect(screen.getByText("C:\\ffmpeg\\bin\\ffmpeg.exe")).toBeInTheDocument();
  });

  it("should update decode policy and run FFmpeg install actions", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("网络"));
    await userEvent.selectOptions(await screen.findByLabelText("解码策略"), "nvdec");

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_decode_policy", {
        decodePolicy: "nvdec",
      });
    });

    await userEvent.click(screen.getByRole("button", { name: "下载或更新 FFmpeg" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ffmpeg_download", undefined);
    });
  });

  it("should handle service status refresh on button click", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section first
    await userEvent.click(screen.getByText("网络"));

    // Now refresh button should be visible
    const refreshButton = await screen.findByRole("button", { name: "刷新" });
    await userEvent.click(refreshButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalled();
    });
  });

  it("should call service lifecycle commands when buttons are clicked", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "shell_get_status") {
        return Promise.reject(new Error("connection refused"));
      }
      if (cmd === "service_bootstrap_if_needed") {
        return Promise.resolve(true);
      }
      return Promise.resolve(true);
    });

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section
    await userEvent.click(screen.getByText("网络"));

    // Wait for service status to load
    await waitFor(() => {
      expect(screen.getByText("未运行")).toBeInTheDocument();
    });

    // Find and click "启动" button
    const startButtons = screen.getAllByText("启动");
    const startButton = startButtons.find((btn) => btn.tagName === "BUTTON");

    if (startButton) {
      await userEvent.click(startButton);

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith("service_bootstrap_if_needed", undefined);
      });
    }
  });

  it("should show error message when service operation fails", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "shell_get_status") {
        return Promise.reject(new Error("Unexpected response"));
      }
      return Promise.resolve(true);
    });

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("网络"));

    await waitFor(() => {
      expect(screen.getByText(/读取服务状态失败|Unexpected response/)).toBeInTheDocument();
    });
  });

  it("should close modal when close button is clicked", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Find the close button (X icon) - button with icon but no text
    const buttons = screen.getAllByRole("button");
    const closeButton = buttons.find(
      (btn) => btn.querySelector("svg") !== null && btn.textContent === ""
    );

    if (closeButton) {
      await userEvent.click(closeButton);
      expect(defaultProps.onClose).toHaveBeenCalled();
    }
  });

  it("should switch between sections when clicking nav items", async () => {
    const mockInvoke = getMockInvoke();
    mockSettingsCommands(mockInvoke);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Click on "安全" section
    const securityButton = screen.getByText("安全");
    await userEvent.click(securityButton);

    await waitFor(() => {
      expect(screen.getByText("双因素认证")).toBeInTheDocument();
    });
  });

  it("should not crash when services are unavailable", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "shell_get_status") {
        return Promise.reject(new Error("Unexpected response"));
      }
      return Promise.resolve(true);
    });
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    render(<SettingsModal {...defaultProps} />);

    // Wait for the modal to render and async effects to settle
    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section to verify error state is handled
    await userEvent.click(screen.getByText("网络"));

    // Should show error message or handle the error gracefully
    await waitFor(() => {
      expect(screen.getByText(/读取服务状态失败|Unexpected response/)).toBeInTheDocument();
    });

    const sawActWarning = consoleErrorSpy.mock.calls.some((call) =>
      call.some((arg) => typeof arg === "string" && arg.includes("not wrapped in act"))
    );

    expect(sawActWarning).toBe(false);
    consoleErrorSpy.mockRestore();
  });
});
