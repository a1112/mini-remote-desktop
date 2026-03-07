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
});
