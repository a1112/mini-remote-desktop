import { describe, expect, it } from "vitest";

describe("realtimeSessionService (DEPRECATED)", () => {
  it("registerRealtimeSession throws deprecation error", async () => {
    const { registerRealtimeSession } = await import("./realtimeSessionService");

    await expect(registerRealtimeSession({
      role: "controller",
      deviceId: "controller-1",
      name: "Rdesk",
    })).rejects.toThrow("realtime_register 命令已移除");
  });

  it("requestRealtimeSession throws deprecation error", async () => {
    const { requestRealtimeSession } = await import("./realtimeSessionService");

    await expect(requestRealtimeSession({
      handle: 1,
      sessionId: "session-1",
      targetDeviceId: "agent-1",
    })).rejects.toThrow("realtime_request_session 命令已移除");
  });

  it("acceptRealtimeSession throws deprecation error", async () => {
    const { acceptRealtimeSession } = await import("./realtimeSessionService");

    await expect(acceptRealtimeSession({
      handle: 1,
      sessionId: "session-1",
    })).rejects.toThrow("realtime_accept_session 命令已移除");
  });

  it("drainRealtimeEvents throws deprecation error", async () => {
    const { drainRealtimeEvents } = await import("./realtimeSessionService");

    await expect(drainRealtimeEvents(1)).rejects.toThrow("realtime_drain_events 命令已移除");
  });

  it("sendRealtimeOffer throws deprecation error", async () => {
    const { sendRealtimeOffer } = await import("./realtimeSessionService");

    await expect(sendRealtimeOffer({
      handle: 1,
      sessionId: "session-1",
      sdp: "offer-sdp",
    })).rejects.toThrow("realtime_send_offer 命令已移除");
  });

  it("sendRealtimeAnswer throws deprecation error", async () => {
    const { sendRealtimeAnswer } = await import("./realtimeSessionService");

    await expect(sendRealtimeAnswer({
      handle: 1,
      sessionId: "session-1",
      sdp: "answer-sdp",
    })).rejects.toThrow("realtime_send_answer 命令已移除");
  });

  it("sendRealtimeIceCandidate throws deprecation error", async () => {
    const { sendRealtimeIceCandidate } = await import("./realtimeSessionService");

    await expect(sendRealtimeIceCandidate({
      handle: 1,
      sessionId: "session-1",
      candidate: "candidate",
      sdpMid: "0",
      sdpMlineIndex: 0,
    })).rejects.toThrow("realtime_send_ice_candidate 命令已移除");
  });

  it("getWebrtcSnapshot throws deprecation error", async () => {
    const { getWebrtcSnapshot } = await import("./realtimeSessionService");

    await expect(getWebrtcSnapshot("session-1")).rejects.toThrow("webrtc_snapshot 命令已移除");
  });

  it("getWebrtcHostSnapshot throws deprecation error", async () => {
    const { getWebrtcHostSnapshot } = await import("./realtimeSessionService");

    await expect(getWebrtcHostSnapshot("session-1")).rejects.toThrow("webrtc_host_snapshot 命令已移除");
  });

  it("getDecodedFrameSnapshot throws deprecation error", async () => {
    const { getDecodedFrameSnapshot } = await import("./realtimeSessionService");

    await expect(getDecodedFrameSnapshot("session-1")).rejects.toThrow("decoded_frame_snapshot 命令已移除");
  });

  it("getSessionRuntimeSnapshot throws deprecation error", async () => {
    const { getSessionRuntimeSnapshot } = await import("./realtimeSessionService");

    await expect(getSessionRuntimeSnapshot("session-1")).rejects.toThrow("session_runtime_snapshot 命令已移除");
  });
});
