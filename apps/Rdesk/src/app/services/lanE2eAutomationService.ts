import type {
  AdapterResult,
  CaptureSource,
  CaptureSourceSelection,
  LanDiscoverySnapshot,
  LanPeerInfo,
  MediaProfile,
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
  | "peer_not_ready"
  | "session_start_failed"
  | "capture_source_failed"
  | "receiver_start_failed"
  | "display_window_failed"
  | "fault_injection_unsupported"
  | "fault_injection_failed"
  | "no_remote_frames"
  | "media_profile_mismatch"
  | "runtime_error"
  | "stop_failed";

export interface LanE2EStageEvent {
  stage: "preflight" | "pairing" | "session" | "capture_source" | "receiver" | "display" | "fault" | "sample" | "assert" | "cleanup";
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
  sessionSnapshot?: SessionRuntimeSnapshot;
  probeSnapshot?: ProbeSnapshot;
  profileProbeResult?: ProfileProbeResult;
  requestedProfile?: MediaProfile;
  faultPlan?: CrossDeviceFaultPlan;
  faultEvents: CrossDeviceFaultEvent[];
  validationMode: "quic_datagram" | "webrtc_rtp";
  dataPlaneVerified: boolean;
  mediaVerified: boolean;
  sampleDurationMs: number;
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
  ipcListRemoteCaptureSources(
    sessionId: string,
    includePreviews: boolean,
    limit?: number
  ): Promise<AdapterResult<CaptureSource[]>>;
  ipcSelectRemoteCaptureSource(
    sessionId: string,
    sourceId: string
  ): Promise<AdapterResult<CaptureSourceSelection>>;
  ipcStartReceiver(sessionId: string): Promise<AdapterResult<string>>;
  openRemoteDisplayWindow(params: {
    sessionId: string;
  }): Promise<AdapterResult<RemoteDisplayWindowContext>>;
  ipcSessionSnapshot(sessionId: string): Promise<AdapterResult<SessionRuntimeSnapshot>>;
  ipcProbeSnapshot(sessionId: string): Promise<AdapterResult<ProbeSnapshot>>;
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
const MEDIA_PROFILE_CONTROL_CAPABILITY = "media_profile_control_v1";
const REQUIRED_WINDOWS_MEDIA_CAPABILITIES = [
  "dxgi_capture",
  "nvenc_h264",
  "nvdec",
  "d3d11_native_render",
];
const DEFAULT_LAN_MEDIA_PROFILE: MediaProfile = {
  width: 2560,
  height: 1440,
  fps: 144,
  bitrate_mbps: 64,
  codec: "h264",
};

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
  const minDecodedFrames = options.minDecodedFrames ?? DEFAULT_MIN_DECODED_FRAMES;
  const minFps = options.minFps ?? DEFAULT_MIN_FPS;
  const stopOnComplete = options.stopOnComplete ?? true;
  const transportKind = options.transportKind ?? "quic";
  const requestedProfile =
    shouldRequestMediaProfile(scenarioId, transportKind)
      ? options.requestedProfile ?? DEFAULT_LAN_MEDIA_PROFILE
      : options.requestedProfile;
  const validationMode = transportKind === "webrtc" ? "webrtc_rtp" : "quic_datagram";
  let sessionId: string | undefined;
  let peer: LanPeerInfo | undefined;
  let displayWindow: RemoteDisplayWindowContext | undefined;
  let captureSource: CaptureSource | undefined;
  let captureSourceSelection: CaptureSourceSelection | undefined;
  let sessionSnapshot: SessionRuntimeSnapshot | undefined;
  let probeSnapshot: ProbeSnapshot | undefined;
  let profileProbeResult: ProfileProbeResult | undefined;
  let controllerDeviceId: string | null | undefined;
  let sessionStarted = false;
  let sampleDurationMs = 0;

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
    sessionSnapshot,
    probeSnapshot,
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
    stage("preflight", "completed");

    if (scenarioId === "cross.e2e.discovery") {
      stage("assert", "completed");
      return finish("completed");
    }

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
        requestedProfile
      ),
      "session_start_failed"
    );
    sessionStarted = true;
    stage("pairing", "completed");

    stage("capture_source", "started");
    captureSourceSelection = await selectRemoteCaptureSourceForSession(commands, sessionId);
    captureSource = captureSourceSelection.source;
    stage("capture_source", "completed");

    stage("receiver", "started");
    await unwrap(commands.ipcStartReceiver(sessionId), "receiver_start_failed");
    stage("receiver", "completed");

    stage("display", "started");
    displayWindow = await unwrap(
      commands.openRemoteDisplayWindow({ sessionId }),
      "display_window_failed"
    );
    stage("display", "completed");

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
    const deadline = Date.now() + timeoutMs;
    const sampleStartedAt = Date.now();
    while (Date.now() <= deadline) {
      sessionSnapshot = await unwrap(
        commands.ipcSessionSnapshot(sessionId),
        "runtime_error"
      );
      probeSnapshot = await unwrap(commands.ipcProbeSnapshot(sessionId), "runtime_error");
      sampleDurationMs = Date.now() - sampleStartedAt;

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
        validationMode
      );
      const profileMismatch = describeProfileProbeFailure(profileProbeResult);
      if (profileMismatch) {
        stage("assert", "failed", profileMismatch);
        return finish("failed", "media_profile_mismatch", profileMismatch);
      }
      if (
        sessionSnapshot.receiver_active &&
        probeSnapshot.frames_decoded >= minDecodedFrames &&
        (validationMode !== "quic_datagram" || probeSnapshot.media_probe_valid === true) &&
        (probeSnapshot.current_fps ?? 0) >= minFps &&
        sampleDurationMs >= minSampleDurationMs
      ) {
        stage("sample", "completed");
        stage("assert", "completed");
        return finish("completed");
      }

      await sleep(sampleIntervalMs);
    }

    const message = `LAN ${formatValidationMode(validationMode)} did not reach threshold: decoded ${probeSnapshot?.frames_decoded ?? 0}/${minDecodedFrames}, fps ${probeSnapshot?.current_fps ?? 0}/${minFps}, sample ${sampleDurationMs}/${minSampleDurationMs} ms`;
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
      const stopResult = await commands.ipcStopSession(sessionId);
      if (stopResult.ok) {
        stage("cleanup", "completed");
      } else {
        stage("cleanup", "failed", stopResult.error.message);
      }
    }
  }
}

