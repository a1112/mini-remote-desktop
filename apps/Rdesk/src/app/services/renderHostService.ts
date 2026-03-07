import { invoke } from "@tauri-apps/api/tauri";

export type RenderHostSnapshot = {
  attached: boolean;
  surface_count: number;
  attached_surface_ids: string[];
  frame: {
    frame_count: number;
    width: number;
    height: number;
    pixel_format: string;
    bytes: number;
  } | null;
  preview_data_url: string | null;
  renderer_backend: string | null;
  renderer_snapshot: {
    attached_to_target: boolean;
    uploaded_frame_count: number;
    last_width: number;
    last_height: number;
    last_pixel_format: string | null;
  } | null;
  surface_source_bindings: {
    surface_id: string;
    source_id: string;
  }[];
  available_source_ids: string[];
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

export const bindRenderSurfaceSource = async (
  sessionId: string,
  surfaceId: string,
  sourceId: string
): Promise<void> =>
  invoke("bind_render_surface_source", {
    sessionId,
    surfaceId,
    sourceId,
  });
