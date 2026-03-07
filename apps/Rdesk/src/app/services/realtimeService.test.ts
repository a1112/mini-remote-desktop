import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/tauri", () => ({
  invoke: invokeMock,
}));

describe("realtimeService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("reads realtime status via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      running: true,
      reachable: true,
      status: "ok",
      pid: 9532,
    });

    const { getRealtimeStatus } = await import("./realtimeService");
    const result = await getRealtimeStatus();

    expect(invokeMock).toHaveBeenCalledWith("realtime_status");
    expect(result.running).toBe(true);
    expect(result.pid).toBe(9532);
  });

  it("restarts realtime sidecar via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      running: true,
      reachable: true,
      status: "ok",
      pid: 9532,
    });

    const { restartRealtime } = await import("./realtimeService");
    const result = await restartRealtime();

    expect(invokeMock).toHaveBeenCalledWith("realtime_restart");
    expect(result.status).toBe("ok");
  });
});
