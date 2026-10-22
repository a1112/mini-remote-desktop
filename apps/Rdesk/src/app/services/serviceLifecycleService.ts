/**
 * Service lifecycle management
 *
 * Phase 6: Rdesk no longer owns service lifecycle - mrd-service is the owner.
 * This service now only provides bootstrap behavior and IPC-based status queries.
 * All actual lifecycle operations go through mrd-service IPC commands.
 *
 * For service lifecycle operations (start, stop, restart), use the shell commands:
 * - ipc_service_health: check service status and health
 * - shell_shutdown_service: request service shutdown
 */

import * as tauriAdapter from '../adapters/tauri';
import type {
  AdapterResult,
  AutostartStatus,
  CloseBehavior,
  UiPreferences,
  ServiceStatusInfo,
  AppSettings,
  DecodePolicy,
  DecodePolicyResponse,
  FfmpegInstallResult,
  FfmpegProbeResult,
} from '../adapters/tauri';

// Re-export types from adapter
export type {
  CloseBehavior,
  UiPreferences,
  ServiceStatusInfo,
  AppSettings,
  DecodePolicy,
  DecodePolicyResponse,
  FfmpegInstallResult,
  FfmpegProbeResult,
} from '../adapters/tauri';

/** @deprecated Use DecodePolicy instead */
export type DecoderPolicy = DecodePolicy;

/**
 * Error thrown when a service command fails
 */
export class ServiceError extends Error {
  constructor(message: string, public readonly code?: string) {
    super(message);
    this.name = 'ServiceError';
  }
}

/**
 * Unwrap an adapter result, throwing a ServiceError if failed
 */
function unwrapAdapterResult<T>(result: AdapterResult<T>): T {
  if (result.ok) {
    return result.value;
  }
  throw new ServiceError(result.error.message, result.error.code);
}

function isServiceUnavailable(message: string, code?: string): boolean {
  const normalized = message.toLowerCase();
  if (/denied|busy|invalid_response/i.test(code ?? '') || /access denied|permission denied|pipe instances are busy/.test(normalized)) {
    return false;
  }
  return (
    normalized.includes('connection refused') ||
    normalized.includes('endpoint not found') ||
    normalized.includes('cannot find the file') ||
    normalized.includes('no such file') ||
    /\bos error 2\b/.test(normalized)
  );
}

export const getServiceHealth = async (): Promise<ServiceStatusInfo> => {
  const result = await tauriAdapter.ipcServiceHealth();
  if (result.ok) {
    return result.value;
  }

  if (isServiceUnavailable(result.error.message, result.error.code)) {
    return { running: false, healthy: false, pid: null };
  }

  throw new ServiceError(result.error.message, result.error.code);
};

export const getUiPreferences = async (): Promise<UiPreferences> => {
  return unwrapAdapterResult(await tauriAdapter.getUiPreferences());
};

export const setCloseBehavior = async (closeBehavior: CloseBehavior): Promise<UiPreferences> => {
  return unwrapAdapterResult(await tauriAdapter.setCloseBehavior(closeBehavior));
};

// ============================================================================
// Bootstrap Commands (Phase 6: bootstrap-only behavior)
// ============================================================================

/**
 * Bootstrap mrd-service if not already running via IPC
 *
 * Phase 6: This is the ONLY start method. It checks IPC first,
 * and only spawns the process if service is unreachable.
 * Returns true if bootstrap was performed.
 */
export const bootstrapServiceIfNeeded = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceBootstrapIfNeeded();
  return unwrapAdapterResult(result);
};

/**
 * Wait for service to be healthy (with timeout)
 */
export const waitForServiceHealthy = async (
  timeoutSecs: number
): Promise<boolean> => {
  const result = await tauriAdapter.serviceWaitForHealthy(timeoutSecs);
  return unwrapAdapterResult(result);
};

export const waitForServiceStopped = async (timeoutSecs: number): Promise<boolean> => {
  return unwrapAdapterResult(await tauriAdapter.serviceWaitForStopped(timeoutSecs));
};

export const getServiceAutostart = async (): Promise<AutostartStatus> => {
  return unwrapAdapterResult(await tauriAdapter.shellGetAutostartStatus());
};

export const setServiceAutostart = async (enabled: boolean): Promise<AutostartStatus> => {
  unwrapAdapterResult(await tauriAdapter.shellSetAutostart(enabled));
  return getServiceAutostart();
};

export const quitUiAndStopService = async (): Promise<void> => {
  unwrapAdapterResult(await tauriAdapter.shellQuitUiAndStopService());
};

