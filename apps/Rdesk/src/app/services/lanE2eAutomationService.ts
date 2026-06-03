import type {
  AdapterResult,
  AdaptiveMediaConfig,
  CaptureSource,
  CaptureSourceSelection,
  DisplayMode,
  DisplayModeChange,
  LanDiscoverySnapshot,
  LanPeerInfo,
  MediaPipelineSnapshot,
  MediaProfile,
  MediaAdaptationSnapshot,
  ProbeSnapshot,
  RemoteDisplayWindowContext,
  RuntimeSnapshot,
  SessionRuntimeSnapshot,
} from "../adapters/tauri";
import {
  evaluateProfileProbe,
  type CapabilityProfile,
  type ProfileProbeResult,
} from "./capabilityMatrix";

export type LanE2EStatus = "running" | "completed" | "failed" | "skipped";

export type CrossDeviceScenarioId =
  | "lan.e2e.remote_display"
  | "cross.e2e.discovery"
  | "cross.e2e.remote_display_smoke"
  | "cross.e2e.media_profile"
  | "cross.fault.recovery";

export type CrossDeviceFaultType =
  | "network.pause_peer"
  | "renderer.detach_surface";

export interface CrossDeviceFaultPlan {
  type: CrossDeviceFaultType;
  durationMs?: number;
  note?: string;
}

export interface CrossDeviceFaultEvent {
  type: CrossDeviceFaultType;
  status: "injected" | "unsupported" | "failed";
  timestamp: number;
  message?: string;
}

export type LanE2EFailureReason =
  | "service_unhealthy"
  | "local_device_registration_failed"
  | "peer_not_found"
  | "peer_version_mismatch"
  | "peer_not_ready"
  | "session_start_failed"
  | "capture_source_failed"
  | "display_mode_failed"
  | "receiver_start_failed"
  | "display_window_failed"
  | "fault_injection_unsupported"
  | "fault_injection_failed"
  | "no_remote_frames"
  | "media_profile_mismatch"
  | "profile_downgraded"
  | "performance_threshold"
  | "runtime_error"
  | "stop_failed";

export interface LanE2EStageEvent {
  stage: "preflight" | "pairing" | "session" | "capture_source" | "display_mode" | "adaptation" | "receiver" | "display" | "fault" | "sample" | "assert" | "cleanup";
  status: "started" | "completed" | "failed" | "skipped";
  timestamp: number;
  error?: string;
}

export interface LanE2EAutomationOptions {
  scenarioId?: CrossDeviceScenarioId;
  targetDeviceId?: string;
  transportKind?: "quic" | "webrtc";
  timeoutMs?: number;
  sampleIntervalMs?: number;
  minSampleDurationMs?: number;
  minDecodedFrames?: number;
  minFps?: number;
  stopOnComplete?: boolean;
  requestedProfile?: MediaProfile;
  displayModePolicy?: "none" | "temporary" | "required";
  preferredCaptureSourceId?: string;
  preferredCaptureSourceKind?: string;
  preferredRenderDisplaySourceId?: string;
  expectedPeerBuildId?: string;
  renderProfileCap?: boolean;
  renderDisplay?: boolean;
  adaptive?: boolean;
  adaptiveConfig?: AdaptiveMediaConfig;
  faultPlan?: CrossDeviceFaultPlan;
  createSessionId?: () => string;
  now?: () => number;
}

export interface LanE2EAutomationReport {
  status: LanE2EStatus;
  scenarioId: CrossDeviceScenarioId;
  sessionId?: string;
  controllerDeviceId?: string | null;
  peer?: LanPeerInfo;
  displayWindow?: RemoteDisplayWindowContext;
  captureSource?: CaptureSource;
  captureSourceSelection?: CaptureSourceSelection;
  displayModeChange?: DisplayModeChange;
  sessionSnapshot?: SessionRuntimeSnapshot;
  probeSnapshot?: ProbeSnapshot;
  mediaPipelineSnapshot?: MediaPipelineSnapshot;
  mediaAdaptationSnapshot?: MediaAdaptationSnapshot;
  profileProbeResult?: ProfileProbeResult;
  requestedProfile?: MediaProfile;
  faultPlan?: CrossDeviceFaultPlan;
  faultEvents: CrossDeviceFaultEvent[];
  validationMode: "quic_datagram" | "webrtc_rtp";
  dataPlaneVerified: boolean;
  mediaVerified: boolean;
  sampleDurationMs: number;
  sampleFramesDecoded: number;
  sampleFramesDropped: number;
  sampleSequenceGapDrops?: number;
  sampleDecodeErrorDrops?: number;
  sampleTransientDrops?: number;
  sampleFpsElapsedMs?: number;
  sampleFpsTargetDurationMs?: number;
  sampleObservedFps?: number;
  sampleObservedFpsAtTargetDuration?: number;
  sampleRenderFramesPresented: number;
  sampleObservedRenderFps?: number;
  sampleObservedRenderFpsAtTargetDuration?: number;
  sampleRenderQueueReplacements?: number;
  sampleRenderPresentSkips?: number;
  thresholds: {
    minSampleDurationMs: number;
    minDecodedFrames: number;
    minFps: number;
  };
  failureReason?: LanE2EFailureReason;
  errorMessage?: string;
  startedAt: number;
  finishedAt: number;
  stages: LanE2EStageEvent[];
}

export interface LanE2EAutomationCommands {
  serviceBootstrapIfNeeded(): Promise<AdapterResult<boolean>>;
  serviceWaitForHealthy(timeoutSecs?: number): Promise<AdapterResult<boolean>>;
  ipcRuntimeSnapshot(): Promise<AdapterResult<RuntimeSnapshot>>;
  getHardwareInfo(): Promise<AdapterResult<LanE2EHardwareInfo>>;
  ipcRegisterDevice(deviceId: string, deviceName: string): Promise<AdapterResult<string>>;
  ipcRefreshLanDiscovery(): Promise<AdapterResult<LanDiscoverySnapshot>>;
  ipcStartLanRemoteSession(
    sessionId: string,
    targetDeviceId: string,
    transportKind: string,
    requestedProfile?: MediaProfile
  ): Promise<AdapterResult<string>>;
  ipcUpdateMediaProfile?(
    sessionId: string,
    requestedProfile: MediaProfile
  ): Promise<AdapterResult<unknown>>;
  ipcConfigureMediaAdaptation?(
    sessionId: string,
    config: AdaptiveMediaConfig
  ): Promise<AdapterResult<MediaAdaptationSnapshot>>;
  ipcListRemoteCaptureSources(
    sessionId: string,
    includePreviews: boolean,
    limit?: number
  ): Promise<AdapterResult<CaptureSource[]>>;
  ipcSelectRemoteCaptureSource(
    sessionId: string,
    sourceId: string
  ): Promise<AdapterResult<CaptureSourceSelection>>;
  ipcListRemoteDisplayModes?(sessionId: string): Promise<AdapterResult<DisplayMode[]>>;
  ipcSetRemoteDisplayMode?(
    sessionId: string,
    mode: DisplayMode,
    restoreAfterSession: boolean
  ): Promise<AdapterResult<DisplayModeChange>>;
  ipcRestoreRemoteDisplayMode?(sessionId: string): Promise<AdapterResult<DisplayModeChange>>;
  ipcStartReceiver(sessionId: string): Promise<AdapterResult<string>>;
  openRemoteDisplayWindow(params: {
    sessionId: string;
    preferredDisplaySourceId?: string;
    avoidCaptureSourceId?: string;
  }): Promise<AdapterResult<RemoteDisplayWindowContext>>;
  ipcSessionSnapshot(sessionId: string): Promise<AdapterResult<SessionRuntimeSnapshot>>;
  ipcProbeSnapshot(sessionId: string): Promise<AdapterResult<ProbeSnapshot>>;
  ipcMediaPipelineSnapshot(sessionId: string): Promise<AdapterResult<MediaPipelineSnapshot>>;
  crossE2EInjectFault?(
    sessionId: string,
    faultPlan: CrossDeviceFaultPlan
  ): Promise<AdapterResult<string>>;
  ipcStopSession(sessionId: string): Promise<AdapterResult<string>>;
}

export interface LanE2EHardwareInfo {
  motherboard_serial: string;
  hostname: string;
  os_type: string;
  os_version: string;
  cpu_info: {
    name: string;
    vendor_id: string;
    cores: number;
    max_frequency_mhz?: number | null;
  };
  total_memory_mb: number;
  gpu_info: Array<{
    name: string;
    vendor: string;
    memory_mb?: number | null;
  }>;
}

