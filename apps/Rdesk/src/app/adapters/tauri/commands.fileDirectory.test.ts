import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import { ipcListDirectory } from './commands';
import { resetServiceBridgeConfigForTest } from '../serviceBridge/client';

describe('file directory command adapter', () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it('lists local service-owned directory entries through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      path: 'C:\\Users\\tester',
      parent_path: 'C:\\Users',
      entries: [
        {
          name: 'Downloads',
          path: 'C:\\Users\\tester\\Downloads',
          kind: 'directory',
          size_bytes: null,
          modified_ms: 1776000000000,
          readonly: false,
        },
      ],
    });

    const result = await ipcListDirectory('C:\\Users\\tester');

    expect(result).toEqual({
      ok: true,
      value: {
        path: 'C:\\Users\\tester',
        parent_path: 'C:\\Users',
        entries: [
          {
            name: 'Downloads',
            path: 'C:\\Users\\tester\\Downloads',
            kind: 'directory',
            size_bytes: null,
            modified_ms: 1776000000000,
            readonly: false,
          },
        ],
      },
    });
    expect(invoke).toHaveBeenCalledWith('ipc_list_directory', {
      path: 'C:\\Users\\tester',
    });
  });

  it('lists directory entries through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'DirectoryList',
          listing: {
            path: '/Users/tester',
            parent_path: '/Users',
            entries: [
              {
                name: 'Downloads',
                path: '/Users/tester/Downloads',
                kind: 'directory',
                size_bytes: null,
                modified_ms: 1776000000000,
                readonly: false,
              },
            ],
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcListDirectory('/Users/tester');
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.entries[0]?.name).toBe('Downloads');
    expect(requestBody.request).toEqual({
      type: 'ListDirectory',
      path: '/Users/tester',
    });
    expect(invoke).not.toHaveBeenCalled();
  });
});
