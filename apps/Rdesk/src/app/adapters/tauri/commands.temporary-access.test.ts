import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import * as commands from './commands';

const nativeWindow = window as Window & { __TAURI_INTERNALS__?: unknown; __MRD_FORCE_WEB_BRIDGE__?: boolean };
const status = { enabled: true, ready: true, generation: 12, expires_at_ms: Date.now() + 600000, reason: null };
afterEach(() => { delete nativeWindow.__TAURI_INTERNALS__; delete nativeWindow.__MRD_FORCE_WEB_BRIDGE__; vi.unstubAllGlobals(); });

describe('local temporary access IPC boundary', () => {
  it('does not expose local passwords through a browser or configured web bridge', async () => {
    nativeWindow.__MRD_FORCE_WEB_BRIDGE__ = true;
    const request = vi.fn(); vi.stubGlobal('fetch', request);
    for (const operation of [commands.getTemporaryAccessStatus, commands.readTemporaryAccessPassword, commands.rotateTemporaryAccessPassword, commands.disableTemporaryAccess]) {
      expect(await operation()).toMatchObject({ ok: false, error: { code: 'E_NATIVE_REQUIRED' } });
    }
    expect(getMockInvoke()).not.toHaveBeenCalled(); expect(request).not.toHaveBeenCalled();
  });

  it.each([
    ['getTemporaryAccessStatus', 'ipc_temporary_access_status', status],
    ['readTemporaryAccessPassword', 'ipc_temporary_access_secret', { status, password: 'ABCD2345' }],
    ['rotateTemporaryAccessPassword', 'ipc_temporary_access_rotate', status],
    ['disableTemporaryAccess', 'ipc_temporary_access_disable', { ...status, enabled: false, ready: false }],
  ] as const)('uses the signed native IPC command for %s', async (operation, command, response) => {
    nativeWindow.__TAURI_INTERNALS__ = {};
    nativeWindow.__MRD_FORCE_WEB_BRIDGE__ = true;
    getMockInvoke().mockResolvedValue(response);
    expect(await commands[operation]()).toEqual({ ok: true, value: response });
    expect(getMockInvoke().mock.calls).toEqual([[command, undefined]]);
  });

  it('rejects a secret-bearing reply that claims it is not ready', async () => {
    nativeWindow.__TAURI_INTERNALS__ = {};
    getMockInvoke().mockResolvedValue({ status: { ...status, ready: false }, password: 'ABCD2345' });
    const result = await commands.readTemporaryAccessPassword();
    expect(result).toMatchObject({ ok: false, error: { code: 'E_INVALID_RESPONSE' } });
    expect(JSON.stringify(result)).not.toContain('ABCD2345');
  });

  it.each(['manual', 'automatic'] as const)('preserves the %s refresh mode from the native service', async refresh_mode => {
    nativeWindow.__TAURI_INTERNALS__ = {};
    const response = { ...status, refresh_mode };
    getMockInvoke().mockResolvedValue(response);
    expect(await commands.getTemporaryAccessStatus()).toEqual({ ok: true, value: response });
    getMockInvoke().mockResolvedValue({ status: response, password: 'ABCD2345' });
    expect(await commands.readTemporaryAccessPassword()).toEqual({ ok: true, value: { status: response, password: 'ABCD2345' } });
  });

  it.each(['hourly', '', null, 1, true])('rejects unsupported refresh mode %s without exposing a secret', async refresh_mode => {
    nativeWindow.__TAURI_INTERNALS__ = {};
    const response = { ...status, refresh_mode };
    for (const operation of [commands.getTemporaryAccessStatus, commands.rotateTemporaryAccessPassword, commands.disableTemporaryAccess]) {
      getMockInvoke().mockResolvedValue(response);
      expect(await operation()).toMatchObject({ ok: false, error: { code: 'E_INVALID_RESPONSE' } });
    }
    getMockInvoke().mockResolvedValue({ status: response, password: 'ABCD2345' });
    const result = await commands.readTemporaryAccessPassword();
    expect(result).toMatchObject({ ok: false, error: { code: 'E_INVALID_RESPONSE' } });
    expect(JSON.stringify(result)).not.toContain('ABCD2345');
  });
});