const DEFAULT_TIMEOUT_MS = 10_000;
const DEFAULT_SAMPLE_INTERVAL_MS = 500;
const DEFAULT_MIN_DECODED_FRAMES = 1;
const DEFAULT_MIN_FPS = 1;
const DEFAULT_MIN_SAMPLE_DURATION_MS = 0;
const QUIC_DATAGRAM_MEDIA_CAPABILITY = "quic_datagram";
const QUIC_2K144_MEDIA_CAPABILITY = "quic_datagram_2k144";
const QUIC_DATAGRAM_MEDIA_V2_CAPABILITY = "quic_datagram_media_v2";
const QUIC_DATAGRAM_MEDIA_V3_CAPABILITY = "quic_datagram_media_v3";
const MEDIA_PROFILE_CONTROL_CAPABILITY = "media_profile_control_v1";
interface RequiredPlatformMediaCapabilityProfile {
  id: "windows" | "macos" | "linux";
  label: string;
  verifyHint: string;
  capabilities: string[];
}

const REQUIRED_PLATFORM_MEDIA_CAPABILITY_PROFILES: RequiredPlatformMediaCapabilityProfile[] = [
  {
    id: "windows",
    label: "Windows",
    verifyHint: "verify DXGI/NVENC/NVDEC/D3D11 native support",
    capabilities: ["dxgi_capture", "nvenc_h264", "nvdec", "d3d11_native_render"],
  },
  {
    id: "macos",
    label: "macOS",
    verifyHint: "verify ScreenCaptureKit/VideoToolbox/Metal native support",
    capabilities: [
      "macos_capture",
      "videotoolbox_h264",
      "videotoolbox",
      "macos_native_render",
    ],
  },
  {
    id: "linux",
    label: "Linux",
    verifyHint: "verify PipeWire/Linux decode/native render support",
    capabilities: ["pipewire_capture", "software_decode"],
  },
];
const DEFAULT_LAN_HEVC_MEDIA_PROFILE: MediaProfile = {
  width: 2560,
  height: 1600,
  fps: 165,
  bitrate_mbps: 80,
  codec: "hevc",
  codec_profile: "main",
  bit_depth: 8,
  chroma_subsampling: "4:2:0",
  pixel_format: "nv12",
  hdr_enabled: false,
};
const DEFAULT_LAN_H264_FALLBACK_MEDIA_PROFILE: MediaProfile = {
  width: 2560,
  height: 1600,
  fps: 165,
  bitrate_mbps: 80,
  codec: "h264",
};
const DEFAULT_LAN_MACOS_H264_MEDIA_PROFILE: MediaProfile = {
  width: 2560,
  height: 1440,
  fps: 144,
  bitrate_mbps: 80,
  codec: "h264",
};
const ADAPTIVE_STARTUP_SAFE_MIN_FPS = 120;
const ADAPTIVE_STARTUP_SAFE_MIN_BITRATE_MBPS = 80;
const ADAPTIVE_STARTUP_SAFE_BITRATE_RATIO = 0.8;
const SAMPLE_DURATION_TOLERANCE_MS = 250;
const REMOTE_DISPLAY_SURFACE_ATTACH_TIMEOUT_MS = 10_000;
const REMOTE_DISPLAY_SURFACE_ATTACH_POLL_MS = 100;

