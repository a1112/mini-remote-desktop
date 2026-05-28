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
  AppSettings,
  FfmpegInstallResult,
  FfmpegProbeResult,
  HardwareInfo,
  SystemResourceSnapshot,
  NativeBackdropStatus,
  ShutdownMode,
  ShellStatusSnapshot,
  SessionInfo,
  SessionRuntimeSnapshot,
  RuntimeSnapshot,
  AuditEvent,
  AuditLogQuery,
  CapabilitySnapshot,
  CaptureSource,
  CaptureSourceSelection,
  DisplayMode,
  DisplayModeChange,
  AdaptiveMediaConfig,
  MediaAdaptationSnapshot,
  MediaProfile,
  MediaProfileNegotiation,
  MediaPipelineSnapshot,
  ProbeSnapshot,
  HarnessMetrics,
  FrameData,
  // Test Workbench Types
  TestScenario,
  TestRun,
  ExternalTestRunRecord,
  TestConfig,
  TestStageEvent,
  MetricSeries,
  Artifact,
  TelemetryBundle,
  TelemetryQuery,
  TestPreset,
  EnvironmentSnapshot,
  WindowCaptureTarget,
  CaptureShareSourceTarget,
  RemoteDisplayWindowContext,
  NativeSurfaceRect,
  NativeRenderSurfaceSnapshot,
  BrowserWebrtcPreviewAnswer,
  TestMatrixConfig,
  PipelineComparisonResult,
} from './types';
import {
  invokeServiceBridgeIpc,
  postServiceBridgeJson,
  serviceBridgeHealth,
  serviceBridgeWebSocketUrl,
  type ServiceBridgeIpcRequest,
  type ServiceBridgeIpcResponse,
} from '../serviceBridge/client';

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

async function invokeBridgeOrTauri<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  request: ServiceBridgeIpcRequest,
  unwrap: (response: ServiceBridgeIpcResponse) => T
): Promise<AdapterResult<T>> {
  if (shouldUseServiceBridge()) {
    return invokeServiceBridgeIpc<T>(request, unwrap);
  }
  return invokeAdapter<T>(command, args);
}

function responseField<T>(field: string): (response: ServiceBridgeIpcResponse) => T {
  return (response) => response[field] as T;
}

function environmentFromCapabilitySnapshot(snapshot: CapabilitySnapshot): EnvironmentSnapshot {
  const captures: string[] = [];
  const encoders: string[] = [];
  const decoders: string[] = ['none'];
  const renderers: string[] = ['none'];
  const memoryModes: string[] = [];

  for (const capability of snapshot.capabilities) {
    if (
      capability.status !== 'available' &&
      capability.status !== 'supported' &&
      capability.status !== 'usable' &&
      capability.status !== 'degraded'
    ) {
      continue;
    }
    const [domain, ...rest] = capability.id.split('.');
    const value = rest.join('.');
    if (!value) continue;
    if (domain === 'capture') captures.push(value);
    if (domain === 'encode') encoders.push(value);
    if (domain === 'decode') decoders.push(value);
    if (domain === 'render') renderers.push(value === 'd3d12_native' ? 'd3d12' : value);
    if (domain === 'memory') memoryModes.push(value);
  }

  return {
    os_type: snapshot.platform,
    cpu_brand: 'mrd-service capability snapshot',
    cpu_cores: 0,
    memory_gb: 0,
    gpu_info: 'Reported by mrd-service',
    available_captures: Array.from(new Set(captures)),
    available_encoders: Array.from(new Set(encoders)),
    available_decoders: Array.from(new Set(decoders)),
    available_renderers: Array.from(new Set(renderers)),
    available_memory_modes: Array.from(new Set(memoryModes)),
  };
}

function isLocalBrowserFallbackAllowed(): boolean {
  if (typeof window === 'undefined') return false;
  if ('__TAURI_INTERNALS__' in window) return false;
  const host = window.location.hostname;
  return host === 'localhost' || host === '127.0.0.1' || host === '::1';
}

