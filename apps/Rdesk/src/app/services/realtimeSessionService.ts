import { invoke } from "@tauri-apps/api/tauri";

export type RealtimeRole = "controller" | "agent";

export type RealtimeRegistration = {
  handle: number;
  deviceId: string;
};

export type RealtimeRegistrationRequest = {
  role: RealtimeRole;
  deviceId?: string;
  name: string;
};

export type RealtimeSessionRequest = {
  handle: number;
  sessionId: string;
  targetDeviceId: string;
};

export type RealtimeSessionAccept = {
  handle: number;
  sessionId: string;
};

type RealtimeRegistrationPayload = {
  handle: number;
  device_id: string;
};

export const registerRealtimeSession = async (
  request: RealtimeRegistrationRequest
): Promise<RealtimeRegistration> => {
  const payload = await invoke<RealtimeRegistrationPayload>("realtime_register", {
    role: request.role,
    deviceId: request.deviceId,
    name: request.name,
  });

  return {
    handle: payload.handle,
    deviceId: payload.device_id,
  };
};

export const requestRealtimeSession = async (
  request: RealtimeSessionRequest
): Promise<void> =>
  invoke("realtime_request_session", {
    handle: request.handle,
    sessionId: request.sessionId,
    targetDeviceId: request.targetDeviceId,
  });

export const acceptRealtimeSession = async (
  request: RealtimeSessionAccept
): Promise<void> =>
  invoke("realtime_accept_session", {
    handle: request.handle,
    sessionId: request.sessionId,
  });

export const drainRealtimeEvents = async (handle: number): Promise<string[]> =>
  invoke<string[]>("realtime_drain_events", { handle });