export async function runLanE2EAutomation(
  commands: LanE2EAutomationCommands,
  options: LanE2EAutomationOptions = {}
): Promise<LanE2EAutomationReport> {
  const now = options.now ?? Date.now;
  const startedAt = now();
  const stages: LanE2EStageEvent[] = [];
  const scenarioId = options.scenarioId ?? "lan.e2e.remote_display";
  const faultEvents: CrossDeviceFaultEvent[] = [];
  const sampleIntervalMs = options.sampleIntervalMs ?? DEFAULT_SAMPLE_INTERVAL_MS;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const minSampleDurationMs = options.minSampleDurationMs ?? DEFAULT_MIN_SAMPLE_DURATION_MS;
  const sampleDurationToleranceMs = sampleDurationToleranceFor(minSampleDurationMs);
  const sampleWindowTimeoutMs = Math.max(
    timeoutMs,
    minSampleDurationMs + sampleIntervalMs + sampleDurationToleranceMs
  );
  const minDecodedFrames = options.minDecodedFrames ?? DEFAULT_MIN_DECODED_FRAMES;
  const minFps = options.minFps ?? DEFAULT_MIN_FPS;
  const stopOnComplete = options.stopOnComplete ?? true;
  const transportKind = options.transportKind ?? "quic";
  const displayModePolicy = options.displayModePolicy ?? "none";
  const renderProfileCapEnabled = options.renderProfileCap ?? true;
  const renderDisplayEnabled = options.renderDisplay ?? true;
  const requestMediaProfile = shouldRequestMediaProfile(scenarioId, transportKind);
  let requestedProfile = options.requestedProfile;
  const validationMode = transportKind === "webrtc" ? "webrtc_rtp" : "quic_datagram";
  let sessionId: string | undefined;
  let peer: LanPeerInfo | undefined;
  let displayWindow: RemoteDisplayWindowContext | undefined;
  let captureSource: CaptureSource | undefined;
  let captureSourceSelection: CaptureSourceSelection | undefined;
  let displayModeChange: DisplayModeChange | undefined;
  let sessionSnapshot: SessionRuntimeSnapshot | undefined;
  let probeSnapshot: ProbeSnapshot | undefined;
  let mediaPipelineSnapshot: MediaPipelineSnapshot | undefined;
  let mediaAdaptationSnapshot: MediaAdaptationSnapshot | undefined;
  let profileProbeResult: ProfileProbeResult | undefined;
  let controllerDeviceId: string | null | undefined;
  let sessionStarted = false;
  let sampleDurationMs = 0;
  let sampleFramesDecoded = 0;
  let sampleFramesDropped = 0;
  let sampleSequenceGapDrops: number | undefined;
  let sampleDecodeErrorDrops: number | undefined;
  let sampleTransientDrops: number | undefined;
  let sampleFpsElapsedMs: number | undefined;
  let sampleFpsTargetDurationMs: number | undefined;
  let sampleObservedFps: number | undefined;
  let sampleObservedFpsAtTargetDuration: number | undefined;
  let sampleRenderFramesPresented = 0;
  let sampleObservedRenderFps: number | undefined;
  let sampleObservedRenderFpsAtTargetDuration: number | undefined;
  let sampleRenderQueueReplacements: number | undefined;
  let sampleRenderPresentSkips: number | undefined;
  let renderCappedProfileApplied = false;
  let sampleFpsBaselineDeferred = false;
  let sampleFpsBaseline:
    | {
        framesDecoded: number;
        framesDropped: number;
        sequenceGapDrops: number;
        decodeErrorDrops: number;
        transientDrops: number;
        renderPresentedFrames: number;
        renderQueueReplacements: number;
        renderPresentSkips: number;
        sampleDurationMs: number;
      }
    | undefined;

  const stage = (
    event: LanE2EStageEvent["stage"],
    status: LanE2EStageEvent["status"],
    error?: string
  ) => {
    stages.push({ stage: event, status, timestamp: now(), error });
  };

  const finish = (
    status: LanE2EStatus,
    failureReason?: LanE2EFailureReason,
    errorMessage?: string
  ): LanE2EAutomationReport => ({
    status,
    scenarioId,
    sessionId,
    controllerDeviceId,
    peer,
    displayWindow,
    captureSource,
    captureSourceSelection,
    displayModeChange,
    sessionSnapshot,
    probeSnapshot,
    mediaPipelineSnapshot,
    mediaAdaptationSnapshot,
    profileProbeResult,
    requestedProfile,
    faultPlan: options.faultPlan,
    faultEvents,
    validationMode,
    dataPlaneVerified: status === "completed" && scenarioRequiresMediaPipeline(scenarioId),
    mediaVerified:
      status === "completed" &&
      scenarioRequiresMediaPipeline(scenarioId) &&
      (validationMode === "webrtc_rtp" || profileProbeResult?.status === "passed"),
    sampleDurationMs,
    sampleFramesDecoded,
    sampleFramesDropped,
    sampleSequenceGapDrops,
    sampleDecodeErrorDrops,
    sampleTransientDrops,
    sampleFpsElapsedMs,
    sampleFpsTargetDurationMs,
    sampleObservedFps,
    sampleObservedFpsAtTargetDuration,
    sampleRenderFramesPresented,
    sampleObservedRenderFps,
    sampleObservedRenderFpsAtTargetDuration,
    sampleRenderQueueReplacements,
    sampleRenderPresentSkips,
    thresholds: {
      minSampleDurationMs,
      minDecodedFrames,
      minFps,
    },
    failureReason,
    errorMessage,
    startedAt,
    finishedAt: now(),
    stages,
  });

  try {
    stage("preflight", "started");
    await unwrap(commands.serviceBootstrapIfNeeded(), "service_unhealthy");
    await unwrap(commands.serviceWaitForHealthy(10), "service_unhealthy");
    const runtime = await unwrap(commands.ipcRuntimeSnapshot(), "service_unhealthy");
    controllerDeviceId = await ensureLocalDeviceRegistered(commands, runtime, now);
    const peerSelection = await waitForLanPeer(
      commands,
      options.targetDeviceId,
      transportKind,
      timeoutMs,
      sampleIntervalMs,
      scenarioRequiresReadyPeer(scenarioId)
    );
    const selectedPeer = peerSelection.peer;
    peer = selectedPeer;
    if (peerSelection.failureReason) {
      stage("preflight", "failed", peerSelection.message);
      return finish("failed", peerSelection.failureReason, peerSelection.message);
    }
    if (!selectedPeer) {
      stage("preflight", "failed", "No LAN peer available");
      return finish("failed", "peer_not_found", "No LAN peer available");
    }
    if (
      options.expectedPeerBuildId &&
      !lanBuildIdsMatch(selectedPeer.service_build_id, options.expectedPeerBuildId)
    ) {
      const actualBuild = selectedPeer.service_build_id?.trim() || "unknown";
      const message = `LAN peer build mismatch: expected ${options.expectedPeerBuildId}, got ${actualBuild}. Rebuild and restart the peer before running paired media canaries.`;
      stage("preflight", "skipped", message);
      return finish("skipped", "peer_version_mismatch", message);
    }
    stage("preflight", "completed");

    if (scenarioId === "cross.e2e.discovery") {
      stage("assert", "completed");
      return finish("completed");
    }

    if (requestMediaProfile && !options.requestedProfile) {
      requestedProfile = defaultLanMediaProfileForPeer(selectedPeer);
    }
    const sessionStartProfile =
      options.adaptive ? buildAdaptiveStartupMediaProfile(requestedProfile) : requestedProfile;

    if (scenarioId === "cross.fault.recovery" && !commands.crossE2EInjectFault) {
      const message = "Cross-device fault recovery requires mrd-service fault injection support";
      faultEvents.push({
        type: options.faultPlan?.type ?? "network.pause_peer",
        status: "unsupported",
        timestamp: now(),
        message,
      });
      stage("fault", "skipped", message);
      return finish("skipped", "fault_injection_unsupported", message);
    }

    stage("pairing", "started");
    sessionId = options.createSessionId?.() ?? createDefaultSessionId(selectedPeer.device_id, now());
    await unwrap(
      commands.ipcStartLanRemoteSession(
        sessionId,
        selectedPeer.device_id,
        transportKind,
        sessionStartProfile
      ),
      "session_start_failed"
    );
    sessionStarted = true;
    stage("pairing", "completed");

    stage("capture_source", "started");
    captureSourceSelection = await selectRemoteCaptureSourceForSession(
      commands,
      sessionId,
      options.preferredCaptureSourceId,
      options.preferredCaptureSourceKind,
      requestedProfile
    );
    captureSource = captureSourceSelection.source;
    stage("capture_source", "completed");

    if (displayModePolicy !== "none" && requestedProfile) {
      stage("display_mode", "started");
      const modeResult = await maybeApplyRemoteDisplayMode(
        commands,
        sessionId,
        requestedProfile,
        displayModePolicy
      );
      if (modeResult.status === "failed") {
        stage("display_mode", "failed", modeResult.error);
        return finish("failed", "display_mode_failed", modeResult.error);
      }
      if (modeResult.status === "skipped") {
        stage("display_mode", displayModePolicy === "required" ? "failed" : "skipped", modeResult.error);
        if (displayModePolicy === "required") {
          return finish("failed", "display_mode_failed", modeResult.error);
        }
      } else {
        displayModeChange = modeResult.change;
        stage("display_mode", "completed");
        if (captureSource) {
          stage("capture_source", "started");
          try {
            const refreshSourceId =
              options.preferredCaptureSourceId?.trim() ||
              displayModeChange.active?.source_id ||
              captureSource.id;
            captureSourceSelection = await selectRemoteCaptureSourceForSession(
              commands,
              sessionId,
              refreshSourceId
            );
            captureSource = captureSourceSelection.source;
            stage("capture_source", "completed");
          } catch (error) {
            const message =
              error instanceof Error
                ? error.message
                : `Remote capture source refresh failed: ${String(error)}`;
            captureSourceSelection = {
              session_id: sessionId,
              source: captureSource,
              status: "selected",
              reason: `Reused pre-display-mode source after refresh failed: ${message}`,
            };
            stage("capture_source", "skipped", captureSourceSelection.reason ?? undefined);
          }
        }
      }
    }

    if (options.adaptive) {
      stage("adaptation", "started");
      if (!commands.ipcConfigureMediaAdaptation) {
        const message = "LAN adaptive media requires ipcConfigureMediaAdaptation";
        stage("adaptation", "failed", message);
        return finish("failed", "runtime_error", message);
      }
      mediaAdaptationSnapshot = await unwrap(
        commands.ipcConfigureMediaAdaptation(
          sessionId,
          buildAdaptiveMediaConfig(requestedProfile, captureSource, options.adaptiveConfig)
        ),
        "runtime_error"
      );
      stage("adaptation", "completed");
    }

    if (renderDisplayEnabled) {
      stage("display", "started");
      displayWindow = await unwrap(
        commands.openRemoteDisplayWindow({
          sessionId,
          ...(options.preferredRenderDisplaySourceId
            ? { preferredDisplaySourceId: options.preferredRenderDisplaySourceId }
            : {}),
          ...(captureSource
            ? { avoidCaptureSourceId: captureSourceDisplayPlacementRef(captureSource) }
            : {}),
        }),
        "display_window_failed"
      );

      const nativeSurface = await waitForRemoteDisplayNativeSurface(
        commands,
        sessionId,
        displayWindow,
        timeoutMs,
        now
      );
      displayWindow = nativeSurface.displayWindow;
      mediaPipelineSnapshot = nativeSurface.mediaPipelineSnapshot;
      if (!nativeSurface.attached) {
        stage("display", "failed", nativeSurface.message);
        return finish("failed", "display_window_failed", nativeSurface.message);
      }
      stage("display", "completed");
    } else {
      stage("display", "skipped", "Render display disabled for diagnostics");
    }

    stage("receiver", "started");
    await unwrap(commands.ipcStartReceiver(sessionId), "receiver_start_failed");
    stage("receiver", "completed");

    if (scenarioId === "cross.fault.recovery") {
      const faultPlan = options.faultPlan ?? { type: "network.pause_peer" as const, durationMs: 1000 };
      stage("fault", "started");
      const faultResult = await commands.crossE2EInjectFault?.(sessionId, faultPlan);
      if (!faultResult) {
        const message = "Cross-device fault injection command is unavailable";
        faultEvents.push({
          type: faultPlan.type,
          status: "unsupported",
          timestamp: now(),
          message,
        });
        stage("fault", "skipped", message);
        return finish("skipped", "fault_injection_unsupported", message);
      }
      if (!faultResult.ok) {
        const message = faultResult.error.message;
        faultEvents.push({
          type: faultPlan.type,
          status: "failed",
          timestamp: now(),
          message,
        });
        stage("fault", "failed", message);
        return finish("failed", "fault_injection_failed", message);
      }
      faultEvents.push({
        type: faultPlan.type,
        status: "injected",
        timestamp: now(),
        message: faultResult.value,
      });
      stage("fault", "completed");
    }

    stage("sample", "started");
    let deadline = now() + sampleWindowTimeoutMs;
    let sampleStartedAt = now();
    while (now() <= deadline) {
      sessionSnapshot = await unwrap(
        commands.ipcSessionSnapshot(sessionId),
        "runtime_error"
      );
      probeSnapshot = await unwrap(commands.ipcProbeSnapshot(sessionId), "runtime_error");
      mediaPipelineSnapshot = await unwrap(
        commands.ipcMediaPipelineSnapshot(sessionId),
        "runtime_error"
      );
      mediaAdaptationSnapshot = mediaPipelineSnapshot.adaptation ?? mediaAdaptationSnapshot;
      if (displayWindow) {
        displayWindow = syncDisplayWindowFromPipeline(displayWindow, mediaPipelineSnapshot);
      }
      const renderCappedProfile = renderProfileCapEnabled
        ? buildRenderCappedMediaProfile(
            requestedProfile,
            mediaPipelineSnapshot,
            renderCappedProfileApplied
          )
        : undefined;
      if (renderCappedProfile && commands.ipcUpdateMediaProfile && !options.adaptive) {
        await unwrap(
          commands.ipcUpdateMediaProfile(sessionId, renderCappedProfile),
          "runtime_error"
        );
        requestedProfile = renderCappedProfile;
        renderCappedProfileApplied = true;
        sampleStartedAt = now();
        deadline = sampleStartedAt + sampleWindowTimeoutMs;
        sampleDurationMs = 0;
        sampleFramesDecoded = 0;
        sampleFramesDropped = 0;
        sampleSequenceGapDrops = undefined;
        sampleDecodeErrorDrops = undefined;
        sampleTransientDrops = undefined;
        sampleFpsElapsedMs = undefined;
        sampleFpsTargetDurationMs = undefined;
        sampleObservedFps = undefined;
        sampleObservedFpsAtTargetDuration = undefined;
        sampleRenderFramesPresented = 0;
        sampleObservedRenderFps = undefined;
        sampleObservedRenderFpsAtTargetDuration = undefined;
        sampleRenderQueueReplacements = undefined;
        sampleRenderPresentSkips = undefined;
        sampleFpsBaseline = undefined;
        sampleFpsBaselineDeferred = false;
        await sleep(sampleIntervalMs);
        continue;
      }
      sampleDurationMs = now() - sampleStartedAt;
      const renderPresentedFrames = mediaPipelineSnapshot.render_presented_frames;
      const sampleFpsBaselineReady =
        !displayWindow ||
        typeof renderPresentedFrames !== "number" ||
        renderPresentedFrames > 0;
      if (!sampleFpsBaseline && !sampleFpsBaselineReady) {
        sampleFpsBaselineDeferred = true;
        await sleep(sampleIntervalMs);
        continue;
      }
      if (!sampleFpsBaseline) {
        const waitForDeltaSample = sampleFpsBaselineDeferred;
        sampleFpsBaseline = {
          framesDecoded: probeSnapshot.frames_decoded,
          framesDropped: probeSnapshot.frames_dropped,
          sequenceGapDrops: probeSnapshot.sequence_gap_drops ?? 0,
          decodeErrorDrops: probeSnapshot.decode_error_drops ?? 0,
          transientDrops: probeSnapshot.transient_drops ?? 0,
          renderPresentedFrames: renderPresentedFrames ?? 0,
          renderQueueReplacements: mediaPipelineSnapshot.render_queue_replacements ?? 0,
          renderPresentSkips: mediaPipelineSnapshot.render_present_skips ?? 0,
          sampleDurationMs,
        };
        sampleFpsBaselineDeferred = false;
        if (waitForDeltaSample) {
          await sleep(sampleIntervalMs);
          continue;
        }
      } else {
        sampleFramesDecoded = Math.max(
          0,
          probeSnapshot.frames_decoded - sampleFpsBaseline.framesDecoded
        );
        sampleFramesDropped = Math.max(
          0,
          probeSnapshot.frames_dropped - sampleFpsBaseline.framesDropped
        );
        sampleSequenceGapDrops = Math.max(
          0,
          (probeSnapshot.sequence_gap_drops ?? 0) - sampleFpsBaseline.sequenceGapDrops
        );
        sampleDecodeErrorDrops = Math.max(
          0,
          (probeSnapshot.decode_error_drops ?? 0) - sampleFpsBaseline.decodeErrorDrops
        );
        sampleTransientDrops = Math.max(
          0,
          (probeSnapshot.transient_drops ?? 0) - sampleFpsBaseline.transientDrops
        );
        sampleFpsElapsedMs =
          sampleDurationMs - sampleFpsBaseline.sampleDurationMs;
        sampleObservedFps =
          sampleFpsElapsedMs > 0
            ? (sampleFramesDecoded * 1000) / sampleFpsElapsedMs
            : undefined;
        sampleFpsTargetDurationMs =
          minSampleDurationMs > 0 &&
          isWithinSampleDurationTolerance(sampleFpsElapsedMs, minSampleDurationMs)
            ? minSampleDurationMs
            : undefined;
        sampleObservedFpsAtTargetDuration =
          sampleFpsTargetDurationMs != null
            ? (sampleFramesDecoded * 1000) / sampleFpsTargetDurationMs
            : undefined;
        sampleRenderFramesPresented = Math.max(
          0,
          (mediaPipelineSnapshot.render_presented_frames ?? 0) -
            sampleFpsBaseline.renderPresentedFrames
        );
        sampleObservedRenderFps =
          sampleFpsElapsedMs > 0
            ? (sampleRenderFramesPresented * 1000) / sampleFpsElapsedMs
            : undefined;
        sampleObservedRenderFpsAtTargetDuration =
          sampleFpsTargetDurationMs != null
            ? (sampleRenderFramesPresented * 1000) / sampleFpsTargetDurationMs
            : undefined;
        sampleRenderQueueReplacements = Math.max(
          0,
          (mediaPipelineSnapshot.render_queue_replacements ?? 0) -
            sampleFpsBaseline.renderQueueReplacements
        );
        sampleRenderPresentSkips = Math.max(
          0,
          (mediaPipelineSnapshot.render_present_skips ?? 0) -
            sampleFpsBaseline.renderPresentSkips
        );
      }
      const fpsForThreshold = sampleObservedFps ?? probeSnapshot.current_fps ?? 0;

      if (sessionSnapshot.state === "failed" || sessionSnapshot.last_error) {
        const message = sessionSnapshot.last_error ?? "LAN session entered failed state";
        stage("sample", "failed", message);
        return finish("failed", "runtime_error", message);
      }
      if (probeSnapshot.last_error) {
        stage("sample", "failed", probeSnapshot.last_error);
        return finish("failed", "runtime_error", probeSnapshot.last_error);
      }
      profileProbeResult = evaluateMediaProfileProbe(
        probeSnapshot,
        requestedProfile,
        captureSource,
        validationMode
      );
      const profileMismatch = describeProfileProbeFailure(profileProbeResult);
      const sampleReadinessDurationMs = sampleFpsBaseline
        ? sampleFpsElapsedMs ?? 0
        : sampleDurationMs;
      const sampleDurationReady = hasReachedSampleDuration(
        sampleReadinessDurationMs,
        minSampleDurationMs
      );
      if (!options.adaptive && profileMismatch && sampleDurationReady) {
        stage("assert", "failed", profileMismatch);
        return finish("failed", "media_profile_mismatch", profileMismatch);
      }
      if (
        !options.adaptive &&
        profileProbeResult?.status === "degraded" &&
        probeSnapshot.frames_decoded >= minDecodedFrames &&
        sampleDurationReady
      ) {
        const message =
          profileProbeResult.error ?? "Runtime media profile was downgraded by the remote source";
        stage("assert", "skipped", message);
        return finish("skipped", "profile_downgraded", message);
      }
      if (
        sessionSnapshot.receiver_active &&
        probeSnapshot.frames_decoded >= minDecodedFrames &&
        (validationMode !== "quic_datagram" || probeSnapshot.media_probe_valid === true) &&
        fpsForThreshold >= minFps &&
        sampleDurationReady
      ) {
        stage("sample", "completed");
        stage("assert", "completed");
        return finish("completed");
      }

      await sleep(sampleIntervalMs);
    }

    const finalFps = sampleObservedFps ?? probeSnapshot?.current_fps ?? 0;
    const finalSampleDurationMs = sampleFpsElapsedMs ?? sampleDurationMs;
    const message = `LAN ${formatValidationMode(validationMode)} did not reach threshold: decoded ${probeSnapshot?.frames_decoded ?? 0}/${minDecodedFrames}, fps ${finalFps}/${minFps}, sample ${finalSampleDurationMs}/${minSampleDurationMs} ms`;
    stage("assert", "failed", message);
    return finish("failed", "no_remote_frames", message);
  } catch (error) {
    const mapped = error instanceof LanE2ECommandError ? error.reason : "runtime_error";
    const message = error instanceof Error ? error.message : String(error);
    stage(stageForFailure(mapped), "failed", message);
    return finish("failed", mapped, message);
  } finally {
    if (stopOnComplete && sessionStarted && sessionId) {
      stage("cleanup", "started");
      if (
        displayModeChange?.restore_required &&
        commands.ipcRestoreRemoteDisplayMode
      ) {
        const restoreResult = await commands.ipcRestoreRemoteDisplayMode(sessionId);
        if (!restoreResult.ok) {
          stage("cleanup", "failed", restoreResult.error.message);
        }
      }
      const stopResult = await commands.ipcStopSession(sessionId);
      if (stopResult.ok) {
        stage("cleanup", "completed");
      } else {
        stage("cleanup", "failed", stopResult.error.message);
      }
    }
  }
}

