/**
 * Service lifecycle management
 *
 * Phase 6: Rdesk no longer owns service lifecycle - mrd-service is the owner.
 * This service now only provides bootstrap behavior and IPC-based status queries.
 * All actual lifecycle operations go through mrd-service IPC commands.
 *
 * For service lifecycle operations (start, stop, restart), use the shell commands:
 * - shell_get_status: check service status
 * - shell_shutdown_service: request service shutdown
 */

import * as tauriAdapter from '../adapters/tauri';
import type {
  AdapterResult,
  AppSettings,
  DecodePolicy,
  DecodePolicyResponse,
  FfmpegInstallResult,
  FfmpegProbeResult,
  ShutdownMode,
  ShellStatusSnapshot,
} from '../adapters/tauri';

// Re-export types from adapter
export type {
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

function isServiceUnavailable(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes('connection refused') ||
    normalized.includes('cannot find the file') ||
    normalized.includes('no such file') ||
    normalized.includes('pipe') ||
    normalized.includes('os error 2')
  );
}

async function getShellStatusSnapshot(): Promise<ShellStatusSnapshot | null> {
  const result = await tauriAdapter.shellGetStatus();
  if (result.ok) {
    return result.value;
  }

  if (isServiceUnavailable(result.error.message)) {
    return null;
  }

  throw new ServiceError(result.error.message, result.error.code);
}

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
export const startService = bootstrapServiceIfNeeded;

/** @deprecated Service is no longer owned by Rdesk - use shell_shutdown_service IPC command */
export const stopService = async (): Promise<boolean> => {
  const result = await tauriAdapter.shellShutdownService('graceful');
  unwrapAdapterResult(result);
  return true;
};

/** @deprecated Use shell_get_status IPC command instead */
export const getServiceStatus = async (): Promise<boolean> => {
  const snapshot = await getShellStatusSnapshot();
  return snapshot !== null;
};

/** @deprecated Use shell_get_status IPC command instead */
export const serviceHealthCheck = async (): Promise<boolean> => {
  const snapshot = await getShellStatusSnapshot();
  return snapshot !== null && snapshot.last_error === null;
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
  const snapshot = await getShellStatusSnapshot();
  return snapshot?.service_pid ?? null;
};

/** @deprecated Service restart is no longer owned by Rdesk */
export const serviceRestart = async (): Promise<boolean> => {
  const stopResult = await tauriAdapter.shellShutdownService('graceful' as ShutdownMode);
  unwrapAdapterResult(stopResult);

  const started = await bootstrapServiceIfNeeded();
  await waitForServiceHealthy(10).catch(() => false);
  return started;
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

export const ffmpegProbe = async (): Promise<FfmpegProbeResult> => {
  const result = await tauriAdapter.ffmpegProbe();
  return unwrapAdapterResult(result);
};

export const ffmpegDownload = async (): Promise<FfmpegInstallResult> => {
  const result = await tauriAdapter.ffmpegDownload();
  return unwrapAdapterResult(result);
};

export const ffmpegResetGoldenSettings = async (): Promise<AppSettings> => {
  const result = await tauriAdapter.ffmpegResetGoldenSettings();
  return unwrapAdapterResult(result);
};
