/**
 * Tauri command adapters
 *
 * This is the SINGLE source of truth for invoking Tauri commands.
 * All frontend code must go through these adapters, not call invoke() directly.
 *
 * When a command is removed or renamed from main.rs, update this file
 * and the change will propagate to all frontend code.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  AdapterResult,
  DeviceInfo,
  DeviceRegistrationResponse,
  DecodePolicy,
  DecodePolicyResponse,
  HardwareInfo,
  SessionRuntimeSnapshot,
  HarnessMetrics,
  FrameData,
  // Test Workbench Types
  TestScenario,
  TestRun,
  TestConfig,
  TestStageEvent,
  MetricSeries,
  Artifact,
  TestPreset,
  EnvironmentSnapshot,
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

// ============================================================================
// Test Workbench Commands (New Unified Test API)
// ============================================================================

/**
 * List all available test scenarios
 */
export async function testListScenarios(): Promise<AdapterResult<TestScenario[]>> {
  return invokeAdapter<TestScenario[]>('test_list_scenarios');
}

/**
 * Get current environment capabilities
 */
export async function testGetCapabilities(): Promise<AdapterResult<EnvironmentSnapshot>> {
  return invokeAdapter<EnvironmentSnapshot>('test_get_capabilities');
}

/**
 * Start a test run with specified scenario and config
 */
export async function testStartRun(params: {
  scenarioId: string;
  config: TestConfig;
}): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('test_start_run', {
    scenarioId: params.scenarioId,
    config: params.config,
  });
}

/**
 * Stop a running test
 */
export async function testStopRun(runId: string): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('test_stop_run', { runId });
}

/**
 * List all test runs (with optional filters)
 */
export async function testListRuns(params?: {
  scenarioId?: string;
  status?: string;
  limit?: number;
}): Promise<AdapterResult<TestRun[]>> {
  return invokeAdapter<TestRun[]>('test_list_runs', params);
}

/**
 * Get details of a specific test run
 */
export async function testGetRun(runId: string): Promise<AdapterResult<TestRun | null>> {
  return invokeAdapter<TestRun | null>('test_get_run', { runId });
}

/**
 * Get metrics for a specific test run
 */
export async function testGetRunMetrics(runId: string): Promise<AdapterResult<Record<string, MetricSeries>>> {
  return invokeAdapter<Record<string, MetricSeries>>('test_get_run_metrics', { runId });
}

/**
 * Get stage events for a specific test run
 */
export async function testGetRunEvents(runId: string): Promise<AdapterResult<TestStageEvent[]>> {
  return invokeAdapter<TestStageEvent[]>('test_get_run_events', { runId });
}

/**
 * Get artifacts for a specific test run
 */
export async function testGetRunArtifacts(runId: string): Promise<AdapterResult<Artifact[]>> {
  return invokeAdapter<Artifact[]>('test_get_run_artifacts', { runId });
}

/**
 * List all test presets
 */
export async function testListPresets(): Promise<AdapterResult<TestPreset[]>> {
  return invokeAdapter<TestPreset[]>('test_list_presets');
}

/**
 * Save a new test preset
 */
export async function testSavePreset(params: {
  name: string;
  description: string;
  scenarioId: string;
  config: TestConfig;
}): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('test_save_preset', {
    name: params.name,
    description: params.description,
    scenarioId: params.scenarioId,
    config: params.config,
  });
}

/**
 * Delete a test preset
 */
export async function testDeletePreset(presetId: string): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('test_delete_preset', { presetId });
}

// ============================================================================
// Legacy Test Harness Commands (for backward compatibility)
// ============================================================================

/**
 * Start the test harness
 */
export async function testHarnessStart(chain?: string): Promise<AdapterResult<null>> {
  if (chain) {
    const setChainResult = await testHarnessSetChain(chain);
    if (!setChainResult.ok) {
      return setChainResult;
    }
  }

  return invokeAdapter<null>('test_harness_start');
}

/**
 * Stop the test harness
 */
export async function testHarnessStop(): Promise<AdapterResult<null>> {
  return invokeAdapter<null>('test_harness_stop');
}

/**
 * Set the test chain configuration
 */
export async function testHarnessSetChain(chain: string): Promise<AdapterResult<null>> {
  return invokeAdapter<null>('test_harness_set_chain', { chain });
}

/**
 * Get the current test chain configuration
 */
export async function testHarnessGetChain(): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('test_harness_get_chain');
}

/**
 * Get current test harness metrics
 */
export async function testHarnessGetMetrics(): Promise<AdapterResult<HarnessMetrics>> {
  return invokeAdapter<HarnessMetrics>('test_harness_get_metrics');
}

/**
 * Get latest captured and rendered frames as base64
 */
export async function testHarnessGetFrames(): Promise<
  AdapterResult<[FrameData | null, FrameData | null]>
> {
  return invokeAdapter<[FrameData | null, FrameData | null]>(
    'test_harness_get_frames'
  );
}
