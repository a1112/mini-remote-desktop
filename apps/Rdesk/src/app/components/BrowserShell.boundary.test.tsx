import { act, render, renderHook, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useDevices } from './deviceData';
import { IncomingSessionConsentHost } from './IncomingSessionConsentHost';
import { ServiceStatusPanel } from './ServiceStatusPanel';
import { SettingsContent } from './SettingsContent';
import { SERVER_API_URL } from '../services/serverConfig';
const mocks = vi.hoisted(() => ({
  subscribe: vi.fn(), publicStatus: vi.fn(), localInit: vi.fn(), localInfo: vi.fn(), localId: vi.fn(), prefs: vi.fn(), lan: vi.fn(), localLabel: vi.fn(),
  settings: { getUiPreferences: vi.fn(), getServiceAutostart: vi.fn(), getServiceHealth: vi.fn(), getDecodePolicy: vi.fn(), ffmpegProbe: vi.fn(), ffmpegDownload: vi.fn(), ffmpegResetGoldenSettings: vi.fn(), serviceStart: vi.fn(), serviceStop: vi.fn(), serviceRestart: vi.fn(), quitUiAndStopService: vi.fn(), setCloseBehavior: vi.fn(), setServiceAutostart: vi.fn() },
  listSessions: vi.fn(),
}));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => false }));
vi.mock('./AuthContext', () => ({ useAuth: () => ({ isLoggedIn: true, token: 'user-token', user: { username: 'browser-user', role: 'user' }, logout: vi.fn(), logoutError: null }) }));
vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false, theme: 'light', setTheme: vi.fn() }) }));
vi.mock('../utils/tauriWindow', () => ({ getTauriWindowLabel: mocks.localLabel }));
vi.mock('../adapters/tauri', () => ({ ipcSubscribeSessionEvents: mocks.subscribe, ipcGetRemoteSession: vi.fn(), ipcRespondToConsent: vi.fn(), showWindow: vi.fn(), ipcLanDiscoverySnapshot: mocks.lan, ipcRefreshLanDiscovery: mocks.lan }));
vi.mock('../adapters/tauri/commands', () => ({ ipcPublicServerStatus: mocks.publicStatus }));
vi.mock('../services/deviceService', () => ({ deviceService: { initialize: mocks.localInit, getDeviceInfo: mocks.localInfo, getDeviceId: mocks.localId } }));
vi.mock('../services/deviceActionService', () => ({ deviceActionService: { refreshDevicePreferences: mocks.prefs, applyDevicePreferences: (devices: unknown[]) => devices } }));
vi.mock('../services/serviceLifecycleService', () => mocks.settings);
vi.mock('../services/ipcSessionService', () => ({ listSessions: mocks.listSessions }));

describe('zero-install browser boundaries', () => {
  beforeEach(() => {
    vi.clearAllMocks(); localStorage.clear();
    mocks.localInit.mockResolvedValue(null); mocks.localInfo.mockReturnValue(null); mocks.localId.mockReturnValue(null);
    mocks.localLabel.mockResolvedValue(null);
    mocks.subscribe.mockResolvedValue({ ok: true, value: { events: [], pending_sessions: [], poll_after_ms: 60000, cursor_state: 'current' } });
    mocks.publicStatus.mockResolvedValue({ ok: false, error: { message: 'No localhost service' } });
    mocks.prefs.mockRejectedValue(new Error('Local IPC does not exist'));
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => [{ id: 'remote-1', name: 'Remote Windows', device_id: '0123456789', os: 'Windows', icon: 'Monitor', status: 'online', location: '', ping: null, last_seen: 'now', cpu: null, ram: null, disk: null, ip: '', group: 'Devices', favorite: false }] }));
    for (const mock of Object.values(mocks.settings)) mock.mockResolvedValue(null);
  });
  afterEach(() => vi.unstubAllGlobals());

  it('loads the account device registry without querying localhost or registering the browser as a host', async () => {
    const result = renderHook(() => useDevices({ pollInterval: 60000 }));
    await waitFor(() => expect(result.result.current.loading).toBe(false));
    expect(result.result.current.devices).toHaveLength(1);
    expect(result.result.current.devices[0]?.deviceId).toBe('0123456789');
    expect(fetch).toHaveBeenCalledWith(`${SERVER_API_URL}/devices`, { headers: { Authorization: 'Bearer user-token' } });
    expect(mocks.localInfo).not.toHaveBeenCalled(); expect(mocks.localInit).not.toHaveBeenCalled(); expect(mocks.localId).not.toHaveBeenCalled();
    expect(mocks.prefs).not.toHaveBeenCalled(); expect(mocks.lan).not.toHaveBeenCalled();
  });
  it('does not subscribe to native target consent or inspect desktop window identity in a browser', async () => {
    render(<IncomingSessionConsentHost />);
    await act(async () => undefined);
    expect(mocks.subscribe).not.toHaveBeenCalled();
    expect(mocks.localLabel).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
  it('explains the browser controller has no local service requirement without polling it', async () => {
    render(<ServiceStatusPanel />);
    await act(async () => undefined);
    expect(screen.getByText('网页控制端无需本机后台服务')).toBeInTheDocument();
    expect(mocks.publicStatus).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: '刷新连接状态' })).not.toBeInTheDocument();
  });
  it('disables native-only settings without reading local health, media tools, or IPC sessions', async () => {
    render(<SettingsContent />);
    await act(async () => undefined);
    for (const mock of Object.values(mocks.settings)) expect(mock).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: '退出并停止后台服务' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: '网络' }));
    expect(mocks.listSessions).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: '刷新' })).toBeDisabled();
    expect(screen.queryByText('健康')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '显示' }));
    expect(screen.getByRole('button', { name: '下载或更新 FFmpeg' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '刷新 FFmpeg 状态' })).toBeDisabled();
  });
});
