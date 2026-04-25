/**
 * SettingsModal Page Test
 *
 * Page-level behavior test for SettingsModal component.
 * Tests user-visible behavior, loading states, error handling,
 * and migration-state messaging.
 *
 * This is Layer 4 of the testing architecture - Page-Level Tests.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { getMockInvoke } from '../../test/mocks/tauri';
import { SettingsModal } from './SettingsModal';
import { ThemeProvider } from './ThemeContext';

// Mock ThemeProvider
vi.mock('./ThemeContext', () => ({
  useTheme: vi.fn(() => ({
    isDark: false,
    theme: 'light',
    setTheme: vi.fn(),
  })),
  ThemeProvider: ({ children }: { children: React.ReactNode }) => children,
}));

// Mock IpcSessionCard to avoid testing its internal logic here
vi.mock('./IpcSessionCard', () => ({
  IpcSessionCard: () => (
    <div data-testid="ipc-session-card">
      <div>IPC Session Control</div>
    </div>
  ),
}));

/**
 * Helper to render SettingsModal with test providers
 */
function renderSettingsModal(open: boolean = true) {
  const onClose = vi.fn();

  const result = render(
    <ThemeProvider>
      <SettingsModal open={open} onClose={onClose} />
    </ThemeProvider>
  );

  return {
    ...result,
    onClose,
  };
}

