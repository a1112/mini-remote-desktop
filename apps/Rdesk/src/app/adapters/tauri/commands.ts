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
  ClientDiagnostics,
  DeviceInfo,
  LanDiscoverySnapshot,
  DeviceRegistrationResponse,
  DecodePolicy,
  DecodePolicyResponse,
  HardwareInfo,
  SystemResourceSnapshot,
  NativeBackdropStatus,
  ShutdownMode,
  ShellStatusSnapshot,
  SessionInfo,
  SessionRuntimeSnapshot,
  RuntimeSnapshot,
  CapabilitySnapshot,
  CaptureSource,
  CaptureSourceSelection,
  MediaProfile,
  MediaProfileNegotiation,
  ProbeSnapshot,
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
  WindowCaptureTarget,
  RemoteDisplayWindowContext,
  NativeSurfaceRect,
  NativeRenderSurfaceSnapshot,
  BrowserWebrtcPreviewAnswer,
  TestMatrixConfig,
  PipelineComparisonResult,
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

function isLocalBrowserFallbackAllowed(): boolean {
  if (typeof window === 'undefined') return false;
  if ('__TAURI_INTERNALS__' in window) return false;
  const host = window.location.hostname;
  return host === 'localhost' || host === '127.0.0.1' || host === '::1';
}

function browserDevCapabilities(): EnvironmentSnapshot | null {
  if (!isLocalBrowserFallbackAllowed()) return null;

  const userAgent = navigator.userAgent.toLowerCase();
  const platform = navigator.platform.toLowerCase();
  const isMac = platform.includes('mac') || userAgent.includes('mac os x');
  const isWindows = platform.includes('win') || userAgent.includes('windows');

  return {
    os_type: isMac ? 'macos' : isWindows ? 'windows' : 'browser',
    cpu_brand: 'Browser dev fallback',
    cpu_cores: navigator.hardwareConcurrency || 1,
    memory_gb: 0,
    gpu_info: 'Unavailable outside Tauri shell',
    available_captures: isMac
      ? ['macos', 'synthetic']
      : isWindows
        ? ['dxgi', 'winrt', 'synthetic']
        : ['synthetic'],
    available_encoders: isMac
      ? ['none', 'videotoolbox_h264', 'openh264']
      : isWindows
        ? ['none', 'openh264']
        : ['none', 'openh264'],
    available_decoders: isMac ? ['none', 'software', 'videotoolbox'] : ['none', 'software'],
    available_renderers: isMac
      ? ['macos', 'webview']
      : isWindows
      ? ['d3d11', 'd3d12', 'opengl', 'webview']
      : ['webview'],
    available_memory_modes: isWindows ? ['cpu', 'd3d11_shared'] : ['cpu'],
  };
}

// ============================================================================
// Window / Tray Commands
// ============================================================================

export async function startDragWindow(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('start_drag_window');
}

export async function minimizeWindow(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('minimize_window');
}

export async function toggleMaximizeWindow(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('toggle_maximize_window');
}

export async function hideToTray(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('hide_to_tray');
}

export async function showWindow(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('show_window');
}

export async function centerWindow(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('center_window');
}

export async function closeWindow(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('close_window');
}

export async function setWindowDecorations(
  decorated: boolean
): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('set_window_decorations', { decorated });
}

export async function applyNativeChrome(): Promise<AdapterResult<NativeBackdropStatus>> {
  return invokeAdapter<NativeBackdropStatus>('apply_native_chrome');
}

export async function openRemoteDisplayWindow(params: {
  sessionId: string;
  surfaceId?: string | null;
}): Promise<AdapterResult<RemoteDisplayWindowContext>> {
  return invokeAdapter<RemoteDisplayWindowContext>('open_remote_display_window', {
    sessionId: params.sessionId,
    surfaceId: params.surfaceId ?? null,
  });
}

export async function listRemoteDisplayWindows(
  sessionId: string
): Promise<AdapterResult<RemoteDisplayWindowContext[]>> {
  return invokeAdapter<RemoteDisplayWindowContext[]>('list_remote_display_windows', {
    sessionId,
  });
}

export async function currentRemoteDisplayWindowContext(): Promise<
  AdapterResult<RemoteDisplayWindowContext | null>
> {
  return invokeAdapter<RemoteDisplayWindowContext | null>(
    'current_remote_display_window_context'
  );
}

export async function closeRemoteDisplayWindow(
  label: string
): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('close_remote_display_window', { label });
}

export async function configureRemoteDisplayNativeSurface(params: {
  rect: NativeSurfaceRect;
  enabled: boolean;
  visible?: boolean;
}): Promise<AdapterResult<NativeRenderSurfaceSnapshot>> {
  return invokeAdapter<NativeRenderSurfaceSnapshot>(
    'configure_remote_display_native_surface',
    params
  );
}

export async function presentTestHarnessFrameOnNativeSurface(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('present_test_harness_frame_on_native_surface');
}

