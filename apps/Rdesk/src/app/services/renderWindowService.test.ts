import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/tauri", () => ({
  invoke: invokeMock,
}));

describe("renderWindowService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("opens a dedicated render window for a session", async () => {
    invokeMock.mockResolvedValue("render-session-1");

    const { openRenderWindow } = await import("./renderWindowService");
    const label = await openRenderWindow("session-1");

    expect(invokeMock).toHaveBeenCalledWith("open_render_window", {
      sessionId: "session-1",
    });
    expect(label).toBe("render-session-1");
  });
});
