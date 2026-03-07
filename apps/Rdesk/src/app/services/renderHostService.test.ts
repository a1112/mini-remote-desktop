import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/tauri", () => ({
  invoke: invokeMock,
}));

describe("renderHostService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("attaches and detaches render host sessions via tauri invoke", async () => {
    invokeMock.mockResolvedValue(undefined);

    const { attachRenderHostSession, detachRenderHostSession } = await import("./renderHostService");

    await attachRenderHostSession("session-1");
    await detachRenderHostSession("session-1");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "render_host_attach_session", {
      sessionId: "session-1",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "render_host_detach_session", {
      sessionId: "session-1",
    });
  });

  it("reads render host snapshots via tauri invoke", async () => {
    invokeMock.mockResolvedValue({
      attached: true,
      surface_count: 2,
      attached_surface_ids: ["surface-1", "surface-2"],
      frame: {
        frame_count: 1,
        width: 1280,
        height: 720,
        pixel_format: "Rgb24",
        bytes: 2764800,
      },
      preview_data_url: "data:image/png;base64,abc123",
      renderer_backend: "d3d11",
      renderer_snapshot: {
        attached_to_target: true,
        uploaded_frame_count: 1,
        last_width: 1280,
        last_height: 720,
        last_pixel_format: "Rgb24",
      },
    });

    const { getRenderHostSnapshot } = await import("./renderHostService");
    const snapshot = await getRenderHostSnapshot("session-2");

    expect(invokeMock).toHaveBeenCalledWith("render_host_snapshot", {
      sessionId: "session-2",
    });
    expect(snapshot.attached).toBe(true);
    expect(snapshot.surface_count).toBe(2);
    expect(snapshot.attached_surface_ids).toEqual(["surface-1", "surface-2"]);
    expect(snapshot.frame?.width).toBe(1280);
    expect(snapshot.preview_data_url).toBe("data:image/png;base64,abc123");
    expect(snapshot.renderer_backend).toBe("d3d11");
    expect(snapshot.renderer_snapshot?.uploaded_frame_count).toBe(1);
  });
});