export async function browserWebrtcPreviewStart(params: {
  sessionId: string;
  offerSdp: string;
  fps?: number;
  h264Profile?: "baseline" | "high";
}): Promise<AdapterResult<BrowserWebrtcPreviewAnswer>> {
  return invokeAdapter<BrowserWebrtcPreviewAnswer>('browser_webrtc_preview_start', {
    sessionId: params.sessionId,
    offerSdp: params.offerSdp,
    fps: params.fps ?? null,
    h264Profile: params.h264Profile ?? null,
  });
}

export async function browserWebrtcPreviewStop(
  sessionId: string
): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('browser_webrtc_preview_stop', { sessionId });
}

export async function getClientDiagnostics(): Promise<AdapterResult<ClientDiagnostics>> {
  return invokeAdapter<ClientDiagnostics>('get_client_diagnostics');
}

export async function openDiagnosticsFolder(): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('open_diagnostics_folder');
}

export async function automationWriteReport(
  report: unknown
): Promise<AdapterResult<string | null>> {
  return invokeAdapter<string | null>('automation_write_report', { report });
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
export async function serviceBootstrapIfNeeded(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_bootstrap_if_needed');
}

/**
 * Wait for service to be healthy (with timeout)
 */
export async function serviceWaitForHealthy(
  timeoutSecs: number
): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_wait_for_healthy', {
    timeoutSecs,
  });
}

/**
 * Check if this instance bootstrapped the service
 */
export async function serviceDidBootstrap(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_did_bootstrap');
}

/**
 * Get the shell-owned service/UI status snapshot via IPC.
 */
export async function shellGetStatus(): Promise<AdapterResult<ShellStatusSnapshot>> {
  return invokeAdapter<ShellStatusSnapshot>('shell_get_status');
}

/**
 * Ask mrd-service to shut itself down with the given mode.
 */
export async function shellShutdownService(
  mode: ShutdownMode
): Promise<AdapterResult<void>> {
  return invokeAdapter<void>('shell_shutdown_service', { mode });
}

// ============================================================================
// Legacy Service Lifecycle Commands (deprecated - use shell commands instead)
// ============================================================================

/** @deprecated Use serviceBootstrapIfNeeded instead */
export async function serviceStart(): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('service_bootstrap_if_needed');
}

/** @deprecated Service is no longer owned by Rdesk - use shell_shutdown_service instead */
export async function serviceStop(): Promise<AdapterResult<boolean>> {
  return { ok: false, error: { message: 'serviceStop is deprecated. Use shell_shutdown_service IPC command instead.' } };
}

/** @deprecated Use shell_get_status instead */
export async function serviceStatus(): Promise<AdapterResult<boolean>> {
  return { ok: false, error: { message: 'serviceStatus is deprecated. Use shell_get_status IPC command instead.' } };
}

/** @deprecated Use shell_get_status instead */
export async function serviceHealthCheck(): Promise<AdapterResult<boolean>> {
  return { ok: false, error: { message: 'serviceHealthCheck is deprecated. Use shell_get_status IPC command instead.' } };
}

/** @deprecated Service lifecycle is no longer owned by Rdesk */
export async function serviceRestartWithBackoff(
  _maxAttempts: number
): Promise<AdapterResult<boolean>> {
  return { ok: false, error: { message: 'serviceRestartWithBackoff is deprecated. Service restart is owned by mrd-service.' } };
}

/** @deprecated Use shell_get_status instead */
export async function servicePid(): Promise<AdapterResult<number | null>> {
  return { ok: false, error: { message: 'servicePid is deprecated. Use shell_get_status IPC command instead.' } };
}

/** @deprecated Service lifecycle is no longer owned by Rdesk */
export async function serviceRestart(): Promise<AdapterResult<boolean>> {
  return { ok: false, error: { message: 'serviceRestart is deprecated. Service restart is owned by mrd-service.' } };
}

/** @deprecated Service guard is no longer needed - mrd-service manages its own lifecycle */
export async function serviceStartGuard(): Promise<AdapterResult<string>> {
  return { ok: false, error: { message: 'serviceStartGuard is deprecated. mrd-service manages its own lifecycle.' } };
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

/**
 * Get LAN P2P discovery snapshot via IPC.
 */
export async function ipcLanDiscoverySnapshot(): Promise<AdapterResult<LanDiscoverySnapshot>> {
  return invokeAdapter<LanDiscoverySnapshot>('ipc_lan_discovery_snapshot');
}

/**
 * Trigger immediate LAN P2P discovery probe via IPC.
 */
export async function ipcRefreshLanDiscovery(): Promise<AdapterResult<LanDiscoverySnapshot>> {
  return invokeAdapter<LanDiscoverySnapshot>('ipc_refresh_lan_discovery');
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
 * Start a LAN P2P remote session and request automatic accept on the peer.
 */
export async function ipcStartLanRemoteSession(
  sessionId: string,
  targetDeviceId: string,
  transportKind: string,
  requestedProfile?: MediaProfile
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_start_lan_remote_session', {
    sessionId,
    targetDeviceId,
    transportKind,
    ...(requestedProfile ? { requestedProfile } : {}),
  });
}

