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

  it("lists render windows for a session", async () => {
    invokeMock.mockResolvedValue(["render-session-1-1", "render-session-1-2"]);

    const { listRenderWindows } = await import("./renderWindowService");
    const labels = await listRenderWindows("session-1");

    expect(invokeMock).toHaveBeenCalledWith("list_render_windows", {
      sessionId: "session-1",
    });
    expect(labels).toEqual(["render-session-1-1", "render-session-1-2"]);
  });

  it("closes a render window by label", async () => {
    invokeMock.mockResolvedValue(undefined);

    const { closeRenderWindow } = await import("./renderWindowService");
    await closeRenderWindow("render-session-1-2");

    expect(invokeMock).toHaveBeenCalledWith("close_render_window", {
      label: "render-session-1-2",
    });
  });
});
