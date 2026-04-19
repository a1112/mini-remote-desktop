/**
 * IPC-based session service
 *
 * This service uses the new mrd-service IPC interface instead of the old
 * direct realtime_* commands. WebRTC signaling is now handled internally
 * by mrd-service.
 *
 * This service imports from the Tauri adapter layer, not directly from
 * @tauri-apps/api/tauri. This ensures that if commands are removed/renamed
 * in main.rs, the adapter contract tests will fail.
 */

import * as tauriAdapter from '../adapters/tauri';

// ============================================================================
// Types
// ============================================================================

export type SessionRole = "controller" | "agent";
export type TransportKind = "quic" | "webrtc";
export type SessionState =
  | "created"
  | "listening"
  | "connecting"
  | "connected"
  | "streaming"
  | "failed"
  | "closed";

export interface DeviceInfo {
  device_id: string;
  device_name: string;
  is_online: boolean;
}

export interface SessionInfo {
  session_id: string;
  role: SessionRole;
  state: SessionState;
  transport_kind: TransportKind;
}

export interface SessionBootstrap {
  listen_addr?: string;
  server_name?: string;
  cert_der?: string;
}

export interface SessionRuntimeSnapshot {
  session_id: string;
  role: SessionRole;
  state: SessionState;
  transport_kind: TransportKind;
  local_bootstrap?: SessionBootstrap;
  remote_bootstrap?: SessionBootstrap;
  last_error?: string;
  sender_active: boolean;
  receiver_active: boolean;
}

export interface RuntimeSnapshot {
  sessions: SessionRuntimeSnapshot[];
  device_id?: string;
  is_registered: boolean;
}

export interface ProbeSnapshot {
  session_id: string;
  frames_received: number;
  frames_decoded: number;
  frames_dropped: number;
  current_fps?: number;
  bitrate_mbps?: number;
  last_error?: string;
}

/**
 * Error thrown when an adapter command fails
 */
export class ServiceCommandError extends Error {
  constructor(message: string, public readonly code?: string) {
    super(message);
    this.name = 'ServiceCommandError';
  }
}

/**
 * Unwrap an adapter result, throwing a ServiceCommandError if failed
 */
function unwrapAdapterResult<T>(result: tauriAdapter.AdapterResult<T>): T {
  if (result.ok) {
    return result.value;
  }
  throw new ServiceCommandError(result.error.message, result.error.code);
}

// ============================================================================
// Device Commands
// ============================================================================

/**
 * Register this device with mrd-service
 */
export const registerDevice = async (
  deviceId: string,
  deviceName: string
): Promise<string> => {
  const result = await tauriAdapter.ipcRegisterDevice(deviceId, deviceName);
  return unwrapAdapterResult(result);
};

/**
 * List available devices
 */
export const listDevices = async (): Promise<DeviceInfo[]> => {
  const result = await tauriAdapter.ipcListDevices();
  return unwrapAdapterResult(result);
};

// ============================================================================
// Session Commands
// ============================================================================

/**
 * Start a new session as controller
 */
export const startSession = async (
  sessionId: string,
  targetDeviceId: string,
  transportKind: TransportKind = "webrtc"
): Promise<string> => {
  const result = await tauriAdapter.ipcStartSession(
    sessionId,
    targetDeviceId,
    transportKind
  );
  return unwrapAdapterResult(result);
};

/**
 * Accept an incoming session as agent
 */
export const acceptSession = async (
  sessionId: string,
  sourceDeviceId: string
): Promise<string> => {
  const result = await tauriAdapter.ipcAcceptSession(sessionId, sourceDeviceId);
  return unwrapAdapterResult(result);
};

/**
 * Stop a session
 */
export const stopSession = async (sessionId: string): Promise<string> => {
  const result = await tauriAdapter.ipcStopSession(sessionId);
  return unwrapAdapterResult(result);
};

// ============================================================================
// Media Commands
// ============================================================================

/**
 * Start sending media (controller role)
 */
export const startSender = async (sessionId: string): Promise<string> => {
  const result = await tauriAdapter.ipcStartSender(sessionId);
  return unwrapAdapterResult(result);
};

/**
 * Start receiving media (agent role)
 */
export const startReceiver = async (sessionId: string): Promise<string> => {
  const result = await tauriAdapter.ipcStartReceiver(sessionId);
  return unwrapAdapterResult(result);
};

// ============================================================================
// Snapshot Commands
// ============================================================================

/**
 * Get session runtime snapshot
 */
export const getSessionSnapshot = async (
  sessionId: string
): Promise<SessionRuntimeSnapshot> => {
  const result = await tauriAdapter.ipcSessionSnapshot(sessionId);
  return unwrapAdapterResult(result);
};

/**
 * Get aggregated runtime snapshot (all sessions)
 * Note: This command is not yet implemented in main.rs
 */
export const getRuntimeSnapshot = async (): Promise<RuntimeSnapshot> => {
  throw new Error("Runtime snapshot not yet available through IPC");
};

/**
 * Get probe snapshot data
 * Note: This command is not yet implemented in main.rs
 */
export const getProbeSnapshot = async (sessionId: string): Promise<ProbeSnapshot> => {
  void sessionId;
  throw new Error("Probe snapshot not yet available through IPC");
};
