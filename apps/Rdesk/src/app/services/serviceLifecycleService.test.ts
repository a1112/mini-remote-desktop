import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getMockInvoke } from '../../test/mocks/tauri';
import { ffmpegDownload, ffmpegProbe, ffmpegResetGoldenSettings, getServiceHealth, getUiPreferences, setCloseBehavior, serviceHealthCheck, servicePid, serviceRestart, serviceStatus, serviceStop } from './serviceLifecycleService';

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

describe('shared FFmpeg operation queue', () => {
  const probe = { available: true, ffmpeg_version: 'ffmpeg 8' };
  const installation = { install_dir: 'C:/ffmpeg', probe };
  const settings = {
    decode_policy: 'auto',
    ffmpeg: {
      enabled: true,
      channel: 'stable',
      download: { archive_url: 'https://example.test/ffmpeg.zip', require_sha256: true },
    },
  };

  it('keeps probe and reset queued until an active download and then probe finish', async () => {
    const invoke = getMockInvoke();
    let finishDownload!: (value: unknown) => void;
    let finishProbe!: (value: unknown) => void;
    const downloadPending = new Promise((resolve) => { finishDownload = resolve; });
    const probePending = new Promise((resolve) => { finishProbe = resolve; });
    invoke.mockImplementation((command: string) => {
      if (command === 'ffmpeg_download') return downloadPending;
      if (command === 'ffmpeg_probe') return probePending;
      return Promise.resolve(settings);
    });

    const downloaded = ffmpegDownload();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('ffmpeg_download', undefined));
    const probed = ffmpegProbe();
    const reset = ffmpegResetGoldenSettings();
    try {
      await Promise.resolve();
      expect(invoke.mock.calls.map(([command]) => command)).toEqual(['ffmpeg_download']);
      finishDownload(installation);
      await expect(downloaded).resolves.toEqual(installation);
      await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('ffmpeg_probe', undefined));
      expect(invoke.mock.calls.map(([command]) => command)).toEqual(['ffmpeg_download', 'ffmpeg_probe']);
      finishProbe(probe);
      await expect(probed).resolves.toEqual(probe);
      await expect(reset).resolves.toEqual(settings);
      expect(invoke.mock.calls.map(([command]) => command)).toEqual([
        'ffmpeg_download', 'ffmpeg_probe', 'ffmpeg_reset_golden_settings',
      ]);
    } finally {
      finishDownload(installation);
      finishProbe(probe);
      await Promise.all([downloaded, probed, reset]);
    }
  });

  it('releases the queue after a failed download while preserving its rejection', async () => {
    const invoke = getMockInvoke();
    let failDownload!: (error: unknown) => void;
    const downloadPending = new Promise((_resolve, reject) => { failDownload = reject; });
    invoke.mockImplementation((command: string) => command === 'ffmpeg_download'
      ? downloadPending : Promise.resolve(probe));

    const downloaded = ffmpegDownload();
    const rejected = expect(downloaded).rejects.toMatchObject({
      code: 'E_DOWNLOAD_FAILED', message: 'E_DOWNLOAD_FAILED: archive checksum mismatch',
    });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('ffmpeg_download', undefined));
    const probed = ffmpegProbe();
    try {
      await Promise.resolve();
      expect(invoke.mock.calls.map(([command]) => command)).toEqual(['ffmpeg_download']);
    } finally {
      failDownload('E_DOWNLOAD_FAILED: archive checksum mismatch');
      await rejected;
      await expect(probed).resolves.toEqual(probe);
    }
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(['ffmpeg_download', 'ffmpeg_probe']);
  });

  it('runs repeated downloads in FIFO order instead of rejecting or merging queued requests', async () => {
    const invoke = getMockInvoke();
    let finishDownload!: (value: unknown) => void;
    const downloadPending = new Promise((resolve) => { finishDownload = resolve; });
    let downloadsStarted = 0;
    const secondInstallation = { ...installation, install_dir: 'C:/ffmpeg/second' };
    invoke.mockImplementation((command: string) => {
      if (command === 'ffmpeg_download') {
        downloadsStarted += 1;
        return downloadsStarted === 1 ? downloadPending : Promise.resolve(secondInstallation);
      }
      return Promise.resolve(probe);
    });

    const first = ffmpegDownload();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('ffmpeg_download', undefined));
    const second = ffmpegDownload();
    const probed = ffmpegProbe();
    try {
      await Promise.resolve();
      expect(downloadsStarted).toBe(1);
      expect(invoke.mock.calls.map(([command]) => command)).toEqual(['ffmpeg_download']);
    } finally {
      finishDownload(installation);
      await expect(first).resolves.toEqual(installation);
      await expect(second).resolves.toEqual(secondInstallation);
      await expect(probed).resolves.toEqual(probe);
    }
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      'ffmpeg_download', 'ffmpeg_download', 'ffmpeg_probe',
    ]);
  });
});

