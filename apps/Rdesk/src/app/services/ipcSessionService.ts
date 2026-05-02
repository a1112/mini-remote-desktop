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
  role: SessionRole | "unknown";
  state: SessionState;
  transport_kind: TransportKind | string;
  last_error?: string | null;
  sender_active: boolean;
  receiver_active: boolean;
}

export interface SessionBootstrap {
  listen_addr?: string;
  server_name?: string;
  cert_der?: string;
}

export interface SessionRuntimeSnapshot {
  session_id: string;
  role: SessionRole | "unknown";
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
  device_id?: string | null;
  is_registered: boolean;
}

export interface ProbeSnapshot {
  session_id: string;
  frames_received: number;
  frames_decoded: number;
  frames_dropped: number;
  current_fps?: number | null;
  bitrate_mbps?: number | null;
  media_probe_valid?: boolean;
  media_probe_format?: string | null;
  media_probe_width?: number | null;
  media_probe_height?: number | null;
  media_probe_target_fps?: number | null;
  media_probe_target_bitrate_mbps?: number | null;
  media_probe_payload_bytes?: number | null;
  last_media_sequence?: number | null;
  last_media_timestamp_us?: number | null;
  last_media_payload_hash?: string | null;
  last_error?: string | null;
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
 * Start a LAN P2P session as controller; the discovered peer auto-accepts.
 */
export const startLanRemoteSession = async (
  sessionId: string,
  targetDeviceId: string,
  transportKind: TransportKind = "webrtc"
): Promise<string> => {
  const result = await tauriAdapter.ipcStartLanRemoteSession(
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

/**
 * Mark a session failed.
 */
export const failSession = async (
  sessionId: string,
  reason: string
): Promise<string> => {
  const result = await tauriAdapter.ipcFailSession(sessionId, reason);
  return unwrapAdapterResult(result);
};

/**
 * Recover a failed or closed session.
 */
export const recoverSession = async (sessionId: string): Promise<string> => {
  const result = await tauriAdapter.ipcRecoverSession(sessionId);
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
 * List session summaries.
 */
export const listSessions = async (): Promise<SessionInfo[]> => {
  const result = await tauriAdapter.ipcListSessions();
  return unwrapAdapterResult(result);
};

/**
 * Get aggregated runtime snapshot (all sessions)
 */
export const getRuntimeSnapshot = async (): Promise<RuntimeSnapshot> => {
  const result = await tauriAdapter.ipcRuntimeSnapshot();
  return unwrapAdapterResult(result);
};

/**
 * Get probe snapshot data
 */
export const getProbeSnapshot = async (sessionId: string): Promise<ProbeSnapshot> => {
  const result = await tauriAdapter.ipcProbeSnapshot(sessionId);
  return unwrapAdapterResult(result);
};