function shouldUseServiceBridge(): boolean {
  if (!isLocalBrowserFallbackAllowed()) return false;
  if (import.meta.env.MODE === 'test') {
    return Boolean((window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__);
  }
  return true;
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
    gpu_info: 'mrd-service not connected; browser diagnostic fallback only',
    available_captures: ['synthetic'],
    available_encoders: ['none', 'openh264'],
    available_decoders: ['none', 'software'],
    available_renderers: ['webview'],
    available_memory_modes: ['cpu'],
  };
}

async function browserDevTestRuns(params?: {
  scenarioId?: string;
  status?: string;
  limit?: number;
}): Promise<AdapterResult<TestRun[]>> {
  if (!isLocalBrowserFallbackAllowed()) return { ok: true, value: [] };

  try {
    const response = await fetch('/dev-test-runs.json', { cache: 'no-store' });
    if (!response.ok) return { ok: true, value: [] };
    const runs = (await response.json()) as TestRun[];
    const filtered = runs
      .filter((run) => !params?.scenarioId || run.scenario_id === params.scenarioId)
      .filter((run) => !params?.status || run.status === params.status)
      .sort((a, b) => b.started_at - a.started_at);
    return { ok: true, value: filtered.slice(0, params?.limit ?? filtered.length) };
  } catch {
    return { ok: true, value: [] };
  }
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
  preferredDisplaySourceId?: string | null;
  avoidCaptureSourceId?: string | null;
}): Promise<AdapterResult<RemoteDisplayWindowContext>> {
  if (shouldUseServiceBridge()) {
    return {
      ok: true,
      value: {
        label: `web-${params.sessionId}`,
        session_id: params.sessionId,
        surface_id: params.surfaceId ?? `web-${params.sessionId}`,
        role: 'controller',
        renderer_attached: false,
        render_mode: 'web',
        native_surface_attached: false,
        session_window_count: 1,
      },
    };
  }
  return invokeAdapter<RemoteDisplayWindowContext>('open_remote_display_window', {
    sessionId: params.sessionId,
    surfaceId: params.surfaceId ?? null,
    preferredDisplaySourceId: params.preferredDisplaySourceId ?? null,
    avoidCaptureSourceId: params.avoidCaptureSourceId ?? null,
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
  if (shouldUseServiceBridge()) {
    return { ok: true, value: null };
  }
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

export async function presentRemotePreviewFrameOnNativeSurface(
  dataUrl: string
): Promise<AdapterResult<boolean>> {
  return invokeAdapter<boolean>('present_remote_preview_frame_on_native_surface', {
    dataUrl,
  });
}

export async function browserWebrtcPreviewStart(params: {
  sessionId: string;
  offerSdp: string;
  fps?: number;
  width?: number;
  height?: number;
  codec?: "h264" | "hevc" | "av1";
  h264Profile?: "baseline" | "high";
  bitrateMbps?: number;
  sourceId?: string;
}): Promise<AdapterResult<BrowserWebrtcPreviewAnswer>> {
  if (shouldUseServiceBridge()) {
    return postServiceBridgeJson<BrowserWebrtcPreviewAnswer>(
      '/browser/webrtc-preview/start',
      {
        session_id: params.sessionId,
        offer_sdp: params.offerSdp,
        fps: params.fps ?? null,
        width: params.width ?? null,
        height: params.height ?? null,
        codec: params.codec ?? null,
        h264_profile: params.h264Profile ?? null,
        bitrate_mbps: params.bitrateMbps ?? null,
        source_id: params.sourceId ?? null,
      }
    );
  }
  return invokeAdapter<BrowserWebrtcPreviewAnswer>('browser_webrtc_preview_start', {
    sessionId: params.sessionId,
    offerSdp: params.offerSdp,
    fps: params.fps ?? null,
    width: params.width ?? null,
    height: params.height ?? null,
    codec: params.codec ?? null,
    h264Profile: params.h264Profile ?? null,
    bitrateMbps: params.bitrateMbps ?? null,
    sourceId: params.sourceId ?? null,
  });
}

export async function browserWebrtcPreviewStop(
  sessionId: string
): Promise<AdapterResult<void>> {
  if (shouldUseServiceBridge()) {
    const result = await postServiceBridgeJson<{ stopped: boolean }>(
      '/browser/webrtc-preview/stop',
      { session_id: sessionId }
    );
    if (!result.ok) return result;
    return { ok: true, value: undefined };
  }
  return invokeAdapter<void>('browser_webrtc_preview_stop', { sessionId });
}

export function browserWebcodecsPreviewWebSocketUrl(): string {
  return serviceBridgeWebSocketUrl('/browser/webcodecs-preview/ws');
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
  if (shouldUseServiceBridge()) {
    const health = await serviceBridgeHealth();
    if (!health.ok) return health;
    return { ok: true, value: false };
  }
  return invokeAdapter<boolean>('service_bootstrap_if_needed');
}

/**
 * Wait for service to be healthy (with timeout)
 */
export async function serviceWaitForHealthy(
  timeoutSecs: number
): Promise<AdapterResult<boolean>> {
  if (shouldUseServiceBridge()) {
    const deadline = Date.now() + timeoutSecs * 1000;
    let lastError = 'mrd-service web bridge is not healthy';
    do {
      const health = await serviceBridgeHealth();
      if (health.ok && health.value.status === 'ok') {
        return { ok: true, value: true };
      }
      lastError = health.ok ? 'mrd-service web bridge is not healthy' : health.error.message;
      await new Promise((resolve) => window.setTimeout(resolve, 250));
    } while (Date.now() < deadline);
    return { ok: false, error: { message: lastError } };
  }
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
  return invokeBridgeOrTauri<ShellStatusSnapshot>(
    'shell_get_status',
    undefined,
    { type: 'GetShellStatus' },
    responseField<ShellStatusSnapshot>('status')
  );
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
  return invokeBridgeOrTauri<LanDiscoverySnapshot>(
    'ipc_lan_discovery_snapshot',
    undefined,
    { type: 'LanDiscoverySnapshot' },
    responseField<LanDiscoverySnapshot>('snapshot')
  );
}

/**
 * Trigger immediate LAN P2P discovery probe via IPC.
 */
export async function ipcRefreshLanDiscovery(): Promise<AdapterResult<LanDiscoverySnapshot>> {
  return invokeBridgeOrTauri<LanDiscoverySnapshot>(
    'ipc_refresh_lan_discovery',
    undefined,
    { type: 'RefreshLanDiscovery' },
    responseField<LanDiscoverySnapshot>('snapshot')
  );
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
  const args = {
    sessionId,
    targetDeviceId,
    transportKind,
    ...(requestedProfile ? { requestedProfile } : {}),
  };
  return invokeBridgeOrTauri<string>(
    'ipc_start_lan_remote_session',
    args,
    {
      type: 'StartLanRemoteSession',
      session_id: sessionId,
      target_device_id: targetDeviceId,
      transport_kind: transportKind,
      ...(requestedProfile ? { requested_profile: requestedProfile } : {}),
    },
    responseField<string>('session_id')
  );
}

/**
 * Request a runtime media profile switch for an active LAN session.
 */
export async function ipcUpdateMediaProfile(
  sessionId: string,
  requestedProfile: MediaProfile
): Promise<AdapterResult<MediaProfileNegotiation>> {
  return invokeBridgeOrTauri<MediaProfileNegotiation>(
    'ipc_update_media_profile',
    {
      sessionId,
      requestedProfile,
    },
    {
      type: 'UpdateMediaProfile',
      session_id: sessionId,
      requested_profile: requestedProfile,
    },
    responseField<MediaProfileNegotiation>('negotiation')
  );
}

/**
 * Configure runtime LAN media bitrate/FPS/resolution adaptation.
 */
export async function ipcConfigureMediaAdaptation(
  sessionId: string,
  config: AdaptiveMediaConfig
): Promise<AdapterResult<MediaAdaptationSnapshot>> {
  return invokeBridgeOrTauri<MediaAdaptationSnapshot>(
    'ipc_configure_media_adaptation',
    {
      sessionId,
      config,
    },
    {
      type: 'ConfigureMediaAdaptation',
      session_id: sessionId,
      config,
    },
    responseField<MediaAdaptationSnapshot>('snapshot')
  );
}

/**
 * List local capture sources from the mrd-service host.
 */
export async function ipcListLocalCaptureSources(
  includePreviews = true,
  limit?: number
): Promise<AdapterResult<CaptureSource[]>> {
  const args = {
    includePreviews,
    ...(limit === undefined ? {} : { limit }),
  };
  return invokeBridgeOrTauri<CaptureSource[]>(
    'ipc_list_local_capture_sources',
    args,
    {
      type: 'ListLocalCaptureSources',
      include_previews: includePreviews,
      limit: limit ?? null,
    },
    responseField<CaptureSource[]>('sources')
  );
}

/**
 * List remote capture sources for an active LAN session.
 */
export async function ipcListRemoteCaptureSources(
  sessionId: string,
  includePreviews = true,
  limit?: number
): Promise<AdapterResult<CaptureSource[]>> {
  const args = {
    sessionId,
    includePreviews,
    ...(limit === undefined ? {} : { limit }),
  };
  return invokeBridgeOrTauri<CaptureSource[]>(
    'ipc_list_remote_capture_sources',
    args,
    {
      type: 'ListRemoteCaptureSources',
      session_id: sessionId,
      include_previews: includePreviews,
      limit: limit ?? null,
    },
    responseField<CaptureSource[]>('sources')
  );
}

/**
 * Select the remote capture source for an active LAN session.
 */
export async function ipcSelectRemoteCaptureSource(
  sessionId: string,
  sourceId: string
): Promise<AdapterResult<CaptureSourceSelection>> {
  return invokeBridgeOrTauri<CaptureSourceSelection>(
    'ipc_select_remote_capture_source',
    {
      sessionId,
      sourceId,
    },
    {
      type: 'SelectRemoteCaptureSource',
      session_id: sessionId,
      source_id: sourceId,
    },
    responseField<CaptureSourceSelection>('selection')
  );
}

/**
 * List remote display modes for the selected capture display.
 */
export async function ipcListRemoteDisplayModes(
  sessionId: string
): Promise<AdapterResult<DisplayMode[]>> {
  return invokeBridgeOrTauri<DisplayMode[]>(
    'ipc_list_remote_display_modes',
    {
      sessionId,
    },
    {
      type: 'ListRemoteDisplayModes',
      session_id: sessionId,
    },
    responseField<DisplayMode[]>('modes')
  );
}

/**
 * Temporarily set the remote display mode for an active LAN session.
 */
export async function ipcSetRemoteDisplayMode(
  sessionId: string,
  mode: DisplayMode,
  restoreAfterSession = true
): Promise<AdapterResult<DisplayModeChange>> {
  return invokeBridgeOrTauri<DisplayModeChange>(
    'ipc_set_remote_display_mode',
    {
      sessionId,
      mode,
      restoreAfterSession,
    },
    {
      type: 'SetRemoteDisplayMode',
      session_id: sessionId,
      mode,
      restore_after_session: restoreAfterSession,
    },
    responseField<DisplayModeChange>('change')
  );
}

/**
 * Restore a temporary remote display mode.
 */
export async function ipcRestoreRemoteDisplayMode(
  sessionId: string
): Promise<AdapterResult<DisplayModeChange>> {
  return invokeBridgeOrTauri<DisplayModeChange>(
    'ipc_restore_remote_display_mode',
    {
      sessionId,
    },
    {
      type: 'RestoreRemoteDisplayMode',
      session_id: sessionId,
    },
    responseField<DisplayModeChange>('change')
  );
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
  return invokeBridgeOrTauri<SessionRuntimeSnapshot>(
    'ipc_session_snapshot',
    {
      sessionId,
    },
    {
      type: 'SessionRuntimeSnapshot',
      session_id: sessionId,
    },
    responseField<SessionRuntimeSnapshot>('snapshot')
  );
}

/**
 * List session summaries.
 */
export async function ipcListSessions(): Promise<AdapterResult<SessionInfo[]>> {
  return invokeBridgeOrTauri<SessionInfo[]>(
    'ipc_list_sessions',
    undefined,
    { type: 'ListSessions' },
    responseField<SessionInfo[]>('sessions')
  );
}

/**
 * Get aggregated runtime snapshot.
 */
export async function ipcRuntimeSnapshot(): Promise<AdapterResult<RuntimeSnapshot>> {
  return invokeBridgeOrTauri<RuntimeSnapshot>(
    'ipc_runtime_snapshot',
    undefined,
    { type: 'RuntimeSnapshot' },
    responseField<RuntimeSnapshot>('snapshot')
  );
}

/**
 * Query service-owned audit events.
 */
export async function ipcAuditLog(
  query: AuditLogQuery = {}
): Promise<AdapterResult<AuditEvent[]>> {
  return invokeAdapter<AuditEvent[]>('ipc_audit_log', { query });
}

/**
 * Get structured local capability snapshot from mrd-service.
 */
export async function ipcCapabilitySnapshot(): Promise<AdapterResult<CapabilitySnapshot>> {
  return invokeBridgeOrTauri<CapabilitySnapshot>(
    'ipc_capability_snapshot',
    undefined,
    { type: 'CapabilitySnapshot' },
    responseField<CapabilitySnapshot>('snapshot')
  );
}

/**
 * Get probe snapshot.
 */
export async function ipcProbeSnapshot(
  sessionId: string
): Promise<AdapterResult<ProbeSnapshot>> {
  return invokeBridgeOrTauri<ProbeSnapshot>(
    'ipc_probe_snapshot',
    {
      sessionId,
    },
    {
      type: 'ProbeSnapshot',
      session_id: sessionId,
    },
    responseField<ProbeSnapshot>('snapshot')
  );
}

/**
 * Get receiver media pipeline snapshot.
 */
export async function ipcMediaPipelineSnapshot(
  sessionId: string
): Promise<AdapterResult<MediaPipelineSnapshot>> {
  return invokeBridgeOrTauri<MediaPipelineSnapshot>(
    'ipc_media_pipeline_snapshot',
    {
      sessionId,
    },
    {
      type: 'MediaPipelineSnapshot',
      session_id: sessionId,
    },
    responseField<MediaPipelineSnapshot>('snapshot')
  );
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
  return invokeBridgeOrTauri<string>(
    'ipc_start_receiver',
    {
      sessionId,
    },
    {
      type: 'StartReceiver',
      session_id: sessionId,
    },
    responseField<string>('session_id')
  );
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

export type SystemResourceTarget = 'auto' | 'mrd-service' | 'display';

export async function getSystemResourceSnapshot(
  target: SystemResourceTarget = 'auto'
): Promise<AdapterResult<SystemResourceSnapshot>> {
  if (shouldUseServiceBridge()) {
    if (target === 'display') {
      return { ok: true, value: browserDisplayResourceSnapshot() };
    }
    return postServiceBridgeJson<SystemResourceSnapshot>('/resource', {
      target: target === 'auto' ? 'mrd_service' : target.replace('-', '_'),
    });
  }

  return invokeAdapter<SystemResourceSnapshot>(
    'get_system_resource_snapshot',
    target === 'auto' ? undefined : { target }
  );
}

function browserDisplayResourceSnapshot(): SystemResourceSnapshot {
  const memory = (performance as Performance & {
    memory?: {
      usedJSHeapSize?: number;
      totalJSHeapSize?: number;
      jsHeapSizeLimit?: number;
    };
  }).memory;
  const usedMb = bytesToMb(memory?.usedJSHeapSize ?? 0);
  const totalMb = bytesToMb(memory?.jsHeapSizeLimit ?? memory?.totalJSHeapSize ?? 0);

  return {
    target_name: 'Browser display',
    target_pid: null,
    target_found: true,
    cpu_metrics_available: false,
    cpu_usage_percent: 0,
    memory_used_mb: usedMb,
    memory_total_mb: totalMb,
    memory_usage_percent: totalMb > 0 ? Math.min(100, usedMb / totalMb * 100) : 0,
    gpu_usage_percent: null,
    gpu_memory_used_mb: null,
    gpu_memory_total_mb: null,
    gpu_metrics_available: false,
    gpu_metrics_scope: 'unavailable',
    network_rx_bps: 0,
    network_tx_bps: 0,
    network_metrics_available: false,
    network_metrics_scope: 'unavailable',
    sampled_at_ms: Date.now(),
  };
}

function bytesToMb(bytes: number): number {
  return Math.round(Math.max(0, bytes) / 1024 / 1024);
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

export async function ffmpegProbe(): Promise<AdapterResult<FfmpegProbeResult>> {
  return invokeAdapter<FfmpegProbeResult>('ffmpeg_probe');
}

export async function ffmpegDownload(): Promise<AdapterResult<FfmpegInstallResult>> {
  return invokeAdapter<FfmpegInstallResult>('ffmpeg_download');
}

export async function ffmpegResetGoldenSettings(): Promise<AdapterResult<AppSettings>> {
  return invokeAdapter<AppSettings>('ffmpeg_reset_golden_settings');
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
  if (shouldUseServiceBridge()) {
    const serviceSnapshot = await ipcCapabilitySnapshot();
    if (serviceSnapshot.ok) {
      return {
        ok: true,
        value: environmentFromCapabilitySnapshot(serviceSnapshot.value),
      };
    }
  }

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
 * List screen/window sharing sources exposed by the current platform.
 */
export async function testListCaptureShareSources(): Promise<
  AdapterResult<CaptureShareSourceTarget[]>
> {
  return invokeAdapter<CaptureShareSourceTarget[]>('test_list_capture_share_sources');
}

/**
 * List screen/window sharing sources with best-effort preview frames where supported.
 */
export async function testListCaptureShareSourcesWithPreviews(
  limit = 24
): Promise<AdapterResult<CaptureShareSourceTarget[]>> {
  return invokeAdapter<CaptureShareSourceTarget[]>(
    'test_list_capture_share_sources_with_previews',
    { limit }
  );
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

export async function testRecordExternalRun(
  record: ExternalTestRunRecord
): Promise<AdapterResult<string>> {
  return invokeAdapter<string>('test_record_external_run', { record });
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
  if (import.meta.env.MODE !== 'test' && isLocalBrowserFallbackAllowed()) {
    return browserDevTestRuns(params);
  }
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
 * Get a test run with persisted telemetry metrics, logs, events, and artifacts
 */
export async function testGetRunTelemetry(
  runId: string,
  query?: TelemetryQuery
): Promise<AdapterResult<TelemetryBundle>> {
  return invokeBridgeOrTauri<TelemetryBundle>(
    'test_get_run_telemetry',
    {
      runId,
      query: query ?? null,
    },
    {
      type: 'GetTelemetryBundle',
      run_id: runId,
      session_id: query && 'session_id' in query ? (query as Record<string, unknown>).session_id : null,
    },
    responseField<TelemetryBundle>('bundle')
  );
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
