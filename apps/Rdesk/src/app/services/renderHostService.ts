import { invoke } from "@tauri-apps/api/tauri";

export type RenderHostSnapshot = {
  attached: boolean;
  frame: {
    frame_count: number;
    width: number;
    height: number;
    pixel_format: string;
    bytes: number;
  } | null;
  preview_data_url: string | null;
};

export const attachRenderHostSession = async (sessionId: string): Promise<void> =>
  invoke("render_host_attach_session", {
    sessionId,
  });

export const detachRenderHostSession = async (sessionId: string): Promise<void> =>
  invoke("render_host_detach_session", {
    sessionId,
  });

export const getRenderHostSnapshot = async (
  sessionId: string
): Promise<RenderHostSnapshot> =>
  invoke<RenderHostSnapshot>("render_host_snapshot", {
    sessionId,
  });
