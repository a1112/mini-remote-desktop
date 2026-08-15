import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import { crossE2EInjectFault, ipcSendControlInput } from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

describe('control input command adapter', () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('sends typed control input events through the secure Tauri contract', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      type: 'ControlInputAccepted',
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
    expect(invoke).toHaveBeenCalledWith('ipc_secure_remote', {
      request: {
        type: 'SendControlInput',
        session_id: 'session-1',
        event,
      },
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
      'mouse_horizontal_wheel',
      { kind: 'mouse_horizontal_wheel', delta: 120 },
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
      type: 'ControlInputAccepted',
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
    expect(invoke).toHaveBeenCalledWith('ipc_secure_remote', {
      request: {
        type: 'SendControlInput',
        session_id: 'session-1',
        event,
      },
    });
  });

  it('serializes reliable input for one session in invocation order', async () => {
    const invoke = getMockInvoke();
    let resolveFirst!: (value: unknown) => void;
    invoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          })
      )
      .mockResolvedValue({
        type: 'ControlInputAccepted',
        session_id: 'ordered-session',
        lane: 'reliable',
        event_count: 1,
      });
    const keyDown = {
      kind: 'key',
      key: { kind: 'virtual_key', code: 0x41 },
      pressed: true,
    } as const;
    const keyUp = { ...keyDown, pressed: false } as const;

    const down = ipcSendControlInput('ordered-session', keyDown);
    const up = ipcSendControlInput('ordered-session', keyUp);
    await Promise.resolve();
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledTimes(1);

    resolveFirst({
      type: 'ControlInputAccepted',
      session_id: 'ordered-session',
      lane: 'reliable',
      event_count: 1,
    });
    await down;
    await up;

    expect(invoke.mock.calls.map((call) => call[1]?.request?.event)).toEqual([
      keyDown,
      keyUp,
    ]);
  });

  it('preserves typed remote-access failures from the secure Tauri contract', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      type: 'RemoteAccessError',
      session_id: 'session-1',
      peer_key_id: 'target-key',
      failure: {
        code: 'scope_denied',
        message: 'keyboard scope is not granted',
        suggested_action: null,
      },
    });

    const result = await ipcSendControlInput('session-1', {
      kind: 'key',
      key: { kind: 'virtual_key', code: 0x41 },
      pressed: true,
    });

    expect(result).toEqual({
      ok: false,
      error: {
        code: 'scope_denied',
        message: 'keyboard scope is not granted',
      },
    });
  });

  it('blocks control input locally in the browser without contacting the service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    const event = { kind: 'mouse_move', x: 42, y: 24 } as const;

    const result = await ipcSendControlInput('session-1', event);

    expect(result).toEqual({
      ok: false,
      error: {
        code: 'E_WEB_BRIDGE_FORBIDDEN',
        message: 'remote control input is available only in the trusted desktop runtime',
      },
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('injects cross-device E2E faults through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      session_id: 'session-1',
      fault_type: 'renderer.detach_surface',
      status: 'injected',
      message: 'detached 1 native render surface(s)',
      affected_surface_ids: ['surface-1'],
    });

    const result = await crossE2EInjectFault('session-1', 'renderer.detach_surface', 250);

    expect(result.ok && result.value.message).toBe('detached 1 native render surface(s)');
    expect(invoke).toHaveBeenCalledWith('cross_e2e_inject_fault', {
      sessionId: 'session-1',
      faultType: 'renderer.detach_surface',
      durationMs: 250,
    });
  });

  it('injects cross-device E2E faults through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'CrossE2EFaultInjected',
          result: {
            session_id: 'session-1',
            fault_type: 'network.pause_peer',
            status: 'injected',
            message: 'recorded test network pause impairment for 500 ms',
            duration_ms: 500,
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await crossE2EInjectFault('session-1', 'network.pause_peer', 500);
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.duration_ms).toBe(500);
    expect(requestBody.request).toEqual({
      type: 'CrossE2EInjectFault',
      session_id: 'session-1',
      fault_type: 'network.pause_peer',
      duration_ms: 500,
    });
    expect(invoke).not.toHaveBeenCalled();
  });
});