function syncDisplayWindowFromPipeline(
  displayWindow: RemoteDisplayWindowContext,
  snapshot: MediaPipelineSnapshot
): RemoteDisplayWindowContext {
  const attachedSurface = snapshot.attached_surfaces.find(
    (surface) => surface.surface_id === displayWindow.surface_id
  );
  if (!attachedSurface) return displayWindow;

  return {
    ...displayWindow,
    renderer_attached: true,
    native_surface_attached: true,
    render_mode: renderModeForAttachedSurface(attachedSurface.backend),
  };
}

async function waitForRemoteDisplayNativeSurface(
  commands: LanE2EAutomationCommands,
  sessionId: string,
  displayWindow: RemoteDisplayWindowContext,
  timeoutMs: number,
  now: () => number
): Promise<{
  attached: boolean;
  displayWindow: RemoteDisplayWindowContext;
  mediaPipelineSnapshot?: MediaPipelineSnapshot;
  message?: string;
}> {
  const deadline =
    now() + Math.min(timeoutMs, REMOTE_DISPLAY_SURFACE_ATTACH_TIMEOUT_MS);
  let currentWindow = displayWindow;
  let latestSnapshot: MediaPipelineSnapshot | undefined;

  while (true) {
    latestSnapshot = await unwrap(
      commands.ipcMediaPipelineSnapshot(sessionId),
      "runtime_error"
    );
    currentWindow = syncDisplayWindowFromPipeline(currentWindow, latestSnapshot);
    if (remoteDisplayNativeSurfaceAttached(currentWindow, latestSnapshot)) {
      return {
        attached: true,
        displayWindow: currentWindow,
        mediaPipelineSnapshot: latestSnapshot,
      };
    }

    if (now() >= deadline) break;
    await sleep(REMOTE_DISPLAY_SURFACE_ATTACH_POLL_MS);
  }

  return {
    attached: false,
    displayWindow: currentWindow,
    mediaPipelineSnapshot: latestSnapshot,
    message: `Remote display native surface did not attach for ${displayWindow.label}/${displayWindow.surface_id}; attached surfaces: ${
      latestSnapshot?.attached_surfaces.map((surface) => surface.surface_id).join(", ") ||
      "none"
    }`,
  };
}

