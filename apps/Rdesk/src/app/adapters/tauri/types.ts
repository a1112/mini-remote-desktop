/**
 * Tauri adapter types
 *
 * Defines the contract between frontend and Tauri shell.
 * This is the single source of truth for IPC command shapes.
 */

/**
 * Service lifecycle responses
 */
export interface ServiceStatusResponse {
  is_running: boolean;
}

export interface ServiceHealthResponse {
  healthy: boolean;
}

export interface ServicePidResponse {
  pid: number | null;
}

/**
 * IPC Device types
 */
export interface DeviceInfo {
  device_id: string;
  device_name: string;
}

export interface DeviceRegistrationResponse {
  device_id: string;
  device_name: string;
  access_token: string;
}

/**
 * IPC Session types
 */
export interface SessionRuntimeSnapshot {
  session_id: string;
  state: string;
  sender_active: boolean;
  receiver_active: boolean;
}

/**
 * Hardware info
 */
export interface HardwareInfo {
  cpu_brand: string;
  cpu_cores: u32;
  memory_gb: u32;
  gpu_info: string;
}

/**
 * Decode policy
 */
export type DecodePolicy = 'auto' | 'software' | 'd3d11va' | 'nvdec';

export interface DecodePolicyResponse {
  decode_policy: DecodePolicy;
}

/**
 * Error response shape from Tauri commands
 */
export interface TauriError {
  code?: string;
  message: string;
}

/**
 * Result type for adapter responses
 */
export type AdapterResult<T> = Result<T, TauriError>;
