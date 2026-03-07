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

export type RealtimeSessionDescription = {
  handle: number;
  sessionId: string;
  sdp: string;
};

export type RealtimeIceCandidate = {
  handle: number;
  sessionId: string;
  candidate: string;
  sdpMid?: string;
  sdpMlineIndex?: number;
};

export type WebrtcSessionSnapshot = {
  localOffer?: string;
  remoteOffer?: string;
  remoteAnswer?: string;
  remoteIceCandidates: Array<{
    session_id: string;
    candidate: string;
    sdp_mid?: string;
    sdp_mline_index?: number;
  }>;
};

export type WebrtcHostSnapshot = {
  localOffer?: string;
  remoteOffer?: string;
  localAnswer?: string;
  remoteAnswer?: string;
  remoteIceCount: number;
  remoteVideoTrackCount: number;
  remoteRtpPacketCount: number;
  lastRemoteCodec?: string;
  remoteH264AccessUnitCount: number;
  lastRemoteAccessUnitBytes: number;
  decodedFrameCount: number;
  lastDecodedWidth: number;
  lastDecodedHeight: number;
  lastDecodedPixelFormat?: string;
};

export type DecodedFrameSnapshot = {
  frameCount: number;
  width: number;
  height: number;
  pixelFormat?: string;
  bytes: number;
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

export const sendRealtimeOffer = async (
  request: RealtimeSessionDescription
): Promise<void> =>
  invoke("realtime_send_offer", {
    handle: request.handle,
    sessionId: request.sessionId,
    sdp: request.sdp,
  });

export const sendRealtimeAnswer = async (
  request: RealtimeSessionDescription
): Promise<void> =>
  invoke("realtime_send_answer", {
    handle: request.handle,
    sessionId: request.sessionId,
    sdp: request.sdp,
  });

export const sendRealtimeIceCandidate = async (
  request: RealtimeIceCandidate
): Promise<void> =>
  invoke("realtime_send_ice_candidate", {
    handle: request.handle,
    sessionId: request.sessionId,
    candidate: request.candidate,
    sdpMid: request.sdpMid,
    sdpMlineIndex: request.sdpMlineIndex,
  });

export const createWebrtcLocalOffer = async (
  sessionId: string,
  sdp: string
): Promise<string> =>
  invoke<string>("webrtc_create_local_offer", {
    sessionId,
    sdp,
  });

export const applyWebrtcRemoteAnswer = async (
  sessionId: string,
  sdp: string
): Promise<void> =>
  invoke("webrtc_apply_remote_answer", {
    sessionId,
    sdp,
  });

export const applyWebrtcRemoteIceCandidate = async (
  request: Omit<RealtimeIceCandidate, "handle">
): Promise<void> =>
  invoke("webrtc_apply_remote_ice_candidate", {
    sessionId: request.sessionId,
    candidate: request.candidate,
    sdpMid: request.sdpMid,
    sdpMlineIndex: request.sdpMlineIndex,
  });

export const syncWebrtcRealtimeEvents = async (
  handle: number
): Promise<WebrtcSessionSnapshot> =>
  invoke<WebrtcSessionSnapshot>("webrtc_sync_realtime_events", {
    handle,
  });

export const getWebrtcSnapshot = async (
  sessionId: string
): Promise<WebrtcSessionSnapshot | null> =>
  invoke<WebrtcSessionSnapshot | null>("webrtc_snapshot", {
    sessionId,
  });

export const createWebrtcHostOffer = async (
  sessionId: string
): Promise<string> =>
  invoke<string>("webrtc_host_create_offer", {
    sessionId,
  });

export const applyWebrtcHostRemoteOffer = async (
  sessionId: string,
  sdp: string
): Promise<void> =>
  invoke("webrtc_host_apply_remote_offer", {
    sessionId,
    sdp,
  });

export const createWebrtcHostAnswer = async (
  sessionId: string
): Promise<string> =>
  invoke<string>("webrtc_host_create_answer", {
    sessionId,
  });

export const applyWebrtcHostRemoteAnswer = async (
  sessionId: string,
  sdp: string
): Promise<void> =>
  invoke("webrtc_host_apply_remote_answer", {
    sessionId,
    sdp,
  });

export const applyWebrtcHostRemoteIceCandidate = async (
  request: Omit<RealtimeIceCandidate, "handle">
): Promise<void> =>
  invoke("webrtc_host_apply_remote_ice_candidate", {
    sessionId: request.sessionId,
    candidate: request.candidate,
    sdpMid: request.sdpMid,
    sdpMlineIndex: request.sdpMlineIndex,
  });

export const getWebrtcHostSnapshot = async (
  sessionId: string
): Promise<WebrtcHostSnapshot | null> =>
  invoke<WebrtcHostSnapshot | null>("webrtc_host_snapshot", {
    sessionId,
  });

export const getDecodedFrameSnapshot = async (
  sessionId: string
): Promise<DecodedFrameSnapshot | null> =>
  invoke<DecodedFrameSnapshot | null>("decoded_frame_snapshot", {
    sessionId,
  });
