import { beforeEach, describe, expect, it, vi } from "vitest";

const adapterMock = vi.hoisted(() => ({
  ipcListSessions: vi.fn(),
  ipcStopSession: vi.fn(),
}));

vi.mock("../adapters/tauri", () => ({
  ipcListSessions: adapterMock.ipcListSessions,
  ipcStopSession: adapterMock.ipcStopSession,
}));

import { disconnectDeviceSessions } from "./ipcSessionService";

const ok = <T,>(value: T) => ({ ok: true as const, value });

describe("disconnectDeviceSessions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    adapterMock.ipcStopSession.mockResolvedValue(ok("stopped"));
  });

  it("stops active sessions that reference the device as target or source", async () => {
    adapterMock.ipcListSessions.mockResolvedValue(
      ok([
        {
          session_id: "target-session",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          source_device_id: null,
          target_device_id: "agent-device",
          last_error: null,
          sender_active: true,
          receiver_active: true,
        },
        {
          session_id: "source-session",
          role: "agent",
          state: "connected",
          transport_kind: "quic",
          source_device_id: "agent-device",
          target_device_id: null,
          last_error: null,
          sender_active: true,
          receiver_active: false,
        },
        {
          session_id: "closed-session",
          role: "controller",
          state: "closed",
          transport_kind: "quic",
          source_device_id: null,
          target_device_id: "agent-device",
          last_error: null,
          sender_active: false,
          receiver_active: false,
        },
      ])
    );

    const stopped = await disconnectDeviceSessions("agent-device");

    expect(stopped).toBe(2);
    expect(adapterMock.ipcStopSession).toHaveBeenCalledWith("target-session");
    expect(adapterMock.ipcStopSession).toHaveBeenCalledWith("source-session");
    expect(adapterMock.ipcStopSession).not.toHaveBeenCalledWith("closed-session");
  });
});
