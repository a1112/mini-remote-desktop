import { invoke } from "@tauri-apps/api/core";

/**
 * Realtime service status (DEPRECATED - realtime_* commands removed)
 * Use mrd-service IPC interface instead.
 */
export type RealtimeStatus = {
  running: boolean;
  reachable: boolean;
  status: string;
  pid: number | null;
};

export type NvdecCapabilityProbe = {
  codec: string;
  bit_depth_minus8: number;
  chroma_format: number;
  runtime_supported: boolean;
  runtime_reason: string;
  wired_supported: boolean;
  wired_reason: string;
};

export type NvdecRuntimeProbe = {
  backend: string;
  summary: string;
  checked_items: string[];
  capability_probes: NvdecCapabilityProbe[];
};

export type DecoderPolicy = "auto" | "software" | "d3d11va" | "nvdec";

export type DecodePolicyResponse = {
  decode_policy: DecoderPolicy;
};

/**
 * mrd-service status response
 */
export type ServiceStatus = {
  running: boolean;
  healthy: boolean;
  pid?: number;
};

// ============================================================================
// DEPRECATED: Realtime sidecar commands (removed in hard-cut migration)
// ============================================================================

/**
 * @deprecated realtime_status command removed - use mrd-service IPC instead
 */
export const getRealtimeStatus = async (): Promise<RealtimeStatus> => {
  throw new Error(
    "realtime_status 命令已移除。请使用 mrd-service IPC 接口代替。"
  );
};

/**
 * @deprecated realtime_start command removed - use mrd-service lifecycle instead
 */
export const startRealtime = async (): Promise<RealtimeStatus> => {
  throw new Error(
    "realtime_start 命令已移除。请使用 mrd-service lifecycle 代替。"
  );
};

/**
 * @deprecated realtime_stop command removed - use mrd-service lifecycle instead
 */
export const stopRealtime = async (): Promise<RealtimeStatus> => {
  throw new Error(
    "realtime_stop 命令已移除。请使用 mrd-service lifecycle 代替。"
  );
};

/**
 * @deprecated realtime_restart command removed - use mrd-service lifecycle instead
 */
export const restartRealtime = async (): Promise<RealtimeStatus> => {
  throw new Error(
    "realtime_restart 命令已移除。请使用 mrd-service lifecycle 代替。"
  );
};

// ============================================================================
// NVDEC runtime probe (moved to rdesk-legacy-harness)
// ============================================================================

/**
 * @deprecated nvdec_runtime_probe moved to mrd-service
 * Use rdesk-legacy-harness for testing only
 */
export const getNvdecRuntimeProbe = async (): Promise<NvdecRuntimeProbe> => {
  try {
    return await invoke<NvdecRuntimeProbe>("nvdec_runtime_probe");
  } catch (error) {
    // nvdec_runtime_probe now returns an error in production builds
    if (error instanceof Error && error.message.includes("moved to mrd-service")) {
      throw new Error(
        "NVDEC runtime probe 已迁移到 mrd-service。仅在测试时使用 rdesk-legacy-harness。"
      );
    }
    throw error;
  }
};

// ============================================================================
// Decode policy (now managed by mrd-service via IPC)
// ============================================================================

/**
 * @deprecated decode_policy now managed by mrd-service
 */
export const getDecodePolicy = async (): Promise<DecodePolicyResponse> => {
  try {
    return await invoke<DecodePolicyResponse>("decode_policy");
  } catch (error) {
    // decode_policy now returns an error - use IPC to mrd-service instead
    if (error instanceof Error && error.message.includes("Use IPC to query")) {
      throw new Error(
        "Decode policy 现在由 mrd-service 管理。请使用 IPC 接口查询。"
      );
    }
    throw error;
  }
};

/**
 * Set decode policy (still saves to settings file)
 * Note: Actual policy application happens in mrd-service
 */
export const setDecodePolicy = async (
  decodePolicy: DecoderPolicy
): Promise<DecodePolicyResponse> =>
  invoke<DecodePolicyResponse>("set_decode_policy", {
    decodePolicy,
  });

// ============================================================================
// mrd-service lifecycle commands (new)
// ============================================================================

/**
 * Start the mrd-service process
 */
export const serviceStart = async (): Promise<boolean> =>
  invoke<boolean>("service_start");

/**
 * Stop the mrd-service process
 */
export const serviceStop = async (): Promise<boolean> =>
  invoke<boolean>("service_stop");

/**
 * Check if mrd-service is running
 */
export const serviceStatus = async (): Promise<boolean> =>
  invoke<boolean>("service_status");

/**
 * Perform health check on mrd-service
 */
export const serviceHealthCheck = async (): Promise<boolean> =>
  invoke<boolean>("service_health_check");

/**
 * Wait for mrd-service to become healthy (with timeout)
 */
export const serviceWaitForHealthy = async (timeoutSecs: number): Promise<boolean> =>
  invoke<boolean>("service_wait_for_healthy", { timeoutSecs });

/**
 * Restart mrd-service with backoff retry
 */
export const serviceRestartWithBackoff = async (maxAttempts: number): Promise<boolean> =>
  invoke<boolean>("service_restart_with_backoff", { maxAttempts });

/**
 * Get mrd-service process ID
 */
export const servicePid = async (): Promise<number | null> =>
  invoke<number | null>("service_pid");

/**
 * Restart mrd-service
 */
export const serviceRestart = async (): Promise<boolean> =>
  invoke<boolean>("service_restart");

/**
 * Start service guard (monitors and auto-restarts service)
 */
export const serviceStartGuard = async (): Promise<string> =>
  invoke<string>("service_start_guard");
