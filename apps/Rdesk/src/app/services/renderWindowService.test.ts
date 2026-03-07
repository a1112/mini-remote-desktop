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
    invokeMock.mockResolvedValue([
      {
        label: "render-session-1-1",
        session_id: "session-1",
        surface_id: "surface-1",
        role: "controller",
        renderer_attached: true,
        session_window_count: 2,
      },
      {
        label: "render-session-1-2",
        session_id: "session-1",
        surface_id: "surface-2",
        role: "controller",
        renderer_attached: false,
        session_window_count: 2,
      },
    ]);

    const { listRenderWindows } = await import("./renderWindowService");
    const windows = await listRenderWindows("session-1");

    expect(invokeMock).toHaveBeenCalledWith("list_render_windows", {
      sessionId: "session-1",
    });
    expect(windows.map((window) => window.label)).toEqual([
      "render-session-1-1",
      "render-session-1-2",
    ]);
    expect(windows[0]?.surface_id).toBe("surface-1");
    expect(windows[1]?.renderer_attached).toBe(false);
  });

  it("closes a render window by label", async () => {
    invokeMock.mockResolvedValue(undefined);

    const { closeRenderWindow } = await import("./renderWindowService");
    await closeRenderWindow("render-session-1-2");

    expect(invokeMock).toHaveBeenCalledWith("close_render_window", {
      label: "render-session-1-2",
    });
  });

  it("reads the current render window context", async () => {
    invokeMock.mockResolvedValue({
      label: "render-session-1-2",
      session_id: "session-1",
      surface_id: "surface-2",
      role: "controller",
      renderer_attached: true,
      session_window_count: 2,
    });

    const { getRenderWindowContext } = await import("./renderWindowService");
    const context = await getRenderWindowContext();

    expect(invokeMock).toHaveBeenCalledWith("render_window_context");
    expect(context).toEqual({
      label: "render-session-1-2",
      session_id: "session-1",
      surface_id: "surface-2",
      role: "controller",
      renderer_attached: true,
      session_window_count: 2,
    });
  });

  it("opens a dedicated render window for a specific surface", async () => {
    invokeMock.mockResolvedValue("render-session-1-3");

    const { openRenderSurfaceWindow } = await import("./renderWindowService");
    const label = await openRenderSurfaceWindow("session-1", "surface-2");

    expect(invokeMock).toHaveBeenCalledWith("open_render_surface_window", {
      sessionId: "session-1",
      surfaceId: "surface-2",
    });
    expect(label).toBe("render-session-1-3");
  });
});
