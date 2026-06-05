import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import { ipcSendControlInput } from './commands';
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

  it.each([
    [
      'mouse_move',
      { kind: 'mouse_move', x: 42, y: 24 },
      'realtime',
    ],
    [
      'mouse_wheel',
      { kind: 'mouse_wheel', delta: -120 },
      'realtime',
    ],
    [
      'key',
      { kind: 'key', key: { kind: 'virtual_key', code: 0x41 }, pressed: true },
      'reliable',
    ],
  ] as const)('sends %s control input events through the Tauri command', async (_name, event, lane) => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      session_id: 'session-1',
      lane,
      event_count: 1,
    });

    const result = await ipcSendControlInput('session-1', event);

    expect(result).toEqual({
      ok: true,
      value: {
        session_id: 'session-1',
        lane,
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
});