function remoteDisplayNativeSurfaceAttached(
  displayWindow: RemoteDisplayWindowContext,
  snapshot: MediaPipelineSnapshot
): boolean {
  return (
    displayWindow.native_surface_attached === true &&
    snapshot.attached_surfaces.some(
      (surface) => surface.surface_id === displayWindow.surface_id
    )
  );
}

function renderModeForAttachedSurface(backend: string): RemoteDisplayWindowContext["render_mode"] {
  if (backend === "macos") return "macos_native";
  if (backend === "linux") return "linux_native";
  if (backend === "d3d12") return "d3d12_native";
  return "d3d11_native";
}

function buildRenderCappedMediaProfile(
  requestedProfile: MediaProfile | undefined,
  snapshot: MediaPipelineSnapshot | undefined,
  alreadyApplied: boolean
): MediaProfile | undefined {
  if (alreadyApplied || !requestedProfile || !snapshot) return undefined;
  const renderTargetFps = snapshot.render_pacing_target_fps;
  if (
    !Number.isFinite(renderTargetFps) ||
    !renderTargetFps ||
    renderTargetFps <= 0 ||
    renderTargetFps >= requestedProfile.fps
  ) {
    return undefined;
  }
  return {
    ...requestedProfile,
    fps: Math.max(1, Math.floor(renderTargetFps)),
  };
}

function hasReachedSampleDuration(
  sampleDurationMs: number,
  minSampleDurationMs: number
): boolean {
  return sampleDurationMs + sampleDurationToleranceFor(minSampleDurationMs) >= minSampleDurationMs;
}

function isWithinSampleDurationTolerance(
  sampleDurationMs: number,
  targetDurationMs: number
): boolean {
  const toleranceMs = sampleDurationToleranceFor(targetDurationMs);
  return Math.abs(sampleDurationMs - targetDurationMs) <= toleranceMs;
}

function sampleDurationToleranceFor(minSampleDurationMs: number): number {
  if (minSampleDurationMs <= 0) return 0;
  return Math.min(
    SAMPLE_DURATION_TOLERANCE_MS,
    Math.floor(minSampleDurationMs * 0.01)
  );
}

function buildAdaptiveMediaConfig(
  requestedProfile: MediaProfile | undefined,
  captureSource: CaptureSource | undefined,
  overrides: AdaptiveMediaConfig | undefined
): AdaptiveMediaConfig {
  const ceilingProfile = requestedProfile ?? DEFAULT_LAN_HEVC_MEDIA_PROFILE;
  const normalizedCeilingProfile = {
    ...ceilingProfile,
    bitrate_mbps: Math.max(80, ceilingProfile.bitrate_mbps),
    codec: ceilingProfile.codec || "h264",
  };
  const sourceAspect =
    captureSource && captureSource.width > 0 && captureSource.height > 0
      ? captureSource.width / captureSource.height
      : ceilingProfile.width / ceilingProfile.height;
  const floorWidth = 1280;
  const floorHeight = Math.max(2, Math.floor(floorWidth / sourceAspect / 2) * 2);
  return {
    enabled: true,
    mode: "keyframe_ladder",
    ceiling_profile: normalizedCeilingProfile,
    floor_profile: {
      ...normalizedCeilingProfile,
      width: floorWidth,
      height: floorHeight,
      fps: 60,
      bitrate_mbps: 10,
    },
    ladder: [],
    dynamic_resolution_enabled: false,
    downshift_cooldown_ms: 2_000,
    upshift_hold_ms: 5_000,
    ...overrides,
  };
}

function buildAdaptiveStartupMediaProfile(
  requestedProfile: MediaProfile | undefined
): MediaProfile | undefined {
  if (!requestedProfile) return undefined;
  if (
    requestedProfile.fps < ADAPTIVE_STARTUP_SAFE_MIN_FPS ||
    requestedProfile.bitrate_mbps < ADAPTIVE_STARTUP_SAFE_MIN_BITRATE_MBPS
  ) {
    return requestedProfile;
  }

  return {
    ...requestedProfile,
    bitrate_mbps: Math.max(
      1,
      Math.floor(requestedProfile.bitrate_mbps * ADAPTIVE_STARTUP_SAFE_BITRATE_RATIO)
    ),
  };
}

async function maybeApplyRemoteDisplayMode(
  commands: LanE2EAutomationCommands,
  sessionId: string,
  requestedProfile: MediaProfile,
  policy: "temporary" | "required"
): Promise<
  | { status: "changed"; change: DisplayModeChange }
  | { status: "skipped"; error: string }
  | { status: "failed"; error: string }
