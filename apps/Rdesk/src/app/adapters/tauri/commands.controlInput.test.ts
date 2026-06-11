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
