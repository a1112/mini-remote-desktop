import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/tauri", () => ({
  invoke: invokeMock,
}));

describe("realtimeSessionService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("registers realtime controller connections via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      handle: 1,
      device_id: "controller-1",
    });

    const { registerRealtimeSession } = await import("./realtimeSessionService");
    const result = await registerRealtimeSession({
      role: "controller",
      deviceId: "controller-1",
      name: "Rdesk",
    });

    expect(invokeMock).toHaveBeenCalledWith("realtime_register", {
      role: "controller",
      deviceId: "controller-1",
      name: "Rdesk",
    });
    expect(result.handle).toBe(1);
    expect(result.deviceId).toBe("controller-1");
  });

  it("requests and accepts sessions via tauri invoke", async () => {
    invokeMock.mockResolvedValue(undefined);

    const { acceptRealtimeSession, requestRealtimeSession } = await import(
      "./realtimeSessionService"
    );

    await requestRealtimeSession({
      handle: 1,
      sessionId: "session-1",
      targetDeviceId: "agent-1",
    });
    await acceptRealtimeSession({
      handle: 1,
      sessionId: "session-1",
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "realtime_request_session", {
      handle: 1,
      sessionId: "session-1",
      targetDeviceId: "agent-1",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "realtime_accept_session", {
      handle: 1,
      sessionId: "session-1",
    });
  });

  it("drains cached realtime events via tauri invoke", async () => {
    invokeMock.mockResolvedValue([
      "{\"type\":\"session\",\"action\":\"request\",\"payload\":{\"sessionId\":\"session-1\"}}",
      "{\"type\":\"session\",\"action\":\"accept\",\"payload\":{\"sessionId\":\"session-1\"}}",
    ]);

    const { drainRealtimeEvents } = await import("./realtimeSessionService");
    const events = await drainRealtimeEvents(1);

    expect(invokeMock).toHaveBeenCalledWith("realtime_drain_events", {
      handle: 1,
    });
    expect(events).toEqual([
      "{\"type\":\"session\",\"action\":\"request\",\"payload\":{\"sessionId\":\"session-1\"}}",
      "{\"type\":\"session\",\"action\":\"accept\",\"payload\":{\"sessionId\":\"session-1\"}}",
    ]);
  });

  it("sends webrtc offer answer and ice via tauri invoke", async () => {
    invokeMock.mockResolvedValue(undefined);

    const {
      sendRealtimeAnswer,
      sendRealtimeIceCandidate,
      sendRealtimeOffer,
    } = await import("./realtimeSessionService");

    await sendRealtimeOffer({
      handle: 1,
      sessionId: "session-1",
      sdp: "offer-sdp",
    });
    await sendRealtimeAnswer({
      handle: 1,
      sessionId: "session-1",
      sdp: "answer-sdp",
    });
    await sendRealtimeIceCandidate({
      handle: 1,
      sessionId: "session-1",
      candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "realtime_send_offer", {
      handle: 1,
      sessionId: "session-1",
      sdp: "offer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "realtime_send_answer", {
      handle: 1,
      sessionId: "session-1",
      sdp: "answer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "realtime_send_ice_candidate", {
      handle: 1,
      sessionId: "session-1",
      candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });
  });

  it("creates and syncs webrtc session snapshots via tauri invoke", async () => {
    invokeMock
      .mockResolvedValueOnce("offer-sdp")
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({
        localOffer: "offer-sdp",
        remoteOffer: "remote-offer-sdp",
        remoteAnswer: "answer-sdp",
        remoteIceCandidates: [
          {
            session_id: "session-1",
            candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host",
            sdp_mid: "0",
            sdp_mline_index: 0,
          },
        ],
      })
      .mockResolvedValueOnce({
        localOffer: "offer-sdp",
        remoteOffer: "remote-offer-sdp",
        remoteAnswer: "answer-sdp",
        remoteIceCandidates: [],
      });

    const {
      applyWebrtcRemoteAnswer,
      applyWebrtcRemoteIceCandidate,
      createWebrtcLocalOffer,
      getWebrtcSnapshot,
      syncWebrtcRealtimeEvents,
    } = await import("./realtimeSessionService");

    const localOffer = await createWebrtcLocalOffer("session-1", "offer-sdp");
    await applyWebrtcRemoteAnswer("session-1", "answer-sdp");
    await applyWebrtcRemoteIceCandidate({
      sessionId: "session-1",
      candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });
    const synced = await syncWebrtcRealtimeEvents(1);
    const snapshot = await getWebrtcSnapshot("session-1");

    expect(localOffer).toBe("offer-sdp");
    expect(synced.remoteOffer).toBe("remote-offer-sdp");
    expect(snapshot?.remoteAnswer).toBe("answer-sdp");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "webrtc_create_local_offer", {
      sessionId: "session-1",
      sdp: "offer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "webrtc_apply_remote_answer", {
      sessionId: "session-1",
      sdp: "answer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "webrtc_apply_remote_ice_candidate", {
      sessionId: "session-1",
      candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "webrtc_sync_realtime_events", {
      handle: 1,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(5, "webrtc_snapshot", {
      sessionId: "session-1",
    });
  });

  it("bridges webrtc host offer answer and snapshot commands via tauri invoke", async () => {
    invokeMock
      .mockResolvedValueOnce("generated-offer-sdp")
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce("generated-answer-sdp")
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({
        localOffer: "generated-offer-sdp",
        remoteOffer: "remote-offer-sdp",
        localAnswer: "generated-answer-sdp",
        remoteAnswer: "remote-answer-sdp",
        remoteIceCount: 1,
        remoteVideoTrackCount: 0,
        remoteRtpPacketCount: 0,
        lastRemoteCodec: "video/h264",
        remoteH264AccessUnitCount: 0,
        lastRemoteAccessUnitBytes: 0,
        decodedFrameCount: 1,
        lastDecodedWidth: 16,
        lastDecodedHeight: 16,
        lastDecodedPixelFormat: "Rgb24",
        decodePolicy: "auto",
        preferredDecodeBackend: "nvdec",
        activeDecodeBackend: "h264_software",
        decodeBackendReason: "nvdec unavailable, fell back to h264_software",
        decodeFallbackCount: 1,
        lastDecodeFallbackReason: "nvdec runtime probe unhealthy",
      });

    const {
      applyWebrtcHostRemoteAnswer,
      applyWebrtcHostRemoteIceCandidate,
      applyWebrtcHostRemoteOffer,
      createWebrtcHostAnswer,
      createWebrtcHostOffer,
      getWebrtcHostSnapshot,
    } = await import("./realtimeSessionService");

    const offer = await createWebrtcHostOffer("session-2");
    await applyWebrtcHostRemoteOffer("session-2", "remote-offer-sdp");
    const answer = await createWebrtcHostAnswer("session-2");
    await applyWebrtcHostRemoteAnswer("session-2", "remote-answer-sdp");
    await applyWebrtcHostRemoteIceCandidate({
      sessionId: "session-2",
      candidate: "candidate:2 1 UDP 123 127.0.0.1 5001 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });
    const snapshot = await getWebrtcHostSnapshot("session-2");

    expect(offer).toBe("generated-offer-sdp");
    expect(answer).toBe("generated-answer-sdp");
    expect(snapshot?.remoteIceCount).toBe(1);
    expect(snapshot?.lastRemoteCodec).toBe("video/h264");
    expect(snapshot?.decodedFrameCount).toBe(1);
    expect(snapshot?.lastDecodedPixelFormat).toBe("Rgb24");
    expect(snapshot?.decodePolicy).toBe("auto");
    expect(snapshot?.preferredDecodeBackend).toBe("nvdec");
    expect(snapshot?.activeDecodeBackend).toBe("h264_software");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "webrtc_host_create_offer", {
      sessionId: "session-2",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "webrtc_host_apply_remote_offer", {
      sessionId: "session-2",
      sdp: "remote-offer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "webrtc_host_create_answer", {
      sessionId: "session-2",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "webrtc_host_apply_remote_answer", {
      sessionId: "session-2",
      sdp: "remote-answer-sdp",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(5, "webrtc_host_apply_remote_ice_candidate", {
      sessionId: "session-2",
      candidate: "candidate:2 1 UDP 123 127.0.0.1 5001 typ host",
      sdpMid: "0",
      sdpMlineIndex: 0,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(6, "webrtc_host_snapshot", {
      sessionId: "session-2",
    });
  });

  it("reads decoded frame sink snapshots via tauri invoke", async () => {
    invokeMock.mockResolvedValueOnce({
      frameCount: 2,
      width: 1280,
      height: 720,
      pixelFormat: "Rgb24",
      bytes: 2764800,
    });

    const { getDecodedFrameSnapshot } = await import("./realtimeSessionService");
    const snapshot = await getDecodedFrameSnapshot("session-3");

    expect(invokeMock).toHaveBeenCalledWith("decoded_frame_snapshot", {
      sessionId: "session-3",
    });
    expect(snapshot?.frameCount).toBe(2);
    expect(snapshot?.pixelFormat).toBe("Rgb24");
    expect(snapshot?.bytes).toBe(2764800);
  });

  it("reads decoded frame preview via tauri invoke", async () => {
    invokeMock.mockResolvedValueOnce("data:image/png;base64,abc123");

    const { getDecodedFramePreview } = await import("./realtimeSessionService");
    const preview = await getDecodedFramePreview("session-4");

    expect(invokeMock).toHaveBeenCalledWith("decoded_frame_preview", {
      sessionId: "session-4",
    });
    expect(preview).toBe("data:image/png;base64,abc123");
  });

  it("reads aggregated session runtime snapshots via tauri invoke", async () => {
    invokeMock
      .mockResolvedValueOnce({
        lifecycle: {
          sessionId: "session-5",
          currentSurfaceId: "surface-a",
          surfaces: [
            {
              current: true,
              surfaceId: "surface-a",
              name: "Main",
              role: "viewer",
            },
          ],
          availableSourceIds: ["video-track-1"],
          surfaceSourceBindings: [
            {
              surfaceId: "surface-a",
              sourceId: "video-track-1",
            },
          ],
        },
        renderHost: {
          attached: true,
          surfaceCount: 1,
          attachedSurfaceIds: ["surface-a"],
          availableSourceIds: ["video-track-1"],
          surfaceSourceBindings: [
            {
              surfaceId: "surface-a",
              sourceId: "video-track-1",
            },
          ],
        },
        webrtcHost: {
          remoteIceCount: 1,
          remoteVideoTrackCount: 1,
          remoteRtpPacketCount: 10,
          remoteH264AccessUnitCount: 3,
          lastRemoteAccessUnitBytes: 1200,
          decodedFrameCount: 2,
          lastDecodedWidth: 1280,
          lastDecodedHeight: 720,
          lastDecodedPixelFormat: "Rgb24",
          decodePolicy: "nvdec",
          preferredDecodeBackend: "nvdec",
          activeDecodeBackend: "nvdec",
          decodeBackendReason: "using nvdec for current H264 track",
          decodeFallbackCount: 0,
          lastDecodeFallbackReason: undefined,
        },
        webrtcSignaling: {
          localOffer: "offer-sdp",
          remoteOffer: "remote-offer-sdp",
          remoteAnswer: "answer-sdp",
          remoteIceCandidates: [],
        },
      })
      .mockResolvedValueOnce(null);

    const {
      getSessionRuntimeSnapshot,
      syncRealtimeIntoSessionRuntime,
    } = await import("./realtimeSessionService");

    const snapshot = await getSessionRuntimeSnapshot("session-5");
    const syncedSnapshot = await syncRealtimeIntoSessionRuntime(9);

    expect(invokeMock).toHaveBeenNthCalledWith(1, "session_runtime_snapshot", {
      sessionId: "session-5",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "session_runtime_sync_realtime", {
      handle: 9,
    });
    expect(snapshot.lifecycle.currentSurfaceId).toBe("surface-a");
    expect(snapshot.renderHost.surfaceSourceBindings[0].sourceId).toBe("video-track-1");
    expect(snapshot.webrtcHost.decodedFrameCount).toBe(2);
    expect(syncedSnapshot).toBeNull();
  });
});
