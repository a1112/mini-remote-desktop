import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  invokeServiceBridgeIpc,
  resetServiceBridgeConfigForTest,
  serviceBridgeHealth,
} from './client';

describe('service bridge client', () => {
  afterEach(() => {
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('posts IPC envelopes to the local mrd-service bridge', async () => {
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

    const result = await invokeServiceBridgeIpc(
      { type: 'CapabilitySnapshot' },
      (response) => response.snapshot as { schema_version: number }
    );

    expect(result.ok && result.value.schema_version).toBe(1);
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:9532/ipc',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({ request: { type: 'CapabilitySnapshot' } }),
      })
    );
  });

  it('reports bridge unavailable when fetch fails', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('connection refused')));

    const result = await serviceBridgeHealth();

    expect(result.ok).toBe(false);
    expect(result.ok ? '' : result.error.message).toContain('connection refused');
  });

  it('surfaces service-side error responses as adapter errors', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          response: {
            type: 'Error',
            code: 'E_WEB_BRIDGE_FORBIDDEN',
            message: 'ShutdownService is not available through the web bridge.',
          },
        }),
      })
    );

    const result = await invokeServiceBridgeIpc({ type: 'ShutdownService', mode: 'graceful' });

    expect(result.ok).toBe(false);
    expect(result.ok ? '' : result.error.message).toContain('E_WEB_BRIDGE_FORBIDDEN');
  });
});
