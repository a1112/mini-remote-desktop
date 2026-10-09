import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { attachBrowserRemoteSession, closeAllBrowserRemoteSessions, createBrowserRemoteSession } from './browserRemoteSessionService';
import * as protocol from './browserRemoteProtocol';
import protocolFixture from '../../../../realtime-server/tests/fixtures/browser_protocol_v3.json';
import relayFixture from './__fixtures__/browser-relay-directory.json';
const cryptoModuleName = 'node:crypto';
const { webcrypto } = await import(/* @vite-ignore */ cryptoModuleName) as { webcrypto: Crypto };

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal('crypto', webcrypto);
  vi.stubGlobal('isSecureContext', true);
});
afterEach(async () => { await closeAllBrowserRemoteSessions('test_cleanup').catch(() => undefined); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
const timerModuleName = 'node:timers';
const { setTimeout: realTimeout } = await import(/* @vite-ignore */ timerModuleName) as { setTimeout: (fn: () => void, ms: number) => unknown };
async function until(check: () => boolean) { for (let i = 0; i < 200; i++) { if (check()) return; await new Promise<void>(resolve => realTimeout(resolve, 5)); } throw new Error('Expected request did not occur'); }
async function successfulBootstrap(init: RequestInit) {
  const body = JSON.parse(init.body as string);
  const request = { ...body, controller_device_id: protocolFixture.browser.device_id };
  delete request.controller_public_key;
  const keyId = await protocol.sha256Hex(new Uint8Array(body.controller_public_key));
  const expires = Date.now() + 600000;
  return new Response(JSON.stringify({
    controller_device_id: protocolFixture.browser.device_id, controller_key_id: keyId, expires_at_ms: expires,
    session: { session_id: body.session_id, request, request_commitment: await protocol.wanRequestCommitment(request), status: 'requested' },
    credential: { token: 'browser-only-credential', expires_at_ms: expires, device_id: protocolFixture.browser.device_id, device_key_id: keyId, role: 'Controller' },
    signaling_url: 'wss://signal.example.test/ws', signaling_server_device_id: 'signal-server', signaling_server_key_id: protocolFixture.target.key_id,
    target_key_id: protocolFixture.target.key_id, relay_directory_key_id: relayFixture.key_id, relay_directory_public_key: relayFixture.public_key,
  }));
}

describe('independent browser remote session bootstrap', () => {
  it('requires the actual logged-in user and never contacts a localhost service', async () => {
    const request = vi.fn();
    vi.stubGlobal('fetch', request);
    await expect(createBrowserRemoteSession('123456789012')).rejects.toThrow('请先登录');
    expect(request).not.toHaveBeenCalled();
  });

  it('sends only a temporary public key and scoped H264 request with the user credential', async () => {
    localStorage.setItem('rdesk_access_token', 'user-session-credential');
    const request = vi.fn(async () => new Response(JSON.stringify({ detail: { code: 'target_denied', message: '目标不可访问' } }), { status: 403 }));
    vi.stubGlobal('fetch', request);
    await expect(createBrowserRemoteSession('123456789012', { sessionId: '3a9ac347-d512-494e-ad4a-a139f51bb995' })).rejects.toThrow('目标不可访问');
    expect(request).toHaveBeenCalledTimes(1);
    const [url, init] = request.mock.calls[0]! as unknown as [string, RequestInit];
    expect(url).toMatch(/\/api\/v1\/browser-sessions$/);
    expect(url).not.toContain('127.0.0.1:953');
    expect(init.headers).toMatchObject({ Authorization: 'Bearer user-session-credential' });
    const body = JSON.parse(init.body as string);
    expect(body.controller_public_key).toHaveLength(32);
    expect(body.idempotency_key).toHaveLength(16);
    expect(body.requested_scopes).toEqual(['input.keyboard', 'input.pointer', 'screen.view']);
    expect(body.requested_profile).toMatchObject({ codec: 'h264', fps: 30, width: 1920, height: 1080 });
    expect(body).not.toHaveProperty('private_key');
    expect(body).not.toHaveProperty('device_token');
  });

  it('rejects browsers without an authenticated secure context before sending credentials', async () => {
    localStorage.setItem('rdesk_access_token', 'user-session-credential');
    vi.stubGlobal('isSecureContext', false);
    const request = vi.fn();
    vi.stubGlobal('fetch', request);
    await expect(createBrowserRemoteSession('123456789012')).rejects.toThrow('HTTPS');
    expect(request).not.toHaveBeenCalled();
  });

  it('revokes and discards a successful POST response arriving after closing all sessions', async () => {
    localStorage.setItem('rdesk_access_token', 'user-session-credential');
    let entered = false, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const request = vi.fn(async (url: string, init: RequestInit) => {
      if (url.endsWith('/browser-sessions')) { entered = true; await gate; return successfulBootstrap(init); }
      return new Response(JSON.stringify({ status: 'closed' }));
    });
    vi.stubGlobal('fetch', request);
    const creating = createBrowserRemoteSession(protocolFixture.request.target_device_id, { sessionId: protocolFixture.request.session_id });
    const outcome = creating.then(value => ({ value, error: undefined }), error => ({ value: undefined, error }));
    await until(() => entered);
    await closeAllBrowserRemoteSessions('logout');
    localStorage.removeItem('rdesk_access_token');
    release();
    const result = await outcome;
    expect(result.error?.message).toContain('取消');
    expect(result.value).toBeUndefined();
    expect(request.mock.calls.filter(([url]) => url.endsWith('/close'))).toHaveLength(1);
    const states: string[] = [];
    attachBrowserRemoteSession(protocolFixture.request.session_id, { onState: state => states.push(state.phase), onVideoStream: () => undefined });
    expect(states).toEqual(['failed']);
  });

  it('does not submit a new browser principal when logout happens during key generation', async () => {
    localStorage.setItem('rdesk_access_token', 'user-session-credential');
    const actual = protocol.createBrowserSigningIdentity;
    let entered = false, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(protocol, 'createBrowserSigningIdentity').mockImplementationOnce(async () => { const identity = await actual(); entered = true; await gate; return identity; });
    const request = vi.fn(async () => new Response(JSON.stringify({ detail: 'unexpected request' }), { status: 403 }));
    vi.stubGlobal('fetch', request);
    const creating = createBrowserRemoteSession(protocolFixture.request.target_device_id);
    const outcome = creating.then(value => ({ value, error: undefined }), error => ({ value: undefined, error }));
    await until(() => entered);
    await closeAllBrowserRemoteSessions('logout');
    localStorage.removeItem('rdesk_access_token');
    release();
    const result = await outcome;
    expect(result.error?.message).toContain('取消');
    expect(request).not.toHaveBeenCalled();
  });

  it('preserves unsupported WebRTC failure after actual attachment cleanup', async () => {
    localStorage.setItem('rdesk_access_token', 'user-session-credential');
    const request = vi.fn(async (url: string, init: RequestInit) => url.endsWith('/browser-sessions') ? successfulBootstrap(init) : new Response(JSON.stringify({ status: 'closed' })));
    vi.stubGlobal('fetch', request);
    vi.stubGlobal('RTCPeerConnection', undefined);
    const result = await createBrowserRemoteSession(protocolFixture.request.target_device_id);
    const states: Array<{ phase: string; error?: string }> = [];
    attachBrowserRemoteSession(result.sessionId, { onState: state => states.push(state), onVideoStream: () => undefined });
    await until(() => request.mock.calls.some(([url]) => url.endsWith('/close')));
    await new Promise<void>(resolve => realTimeout(resolve, 10));
    expect(states[states.length - 1]).toMatchObject({ phase: 'failed', error: expect.stringContaining('WebRTC') });
    expect(request.mock.calls.filter(([url]) => url.endsWith('/close'))).toHaveLength(1);
  });
});