> {
  if (!commands.ipcListRemoteDisplayModes || !commands.ipcSetRemoteDisplayMode) {
    return {
      status: policy === "required" ? "failed" : "skipped",
      error: "Remote display mode control commands are unavailable",
    };
  }

  const modesResult = await commands.ipcListRemoteDisplayModes(sessionId);
  if (!modesResult.ok) {
    return {
      status: policy === "required" ? "failed" : "skipped",
      error: modesResult.error.message,
    };
  }

  const mode = chooseRemoteDisplayMode(
    modesResult.value,
    requestedProfile.width,
    requestedProfile.height,
    requestedProfile.fps
  );
  if (!mode) {
    return {
      status: policy === "required" ? "failed" : "skipped",
      error: `No remote display mode matches ${formatMediaProfile(requestedProfile)}`,
    };
  }

  const setResult = await commands.ipcSetRemoteDisplayMode(sessionId, mode, true);
  if (!setResult.ok) {
    return {
      status: policy === "required" ? "failed" : "skipped",
      error: setResult.error.message,
    };
  }

  return { status: "changed", change: setResult.value };
}

function chooseRemoteDisplayMode(
  modes: DisplayMode[],
  width: number,
  height: number,
  refreshHz: number
): DisplayMode | undefined {
  return [...modes]
    .filter((mode) => mode.width > 0 && mode.height > 0 && mode.refresh_hz > 0)
    .sort((left, right) => {
      const leftScore = displayModeScore(left, width, height, refreshHz);
      const rightScore = displayModeScore(right, width, height, refreshHz);
      for (let index = 0; index < leftScore.length; index += 1) {
        const delta = (leftScore[index] ?? 0) - (rightScore[index] ?? 0);
        if (delta !== 0) return delta;
      }
      return right.refresh_hz - left.refresh_hz || right.width * right.height - left.width * left.height;
    })[0];
}

function displayModeScore(
  mode: DisplayMode,
  width: number,
  height: number,
  refreshHz: number
): [number, number, number, number] {
  const targetAspect = width / height;
  const modeAspect = mode.width / mode.height;
  return [
    Math.round(Math.abs(modeAspect - targetAspect) * 10_000),
    Math.abs(mode.height - height),
    Math.abs(mode.width - width),
    Math.abs(mode.refresh_hz - refreshHz),
  ];
}

async function selectRemoteCaptureSourceForSession(
  commands: LanE2EAutomationCommands,
  sessionId: string,
  preferredSourceId?: string,
  preferredSourceKind?: string,
  requestedProfile?: MediaProfile
): Promise<CaptureSourceSelection> {
  const sources = await unwrap(
    commands.ipcListRemoteCaptureSources(sessionId, false, 24),
    "capture_source_failed"
  );
  const normalizedPreferredSourceId = preferredSourceId?.trim().toLowerCase();
  if (normalizedPreferredSourceId) {
    const preferredSource = sources.find(
      (source) => source.id.toLowerCase() === normalizedPreferredSourceId
    );
    if (!preferredSource) {
      throw new LanE2ECommandError(
        "capture_source_failed",
        `Requested remote capture source is unavailable: ${preferredSourceId?.trim()}`
      );
    }
    return selectRemoteCaptureSource(commands, sessionId, preferredSource);
  }

  if (requestedProfile) {
    const profileAwareSelection = await selectDisplayCaptureSourceForProfile(
      commands,
      sessionId,
      sources,
      preferredSourceKind,
      requestedProfile
    );
    if (profileAwareSelection) return profileAwareSelection;
  }

  const preferredSource = pickPreferredCaptureSource(sources, preferredSourceKind);
  if (!preferredSource) {
    throw new LanE2ECommandError(
      "capture_source_failed",
      "No remote capture source available for LAN E2E"
    );
  }

  return selectRemoteCaptureSource(commands, sessionId, preferredSource);
}

async function selectRemoteCaptureSource(
  commands: LanE2EAutomationCommands,
  sessionId: string,
  source: CaptureSource
): Promise<CaptureSourceSelection> {
  const selection = await unwrap(
    commands.ipcSelectRemoteCaptureSource(sessionId, source.id),
    "capture_source_failed"
  );
  if (selection.status.toLowerCase() !== "selected") {
    throw new LanE2ECommandError(
      "capture_source_failed",
      selection.reason ?? `Remote capture source rejected: ${source.id}`
    );
  }
  return selection;
}

function captureSourceDisplayPlacementRef(source: CaptureSource): string {
  if (!source.source_kind.toLowerCase().includes("display")) return source.id;
  return displayNameRefFromCaptureSource(source) ?? source.id;
}

function displayNameRefFromCaptureSource(source: CaptureSource): string | undefined {
  for (const candidate of [source.class_name, source.title]) {
    const value = candidate?.trim();
    if (value && /DISPLAY\d+/i.test(value)) return value;
  }
  return undefined;
}

async function selectDisplayCaptureSourceForProfile(
  commands: LanE2EAutomationCommands,
  sessionId: string,
  sources: CaptureSource[],
  preferredSourceKind: string | undefined,
  requestedProfile: MediaProfile
): Promise<CaptureSourceSelection | undefined> {
  if (!commands.ipcListRemoteDisplayModes) return undefined;

  const candidates = displayCaptureCandidatesForProfile(sources, preferredSourceKind);
  if (candidates.length < 2) return undefined;

  let best:
    | {
        selection: CaptureSourceSelection;
        score: ReturnType<typeof displayModeScore>;
        refreshHz: number;
        pixels: number;
      }
    | undefined;

  for (const candidate of candidates) {
    const selectionResult = await commands.ipcSelectRemoteCaptureSource(sessionId, candidate.id);
    if (!selectionResult.ok || selectionResult.value.status.toLowerCase() !== "selected") {
      continue;
    }
    const modesResult = await commands.ipcListRemoteDisplayModes(sessionId);
    if (!modesResult.ok) continue;
    const mode = chooseRemoteDisplayMode(
      modesResult.value,
      requestedProfile.width,
      requestedProfile.height,
      requestedProfile.fps
    );
    if (!mode) continue;

    const score = displayModeScore(
      mode,
      requestedProfile.width,
      requestedProfile.height,
      requestedProfile.fps
    );
    const ranked = {
      selection: selectionResult.value,
      score,
      refreshHz: mode.refresh_hz,
      pixels: mode.width * mode.height,
    };
    if (!best || compareDisplayModeScores(ranked, best) < 0) {
      best = ranked;
    }
  }

  if (!best) return undefined;
  const finalSelectionResult = await commands.ipcSelectRemoteCaptureSource(
    sessionId,
    best.selection.source.id
  );
  if (finalSelectionResult.ok && finalSelectionResult.value.status.toLowerCase() === "selected") {
    return finalSelectionResult.value;
  }
  return best.selection;
}

function displayCaptureCandidatesForProfile(
  sources: CaptureSource[],
  preferredSourceKind?: string
): CaptureSource[] {
  const normalizedPreferredKind = preferredSourceKind?.trim();
  const isDisplaySource = (source: CaptureSource) =>
    source.source_kind === "display_shared" || source.source_kind === "display";
  if (normalizedPreferredKind) {
    return sources.filter(
      (source) => source.source_kind === normalizedPreferredKind && isDisplaySource(source)
    );
  }

  const sharedDisplays = sources.filter((source) => source.source_kind === "display_shared");
  if (sharedDisplays.length > 1) return sharedDisplays;
  const displays = sources.filter((source) => source.source_kind === "display");
  if (displays.length > 1) return displays;
  return [];
}

function compareDisplayModeScores(
  left: { score: ReturnType<typeof displayModeScore>; refreshHz: number; pixels: number },
  right: { score: ReturnType<typeof displayModeScore>; refreshHz: number; pixels: number }
): number {
  for (let index = 0; index < left.score.length; index += 1) {
    const delta = (left.score[index] ?? 0) - (right.score[index] ?? 0);
    if (delta !== 0) return delta;
  }
  return right.refreshHz - left.refreshHz || right.pixels - left.pixels;
}

function pickPreferredCaptureSource(
  sources: CaptureSource[],
  preferredSourceKind?: string
): CaptureSource | undefined {
  const normalizedPreferredKind = preferredSourceKind?.trim();
  if (normalizedPreferredKind) {
    const matchingKind = sources.find((source) => source.source_kind === normalizedPreferredKind);
    if (matchingKind) return matchingKind;
  }
  return (
    sources.find((source) => source.source_kind === "display_shared") ??
    sources.find((source) => source.source_kind === "display") ??
    sources.find((source) => source.source_kind === "window") ??
    sources[0]
  );
}

