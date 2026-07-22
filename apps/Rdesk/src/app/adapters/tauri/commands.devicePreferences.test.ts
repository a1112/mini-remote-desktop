import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import {
  ipcGetDevicePreferences,
  ipcUpdateDevicePreference,
} from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

describe('device preference command adapter', () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('gets service-owned device preferences through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue([
      {
        device_id: 'agent-device',
        favorite: true,
        disabled: false,
        removed: false,
      },
    ]);

    const result = await ipcGetDevicePreferences();

    expect(result.ok && result.value[0]?.device_id).toBe('agent-device');
    expect(invoke).toHaveBeenCalledWith('ipc_get_device_preferences', undefined);
  });

  it('updates service-owned device preferences through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      device_id: 'agent-device',
      favorite: true,
      disabled: true,
      removed: false,
    });

    const result = await ipcUpdateDevicePreference('agent-device', {
      favorite: true,
      disabled: true,
    });

    expect(result.ok && result.value.disabled).toBe(true);
    expect(invoke).toHaveBeenCalledWith('ipc_update_device_preference', {
      deviceId: 'agent-device',
      update: {
        favorite: true,
        disabled: true,
      },
    });
  });

  it('updates service-owned device preferences through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'DevicePreferenceUpdated',
          preference: {
            device_id: 'agent-device',
            favorite: false,
            disabled: false,
            removed: true,
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcUpdateDevicePreference('agent-device', {
      removed: true,
    });
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.removed).toBe(true);
    expect(requestBody.request).toEqual({
      type: 'UpdateDevicePreference',
      device_id: 'agent-device',
      update: {
        removed: true,
      },
    });
    expect(invoke).not.toHaveBeenCalled();
  });
});
