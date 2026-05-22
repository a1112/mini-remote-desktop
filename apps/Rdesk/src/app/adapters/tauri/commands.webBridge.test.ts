import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import {
  browserWebrtcPreviewStart,
  browserWebrtcPreviewStop,
  ipcCapabilitySnapshot,
  ipcRefreshLanDiscovery,
  serviceBootstrapIfNeeded,
  shellGetStatus,
} from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

describe('commands service bridge integration', () => {
  beforeEach(() => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
  });

  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('uses the browser service bridge for capability snapshots outside Tauri', async () => {
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'CapabilitySnapshot',
          snapshot: { schema_version: 1, capabilities: [], profiles: [] },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcCapabilitySnapshot();

    expect(result.ok && result.value.schema_version).toBe(1);
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:9532/ipc',
      expect.any(Object)
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it('uses the browser service bridge for LAN discovery refresh outside Tauri', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          response: {
            type: 'LanDiscoverySnapshot',
            snapshot: { peers: [], local_device_id: 'controller', updated_at_ms: 1 },
          },
        }),
      })
    );

    const result = await ipcRefreshLanDiscovery();

    expect(result.ok && result.value.peers).toEqual([]);
  });

  it('treats bridge health as service bootstrap in browser mode', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          status: 'ok',
          service: 'mrd-service',
          bridge_enabled: true,
          bind: '127.0.0.1:9532',
        }),
      })
    );

    const result = await serviceBootstrapIfNeeded();

    expect(result).toEqual({ ok: true, value: false });
  });

  it('uses the browser service bridge for shell status outside Tauri', async () => {
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'ShellStatus',
          status: {
            service_pid: 5300,
            ui_pid: null,
            tray_available: true,
            autostart_enabled: null,
            active_session_count: 0,
            last_error: null,
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await shellGetStatus();

    expect(result.ok && result.value.service_pid).toBe(5300);
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:9532/ipc',
      expect.any(Object)
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it('uses the browser service bridge for WebRTC preview start and stop outside Tauri', async () => {
    const invoke = getMockInvoke();
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          session_id: 'local-display-test-1',
          answer_sdp: 'answer-sdp',
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ stopped: true }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const start = await browserWebrtcPreviewStart({
      sessionId: 'local-display-test-1',
      offerSdp: 'offer-sdp',
      fps: 120,
      h264Profile: 'high',
    });
    const stop = await browserWebrtcPreviewStop('local-display-test-1');

    expect(start.ok && start.value.answer_sdp).toBe('answer-sdp');
    expect(stop.ok).toBe(true);
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      'http://127.0.0.1:9532/browser/webrtc-preview/start',
      expect.objectContaining({ method: 'POST' })
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      'http://127.0.0.1:9532/browser/webrtc-preview/stop',
      expect.objectContaining({ method: 'POST' })
    );
    expect(invoke).not.toHaveBeenCalled();
  });
});
