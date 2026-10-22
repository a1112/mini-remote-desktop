import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AuthProvider, useAuth } from './AuthContext';
import { SERVER_API_URL } from '../services/serverConfig';
const mocks = vi.hoisted(() => ({ closeAll: vi.fn(), native: false }));
vi.mock('../services/browserRemoteSessionService', () => ({ closeAllBrowserRemoteSessions: mocks.closeAll }));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => mocks.native }));
function Controls() {
  const auth = useAuth() as ReturnType<typeof useAuth> & { logoutError?: string | null };
  return <><p>{auth.isLoggedIn ? '已登录' : '未登录'}</p><button onClick={auth.logout}>退出登录</button>{auth.logoutError && <p role="alert">{auth.logoutError}</p>}</>;
}
describe('browser account logout', () => {
  beforeEach(() => {
    vi.clearAllMocks(); localStorage.clear(); mocks.native = false;
    localStorage.setItem('rdesk_access_token', 'captured-user-jwt');
    localStorage.setItem('rdesk_auth_user', JSON.stringify({ id: 'user-1', username: 'tester', role: 'user' }));
    localStorage.setItem('app-theme', 'light');
    mocks.closeAll.mockResolvedValue(undefined);
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true }));
  });
  afterEach(() => vi.unstubAllGlobals());
  it('immediately clears local auth and closes only browser-owned sessions using the captured token to revoke', async () => {
    let finish!: (value: Response) => void;
    vi.mocked(fetch).mockReturnValue(new Promise<Response>(resolve => { finish = resolve; }));
    render(<AuthProvider><Controls /></AuthProvider>);
    await screen.findByText('已登录');
    await userEvent.click(screen.getByRole('button', { name: '退出登录' }));
    expect(screen.getByText('未登录')).toBeInTheDocument();
    expect(localStorage.getItem('rdesk_access_token')).toBeNull();
    expect(localStorage.getItem('app-theme')).toBe('light');
    expect(mocks.closeAll).toHaveBeenCalledWith('logout');
    expect(fetch).toHaveBeenCalledWith(`${SERVER_API_URL}/auth/logout`, expect.objectContaining({ method: 'POST', headers: expect.objectContaining({ Authorization: 'Bearer captured-user-jwt' }), body: '{}' }));
    finish({ ok: true } as Response);
  });
  it('reports failed server revocation without undoing local logout', async () => {
    vi.mocked(fetch).mockResolvedValue({ ok: false, status: 503 } as Response);
    render(<AuthProvider><Controls /></AuthProvider>);
    await screen.findByText('已登录');
    await userEvent.click(screen.getByRole('button', { name: '退出登录' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('本地已退出，服务端撤销失败');
    expect(screen.getByText('未登录')).toBeInTheDocument();
    expect(mocks.closeAll).toHaveBeenCalled();
  });
  it('preserves native local-only logout without touching service or server', async () => {
    mocks.native = true;
    render(<AuthProvider><Controls /></AuthProvider>);
    await screen.findByText('已登录');
    await userEvent.click(screen.getByRole('button', { name: '退出登录' }));
    await waitFor(() => expect(screen.getByText('未登录')).toBeInTheDocument());
    expect(fetch).not.toHaveBeenCalled();
    expect(mocks.closeAll).not.toHaveBeenCalled();
  });
});
