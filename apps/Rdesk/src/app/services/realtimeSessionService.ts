import { invoke } from "@tauri-apps/api/core";

/**
 * Realtime session service
 *
 * DEPRECATED: All direct realtime commands have been removed.
 * Use mrd-service IPC interface (ipc_* commands) instead.
 * See realtimeService.ts for service lifecycle commands.
 */

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
  decodePolicy?: string;
  preferredDecodeBackend?: string;
  activeDecodeBackend?: string;
  decodeBackendReason?: string;
  decodeFallbackCount: number;
  lastDecodeFallbackReason?: string;
};

export type DecodedFrameSnapshot = {
  frameCount: number;
  width: number;
  height: number;
  pixelFormat?: string;
  bytes: number;
};

export type SessionRuntimeSnapshot = {
  lifecycle: {
    sessionId: string;
    currentSurfaceId?: string;
    surfaces: Array<{
      current: boolean;
      surfaceId: string;
      name: string;
      role: string;
    }>;
    availableSourceIds: string[];
    surfaceSourceBindings: Array<{
      surfaceId: string;
      sourceId: string;
    }>;
  };
  renderHost: {
    attached: boolean;
    surfaceCount: number;
    attachedSurfaceIds: string[];
    availableSourceIds: string[];
    surfaceSourceBindings: Array<{
      surfaceId: string;
      sourceId: string;
    }>;
  };
  webrtcHost: WebrtcHostSnapshot;
  webrtcSignaling?: WebrtcSessionSnapshot | null;
};

type RealtimeRegistrationPayload = {
  handle: number;
  device_id: string;
};

/**
 * @deprecated realtime_register command removed - use ipc_register_device instead
 */
export const registerRealtimeSession = async (
  _request: RealtimeRegistrationRequest
): Promise<RealtimeRegistration> => {
  throw new Error(
    "realtime_register 命令已移除。请使用 ipc_register_device 代替。"
  );
};

/**
 * @deprecated realtime_request_session command removed - use ipc_start_session instead
 */
export const requestRealtimeSession = async (
  _request: RealtimeSessionRequest
): Promise<void> => {
  throw new Error(
    "realtime_request_session 命令已移除。请使用 ipc_start_session 代替。"
  );
};

/**
 * @deprecated realtime_accept_session command removed - use ipc_accept_session instead
 */
export const acceptRealtimeSession = async (
  _request: RealtimeSessionAccept
): Promise<void> => {
  throw new Error(
    "realtime_accept_session 命令已移除。请使用 ipc_accept_session 代替。"
  );
};

/**
 * @deprecated realtime_drain_events command removed
 */
export const drainRealtimeEvents = async (_handle: number): Promise<string[]> => {
  throw new Error(
    "realtime_drain_events 命令已移除。事件现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated realtime_send_offer command removed - WebRTC signaling moved to mrd-service
 */
export const sendRealtimeOffer = async (
  _request: RealtimeSessionDescription
): Promise<void> => {
  throw new Error(
    "realtime_send_offer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated realtime_send_answer command removed - WebRTC signaling moved to mrd-service
 */
export const sendRealtimeAnswer = async (
  _request: RealtimeSessionDescription
): Promise<void> => {
  throw new Error(
    "realtime_send_answer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated realtime_send_ice_candidate command removed - WebRTC signaling moved to mrd-service
 */
export const sendRealtimeIceCandidate = async (
  _request: RealtimeIceCandidate
): Promise<void> => {
  throw new Error(
    "realtime_send_ice_candidate 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_create_local_offer command removed - use ipc_start_session instead
 */
export const createWebrtcLocalOffer = async (
  _sessionId: string,
  _sdp: string
): Promise<string> => {
  throw new Error(
    "webrtc_create_local_offer 命令已移除。请使用 ipc_start_session 代替。"
  );
};

/**
 * @deprecated webrtc_apply_remote_answer command removed
 */
export const applyWebrtcRemoteAnswer = async (
  _sessionId: string,
  _sdp: string
): Promise<void> => {
  throw new Error(
    "webrtc_apply_remote_answer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_apply_remote_ice_candidate command removed
 */
export const applyWebrtcRemoteIceCandidate = async (
  _request: Omit<RealtimeIceCandidate, "handle">
): Promise<void> => {
  throw new Error(
    "webrtc_apply_remote_ice_candidate 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_sync_realtime_events command removed
 */
export const syncWebrtcRealtimeEvents = async (
  _handle: number
): Promise<WebrtcSessionSnapshot> => {
  throw new Error(
    "webrtc_sync_realtime_events 命令已移除。事件现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated webrtc_snapshot command removed - use ipc_session_snapshot instead
 */
export const getWebrtcSnapshot = async (
  _sessionId: string
): Promise<WebrtcSessionSnapshot | null> => {
  throw new Error(
    "webrtc_snapshot 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated webrtc_host_create_offer command removed
 */
export const createWebrtcHostOffer = async (
  _sessionId: string
): Promise<string> => {
  throw new Error(
    "webrtc_host_create_offer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_host_apply_remote_offer command removed
 */
export const applyWebrtcHostRemoteOffer = async (
  _sessionId: string,
  _sdp: string
): Promise<void> => {
  throw new Error(
    "webrtc_host_apply_remote_offer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_host_create_answer command removed
 */
export const createWebrtcHostAnswer = async (
  _sessionId: string
): Promise<string> => {
  throw new Error(
    "webrtc_host_create_answer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_host_apply_remote_answer command removed
 */
export const applyWebrtcHostRemoteAnswer = async (
  _sessionId: string,
  _sdp: string
): Promise<void> => {
  throw new Error(
    "webrtc_host_apply_remote_answer 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_host_apply_remote_ice_candidate command removed
 */
export const applyWebrtcHostRemoteIceCandidate = async (
  _request: Omit<RealtimeIceCandidate, "handle">
): Promise<void> => {
  throw new Error(
    "webrtc_host_apply_remote_ice_candidate 命令已移除。WebRTC 信令已迁移到 mrd-service。"
  );
};

/**
 * @deprecated webrtc_host_snapshot command removed - use ipc_session_snapshot instead
 */
export const getWebrtcHostSnapshot = async (
  _sessionId: string
): Promise<WebrtcHostSnapshot | null> => {
  throw new Error(
    "webrtc_host_snapshot 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated decoded_frame_snapshot command removed - use ipc_session_snapshot instead
 */
export const getDecodedFrameSnapshot = async (
  _sessionId: string
): Promise<DecodedFrameSnapshot | null> => {
  throw new Error(
    "decoded_frame_snapshot 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated decoded_frame_preview command removed
 */
export const getDecodedFramePreview = async (
  _sessionId: string
): Promise<string | null> => {
  throw new Error(
    "decoded_frame_preview 命令已移除。帧预览功能已迁移到 mrd-service。"
  );
};

/**
 * @deprecated session_runtime_snapshot command removed - use ipc_session_snapshot instead
 */
export const getSessionRuntimeSnapshot = async (
  _sessionId: string
): Promise<SessionRuntimeSnapshot> => {
  throw new Error(
    "session_runtime_snapshot 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated session_runtime_sync_realtime command removed
 */
export const syncRealtimeIntoSessionRuntime = async (
  _handle: number
): Promise<SessionRuntimeSnapshot | null> => {
  throw new Error(
    "session_runtime_sync_realtime 命令已移除。事件现在由 mrd-service 管理。"
  );
};
