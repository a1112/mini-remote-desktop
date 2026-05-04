import { openRemoteDisplayWindow } from "../adapters/tauri";
import { isTauriRuntime } from "../utils/runtime";
import { deviceService } from "./deviceService";
import {
  startLanRemoteSession,
  startSession,
  type MediaProfile,
  type TransportKind,
} from "./ipcSessionService";
import { saveWebRemoteSession } from "./webRemoteSessionService";

export type RemoteDisplayLaunchResult = {
  sessionId: string;
  windowLabel: string | null;
  mode: "native_window" | "route";
};

const randomToken = () => Math.random().toString(36).slice(2, 8);

const DEFAULT_REMOTE_MEDIA_PROFILE: MediaProfile = {
  width: 2560,
  height: 1440,
  fps: 144,
  bitrate_mbps: 64,
  codec: "h264",
};

export function createSessionId(prefix: string): string {
  return `${prefix}-${Date.now()}-${randomToken()}`;
}

export async function launchRemoteDisplayForDevice(
  targetDeviceId: string,
  options?: {
    transportKind?: TransportKind;
    sessionId?: string;
    openWindow?: boolean;
    targetDeviceName?: string;
    targetOs?: string;
    targetIp?: string;
    localTest?: boolean;
    lanP2P?: boolean;
    requestedProfile?: MediaProfile;
  }
): Promise<RemoteDisplayLaunchResult> {
  const tauriRuntime = isTauriRuntime();
  const transportKind = tauriRuntime
    ? (options?.transportKind ?? (options?.lanP2P ? "quic" : "webrtc"))
    : "webrtc";
  const localDeviceInfo = tauriRuntime
    ? deviceService.getDeviceInfo() ?? (await deviceService.initialize())
    : null;
  const targetIsLocal = Boolean(
    options?.localTest ||
      (localDeviceInfo?.device_id && localDeviceInfo.device_id === targetDeviceId)
  );
  const sessionId =
    options?.sessionId ??
    createSessionId(targetIsLocal ? "local-display-test" : `p2p-${transportKind}`);

  if (!tauriRuntime) {
    saveWebRemoteSession({
      sessionId,
      targetDeviceId,
      targetDeviceName: options?.targetDeviceName ?? "Local WebRTC Device",
      targetOs: options?.targetOs ?? "Browser WebRTC",
      targetIp: options?.targetIp ?? "127.0.0.1",
      transportKind: "webrtc",
      createdAt: Date.now(),
      mode: options?.localTest ? "web_to_local" : "web_to_peer",
    });
    return { sessionId, windowLabel: null, mode: "route" };
  }

  const startedSessionId = targetIsLocal
    ? sessionId
    : options?.lanP2P
      ? await startLanRemoteSession(
          sessionId,
          targetDeviceId,
          transportKind,
          options?.requestedProfile ?? DEFAULT_REMOTE_MEDIA_PROFILE
        )
    : await startSession(sessionId, targetDeviceId, transportKind);

  if (options?.openWindow === false) {
    return { sessionId: startedSessionId, windowLabel: null, mode: "route" };
  }

  const windowResult = await openRemoteDisplayWindow({ sessionId: startedSessionId });
  if (!windowResult.ok) {
    throw new Error(windowResult.error.message);
  }

  return {
    sessionId: startedSessionId,
    windowLabel: windowResult.value.label,
    mode: "native_window",
  };
}

export async function launchLocalRemoteDisplayTest(): Promise<RemoteDisplayLaunchResult> {
  const deviceInfo = deviceService.getDeviceInfo() ?? await deviceService.initialize();
  const targetDeviceId = deviceInfo?.device_id ?? "local-test-device";
  return launchRemoteDisplayForDevice(targetDeviceId, {
    sessionId: createSessionId("local-display-test"),
    transportKind: "webrtc",
    targetDeviceName: deviceInfo?.device_name ?? "Local WebRTC Device",
    targetOs: "Browser WebRTC",
    targetIp: "127.0.0.1",
    localTest: true,
  });
}
