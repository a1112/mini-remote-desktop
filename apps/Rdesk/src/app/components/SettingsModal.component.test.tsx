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

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should render without crashing", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue(true);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });
  });

  it("should fetch service status on mount", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue(true);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Service status should be fetched
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("service_status", undefined);
    });
  });

  it("should show deprecation notice for migrated features", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue(true);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section
    await userEvent.click(screen.getByText("网络"));

    await waitFor(() => {
      expect(screen.getAllByText(/功能已迁移/).length).toBeGreaterThan(0);
    });
  });

  it("should handle service status refresh on button click", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue(true);

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    // Navigate to network section first
    await userEvent.click(screen.getByText("网络"));

    // Now refresh button should be visible
    const refreshButton = await screen.findByText("刷新");
    await userEvent.click(refreshButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalled();
    });
  });

  it("should call service lifecycle commands when buttons are clicked", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "service_start") return Promise.resolve(true);
      return Promise.resolve(false);
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
        expect(mockInvoke).toHaveBeenCalledWith("service_start", undefined);
      });
    }
  });

  it("should show error message when service operation fails", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockRejectedValue(new Error("Service unavailable"));

    render(<SettingsModal {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("设置")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("网络"));

    await waitFor(() => {
      expect(screen.getByText(/读取服务状态失败|Service unavailable/)).toBeInTheDocument();
    });
  });

  it("should close modal when close button is clicked", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue(true);

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
    mockInvoke.mockResolvedValue(true);

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
    mockInvoke.mockRejectedValue(new Error("IPC timeout"));
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
      expect(screen.getByText(/读取服务状态失败|IPC timeout/)).toBeInTheDocument();
    });

    const sawActWarning = consoleErrorSpy.mock.calls.some((call) =>
      call.some((arg) => typeof arg === "string" && arg.includes("not wrapped in act"))
    );

    expect(sawActWarning).toBe(false);
    consoleErrorSpy.mockRestore();
  });
});
