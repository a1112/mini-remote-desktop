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

  it("lists explicit render surfaces for a session", async () => {
    invokeMock.mockResolvedValue([
      {
        surface_id: "surface-1",
        name: "Surface 1",
        role: "controller",
        current: true,
      },
      {
        surface_id: "surface-2",
        name: "Screen B",
        role: "controller",
        current: false,
      },
    ]);

    const { listRenderSurfaces } = await import("./renderWindowService");
    const surfaces = await listRenderSurfaces("session-1");

    expect(invokeMock).toHaveBeenCalledWith("list_render_surfaces", {
      sessionId: "session-1",
    });
    expect(surfaces[0]?.current).toBe(true);
    expect(surfaces[1]?.name).toBe("Screen B");
  });

  it("creates and selects an explicit render surface", async () => {
    invokeMock.mockResolvedValueOnce({
      surface_id: "surface-3",
      name: "Editor",
      role: "controller",
      current: true,
    });
    invokeMock.mockResolvedValueOnce(undefined);

    const { createRenderSurface, selectCurrentRenderSurface } = await import("./renderWindowService");
    const surface = await createRenderSurface("session-1", "Editor");
    await selectCurrentRenderSurface("session-1", "surface-3");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "create_render_surface", {
      sessionId: "session-1",
      name: "Editor",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "select_current_render_surface", {
      sessionId: "session-1",
      surfaceId: "surface-3",
    });
    expect(surface.surface_id).toBe("surface-3");
  });

  it("reads the current render surface id for a session", async () => {
    invokeMock.mockResolvedValue("surface-2");

    const { getCurrentRenderSurface } = await import("./renderWindowService");
    const surfaceId = await getCurrentRenderSurface("session-1");

    expect(invokeMock).toHaveBeenCalledWith("current_render_surface", {
      sessionId: "session-1",
    });
    expect(surfaceId).toBe("surface-2");
  });

  it("rebinds the current render window to another surface", async () => {
    invokeMock.mockResolvedValue(undefined);

    const { bindCurrentRenderWindowSurface } = await import("./renderWindowService");
    await bindCurrentRenderWindowSurface("surface-3");

    expect(invokeMock).toHaveBeenCalledWith("bind_current_render_window_surface", {
      surfaceId: "surface-3",
    });
  });
});
