import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { rAuthAvailable, signInWithRAuth } from './rAuth';

const mocks = vi.hoisted(() => ({ native: false, openNative: vi.fn(), apiUrl: 'https://rdesk.example/api/v1' }));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => mocks.native }));
vi.mock('../adapters/tauri/commands', () => ({ openRAuthLogin: mocks.openNative }));
vi.mock('./serverConfig', () => ({ get SERVER_API_URL() { return mocks.apiUrl; } }));

const authorizationUrl = 'https://identity.example/realms/example/auth?state=test';
const completionToken = 'T'.repeat(43);
const login = { access_token: 'app-session', user_id: 'account-1', username: 'Account', role: 'user' };

const startResponse = (url = authorizationUrl) => Response.json({
  authorization_url: url, completion_token: completionToken, expires_in: 300,
});

describe('R-AUTH sign-in handoff', () => {
  beforeEach(() => {
    mocks.native = false;
    mocks.apiUrl = 'https://rdesk.example/api/v1';
    mocks.openNative.mockReset().mockResolvedValue({ ok: true });
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('uses the system browser and keeps completion credentials off its URL', async () => {
    mocks.native = true;
    const fetch = vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(startResponse())
      .mockResolvedValueOnce(new Response(null, { status: 202 }))
      .mockResolvedValueOnce(Response.json(login));
    const opened = vi.fn();
    const signingIn = signInWithRAuth(new AbortController().signal, opened, 'existing-session');

    await vi.advanceTimersByTimeAsync(2000);
    await expect(signingIn).resolves.toEqual(login);
    expect(opened).toHaveBeenCalledOnce();
    expect(mocks.openNative).toHaveBeenCalledWith(authorizationUrl);
    expect(fetch).toHaveBeenNthCalledWith(1, expect.stringContaining('/link/start'),
      expect.objectContaining({ headers: { 'Content-Type': 'application/json', Authorization: 'Bearer existing-session' } }));
    expect(fetch).toHaveBeenNthCalledWith(2, expect.stringContaining('/complete'),
      expect.objectContaining({ headers: { 'Content-Type': 'application/json', Authorization: 'Bearer existing-session' }, body: JSON.stringify({ completion_token: completionToken }) }));
    expect(fetch).toHaveBeenCalledTimes(3);
  });

  it('closes the isolated popup and stops polling when the initiating view leaves', async () => {
    const popup = { opener: window, location: { replace: vi.fn() }, close: vi.fn() };
    vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(startResponse());
    const control = new AbortController();
    const signingIn = signInWithRAuth(control.signal, vi.fn());
    const rejected = expect(signingIn).rejects.toThrow('view closed');

    await vi.advanceTimersByTimeAsync(0);
    expect(popup.opener).toBeNull();
    expect(popup.location.replace).toHaveBeenCalledWith(authorizationUrl);
    control.abort(new Error('view closed'));
    await rejected;
    expect(popup.close).toHaveBeenCalledOnce();
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('rejects an unsafe provider URL before opening it', async () => {
    mocks.native = true;
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(startResponse('http://identity.example/auth'));

    await expect(signInWithRAuth(new AbortController().signal, vi.fn())).rejects.toThrow('R-AUTH 登录地址无效');
    expect(mocks.openNative).not.toHaveBeenCalled();
  });

  it('refuses to send OIDC credentials to an insecure Rdesk API', async () => {
    mocks.apiUrl = 'http://rdesk.example/api/v1';
    const fetch = vi.spyOn(globalThis, 'fetch');
    const signal = new AbortController().signal;
    await expect(rAuthAvailable(signal)).resolves.toBe(false);
    await expect(signInWithRAuth(signal, vi.fn(), 'existing-session')).rejects.toThrow('需要 HTTPS');
    expect(fetch).not.toHaveBeenCalled();
  });
});