/**
 * Check if this instance bootstrapped the service
 */
export const didBootstrapService = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceDidBootstrap();
  return unwrapAdapterResult(result);
};

// ============================================================================
// Legacy Commands (deprecated - use shell commands instead)
// ============================================================================

/** @deprecated Use bootstrapServiceIfNeeded instead */
export const startService = async (): Promise<boolean> => {
  await bootstrapServiceIfNeeded();
  if (!await waitForServiceHealthy(30)) {
    throw new ServiceError('后台服务未在规定时间内就绪');
  }
  return true;
};

/** @deprecated Service is no longer owned by Rdesk - use shell_shutdown_service IPC command */
export const stopService = async (): Promise<boolean> => {
  const result = await tauriAdapter.shellShutdownService('graceful');
  unwrapAdapterResult(result);
  if (!await waitForServiceStopped(30)) {
    throw new ServiceError('后台服务未在规定时间内停止');
  }
  return true;
};

/** @deprecated Use getServiceHealth for a complete snapshot. */
export const getServiceStatus = async (): Promise<boolean> => {
  return (await getServiceHealth()).running;
};

/** @deprecated Use getServiceHealth for a complete snapshot. */
export const serviceHealthCheck = async (): Promise<boolean> => {
  return (await getServiceHealth()).healthy;
};

/** @deprecated Service restart is no longer owned by Rdesk */
export const restartServiceWithBackoff = async (
  maxAttempts: number
): Promise<boolean> => {
  let attempt = 0;
  let lastError: unknown;

  while (attempt < maxAttempts) {
    try {
      return await serviceRestart();
    } catch (error) {
      lastError = error;
      attempt += 1;
      if (attempt < maxAttempts) {
        await new Promise((resolve) => setTimeout(resolve, attempt * 250));
      }
    }
  }

  if (lastError instanceof Error) {
    throw lastError;
  }
  throw new ServiceError('restartServiceWithBackoff failed');
};

/** @deprecated Service lifecycle is no longer owned by Rdesk */
export const getServicePid = async (): Promise<number | null> => {
  return (await getServiceHealth()).pid ?? null;
};

/** @deprecated Service restart is no longer owned by Rdesk */
export const serviceRestart = async (): Promise<boolean> => {
  await stopService();
  return startService();
};

/** @deprecated Service guard is no longer needed - mrd-service manages its own lifecycle */
export const startServiceGuard = async (): Promise<string> => {
  throw new Error('startServiceGuard is deprecated. mrd-service manages its own lifecycle.');
};

// ============================================================================
// Convenience exports for backward compatibility (all deprecated)
// ============================================================================

/** @deprecated Use bootstrapServiceIfNeeded instead */
export const serviceStart = startService;

/** @deprecated Use shell_shutdown_service instead */
export const serviceStop = stopService;

/** @deprecated Use shell_get_status instead */
export const serviceStatus = getServiceStatus;

/** @deprecated Use shell_get_status instead */
export const servicePid = getServicePid;

/**
 * Read decode policy
 */
export const getDecodePolicy = async (): Promise<DecodePolicyResponse> => {
  const result = await tauriAdapter.decodePolicy();
  return unwrapAdapterResult(result);
};

/**
 * Set decode policy
 */
export const setDecodePolicy = async (
  decodePolicy: DecodePolicy
): Promise<DecodePolicyResponse> => {
  const result = await tauriAdapter.setDecodePolicy(decodePolicy);
  return unwrapAdapterResult(result);
};

// Keep native FFmpeg operations serialized even when their settings view unmounts.
let ffmpegOperationQueue: Promise<void> = Promise.resolve();

function queueFfmpegOperation<T>(operation: () => Promise<AdapterResult<T>>): Promise<T> {
  const result = ffmpegOperationQueue.then(async () => unwrapAdapterResult(await operation()));
  // A failure belongs to its caller; it must not block later queued operations.
  ffmpegOperationQueue = result.then(() => undefined, () => undefined);
  return result;
}

export const ffmpegProbe = (): Promise<FfmpegProbeResult> => {
  return queueFfmpegOperation(() => tauriAdapter.ffmpegProbe());
};

export const ffmpegDownload = (): Promise<FfmpegInstallResult> => {
  return queueFfmpegOperation(() => tauriAdapter.ffmpegDownload());
};

export const ffmpegResetGoldenSettings = (): Promise<AppSettings> => {
  return queueFfmpegOperation(() => tauriAdapter.ffmpegResetGoldenSettings());
};