async function selectRemoteCaptureSourceForSession(
  commands: LanE2EAutomationCommands,
  sessionId: string
): Promise<CaptureSourceSelection> {
  const sources = await unwrap(
    commands.ipcListRemoteCaptureSources(sessionId, false, 24),
    "capture_source_failed"
  );
  const preferredSource = pickPreferredCaptureSource(sources);
  if (!preferredSource) {
    throw new LanE2ECommandError(
      "capture_source_failed",
      "No remote capture source available for LAN E2E"
    );
  }

  const selection = await unwrap(
    commands.ipcSelectRemoteCaptureSource(sessionId, preferredSource.id),
    "capture_source_failed"
  );
  if (selection.status.toLowerCase() !== "selected") {
    throw new LanE2ECommandError(
      "capture_source_failed",
      selection.reason ?? `Remote capture source rejected: ${preferredSource.id}`
    );
  }
  return selection;
}

function pickPreferredCaptureSource(sources: CaptureSource[]): CaptureSource | undefined {
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
    return (
      transports.includes(QUIC_DATAGRAM_MEDIA_CAPABILITY) &&
      transports.includes(QUIC_2K144_MEDIA_CAPABILITY) &&
      transports.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY) &&
      transports.includes(MEDIA_PROFILE_CONTROL_CAPABILITY) &&
      mediaProtocolVersion >= 2 &&
      REQUIRED_WINDOWS_MEDIA_CAPABILITIES.every((capability) =>
        mediaCapabilities.includes(capability)
      )
    );
  }
  return transports.includes(requestedTransport);
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
        QUIC_DATAGRAM_MEDIA_V2_CAPABILITY,
        MEDIA_PROFILE_CONTROL_CAPABILITY,
      ]
        .filter((capability) => !lower.includes(capability))
      if (missing.length > 0) {
        return `LAN peer supports ${QUIC_DATAGRAM_MEDIA_CAPABILITY} but not required media controls [${missing.join(", ")}]: ${peer.device_id} supports ${transportList}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch.`;
      }
    }
    if (lower.includes("quic")) {
      return `LAN peer advertises legacy quic but not ${QUIC_DATAGRAM_MEDIA_CAPABILITY}: ${peer.device_id} supports ${transportList}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch.`;
    }
    if (!lower.includes(QUIC_DATAGRAM_MEDIA_V2_CAPABILITY) || mediaProtocolVersion < 2) {
      return `LAN peer is not on the required QUIC media v2 protocol: ${peer.device_id} supports ${transportList}, media protocol ${mediaProtocolVersion || "unknown"}. Rebuild and restart the peer mrd-service/Rdesk from the same branch.`;
    }
    const missingMediaCapabilities = REQUIRED_WINDOWS_MEDIA_CAPABILITIES.filter(
      (capability) => !mediaCapabilities.includes(capability)
    );
    if (missingMediaCapabilities.length > 0) {
      return `LAN peer is missing required Windows media capabilities [${missingMediaCapabilities.join(", ")}]: ${peer.device_id}. Rebuild/restart the peer and verify DXGI/NVENC/NVDEC/D3D11 native support.`;
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
  validationMode: LanE2EAutomationReport["validationMode"]
): ProfileProbeResult | undefined {
  if (validationMode !== "quic_datagram" || !requestedProfile || probe.media_probe_valid !== true) {
    return undefined;
  }

  return evaluateProfileProbe(toCapabilityProfile(requestedProfile), probe);
}

function describeProfileProbeFailure(result: ProfileProbeResult | undefined): string | null {
  if (!result || result.status === "passed") {
    return null;
  }

  return result.error ?? `Runtime media profile probe failed: ${result.status}`;
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
    case "peer_not_ready":
      return "preflight";
    case "session_start_failed":
      return "pairing";
    case "capture_source_failed":
      return "capture_source";
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
