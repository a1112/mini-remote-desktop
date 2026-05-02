import type {
  AdapterResult,
  LanDiscoverySnapshot,
  LanPeerInfo,
  ProbeSnapshot,
  RemoteDisplayWindowContext,
  RuntimeSnapshot,
  SessionRuntimeSnapshot,
} from "../adapters/tauri";

export type LanE2EStatus = "running" | "completed" | "failed" | "skipped";

export type LanE2EFailureReason =
  | "service_unhealthy"
  | "peer_not_found"
  | "peer_not_ready"
  | "session_start_failed"
  | "receiver_start_failed"
  | "display_window_failed"
  | "no_remote_frames"
  | "runtime_error"
  | "stop_failed";

export interface LanE2EStageEvent {
  stage: "preflight" | "pairing" | "session" | "receiver" | "display" | "sample" | "assert" | "cleanup";
  status: "started" | "completed" | "failed";
  timestamp: number;
  error?: string;
}

export interface LanE2EAutomationOptions {
  targetDeviceId?: string;
  transportKind?: "quic" | "webrtc";
  timeoutMs?: number;
  sampleIntervalMs?: number;
  minDecodedFrames?: number;
  minFps?: number;
  stopOnComplete?: boolean;
  createSessionId?: () => string;
  now?: () => number;
}

export interface LanE2EAutomationReport {
  status: LanE2EStatus;
  scenarioId: "lan.e2e.remote_display";
  sessionId?: string;
  controllerDeviceId?: string | null;
  peer?: LanPeerInfo;
  displayWindow?: RemoteDisplayWindowContext;
  sessionSnapshot?: SessionRuntimeSnapshot;
  probeSnapshot?: ProbeSnapshot;
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
  ipcRefreshLanDiscovery(): Promise<AdapterResult<LanDiscoverySnapshot>>;
  ipcStartLanRemoteSession(
    sessionId: string,
    targetDeviceId: string,
    transportKind: string
  ): Promise<AdapterResult<string>>;
  ipcStartReceiver(sessionId: string): Promise<AdapterResult<string>>;
  openRemoteDisplayWindow(params: {
    sessionId: string;
  }): Promise<AdapterResult<RemoteDisplayWindowContext>>;
  ipcSessionSnapshot(sessionId: string): Promise<AdapterResult<SessionRuntimeSnapshot>>;
  ipcProbeSnapshot(sessionId: string): Promise<AdapterResult<ProbeSnapshot>>;
  ipcStopSession(sessionId: string): Promise<AdapterResult<string>>;
}

const DEFAULT_TIMEOUT_MS = 10_000;
const DEFAULT_SAMPLE_INTERVAL_MS = 500;
const DEFAULT_MIN_DECODED_FRAMES = 1;
const DEFAULT_MIN_FPS = 1;

export async function runLanE2EAutomation(
  commands: LanE2EAutomationCommands,
  options: LanE2EAutomationOptions = {}
): Promise<LanE2EAutomationReport> {
  const now = options.now ?? Date.now;
  const startedAt = now();
  const stages: LanE2EStageEvent[] = [];
  const sampleIntervalMs = options.sampleIntervalMs ?? DEFAULT_SAMPLE_INTERVAL_MS;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const minDecodedFrames = options.minDecodedFrames ?? DEFAULT_MIN_DECODED_FRAMES;
  const minFps = options.minFps ?? DEFAULT_MIN_FPS;
  const stopOnComplete = options.stopOnComplete ?? true;
  const transportKind = options.transportKind ?? "quic";
  let sessionId: string | undefined;
  let peer: LanPeerInfo | undefined;
  let displayWindow: RemoteDisplayWindowContext | undefined;
  let sessionSnapshot: SessionRuntimeSnapshot | undefined;
  let probeSnapshot: ProbeSnapshot | undefined;
  let controllerDeviceId: string | null | undefined;
  let sessionStarted = false;

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
    scenarioId: "lan.e2e.remote_display",
    sessionId,
    controllerDeviceId,
    peer,
    displayWindow,
    sessionSnapshot,
    probeSnapshot,
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
    controllerDeviceId = runtime.device_id ?? null;
    const discovery = await unwrap(commands.ipcRefreshLanDiscovery(), "peer_not_found");
    const peerSelection = selectPeer(discovery, options.targetDeviceId, transportKind);
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

    stage("pairing", "started");
    sessionId = options.createSessionId?.() ?? createDefaultSessionId(selectedPeer.device_id, now());
    await unwrap(
      commands.ipcStartLanRemoteSession(sessionId, selectedPeer.device_id, transportKind),
      "session_start_failed"
    );
    sessionStarted = true;
    stage("pairing", "completed");

    stage("receiver", "started");
    await unwrap(commands.ipcStartReceiver(sessionId), "receiver_start_failed");
    stage("receiver", "completed");

    stage("display", "started");
    displayWindow = await unwrap(
      commands.openRemoteDisplayWindow({ sessionId }),
      "display_window_failed"
    );
    stage("display", "completed");

    stage("sample", "started");
    const deadline = Date.now() + timeoutMs;
    while (Date.now() <= deadline) {
      sessionSnapshot = await unwrap(
        commands.ipcSessionSnapshot(sessionId),
        "runtime_error"
      );
      probeSnapshot = await unwrap(commands.ipcProbeSnapshot(sessionId), "runtime_error");

      if (sessionSnapshot.state === "failed" || sessionSnapshot.last_error) {
        const message = sessionSnapshot.last_error ?? "LAN session entered failed state";
        stage("sample", "failed", message);
        return finish("failed", "runtime_error", message);
      }
      if (probeSnapshot.last_error) {
        stage("sample", "failed", probeSnapshot.last_error);
        return finish("failed", "runtime_error", probeSnapshot.last_error);
      }
      if (
        sessionSnapshot.receiver_active &&
        probeSnapshot.frames_decoded >= minDecodedFrames &&
        (probeSnapshot.current_fps ?? 0) >= minFps
      ) {
        stage("sample", "completed");
        stage("assert", "completed");
        return finish("completed");
      }

      await sleep(sampleIntervalMs);
    }

    const message = `No remote frames reached threshold: decoded ${probeSnapshot?.frames_decoded ?? 0}/${minDecodedFrames}, fps ${probeSnapshot?.current_fps ?? 0}/${minFps}`;
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

function selectPeer(
  snapshot: LanDiscoverySnapshot,
  targetDeviceId: string | undefined,
  transportKind: string
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
    if (!isPeerReady(targetPeer, transportKind)) {
      return {
        peer: targetPeer,
        failureReason: "peer_not_ready",
        message: buildPeerNotReadyMessage(targetPeer, transportKind),
      };
    }
    return { peer: targetPeer };
  }

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
  return peer.p2p_available && peer.transports.includes(transportKind);
}

function buildPeerNotReadyMessage(peer: LanPeerInfo, transportKind: string): string {
  const transportList = peer.transports.length > 0 ? peer.transports.join(", ") : "none";
  if (!peer.p2p_available) {
    return `LAN peer is discovered but not P2P available: ${peer.device_id}`;
  }
  return `LAN peer does not support ${transportKind}: ${peer.device_id} supports ${transportList}`;
}

function createDefaultSessionId(peerDeviceId: string, now: number): string {
  const safePeer = peerDeviceId.replace(/[^a-zA-Z0-9_-]/g, "-");
  return `lan-e2e-${safePeer}-${now}`;
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
    case "peer_not_found":
    case "peer_not_ready":
      return "preflight";
    case "session_start_failed":
      return "pairing";
    case "receiver_start_failed":
      return "receiver";
    case "display_window_failed":
      return "display";
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
