import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import { ipcDeviceDetail, ipcRequestDeviceAction, ipcSendControlInput } from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

describe('control input command adapter', () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('sends typed control input events through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      session_id: 'session-1',
      lane: 'reliable',
      event_count: 1,
    });
    const event = {
      kind: 'mouse_button',
      button: 'left',
      pressed: true,
    } as const;

    const result = await ipcSendControlInput('session-1', event);

    expect(result).toEqual({
      ok: true,
      value: {
        session_id: 'session-1',
        lane: 'reliable',
        event_count: 1,
      },
    });
    expect(invoke).toHaveBeenCalledWith('ipc_send_control_input', {
      sessionId: 'session-1',
      event,
    });
  });

  it('sends typed control input events through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'ControlInputAccepted',
          session_id: 'session-1',
          lane: 'realtime',
          event_count: 1,
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);
    const event = { kind: 'mouse_move', x: 42, y: 24 } as const;

    const result = await ipcSendControlInput('session-1', event);
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.lane).toBe('realtime');
    expect(requestBody.request).toEqual({
      type: 'SendControlInput',
      session_id: 'session-1',
      event,
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('sends device action requests through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'DeviceActionRequested',
          result: {
            device_id: 'agent-device',
            action: 'remote_terminal',
            accepted: false,
            supported: false,
            message: 'Remote terminal requires a service-owned command channel.',
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcRequestDeviceAction('agent-device', 'remote_terminal');
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.action).toBe('remote_terminal');
    expect(requestBody.request).toEqual({
      type: 'RequestDeviceAction',
      device_id: 'agent-device',
      action: 'remote_terminal',
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('sends disconnect device action requests through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'DeviceActionRequested',
          result: {
            device_id: 'agent-device',
            action: 'disconnect',
            accepted: true,
            supported: true,
            message: 'Disconnected 1 active session(s).',
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcRequestDeviceAction('agent-device', 'disconnect');
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.action).toBe('disconnect');
    expect(requestBody.request).toEqual({
      type: 'RequestDeviceAction',
      device_id: 'agent-device',
      action: 'disconnect',
    });
    expect(invoke).not.toHaveBeenCalled();
  });


  it('requests service-owned device detail through Tauri and browser bridge', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      device_id: 'agent-device',
      device_name: 'Agent PC',
      is_local: false,
      is_online: true,
      is_lan_peer: true,
      is_paired: true,
      transports: ['quic'],
      media_capabilities: ['control.keyboard_mouse'],
    });

    const tauriResult = await ipcDeviceDetail('agent-device');

    expect(tauriResult.ok && tauriResult.value.device_name).toBe('Agent PC');
    expect(invoke).toHaveBeenCalledWith('ipc_device_detail', {
      deviceId: 'agent-device',
    });

    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    invoke.mockClear();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'DeviceDetail',
          detail: {
            device_id: 'agent-device',
            is_local: false,
            is_online: true,
            is_lan_peer: true,
            is_paired: false,
            transports: ['quic'],
            media_capabilities: [],
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const bridgeResult = await ipcDeviceDetail('agent-device');
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(bridgeResult.ok && bridgeResult.value.is_lan_peer).toBe(true);
    expect(requestBody.request).toEqual({
      type: 'GetDeviceDetail',
      device_id: 'agent-device',
    });
    expect(invoke).not.toHaveBeenCalled();
  });
});
