import { describe, expect, it } from "vitest";

describe("renderWindowService (DEPRECATED)", () => {
  it("openRenderWindow throws deprecation error", async () => {
    const { openRenderWindow } = await import("./renderWindowService");

    await expect(openRenderWindow("session-1")).rejects.toThrow("open_render_window 命令已移除");
  });

  it("openRenderSurfaceWindow throws deprecation error", async () => {
    const { openRenderSurfaceWindow } = await import("./renderWindowService");

    await expect(openRenderSurfaceWindow("session-1", "surface-2")).rejects.toThrow("open_render_surface_window 命令已移除");
  });

  it("listRenderWindows throws deprecation error", async () => {
    const { listRenderWindows } = await import("./renderWindowService");

    await expect(listRenderWindows("session-1")).rejects.toThrow("list_render_windows 命令已移除");
  });

  it("closeRenderWindow throws deprecation error", async () => {
    const { closeRenderWindow } = await import("./renderWindowService");

    await expect(closeRenderWindow("render-session-1-2")).rejects.toThrow("close_render_window 命令已移除");
  });

  it("getRenderWindowContext throws deprecation error", async () => {
    const { getRenderWindowContext } = await import("./renderWindowService");

    await expect(getRenderWindowContext()).rejects.toThrow("render_window_context 命令已移除");
  });

  it("bindCurrentRenderWindowSurface throws deprecation error", async () => {
    const { bindCurrentRenderWindowSurface } = await import("./renderWindowService");

    await expect(bindCurrentRenderWindowSurface("surface-3")).rejects.toThrow("bind_current_render_window_surface 命令已移除");
  });

  it("listRenderSurfaces throws deprecation error", async () => {
    const { listRenderSurfaces } = await import("./renderWindowService");

    await expect(listRenderSurfaces("session-1")).rejects.toThrow("list_render_surfaces 命令已移除");
  });

  it("createRenderSurface throws deprecation error", async () => {
    const { createRenderSurface } = await import("./renderWindowService");

    await expect(createRenderSurface("session-1", "Editor")).rejects.toThrow("create_render_surface 命令已移除");
  });

  it("selectCurrentRenderSurface throws deprecation error", async () => {
    const { selectCurrentRenderSurface } = await import("./renderWindowService");

    await expect(selectCurrentRenderSurface("session-1", "surface-3")).rejects.toThrow("select_current_render_surface 命令已移除");
  });

  it("getCurrentRenderSurface throws deprecation error", async () => {
    const { getCurrentRenderSurface } = await import("./renderWindowService");

    await expect(getCurrentRenderSurface("session-1")).rejects.toThrow("current_render_surface 命令已移除");
  });
});
