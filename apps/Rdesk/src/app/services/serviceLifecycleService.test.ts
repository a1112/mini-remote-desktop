import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '../../test/mocks/tauri';
import { serviceRestart, serviceStop } from './serviceLifecycleService';

describe('confirmed service lifecycle', () => {
  beforeEach(() => vi.clearAllMocks());

  it('confirms that shutdown finished before bootstrapping a restart', async () => {
    const invoke = getMockInvoke();
    let confirmStopped!: (stopped: boolean) => void;
    invoke.mockImplementation((command: string) => {
      if (command === 'shell_shutdown_service') return Promise.resolve(undefined);
      if (command === 'service_wait_for_stopped') {
        return new Promise<boolean>((resolve) => { confirmStopped = resolve; });
      }
      return Promise.resolve(true);
    });

    const restart = serviceRestart();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('service_wait_for_stopped', { timeoutSecs: 30 }));
    expect(invoke).not.toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
    confirmStopped(true);
    await expect(restart).resolves.toBe(true);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      'shell_shutdown_service', 'service_wait_for_stopped', 'service_bootstrap_if_needed', 'service_wait_for_healthy',
    ]);
  });

  it('keeps a failed shutdown visible and never starts another service', async () => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => command === 'service_wait_for_stopped'
      ? Promise.resolve(false) : Promise.resolve(true));

    await expect(serviceRestart()).rejects.toThrow('后台服务未在规定时间内停止');
    expect(invoke).not.toHaveBeenCalledWith('service_bootstrap_if_needed', undefined);
  });

  it('reports an unhealthy restart instead of reporting success', async () => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => Promise.resolve(command !== 'service_wait_for_healthy'));

    await expect(serviceRestart()).rejects.toThrow('后台服务未在规定时间内就绪');
  });

  it('stop waits for actual termination after the acknowledgement', async () => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => command === 'service_wait_for_stopped'
      ? Promise.resolve(false) : Promise.resolve(undefined));

    await expect(serviceStop()).rejects.toThrow('后台服务未在规定时间内停止');
  });

  it('does not mistake pipe access errors for an absent service', async () => {
    const invoke = getMockInvoke();
    invoke.mockRejectedValue(new Error('named pipe access denied'));
    const { serviceStatus } = await import('./serviceLifecycleService');
    await expect(serviceStatus()).rejects.toThrow('named pipe access denied');
  });

  it('does not mistake a busy IPC endpoint for a missing file error', async () => {
    const invoke = getMockInvoke();
    invoke.mockRejectedValue(new Error('All pipe instances are busy. (os error 231)'));
    const { serviceStatus } = await import('./serviceLifecycleService');
    await expect(serviceStatus()).rejects.toThrow('os error 231');
  });
});