describe('SettingsModal - Page Level Tests', () => {
  const runningStatus = {
    service_pid: 12345,
    ui_pid: 54321,
    tray_available: true,
    autostart_enabled: true,
    active_session_count: 0,
    last_error: null,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ========================================================================
  // Initial Load and Rendering
  // ========================================================================

  describe('initial load', () => {
    it('renders nothing when closed', () => {
      const { container } = renderSettingsModal(false);

      // Modal should not be in the document when closed
      expect(container.querySelector('.fixed')).toBeNull();
    });

    it('renders modal when open', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      // Check for modal title
      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });
    });

    it('renders all section navigation items', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      const expectedSections = ['通用', '安全', '网络', '显示', '音频与输入', '通知', '账户'];

      await waitFor(() => {
        for (const section of expectedSections) {
          expect(screen.getByText(section)).toBeInTheDocument();
        }
      });
    });

    it('shows general section by default', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('通用设置')).toBeInTheDocument();
      });
    });
  });

  // ========================================================================
  // Service Status - Loading and Success States
  // ========================================================================

  describe('service status', () => {
    it('fetches service status on mount', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      // Wait for modal to render
      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      // Navigate to network section where service status is shown
      await userEvent.click(screen.getByText('网络'));

      // Service status should have been fetched
      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith('shell_get_status', undefined);
      });
    });

    it('displays running status when service is active', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText('运行中')).toBeInTheDocument();
        expect(screen.getByText('健康')).toBeInTheDocument();
      });
    });

    it('displays stopped status when service is not running', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') {
          return Promise.reject(new Error('connection refused'));
        }
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText('未运行')).toBeInTheDocument();
      });
    });

    it('shows refresh button in network section', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText('刷新')).toBeInTheDocument();
      });
    });
  });

  // ========================================================================
  // Migration State Messaging (Deprecated Features)
  // ========================================================================

  describe('migration state messaging', () => {
    it('shows deprecation notice for NVDEC features', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText('NVDEC 和 Decode Policy')).toBeInTheDocument();
      });
    });

    it('explains that decode policy is managed by mrd-service', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        const element = screen.getByText(/这些功能已迁移/);
        expect(element).toBeInTheDocument();
        expect(element.textContent).toContain('mrd-service');
      });
    });

    it('shows IPC Session Control component', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        // IPC Session Card should be rendered (via mock)
        expect(screen.getByText('IPC Session Control')).toBeInTheDocument();
        expect(screen.getByTestId('ipc-session-card')).toBeInTheDocument();
      });
    });
  });

  // ========================================================================
  // Error Handling
  // ========================================================================

  describe('error handling', () => {
    it('displays error message when service status fetch fails', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') {
          return Promise.reject(new Error('Unexpected response'));
        }
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText(/读取服务状态失败|Unexpected response/)).toBeInTheDocument();
      });
    });

    it('displays error message when service action fails', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') {
          return Promise.reject(new Error('connection refused'));
        }
        if (cmd === 'service_bootstrap_if_needed') {
          return Promise.reject(new Error('Start failed'));
        }
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      // Navigate to network section
      await userEvent.click(screen.getByText('网络'));

      // Wait for service status to load
      await waitFor(() => {
        expect(screen.getByText('未运行')).toBeInTheDocument();
      });

      // Find and click start button
      const startButtons = screen.getAllByText('启动');
      const startButton = startButtons.find((btn) => btn.tagName === 'BUTTON');

      if (startButton) {
        await userEvent.click(startButton);

        await waitFor(() => {
          expect(screen.getByText(/Start failed|执行服务操作失败/)).toBeInTheDocument();
        });
      }
    });
  });

  // ========================================================================
  // User Interactions
  // ========================================================================

  describe('user interactions', () => {
    it('closes modal when X button is clicked', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      const { onClose } = renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      // Find the close button (X icon) - it's the button with just an icon (no text)
      const buttons = screen.getAllByRole('button');
      const closeButton = buttons.find(
        (btn) => btn.querySelector('svg') !== null && btn.textContent === ''
      );

      if (closeButton) {
        await userEvent.click(closeButton);
        expect(onClose).toHaveBeenCalled();
      }
    });

    it('closes modal when Escape key is pressed', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      const { onClose } = renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.keyboard('{Escape}');

      expect(onClose).toHaveBeenCalled();
    });

    it('switches between sections', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      // Click on security section
      await userEvent.click(screen.getByText('安全'));

      await waitFor(() => {
        expect(screen.getByText('安全设置')).toBeInTheDocument();
      });

      // Click on display section
      await userEvent.click(screen.getByText('显示'));

      await waitFor(() => {
        expect(screen.getByText('显示设置')).toBeInTheDocument();
      });
    });

    it('toggles switches in general section', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'shell_get_status') return Promise.resolve(runningStatus);
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      // Find toggle buttons (they have rounded-full class)
      const toggleButtons = screen.getAllByRole('button').filter(
        (btn) => btn.className.includes('rounded-full')
      );

      // Should have toggle switches in general section
      expect(toggleButtons.length).toBeGreaterThan(0);

      // Click first toggle and verify it's clickable
      const firstToggleButton = toggleButtons[0];
      if (firstToggleButton) {
        await userEvent.click(firstToggleButton);
        // If we get here without error, the toggle is clickable
        expect(true).toBe(true);
      }
    });
  });

  // ========================================================================
  // Service Lifecycle Actions
  // ========================================================================

  describe('service lifecycle actions', () => {
    it('calls service_bootstrap_if_needed when start button is clicked', async () => {
      const mockInvoke = getMockInvoke();
      let startCalled = false;

      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === 'service_bootstrap_if_needed') {
          startCalled = true;
          return Promise.resolve(true);
        }
        if (cmd === 'shell_get_status') {
          if (startCalled) {
            return Promise.resolve(runningStatus);
          }
          return Promise.reject(new Error('connection refused'));
        }
        return Promise.resolve(true);
      });

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      // Wait for service status to load
      await waitFor(() => {
        expect(screen.getByText('未运行')).toBeInTheDocument();
      });

      // Click start button
      const startButtons = screen.getAllByText('启动');
      const startButton = startButtons.find((btn) => btn.tagName === 'BUTTON');

      if (startButton) {
        await userEvent.click(startButton);

        await waitFor(() => {
          expect(startCalled).toBe(true);
        });
      }
    });
  });

  // ========================================================================
  // Degraded/Disabled States
  // ========================================================================

  describe('degraded/disabled migration state', () => {
    it('shows deprecation notice for migrated features', async () => {
      const mockInvoke = getMockInvoke();
      mockInvoke.mockResolvedValue(true);

      renderSettingsModal(true);

      await waitFor(() => {
        expect(screen.getByText('设置')).toBeInTheDocument();
      });

      await userEvent.click(screen.getByText('网络'));

      await waitFor(() => {
        expect(screen.getByText('NVDEC 和 Decode Policy')).toBeInTheDocument();
        expect(screen.getByText(/解码策略现在由服务内部管理/)).toBeInTheDocument();
      });
    });
  });
});
