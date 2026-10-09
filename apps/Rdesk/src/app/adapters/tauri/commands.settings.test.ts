import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import * as commands from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

const testWindow = window as Window & {
  __TAURI_INTERNALS__?: unknown;
  __MRD_FORCE_WEB_BRIDGE__?: boolean;
};

describe('settings command contracts', () => {
  afterEach(() => {
    delete testWindow.__TAURI_INTERNALS__;
    delete testWindow.__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('reads the complete native service health snapshot without inferring shell health', async () => {
    const invoke = getMockInvoke();
    const status = { running: true, healthy: false, pid: 5300 };
    invoke.mockResolvedValue(status);

    expect(await commands.ipcServiceHealth()).toEqual({ ok: true, value: status });
    expect(invoke.mock.calls).toEqual([['ipc_service_health', undefined]]);
  });

  it('unwraps the web ServiceHealth status through the explicit bridge', async () => {
    testWindow.__MRD_FORCE_WEB_BRIDGE__ = true;
    const status = { running: true, healthy: false, pid: 5300 };
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      response: { type: 'ServiceHealth', status },
    }), { status: 200 }));
    vi.stubGlobal('fetch', fetch);

    expect(await commands.ipcServiceHealth()).toEqual({ ok: true, value: status });
    expect(JSON.parse(fetch.mock.calls[0]?.[1]?.body ?? '{}')).toEqual({
      request: { type: 'ServiceHealth' },
    });
    expect(getMockInvoke()).not.toHaveBeenCalled();
  });

  it.each([true, undefined, { healthy: true }, { running: true, healthy: true, pid: '5300' }])(
    'rejects incomplete native service health response %j', async (status) => {
      getMockInvoke().mockResolvedValue(status);
      expect(await commands.ipcServiceHealth()).toEqual({
        ok: false,
        error: { code: 'E_INVALID_RESPONSE', message: 'Invalid service health response' },
      });
    },
  );

  it('preserves a bridge denial instead of interpreting it as stopped', async () => {
    testWindow.__MRD_FORCE_WEB_BRIDGE__ = true;
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({
      response: { type: 'Error', code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'ServiceHealth denied' },
    }), { status: 200 })));

    expect(await commands.ipcServiceHealth()).toEqual({
      ok: false, error: { code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'ServiceHealth denied' },
    });
  });

  it.each([
    { rejection: 'E_MANAGEMENT_COMMAND_DENIED: endpoint not found', error: { code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'E_MANAGEMENT_COMMAND_DENIED: endpoint not found' } },
    { rejection: { code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'endpoint not found' }, error: { code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'endpoint not found' } },
  ])('preserves native health rejection codes: $error.code', async ({ rejection, error }) => {
    getMockInvoke().mockRejectedValue(rejection);
    expect(await commands.ipcServiceHealth()).toEqual({ ok: false, error });
  });

  it('gets and saves native close behavior with the registered command argument', async () => {
    testWindow.__TAURI_INTERNALS__ = {};
    const invoke = getMockInvoke();
    invoke.mockResolvedValueOnce({ close_behavior: 'hide_to_tray' })
      .mockResolvedValueOnce({ close_behavior: 'exit_ui' });

    expect(await commands.getUiPreferences()).toEqual({ ok: true, value: { close_behavior: 'hide_to_tray' } });
    expect(await commands.setCloseBehavior('exit_ui')).toEqual({ ok: true, value: { close_behavior: 'exit_ui' } });
    expect(invoke.mock.calls).toEqual([
      ['get_ui_preferences', undefined],
      ['set_close_behavior', { closeBehavior: 'exit_ui' }],
    ]);
  });

  it.each([true, undefined, { close_behavior: 'stop_service' }])(
    'rejects invalid native window preference response %j', async (preferences) => {
      testWindow.__TAURI_INTERNALS__ = {};
      getMockInvoke().mockResolvedValue(preferences);
      const invalid = {
        ok: false,
        error: { code: 'E_INVALID_RESPONSE', message: 'Invalid UI preferences response' },
      };
      expect(await commands.getUiPreferences()).toEqual(invalid);
      expect(await commands.setCloseBehavior('exit_ui')).toEqual(invalid);
    },
  );

  it('keeps failed native preference saves as failures', async () => {
    testWindow.__TAURI_INTERNALS__ = {};
    getMockInvoke().mockRejectedValue(new Error('Cannot persist app-settings.json'));
    expect(await commands.setCloseBehavior('exit_ui')).toEqual({
      ok: false, error: { message: 'Cannot persist app-settings.json' },
    });
  });

  it('reports unsupported browser close behavior without simulated persistence', async () => {
    testWindow.__MRD_FORCE_WEB_BRIDGE__ = true;
    const storage = vi.spyOn(window.localStorage, 'setItem');
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);

    expect(await commands.getUiPreferences()).toMatchObject({ ok: false, error: { code: 'E_UNSUPPORTED' } });
    expect(await commands.setCloseBehavior('exit_ui')).toMatchObject({ ok: false, error: { code: 'E_UNSUPPORTED' } });
    expect(getMockInvoke()).not.toHaveBeenCalled();
    expect(fetch).not.toHaveBeenCalled();
    expect(storage).not.toHaveBeenCalled();
    storage.mockRestore();
  });
});