async function waitForLanPeer(
  commands: LanE2EAutomationCommands,
  targetDeviceId: string | undefined,
  transportKind: string,
  timeoutMs: number,
  pollIntervalMs: number,
  requireReadyPeer: boolean
): Promise<ReturnType<typeof selectPeer>> {
  const deadline = Date.now() + timeoutMs;
  let lastSelection: ReturnType<typeof selectPeer> | undefined;

  while (true) {
    const discovery = await unwrap(commands.ipcRefreshLanDiscovery(), "peer_not_found");
    lastSelection = selectPeer(discovery, targetDeviceId, transportKind, requireReadyPeer);

    if (lastSelection.failureReason !== "peer_not_found") {
      return lastSelection;
    }
    if (Date.now() >= deadline) {
      return lastSelection;
    }

    await sleep(pollIntervalMs);
  }
}

function selectPeer(
  snapshot: LanDiscoverySnapshot,
  targetDeviceId: string | undefined,
  transportKind: string,
  requireReadyPeer: boolean
): {
  peer?: LanPeerInfo;
  failureReason?: "peer_not_found" | "peer_not_ready";
  message?: string;
} {
  const peers = snapshot.peers;
  if (targetDeviceId) {
    const targetPeer = peers.find((peer) => peer.device_id === targetDeviceId);
    if (!targetPeer) {
      return {
        failureReason: "peer_not_found",
        message: `LAN peer not found: ${targetDeviceId}`,
      };
    }
    if (!requireReadyPeer) return { peer: targetPeer };
    if (!isPeerReady(targetPeer, transportKind)) {
      return {
        peer: targetPeer,
        failureReason: "peer_not_ready",
        message: buildPeerNotReadyMessage(targetPeer, transportKind),
      };
    }
    return { peer: targetPeer };
  }

  if (!requireReadyPeer && peers[0]) return { peer: peers[0] };

  const readyPeer = peers.find((peer) => isPeerReady(peer, transportKind));
  if (readyPeer) return { peer: readyPeer };
  if (peers[0]) {
    return {
      peer: peers[0],
      failureReason: "peer_not_ready",
      message: buildPeerNotReadyMessage(peers[0], transportKind),
    };
  }

  return {
    failureReason: "peer_not_found",
    message: "No LAN peer available",
  };
}

function isPeerReady(peer: LanPeerInfo, transportKind: string): boolean {
  return peer.p2p_available && peerSupportsTransport(peer, transportKind);
}

function scenarioRequiresReadyPeer(scenarioId: CrossDeviceScenarioId): boolean {
  return scenarioId !== "cross.e2e.discovery";
}

function scenarioRequiresMediaPipeline(scenarioId: CrossDeviceScenarioId): boolean {
  return scenarioId !== "cross.e2e.discovery";
}

function shouldRequestMediaProfile(
  scenarioId: CrossDeviceScenarioId,
  transportKind: string
): boolean {
  return scenarioRequiresMediaPipeline(scenarioId) && transportKind === "quic";
}

function peerSupportsTransport(peer: LanPeerInfo, transportKind: string): boolean {
  const transports = peer.transports.map((transport) => transport.toLowerCase());
  const requestedTransport = transportKind.toLowerCase();
  if (requestedTransport === "quic") {
    const mediaProtocolVersion = peer.media_protocol_version ?? 0;
    const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
      capability.toLowerCase()
    );
    const supportsMediaV3 =
      mediaProtocolVersion >= 3 &&
      (transports.includes(QUIC_DATAGRAM_MEDIA_V3_CAPABILITY) ||
        mediaCapabilities.includes(QUIC_DATAGRAM_MEDIA_V3_CAPABILITY));
    const supportsMediaV2 =
      mediaProtocolVersion >= 2 &&
      (transports.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY) ||
        mediaCapabilities.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY));
    return (
      transports.includes(QUIC_DATAGRAM_MEDIA_CAPABILITY) &&
      transports.includes(QUIC_2K144_MEDIA_CAPABILITY) &&
      transports.includes(MEDIA_PROFILE_CONTROL_CAPABILITY) &&
      (supportsMediaV3 || supportsMediaV2) &&
      findSupportedPlatformMediaCapabilityProfile(mediaCapabilities) !== undefined
    );
  }
  return transports.includes(requestedTransport);
}

function defaultLanMediaProfileForPeer(peer: LanPeerInfo): MediaProfile {
  if (peerSupportsMacosNativeH264(peer)) {
    return { ...DEFAULT_LAN_MACOS_H264_MEDIA_PROFILE };
  }
  return peerSupportsHevcMain(peer)
    ? { ...DEFAULT_LAN_HEVC_MEDIA_PROFILE }
    : { ...DEFAULT_LAN_H264_FALLBACK_MEDIA_PROFILE };
}

function peerSupportsMacosNativeH264(peer: LanPeerInfo): boolean {
  const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
    capability.toLowerCase()
  );
  const macosProfile = REQUIRED_PLATFORM_MEDIA_CAPABILITY_PROFILES.find(
    (profile) => profile.id === "macos"
  );
  return (
    macosProfile?.capabilities.every((capability) =>
      mediaCapabilities.includes(capability.toLowerCase())
    ) ?? false
  );
}

function peerSupportsHevcMain(peer: LanPeerInfo): boolean {
  const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
    capability.toLowerCase()
  );
  const hasAny = (aliases: string[]) =>
    aliases.some((capability) => mediaCapabilities.includes(capability));
  return (
    hasAny(["nvenc_hevc", "encode.nvenc_hevc"]) &&
    hasAny(["nvdec_hevc", "decode.nvdec_hevc"]) &&
    hasAny(["media.hevc_main_420_8bit"])
  );
}

function findSupportedPlatformMediaCapabilityProfile(
  mediaCapabilities: string[]
): RequiredPlatformMediaCapabilityProfile | undefined {
  return REQUIRED_PLATFORM_MEDIA_CAPABILITY_PROFILES.find((profile) =>
    profile.capabilities.every((capability) => mediaCapabilities.includes(capability))
  );
}

function describeMissingPlatformMediaCapabilities(mediaCapabilities: string[]): string {
  return REQUIRED_PLATFORM_MEDIA_CAPABILITY_PROFILES.map((profile) => {
    const missing = profile.capabilities.filter(
      (capability) => !mediaCapabilities.includes(capability)
    );
    return `${profile.label}: ${missing.length > 0 ? missing.join(", ") : "none"}`;
  }).join("; ");
}

function buildPeerNotReadyMessage(peer: LanPeerInfo, transportKind: string): string {
  const transportList = peer.transports.length > 0 ? peer.transports.join(", ") : "none";
  if (!peer.p2p_available) {
    return `LAN peer is discovered but not P2P available: ${peer.device_id}`;
  }
  if (transportKind.toLowerCase() === "quic") {
    const lower = peer.transports.map((transport) => transport.toLowerCase());
    const mediaCapabilities = (peer.media_capabilities ?? []).map((capability) =>
      capability.toLowerCase()
    );
    const mediaProtocolVersion = peer.media_protocol_version ?? 0;
    if (lower.includes(QUIC_DATAGRAM_MEDIA_CAPABILITY)) {
      const missing = [
        QUIC_2K144_MEDIA_CAPABILITY,
        MEDIA_PROFILE_CONTROL_CAPABILITY,
      ]
        .filter((capability) => !lower.includes(capability));
      if (missing.length > 0) {
        return `LAN peer supports ${QUIC_DATAGRAM_MEDIA_CAPABILITY} but not required media controls [${missing.join(", ")}]: ${peer.device_id} supports ${transportList}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch.`;
      }
    }
    if (lower.includes("quic") && !lower.includes(QUIC_DATAGRAM_MEDIA_CAPABILITY)) {
      return `LAN peer advertises legacy quic but not ${QUIC_DATAGRAM_MEDIA_CAPABILITY}: ${peer.device_id} supports ${transportList}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch.`;
    }
    const hasMediaV3 =
      lower.includes(QUIC_DATAGRAM_MEDIA_V3_CAPABILITY) ||
      mediaCapabilities.includes(QUIC_DATAGRAM_MEDIA_V3_CAPABILITY);
    const hasMediaV2 =
      lower.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY) ||
      mediaCapabilities.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY);
    const supportsMediaProtocol =
      (mediaProtocolVersion >= 3 && hasMediaV3) ||
      (mediaProtocolVersion >= 2 && hasMediaV2);
    if (!supportsMediaProtocol) {
      return `LAN peer is not on a compatible QUIC media protocol: ${peer.device_id} supports ${transportList}, media protocol ${mediaProtocolVersion || "unknown"}. Rebuild and restart the peer mrd-service/Rdesk from the same branch.`;
    }
    const supportedPlatformProfile =
      findSupportedPlatformMediaCapabilityProfile(mediaCapabilities);
    if (!supportedPlatformProfile) {
      return `LAN peer is missing a complete platform media capability profile: ${peer.device_id}. Missing by profile [${describeMissingPlatformMediaCapabilities(mediaCapabilities)}]. Rebuild/restart the peer and verify native capture/codec/render support.`;
    }
    return `LAN peer does not support ${QUIC_2K144_MEDIA_CAPABILITY}: ${peer.device_id} supports ${transportList}`;
  }
  return `LAN peer does not support ${transportKind}: ${peer.device_id} supports ${transportList}`;
}

