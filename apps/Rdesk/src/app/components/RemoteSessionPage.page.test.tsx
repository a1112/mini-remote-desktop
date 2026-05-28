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

const mockNavigate = vi.hoisted(() => vi.fn());
const mockRuntimeState = vi.hoisted(() => ({ isTauri: false }));
const mockListRemoteDisplayWindows = vi.hoisted(() => vi.fn());
const mockOpenRemoteDisplayWindow = vi.hoisted(() => vi.fn());
const mockGetSessionSnapshot = vi.hoisted(() => vi.fn());
const mockGetProbeSnapshot = vi.hoisted(() => vi.fn());
const mockStopSession = vi.hoisted(() => vi.fn());
const mockGetDecodePolicy = vi.hoisted(() => vi.fn());
const mockSetDecodePolicy = vi.hoisted(() => vi.fn());
const mockFfmpegProbe = vi.hoisted(() => vi.fn());

// Mock fetch globally to prevent HTTP requests
globalThis.fetch = vi.fn();

// Mock react-router - use importOriginal for components we need
vi.mock('react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router')>();
  return {
    ...actual,
    useParams: () => ({ id: 'test-device-1' }),
    useNavigate: () => mockNavigate,
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
  isTauriRuntime: () => mockRuntimeState.isTauri,
}));

// Mock services
vi.mock('../services/realtimeService', () => ({
  getDecodePolicy: () => Promise.reject(new Error('Deprecated')),
  getNvdecRuntimeProbe: () => Promise.reject(new Error('Deprecated')),
  setDecodePolicy: () => Promise.reject(new Error('Deprecated')),
}));

vi.mock('../services/serviceLifecycleService', () => ({
  getDecodePolicy: mockGetDecodePolicy,
  setDecodePolicy: mockSetDecodePolicy,
  ffmpegProbe: mockFfmpegProbe,
}));

vi.mock('../services/realtimeSessionService', () => ({
  getWebrtcHostSnapshot: () => Promise.reject(new Error('Deprecated')),
}));

vi.mock('../adapters/tauri', () => ({
  listRemoteDisplayWindows: mockListRemoteDisplayWindows,
  openRemoteDisplayWindow: mockOpenRemoteDisplayWindow,
}));

vi.mock('../services/ipcSessionService', () => ({
  getSessionSnapshot: mockGetSessionSnapshot,
  getProbeSnapshot: mockGetProbeSnapshot,
  stopSession: mockStopSession,
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
    discoverySources: ['server'],
    primarySource: 'server',
    sourceLabel: '服务器',
    isLocal: false,
    p2pAvailable: false,
    serverAvailable: true,
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
    mockRuntimeState.isTauri = false;
    mockListRemoteDisplayWindows.mockResolvedValue({ ok: true, value: [] });
    mockOpenRemoteDisplayWindow.mockResolvedValue({
      ok: true,
      value: {
        label: 'web-test-device-1',
        session_id: 'test-device-1',
        surface_id: 'web-test-device-1',
        role: 'primary',
        renderer_attached: true,
        render_mode: 'd3d11_native',
      },
    });
    mockGetSessionSnapshot.mockResolvedValue({
      session_id: 'test-device-1',
      state: 'streaming',
      transport_kind: 'quic',
      last_error: null,
    });
    mockGetProbeSnapshot.mockResolvedValue({
      current_fps: 144,
      frames_decoded: 42,
      last_error: null,
    });
    mockStopSession.mockResolvedValue(undefined);
    mockGetDecodePolicy.mockResolvedValue({ decode_policy: 'auto' });
    mockSetDecodePolicy.mockResolvedValue({ decode_policy: 'nvdec' });
    mockFfmpegProbe.mockResolvedValue({
      available: true,
      ffmpeg_path: 'C:\\ffmpeg\\bin\\ffmpeg.exe',
      ffprobe_path: 'C:\\ffmpeg\\bin\\ffprobe.exe',
      ffmpeg_version: 'ffmpeg version 8.1.1',
      ffprobe_version: 'ffprobe version 8.1.1',
      reason: null,
    });
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
    it('navigates to the route fallback when popping out outside Tauri', async () => {
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

        expect(mockNavigate).toHaveBeenCalledWith('/display/test-device-1');
        expect(mockAlert).not.toHaveBeenCalled();
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

  describe('media decode controls', () => {
    it('loads FFmpeg fallback status and saves decode policy in Tauri sessions', async () => {
      mockRuntimeState.isTauri = true;

      render(
        <MemoryRouter initialEntries={['/sessions/test-device-1']}>
          <RemoteSessionPage />
        </MemoryRouter>
      );

      expect(await screen.findByText('Decoder Policy')).toBeInTheDocument();
      expect(screen.getByText('ffmpeg version 8.1.1')).toBeInTheDocument();

      await userEvent.selectOptions(screen.getByLabelText('会话解码策略'), 'nvdec');

      await waitFor(() => {
        expect(mockSetDecodePolicy).toHaveBeenCalledWith('nvdec');
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

      expect(screen.getByText('原生显示窗口承载画面')).toBeInTheDocument();
    });
  });
});
