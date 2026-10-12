import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '../../../test/mocks/tauri';
import { openMacosPermissionSettings } from './commands';

// Local Settings navigation must not go through a configured remote bridge.
describe('local macOS permission settings adapter', () => {
  afterEach(() => vi.clearAllMocks());
  it.each(['screen_capture', 'accessibility'] as const)('opens the fixed %s native destination', async permission => {
    getMockInvoke().mockResolvedValue(undefined);
    expect(await openMacosPermissionSettings(permission)).toEqual({ ok: true, value: undefined });
    expect(getMockInvoke()).toHaveBeenCalledWith('open_macos_permission_settings', { permission });
  });
  it('preserves failure instead of claiming authorization', async () => {
    getMockInvoke().mockRejectedValue('Unable to open System Settings');
    expect(await openMacosPermissionSettings('screen_capture')).toEqual({
      ok: false, error: { message: 'Unable to open System Settings' },
    });
  });
});
