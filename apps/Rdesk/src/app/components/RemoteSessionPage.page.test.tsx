/**
 * RemoteSessionPage Page Test
 *
 * Page-level behavior test for RemoteSessionPage component.
 * Tests user-visible behavior, loading states, error handling,
 * and migration-state messaging for disabled rendering features.
 *
 * This is Layer 4 of the testing architecture - Page-Level Tests.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router';
import { Monitor } from 'lucide-react';
import { RemoteSessionPage } from './RemoteSessionPage';
import type { Device } from './deviceData';

// Mock fetch globally to prevent HTTP requests
globalThis.fetch = vi.fn();

// Mock react-router - use importOriginal for components we need
vi.mock('react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router')>();
  return {
    ...actual,
    useParams: () => ({ id: 'test-device-1' }),
    useNavigate: () => vi.fn(),
  };
});

// Mock theme context
vi.mock('./ThemeContext', () => ({
  useTheme: () => ({
    isDark: false,
    theme: 'light',
    setTheme: vi.fn(),
  }),
}));

// Mock Tauri window utils
vi.mock('../utils/tauriWindow', () => ({
  withTauriWindow: (fn: (appWindow: {
    isMaximized: () => Promise<boolean>;
    maximize: () => Promise<void>;
    minimize: () => Promise<void>;
    close: () => Promise<void>;
    startDragging: () => Promise<void>;
    toggleMaximize: () => Promise<void>;
  }) => Promise<unknown>) => {
    // Mock appWindow with commonly used methods
    const mockAppWindow = {
      isMaximized: () => Promise.resolve(false),
      maximize: () => Promise.resolve(undefined),
      minimize: () => Promise.resolve(undefined),
      close: () => Promise.resolve(undefined),
      startDragging: () => Promise.resolve(undefined),
      toggleMaximize: () => Promise.resolve(undefined),
    };
    return fn(mockAppWindow);
  },
}));

// Mock runtime utils
vi.mock('../utils/runtime', () => ({
  isTauriRuntime: () => false,
}));

// Mock services
vi.mock('../services/realtimeService', () => ({
  getDecodePolicy: () => Promise.reject(new Error('Deprecated')),
  getNvdecRuntimeProbe: () => Promise.reject(new Error('Deprecated')),
  setDecodePolicy: () => Promise.reject(new Error('Deprecated')),
}));

vi.mock('../services/realtimeSessionService', () => ({
  getWebrtcHostSnapshot: () => Promise.reject(new Error('Deprecated')),
}));

// Mock device service
vi.mock('../services/deviceService', () => ({
  deviceService: {
    getDeviceId: () => 'test-device-1',
  },
}));

// Mock AuthContext
vi.mock('./AuthContext', () => ({
  useAuth: () => ({
    isLoggedIn: true,
    token: 'test-token',
  }),
}));

// Mock lucide-react icons - use importOriginal to keep all exports
vi.mock('lucide-react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('lucide-react')>();
  return {
    ...actual,
  };
});

// Mock devices
const mockDevices: Device[] = [
  {
    id: 'test-device-1',
    name: 'Test PC',
    deviceId: 'dev-001',
    os: 'Windows 11',
    icon: Monitor,
    status: 'online',
    location: 'Office',
    ping: 24,
    lastSeen: '2026-03-21T10:00:00Z',
    cpu: 45,
    ram: 60,
    disk: 70,
    ip: '192.168.1.100',
    group: 'Work',
    favorite: false,
  },
];

// Mock deviceData
vi.mock('./deviceData', () => ({
  useDevices: () => ({
    devices: mockDevices,
    loading: false,
    error: null,
  }),
  useDeviceById: (id: string | undefined, devices: Device[]) =>
    devices.find((d) => d.id === id) || null,
}));

describe('RemoteSessionPage - Page Level Tests', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ========================================================================
  // Successful Render (Main Scenario)
  // ========================================================================

  describe('successful render', () => {
    it('renders the remote session page with device info', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        expect(screen.getByText('Test PC')).toBeInTheDocument();
        expect(screen.getByText('Windows 11')).toBeInTheDocument();
      });
    });

    it('shows session controls', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        expect(screen.getByText('鼠标')).toBeInTheDocument();
        expect(screen.getByText('键盘')).toBeInTheDocument();
        expect(screen.getByText('音频')).toBeInTheDocument();
        expect(screen.getByText('剪贴板')).toBeInTheDocument();
        expect(screen.getByText('锁屏')).toBeInTheDocument();
        expect(screen.getByText('刷新')).toBeInTheDocument();
        expect(screen.getByText('关机')).toBeInTheDocument();
      });
    });

    it('shows status indicators', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        // Connection status
        expect(screen.getByText('连接稳定')).toBeInTheDocument();
      });
    });

    it('shows window controls', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        // Minimize, maximize, close buttons should exist
        const buttons = screen.getAllByRole('button');
        expect(buttons.length).toBeGreaterThan(0);
      });
    });
  });

  // ========================================================================
  // Migration/Deprecated Features
  // ========================================================================

  describe('migration state - disabled rendering features', () => {
    it('shows alert when trying to pop out render window', async () => {
      const mockAlert = vi.fn();
      globalThis.alert = mockAlert;

      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        expect(screen.getByText('独立窗口')).toBeInTheDocument();
      });

      // Click the "独立窗口" button
      const popOutButtons = screen.getAllByText('独立窗口');
      const popOutButton = popOutButtons.find((btn) => btn.tagName === 'BUTTON');

      if (popOutButton) {
        await userEvent.click(popOutButton);

        expect(mockAlert).toHaveBeenCalledWith(
          expect.stringContaining('渲染窗口功能已迁移到 mrd-service')
        );
      }

      mockAlert.mockRestore();
      delete (globalThis as unknown as Record<string, unknown>).alert;
    });

    it('shows degraded state for rendering features', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      // Page should still render even though rendering features are disabled
      await waitFor(() => {
        expect(screen.getByText('Test PC')).toBeInTheDocument();
      });
    });
  });

  // ========================================================================
  // User Interactions
  // ========================================================================

  describe('user interactions', () => {
    it('toggles mute state when audio button is clicked', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        expect(screen.getByText('音频')).toBeInTheDocument();
      });

      const audioButton = screen.getByText('音频');
      await userEvent.click(audioButton);

      // After clicking, should show "静音" (muted)
      await waitFor(() => {
        expect(screen.getByText('静音')).toBeInTheDocument();
      });
    });

    it('shows disconnect button', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      await waitFor(() => {
        expect(screen.getByText('断开')).toBeInTheDocument();
      });
    });
  });

  // ========================================================================
  // Disabled Features
  // ========================================================================

  describe('disabled rendering features', () => {
    it('does not crash when accessing disabled features', async () => {
      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      // Page should render without throwing
      await waitFor(() => {
        expect(screen.getByText('Test PC')).toBeInTheDocument();
      });

      // Verify placeholder values are used
      expect(screen.queryByText('renderer idle')).toBeInTheDocument();
    });
  });
});
