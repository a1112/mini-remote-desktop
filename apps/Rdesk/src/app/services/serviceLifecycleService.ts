/**
 * Service lifecycle management
 *
 * Wraps mrd-service lifecycle commands through the Tauri adapter.
 * This is the only place in the frontend that should call these commands.
 */

import * as tauriAdapter from '../adapters/tauri';

// Re-export types from adapter
export type { DecodePolicy, DecodePolicyResponse } from '../adapters/tauri';

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
function unwrapAdapterResult<T>(result: tauriAdapter.AdapterResult<T>): T {
  if (result.ok) {
    return result.value;
  }
  throw new ServiceError(result.error.message, result.error.code);
}

// ============================================================================
// Service Lifecycle Commands
// ============================================================================

/**
 * Start the mrd-service
 */
export const startService = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceStart();
  return unwrapAdapterResult(result);
};

/**
 * Stop the mrd-service
 */
export const stopService = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceStop();
  return unwrapAdapterResult(result);
};

/**
 * Check if mrd-service is running
 */
export const getServiceStatus = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceStatus();
  return unwrapAdapterResult(result);
};

/**
 * Perform health check on mrd-service
 */
export const serviceHealthCheck = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceHealthCheck();
  return unwrapAdapterResult(result);
};

/**
 * Wait for service to be healthy
 */
export const waitForServiceHealthy = async (
  timeoutSecs: number
): Promise<boolean> => {
  const result = await tauriAdapter.serviceWaitForHealthy(timeoutSecs);
  return unwrapAdapterResult(result);
};

/**
 * Restart service with backoff retry
 */
export const restartServiceWithBackoff = async (
  maxAttempts: number
): Promise<boolean> => {
  const result = await tauriAdapter.serviceRestartWithBackoff(maxAttempts);
  return unwrapAdapterResult(result);
};

/**
 * Get service PID
 */
export const getServicePid = async (): Promise<number | null> => {
  const result = await tauriAdapter.servicePid();
  return unwrapAdapterResult(result);
};

/**
 * Restart the service
 */
export const serviceRestart = async (): Promise<boolean> => {
  const result = await tauriAdapter.serviceRestart();
  return unwrapAdapterResult(result);
};

/**
 * Start service guard (monitoring)
 */
export const startServiceGuard = async (): Promise<string> => {
  const result = await tauriAdapter.serviceStartGuard();
  return unwrapAdapterResult(result);
};

// ============================================================================
// Convenience exports for backward compatibility
// ============================================================================

/** @deprecated Use startService instead */
export const serviceStart = startService;

/** @deprecated Use stopService instead */
export const serviceStop = stopService;

/** @deprecated Use getServiceStatus instead */
export const serviceStatus = getServiceStatus;

/** @deprecated Use getServicePid instead */
export const servicePid = getServicePid;

/**
 * Set decode policy
 * @deprecated Decode policy is now managed by mrd-service
 */
export const setDecodePolicy = async (
  decodePolicy: DecodePolicy
): Promise<DecodePolicyResponse> => {
  const result = await tauriAdapter.setDecodePolicy(decodePolicy);
  return unwrapAdapterResult(result);
};