describe('reported service health', () => {
  afterEach(() => {
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    vi.unstubAllGlobals();
  });
  it('returns running, unhealthy and pid from one authoritative snapshot', async () => {
    const invoke = getMockInvoke();
    const status = { running: true, healthy: false, pid: 5300 };
    invoke.mockResolvedValue(status);

    await expect(getServiceHealth()).resolves.toEqual(status);
    expect(invoke.mock.calls).toEqual([['ipc_service_health', undefined]]);
  });

  it('keeps an unhealthy running service unhealthy in compatibility queries', async () => {
    const invoke = getMockInvoke();
    invoke.mockImplementation((command: string) => Promise.resolve(command === 'ipc_service_health'
      ? { running: true, healthy: false, pid: 5300 }
      : { service_pid: 5300, last_error: null }));

    await expect(serviceStatus()).resolves.toBe(true);
    await expect(serviceHealthCheck()).resolves.toBe(false);
    await expect(servicePid()).resolves.toBe(5300);
    expect(invoke.mock.calls.every(([command]) => command === 'ipc_service_health')).toBe(true);
  });

  it.each(['IPC connection refused', 'The system cannot find the file specified. (os error 2)', 'No such file or directory', 'IPC endpoint not found']) (
    'reports a stopped service when its endpoint is absent: %s', async (message) => {
      getMockInvoke().mockRejectedValue(new Error(message));
      await expect(getServiceHealth()).resolves.toEqual({ running: false, healthy: false, pid: null });
    },
  );

  it.each(['named pipe access denied', 'All pipe instances are busy. (os error 231)']) (
    'keeps query errors visible: %s', async (message) => {
      getMockInvoke().mockRejectedValue(new Error(message));
      await expect(getServiceHealth()).rejects.toThrow(message);
    },
  );

  it('preserves a management denial code even when its detail mentions an absent endpoint', async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({
      response: {
        type: 'Error',
        code: 'E_MANAGEMENT_COMMAND_DENIED',
        message: 'Access denied while checking an endpoint: connection refused',
      },
    }), { status: 200 })));

    await expect(getServiceHealth()).rejects.toMatchObject({ code: 'E_MANAGEMENT_COMMAND_DENIED' });
  });

  it.each(['E_MANAGEMENT_COMMAND_DENIED', 'E_IPC_BUSY', 'E_INVALID_RESPONSE'])(
    'preserves native error prefix %s instead of reporting a missing service', async (code) => {
      const message = `${code}: endpoint not found`;
      getMockInvoke().mockRejectedValue(message);

      await expect(getServiceHealth()).rejects.toMatchObject({ code, message });
    },
  );

  it('preserves a structured native denial instead of replacing it with generic object text', async () => {
    const error = { code: 'E_MANAGEMENT_COMMAND_DENIED', message: 'endpoint not found' };
    getMockInvoke().mockRejectedValue(error);

    await expect(getServiceHealth()).rejects.toMatchObject(error);
  });

  it('returns native confirmed preferences from the service facade', async () => {
    (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const invoke = getMockInvoke();
    invoke.mockResolvedValueOnce({ close_behavior: 'hide_to_tray' })
      .mockResolvedValueOnce({ close_behavior: 'exit_ui' });

    await expect(getUiPreferences()).resolves.toEqual({ close_behavior: 'hide_to_tray' });
    await expect(setCloseBehavior('exit_ui')).resolves.toEqual({ close_behavior: 'exit_ui' });
  });

  it('throws an honest unsupported error for browser close behavior', async () => {
    await expect(getUiPreferences()).rejects.toMatchObject({ code: 'E_UNSUPPORTED' });
    await expect(setCloseBehavior('exit_ui')).rejects.toMatchObject({ code: 'E_UNSUPPORTED' });
  });
});
