import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '@/test/mocks/tauri';
import {
  ipcCancelFileTransfer,
  ipcListDirectory,
  ipcListFileTransferProviders,
  ipcListFileTransfers,
  ipcStartFileTransfer,
} from './commands';
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

  it('starts a local file transfer through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      transfer_id: 'file-transfer-1',
      status: 'completed',
      source_device_id: 'source-device',
      target_device_id: 'target-device',
      transport_kind: 'local',
      total_entries: 1,
      copied_entries: 1,
      total_bytes: 5,
      copied_bytes: 5,
      error: null,
      entries: [],
    });

    const request = {
      source_device_id: 'source-device',
      target_device_id: 'target-device',
      entries: [
        {
          source_path: 'C:\\Users\\tester\\source.txt',
          file_name: 'source.txt',
          kind: 'file' as const,
        },
      ],
      target_path: 'C:\\Users\\tester\\Downloads',
      conflict_policy: 'rename' as const,
      transport_hint: 'local',
    };

    const result = await ipcStartFileTransfer(request);

    expect(result.ok && result.value.transfer_id).toBe('file-transfer-1');
    expect(invoke).toHaveBeenCalledWith('ipc_start_file_transfer', { request });
  });

  it('starts a local file transfer through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'FileTransferStarted',
          transfer: {
            transfer_id: 'file-transfer-1',
            status: 'completed',
            source_device_id: 'source-device',
            target_device_id: 'target-device',
            transport_kind: 'local',
            total_entries: 1,
            copied_entries: 1,
            total_bytes: 5,
            copied_bytes: 5,
            error: null,
            entries: [],
          },
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const request = {
      source_device_id: 'source-device',
      target_device_id: 'target-device',
      entries: [
        {
          source_path: '/Users/tester/source.txt',
          file_name: 'source.txt',
          kind: 'file' as const,
        },
      ],
      target_path: '/Users/tester/Downloads',
      conflict_policy: 'rename' as const,
      transport_hint: 'local',
    };

    const result = await ipcStartFileTransfer(request);
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value.status).toBe('completed');
    expect(requestBody.request).toEqual({
      type: 'StartFileTransfer',
      request,
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('lists file transfer tasks through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue([
      {
        transfer_id: 'file-transfer-1',
        status: 'completed',
        transport_kind: 'local',
        total_entries: 1,
        copied_entries: 1,
        total_bytes: 5,
        copied_bytes: 5,
        error: null,
        entries: [],
      },
    ]);

    const result = await ipcListFileTransfers();

    expect(result.ok && result.value[0]?.transfer_id).toBe('file-transfer-1');
    expect(invoke).toHaveBeenCalledWith('ipc_list_file_transfers', undefined);
  });

  it('lists reserved file transfer providers through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue([
      {
        provider_kind: 'mrd-local',
        display_name: 'MRD local file transfer',
        status: 'available',
        capabilities: ['service.file_transfer.local'],
        reason: null,
      },
      {
        provider_kind: 'r-file',
        display_name: 'R-File external bridge',
        status: 'unimplemented',
        capabilities: ['service.file_transfer.external_bridge'],
        reason: 'reserved provider bridge',
      },
    ]);

    const result = await ipcListFileTransferProviders();

    expect(result.ok && result.value[1]?.provider_kind).toBe('r-file');
    expect(result.ok && result.value[1]?.status).toBe('unimplemented');
    expect(invoke).toHaveBeenCalledWith('ipc_list_file_transfer_providers', undefined);
  });

  it('lists reserved file transfer providers through the browser service bridge', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        response: {
          type: 'FileTransferProviderList',
          providers: [
            {
              provider_kind: 'mrd-local',
              display_name: 'MRD local file transfer',
              status: 'available',
              capabilities: ['service.file_transfer.local'],
              reason: null,
            },
            {
              provider_kind: 'r-file',
              display_name: 'R-File external bridge',
              status: 'unimplemented',
              capabilities: ['service.file_transfer.external_bridge'],
              reason: 'reserved provider bridge',
            },
          ],
        },
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const result = await ipcListFileTransferProviders();
    const requestBody = JSON.parse((fetchMock.mock.calls[0]?.[1] as RequestInit).body as string);

    expect(result.ok && result.value[0]?.provider_kind).toBe('mrd-local');
    expect(requestBody.request).toEqual({ type: 'ListFileTransferProviders' });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('cancels a file transfer task through the Tauri command', async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({
      transfer_id: 'file-transfer-1',
      status: 'cancelled',
      transport_kind: 'local',
      total_entries: 1,
      copied_entries: 0,
      total_bytes: 5,
      copied_bytes: 0,
      error: null,
      entries: [],
    });

    const result = await ipcCancelFileTransfer('file-transfer-1');

    expect(result.ok && result.value.status).toBe('cancelled');
    expect(invoke).toHaveBeenCalledWith('ipc_cancel_file_transfer', {
      transferId: 'file-transfer-1',
    });
  });
});