/**
 * Request a runtime media profile switch for an active LAN session.
 */
export async function ipcUpdateMediaProfile(
  sessionId: string,
  requestedProfile: MediaProfile
): Promise<AdapterResult<MediaProfileNegotiation>> {
  return invokeAdapter<MediaProfileNegotiation>('ipc_update_media_profile', {
    sessionId,
    requestedProfile,
  });
}

/**
 * List remote capture sources for an active LAN session.
 */
export async function ipcListRemoteCaptureSources(
  sessionId: string,
  includePreviews = true,
  limit?: number
): Promise<AdapterResult<CaptureSource[]>> {
  return invokeAdapter<CaptureSource[]>('ipc_list_remote_capture_sources', {
    sessionId,
    includePreviews,
    ...(limit === undefined ? {} : { limit }),
  });
}

/**
 * Select the remote capture source for an active LAN session.
 */
export async function ipcSelectRemoteCaptureSource(
  sessionId: string,
  sourceId: string
): Promise<AdapterResult<CaptureSourceSelection>> {
  return invokeAdapter<CaptureSourceSelection>('ipc_select_remote_capture_source', {
    sessionId,
    sourceId,
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
 * Mark a session failed.
 */
export async function ipcFailSession(
  sessionId: string,
  reason: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_fail_session', {
    sessionId,
    reason,
  });
}

/**
 * Recover a failed or closed session.
 */
export async function ipcRecoverSession(
  sessionId: string
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('ipc_recover_session', {
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

/**
 * List session summaries.
 */
export async function ipcListSessions(): Promise<AdapterResult<SessionInfo[]>> {
  return invokeAdapter<SessionInfo[]>('ipc_list_sessions');
}

/**
 * Get aggregated runtime snapshot.
 */
export async function ipcRuntimeSnapshot(): Promise<AdapterResult<RuntimeSnapshot>> {
  return invokeAdapter<RuntimeSnapshot>('ipc_runtime_snapshot');
}

/**
 * Get structured local capability snapshot from mrd-service.
 */
export async function ipcCapabilitySnapshot(): Promise<AdapterResult<CapabilitySnapshot>> {
  return invokeAdapter<CapabilitySnapshot>('ipc_capability_snapshot');
}

/**
 * Get probe snapshot.
 */
export async function ipcProbeSnapshot(
  sessionId: string
): Promise<AdapterResult<ProbeSnapshot>> {
  return invokeAdapter<ProbeSnapshot>('ipc_probe_snapshot', {
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

export async function getSystemResourceSnapshot(): Promise<AdapterResult<SystemResourceSnapshot>> {
  return invokeAdapter<SystemResourceSnapshot>('get_system_resource_snapshot');
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
  const result = await invokeAdapter<EnvironmentSnapshot>('test_get_capabilities');
  if (result.ok) return result;

  const fallback = browserDevCapabilities();
  if (fallback) return { ok: true, value: fallback };

  return result;
}

/**
 * List visible top-level windows available to the platform window-capture path.
 */
export async function testListWindowCaptureTargets(): Promise<AdapterResult<WindowCaptureTarget[]>> {
  return invokeAdapter<WindowCaptureTarget[]>('test_list_window_capture_targets');
}

/**
 * List platform window capture targets with best-effort screenshot previews.
 */
export async function testListWindowCaptureTargetsWithPreviews(
  limit = 24
): Promise<AdapterResult<WindowCaptureTarget[]>> {
  return invokeAdapter<WindowCaptureTarget[]>('test_list_window_capture_targets_with_previews', {
    limit,
  });
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
 * Set a custom test chain configuration.
 */
export async function testHarnessSetCustom(config: TestMatrixConfig): Promise<AdapterResult<null>> {
  return invokeAdapter<null>('test_harness_set_custom', {
    capture: config.capture,
    encoder: config.encoder,
    decoder: config.decoder,
  });
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
 * Get CapTest-compatible comparison metrics for the current harness run.
 */
export async function testHarnessGetComparisonResult(): Promise<AdapterResult<PipelineComparisonResult>> {
  return invokeAdapter<PipelineComparisonResult>('test_harness_get_comparison_result');
}

/**
 * Get latest captured and rendered frames as base64
 */
export async function testHarnessGetFrames(params?: {
  includeCaptured?: boolean;
  includeRendered?: boolean;
  lastCapturedGeneration?: number;
  lastRenderedGeneration?: number;
}): Promise<
  AdapterResult<[FrameData | null, FrameData | null]>
> {
  return invokeAdapter<[FrameData | null, FrameData | null]>(
    'test_harness_get_frames',
    {
      includeCaptured: params?.includeCaptured ?? true,
      includeRendered: params?.includeRendered ?? true,
      lastCapturedGeneration: params?.lastCapturedGeneration,
      lastRenderedGeneration: params?.lastRenderedGeneration,
    }
  );
}