function formatValidationMode(mode: LanE2EAutomationReport["validationMode"]): string {
  return mode === "webrtc_rtp" ? "WebRTC RTP data plane" : "QUIC datagram data plane";
}

function evaluateMediaProfileProbe(
  probe: ProbeSnapshot,
  requestedProfile: MediaProfile | undefined,
  captureSource: CaptureSource | undefined,
  validationMode: LanE2EAutomationReport["validationMode"]
): ProfileProbeResult | undefined {
  if (validationMode !== "quic_datagram" || !requestedProfile || probe.media_probe_valid !== true) {
    return undefined;
  }

  const result = evaluateProfileProbe(toCapabilityProfile(requestedProfile), probe);
  if (
    result.status === "failed" &&
    isExpectedProfileDowngrade(probe, requestedProfile, captureSource)
  ) {
    return {
      ...result,
      status: "degraded",
      error: `Runtime media profile downgraded: requested ${formatMediaProfile(
        requestedProfile
      )}, selected ${formatProbeProfile(probe)}`,
    };
  }

  return result;
}

function describeProfileProbeFailure(result: ProfileProbeResult | undefined): string | null {
  if (!result || result.status === "passed") {
    return null;
  }
  if (result.status === "degraded" || result.status === "skipped") {
    return null;
  }

  return result.error ?? `Runtime media profile probe failed: ${result.status}`;
}

function isExpectedProfileDowngrade(
  probe: ProbeSnapshot,
  requestedProfile: MediaProfile,
  captureSource: CaptureSource | undefined
): boolean {
  const actualWidth = probe.media_probe_width ?? 0;
  const actualHeight = probe.media_probe_height ?? 0;
  const actualFps = probe.media_probe_target_fps ?? 0;
  const actualBitrate = probe.media_probe_target_bitrate_mbps ?? 0;
  if (actualWidth <= 0 || actualHeight <= 0 || actualFps <= 0 || actualBitrate <= 0) {
    return false;
  }
  if (
    actualWidth > requestedProfile.width ||
    actualHeight > requestedProfile.height ||
    actualFps > requestedProfile.fps ||
    actualBitrate > requestedProfile.bitrate_mbps
  ) {
    return false;
  }

  const fit = captureSource
    ? fitSourceWithinProfile(captureSource, requestedProfile)
    : undefined;
  const dimensionsMatchSourceFit =
    !fit || (actualWidth === fit.width && actualHeight === fit.height);
  return dimensionsMatchSourceFit && isActualProfileDowngraded(probe, requestedProfile);
}

function fitSourceWithinProfile(
  source: CaptureSource,
  profile: MediaProfile
): { width: number; height: number } | undefined {
  if (source.width <= 0 || source.height <= 0) return undefined;
  const scale = Math.min(
    profile.width / source.width,
    profile.height / source.height,
    1
  );
  return {
    width: evenDimension(Math.max(2, Math.round(source.width * scale))),
    height: evenDimension(Math.max(2, Math.round(source.height * scale))),
  };
}

function evenDimension(value: number): number {
  return Math.max(2, value & ~1);
}

function isActualProfileDowngraded(
  probe: ProbeSnapshot,
  requestedProfile: MediaProfile
): boolean {
  return (
    (probe.media_probe_width ?? requestedProfile.width) !== requestedProfile.width ||
    (probe.media_probe_height ?? requestedProfile.height) !== requestedProfile.height ||
    (probe.media_probe_target_fps ?? requestedProfile.fps) !== requestedProfile.fps ||
    (probe.media_probe_target_bitrate_mbps ?? requestedProfile.bitrate_mbps) !==
      requestedProfile.bitrate_mbps
  );
}

function formatMediaProfile(profile: MediaProfile): string {
  return `${profile.width}x${profile.height} @ ${profile.fps} FPS / ${profile.bitrate_mbps} Mbps`;
}

function formatProbeProfile(probe: ProbeSnapshot): string {
  return `${probe.media_probe_width ?? 0}x${probe.media_probe_height ?? 0} @ ${
    probe.media_probe_target_fps ?? 0
  } FPS / ${probe.media_probe_target_bitrate_mbps ?? 0} Mbps`;
}

function toCapabilityProfile(profile: MediaProfile): CapabilityProfile {
  return {
    id: "runtime.requested_media_profile",
    width: profile.width,
    height: profile.height,
    fps: profile.fps,
    bitrate_mbps: profile.bitrate_mbps,
    codec: normalizeMediaCodec(profile.codec),
    required_capabilities: [],
  };
}

function normalizeMediaCodec(codec: string): CapabilityProfile["codec"] {
  const normalized = codec.toLowerCase();
  if (normalized === "hevc" || normalized === "h265") return "hevc";
  if (normalized === "av1") return "av1";
  return "h264";
}

function createDefaultSessionId(peerDeviceId: string, now: number): string {
  const safePeer = peerDeviceId.replace(/[^a-zA-Z0-9_-]/g, "-");
  return `lan-e2e-${safePeer}-${now}`;
}

async function ensureLocalDeviceRegistered(
  commands: LanE2EAutomationCommands,
  runtime: RuntimeSnapshot,
  now: () => number
): Promise<string | null> {
  if (runtime.is_registered && runtime.device_id) {
    return runtime.device_id;
  }

  const hardwareInfo = await unwrap(
    commands.getHardwareInfo(),
    "local_device_registration_failed"
  );
  const deviceId = buildLocalLanDeviceId(hardwareInfo.motherboard_serial, now);
  const deviceName = hardwareInfo.hostname?.trim() || "Rdesk LAN Device";
  return await unwrap(
    commands.ipcRegisterDevice(deviceId, deviceName),
    "local_device_registration_failed"
  );
}

function buildLocalLanDeviceId(hardwareSerial: string, now: () => number): string {
  const sanitized = hardwareSerial.replace(/[^a-zA-Z0-9]/g, "").slice(-16);
  return sanitized ? `lan-${sanitized}` : `lan-local-${now()}`;
}

function lanBuildIdsMatch(actual?: string | null, expected?: string | null): boolean {
  const actualBuild = actual?.trim();
  const expectedBuild = expected?.trim();
  if (!expectedBuild) return true;
  if (!actualBuild) return false;
  return (
    actualBuild === expectedBuild ||
    actualBuild.startsWith(expectedBuild) ||
    expectedBuild.startsWith(actualBuild)
  );
}

async function unwrap<T>(
  resultPromise: Promise<AdapterResult<T>>,
  reason: LanE2EFailureReason
): Promise<T> {
  const result = await resultPromise;
  if (result.ok) return result.value;
  throw new LanE2ECommandError(reason, result.error.message);
}

class LanE2ECommandError extends Error {
  constructor(
    public readonly reason: LanE2EFailureReason,
    message: string
  ) {
    super(message);
    this.name = "LanE2ECommandError";
  }
}

function stageForFailure(reason: LanE2EFailureReason): LanE2EStageEvent["stage"] {
  switch (reason) {
    case "service_unhealthy":
    case "local_device_registration_failed":
    case "peer_not_found":
    case "peer_version_mismatch":
    case "peer_not_ready":
      return "preflight";
    case "session_start_failed":
      return "pairing";
    case "capture_source_failed":
      return "capture_source";
    case "display_mode_failed":
      return "display_mode";
    case "receiver_start_failed":
      return "receiver";
    case "display_window_failed":
      return "display";
    case "fault_injection_unsupported":
    case "fault_injection_failed":
      return "fault";
    case "stop_failed":
      return "cleanup";
    default:
      return "sample";
  }
}

function sleep(ms: number): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, ms));
}
