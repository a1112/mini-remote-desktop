/**
 * Tauri command adapters
 *
 * This is the SINGLE source of truth for invoking Tauri commands.
 * All frontend code must go through these adapters, not call invoke() directly.
 *
 * When a command is removed or renamed from main.rs, update this file
 * and the change will propagate to all frontend code.
 */

import { invoke } from '@tauri-apps/api/tauri';
import type {
  AdapterResult,
  DeviceInfo,
  DeviceRegistrationResponse,
  DecodePolicy,
  DecodePolicyResponse,
  HardwareInfo,
  ServicePidResponse,
  SessionRuntimeSnapshot,
} from './types';

/**
 * Wrap Tauri invoke with consistent error handling
 */
async function invokeAdapter<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<AdapterResult<T>> {
  try {
    const result = await invoke<T>(command, args);
    return { ok: true, value: result };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, error: { message } };
  }
}

// ============================================================================
// Service Lifecycle Commands
// ============================================================================

/**
 * Start the mrd-service
 */
export async function serviceStart(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_start');
}

/**
 * Stop the mrd-service
 */
export async function serviceStop(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_stop');
}

/**
 * Check if mrd-service is running
 */
export async function serviceStatus(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_status');
}

/**
 * Health check for mrd-service
 */
export async function serviceHealthCheck(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_health_check');
}

/**
 * Wait for service to be healthy
 */
export async function serviceWaitForHealthy(
  timeoutSecs: number
): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_wait_for_healthy', {
    timeoutSecs,
  });
}

/**
 * Restart service with backoff retry
 */
export async function serviceRestartWithBackoff(
  maxAttempts: number
): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_restart_with_backoff', {
    maxAttempts,
  });
}

/**
 * Get service PID
 */
export async function servicePid(): Promise<AdapterResult<number | null>> {
  return invokeAdapter<number | null>('service_pid');
}

/**
 * Restart the service
 */
export async function serviceRestart(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_restart');
}

/**
 * Start service guard (monitoring)
 */
export async function serviceStartGuard(): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('service_start_guard');
}

// ============================================================================
// IPC Device Commands
// ============================================================================

/**
 * Register device via IPC
 */
export async function ipcRegisterDevice(
  deviceId: string,
  deviceName: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_register_device', {
    deviceId,
    deviceName,
  });
}

/**
 * List all devices via IPC
 */
export async function ipcListDevices(): Promise<AdapterResult<DeviceInfo[]>> {
  return invokeAdapter<DeviceInfo[]>('ipc_list_devices');
}

// ============================================================================
// IPC Session Commands
// ============================================================================

/**
 * Start a new session
 */
export async function ipcStartSession(
  sessionId: string,
  targetDeviceId: string,
  transportKind: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_start_session', {
    sessionId,
    targetDeviceId,
    transportKind,
  });
}

/**
 * Accept an incoming session
 */
export async function ipcAcceptSession(
  sessionId: string,
  sourceDeviceId: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_accept_session', {
    sessionId,
    sourceDeviceId,
  });
}

/**
 * Stop a session
 */
export async function ipcStopSession(
  sessionId: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_stop_session', {
    sessionId,
  });
}

/**
 * Get session runtime snapshot
 */
export async function ipcSessionSnapshot(
  sessionId: string
): Promise<AdapterResult<SessionRuntimeSnapshot>> {
  return invokeAdapter<SessionRuntimeSnapshot>('ipc_session_snapshot', {
    sessionId,
  });
}

// ============================================================================
// IPC Media Commands
// ============================================================================

/**
 * Start sender for a session
 */
export async function ipcStartSender(
  sessionId: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_start_sender', {
    sessionId,
  });
}

/**
 * Start receiver for a session
 */
export async function ipcStartReceiver(
  sessionId: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_start_receiver', {
    sessionId,
  });
}

// ============================================================================
// Hardware and Decode Policy Commands
// ============================================================================

/**
 * Get hardware info
 */
export async function getHardwareInfo(): Promise<AdapterResult<HardwareInfo>> {
  return invokeAdapter<HardwareInfo>('get_hardware_info');
}

/**
 * NVDEC runtime probe - now returns error (moved to mrd-service)
 * @deprecated Use mrd-service for NVDEC probing
 */
export async function nvdecRuntimeProbe(): Promise<
  AdapterResult<Record<string, unknown>>
> {
  return invokeAdapter<Record<string, unknown>>('nvdec_runtime_probe');
}

/**
 * Get decode policy - now returns error (use IPC)
 * @deprecated Use IPC to query decode policy from mrd-service
 */
export async function decodePolicy(): Promise<AdapterResult<DecodePolicyResponse>> {
  return invokeAdapter<DecodePolicyResponse>('decode_policy');
}

/**
 * Set decode policy
 */
export async function setDecodePolicy(
  decodePolicy: DecodePolicy
): Promise<AdapterResult<DecodePolicyResponse>> {
  return invokeAdapter<DecodePolicyResponse>('set_decode_policy', {
    decodePolicy,
  });
}

// ============================================================================
// Legacy HTTP-based Device Registration
// ============================================================================

/**
 * Register device via HTTP API (legacy)
 */
export async function registerDevice(params: {
  motherboardSerial: string;
  hostname: string;
  osVersion: string;
  deviceName?: string;
}): Promise<AdapterResult<DeviceRegistrationResponse>> {
  return invokeAdapter<DeviceRegistrationResponse>('register_device', {
    motherboardSerial: params.motherboardSerial,
    hostname: params.hostname,
    osVersion: params.osVersion,
    deviceName: params.deviceName,
  });
}

/**
 * Check if device is registered via HTTP
 */
export async function checkDeviceRegistration(
  motherboardSerial: string
): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('check_device_registration', {
    motherboardSerial,
  });
}

// ============================================================================
// Legacy WebRTC Commands
// ============================================================================

/**
 * List WebRTC sessions via IPC
 */
export async function webrtcSessionListViaIpc(): Promise<AdapterResult<string[]>> {
  return invokeAdapter<string[]>('webrtc_session_list_via_ipc');
}
