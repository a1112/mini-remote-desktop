import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AuthProvider } from './AuthContext';
import { TitleBar } from './TitleBar';
const mocks = vi.hoisted(() => ({ unbind: vi.fn(), closeAll: vi.fn() }));
vi.mock('../services/browserRemoteSessionService', () => ({ closeAllBrowserRemoteSessions: mocks.closeAll }));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => false }));
vi.mock('./ThemeContext', () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock('./DetailBarContext', () => ({ useDetailBar: () => ({ collapsed: false, payload: null }) }));
vi.mock('../services/fileTransferListService', () => ({ useFileTransfers: () => ({ transfers: [], loading: false, error: null }), transferName: vi.fn(), formatTransferBytes: vi.fn(), transferStatusLabel: vi.fn() }));
vi.mock('../utils/tauriWindow', () => ({ withTauriWindow: vi.fn() }));
vi.mock('../services/deviceService', () => ({ deviceService: { unbindDevice: mocks.unbind } }));
describe('title bar browser logout', () => {
  beforeEach(() => {
    vi.clearAllMocks(); localStorage.clear();
    localStorage.setItem('rdesk_access_token', 'titlebar-user-token');
    localStorage.setItem('rdesk_auth_user', JSON.stringify({ id: 'user-1', username: 'tester', role: 'user' }));
    mocks.unbind.mockResolvedValue(undefined); mocks.closeAll.mockResolvedValue(undefined);
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true }));
  });
  afterEach(() => vi.unstubAllGlobals());
  it('uses shared logout rather than unbinding a device or leaving browser sessions alive', async () => {
    render(<AuthProvider><TitleBar /></AuthProvider>);
    await userEvent.click(await screen.findByTitle('用户菜单'));
    await userEvent.click(screen.getByRole('button', { name: '退出登录' }));
    await waitFor(() => expect(localStorage.getItem('rdesk_access_token')).toBeNull());
    expect(mocks.unbind).not.toHaveBeenCalled();
    expect(mocks.closeAll).toHaveBeenCalledWith('logout');
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining('/auth/logout'), expect.objectContaining({ headers: expect.objectContaining({ Authorization: 'Bearer titlebar-user-token' }) }));
  });
  it('shows the failed remote revocation from the shared auth context', async () => {
    vi.mocked(fetch).mockResolvedValue({ ok: false, status: 503 } as Response);
    render(<AuthProvider><TitleBar /></AuthProvider>);
    await userEvent.click(await screen.findByTitle('用户菜单'));
    await userEvent.click(screen.getByRole('button', { name: '退出登录' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('服务端撤销失败');
  });
});
