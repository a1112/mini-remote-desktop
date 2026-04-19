import { invoke } from "@tauri-apps/api/core";

/**
 * Render host service
 *
 * DEPRECATED: All render host commands have been removed.
 * Rendering control is now managed through mrd-service IPC interface.
 */

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

/**
 * @deprecated render_host_attach_session command removed - use ipc_start_session instead
 */
export const attachRenderHostSession = async (_sessionId: string): Promise<void> => {
  throw new Error(
    "render_host_attach_session 命令已移除。请使用 ipc_start_session 代替。"
  );
};

/**
 * @deprecated render_host_detach_session command removed - use ipc_stop_session instead
 */
export const detachRenderHostSession = async (_sessionId: string): Promise<void> => {
  throw new Error(
    "render_host_detach_session 命令已移除。请使用 ipc_stop_session 代替。"
  );
};

/**
 * @deprecated render_host_snapshot command removed - use ipc_session_snapshot instead
 */
export const getRenderHostSnapshot = async (
  _sessionId: string
): Promise<RenderHostSnapshot> => {
  throw new Error(
    "render_host_snapshot 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated bind_render_surface_source command removed
 */
export const bindRenderSurfaceSource = async (
  _sessionId: string,
  _surfaceId: string,
  _sourceId: string
): Promise<void> => {
  throw new Error(
    "bind_render_surface_source 命令已移除。Surface 绑定现在由 mrd-service 管理。"
  );
};
