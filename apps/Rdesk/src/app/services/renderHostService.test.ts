import { describe, expect, it } from "vitest";

describe("renderHostService (DEPRECATED)", () => {
  it("attachRenderHostSession throws deprecation error", async () => {
    const { attachRenderHostSession } = await import("./renderHostService");

    await expect(attachRenderHostSession("session-1")).rejects.toThrow("render_host_attach_session 命令已移除");
  });

  it("detachRenderHostSession throws deprecation error", async () => {
    const { detachRenderHostSession } = await import("./renderHostService");

    await expect(detachRenderHostSession("session-1")).rejects.toThrow("render_host_detach_session 命令已移除");
  });

  it("getRenderHostSnapshot throws deprecation error", async () => {
    const { getRenderHostSnapshot } = await import("./renderHostService");

    await expect(getRenderHostSnapshot("session-2")).rejects.toThrow("render_host_snapshot 命令已移除");
  });

  it("bindRenderSurfaceSource throws deprecation error", async () => {
    const { bindRenderSurfaceSource } = await import("./renderHostService");

    await expect(bindRenderSurfaceSource("session-1", "surface-1", "source-1")).rejects.toThrow("bind_render_surface_source 命令已移除");
  });
});
