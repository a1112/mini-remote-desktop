import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import {
  browserWebrtcPreviewStart,
  browserWebrtcPreviewStop,
  ipcCapabilitySnapshot,
  ipcListLocalCaptureSources,
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

  it('uses the browser service bridge for local capture source listing outside Tauri', async () => {
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'LocalCaptureSourceList',
          sources: [
            {
              id: 'windows:display-shared:1',
              platform: 'windows',
              source_kind: 'display_shared',
              title: 'Display 2 (D3D11 shared copy)',
              class_name: 'DXGIShared:\\\\.\\DISPLAY2',
              width: 3840,
              height: 2160,
              process_id: 0,
              app_name: 'Display',
              bundle_identifier: null,
              preview_data_url: null,
              preview_width: null,
              preview_height: null,
            },
          ],
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcListLocalCaptureSources(false, 24);
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value[0].id).toBe('windows:display-shared:1');
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:9532/ipc',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"type":"ListLocalCaptureSources"'),
      })
    );
    expect(requestBody.request).toEqual(
      expect.objectContaining({
        type: 'ListLocalCaptureSources',
        include_previews: false,
        limit: 24,
      })
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
      sourceId: 'windows:display-shared:1',
    });
    const stop = await browserWebrtcPreviewStop('local-display-test-1');
    const startBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(start.ok && start.value.answer_sdp).toBe('answer-sdp');
    expect(stop.ok).toBe(true);
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      'http://127.0.0.1:9532/browser/webrtc-preview/start',
      expect.objectContaining({ method: 'POST' })
    );
    expect(startBody).toEqual(
      expect.objectContaining({
        session_id: 'local-display-test-1',
        source_id: 'windows:display-shared:1',
      })
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      'http://127.0.0.1:9532/browser/webrtc-preview/stop',
      expect.objectContaining({ method: 'POST' })
    );
    expect(invoke).not.toHaveBeenCalled();
  });
});
