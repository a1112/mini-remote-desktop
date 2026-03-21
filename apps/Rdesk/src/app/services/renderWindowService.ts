import { invoke } from "@tauri-apps/api/tauri";

/**
 * Render window service
 *
 * DEPRECATED: All render window commands have been removed.
 * Rendering control is now managed through mrd-service IPC interface.
 */

export type RenderWindowContext = {
  label: string;
  session_id: string;
  surface_id: string;
  role: string;
  renderer_attached: boolean;
  session_window_count: number;
};

export type RenderSurfaceDescriptor = {
  surface_id: string;
  name: string;
  role: string;
  current: boolean;
};

/**
 * @deprecated open_render_window command removed - rendering now managed by mrd-service
 */
export const openRenderWindow = async (_sessionId: string): Promise<string> => {
  throw new Error(
    "open_render_window 命令已移除。渲染窗口现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated open_render_surface_window command removed
 */
export const openRenderSurfaceWindow = async (
  _sessionId: string,
  _surfaceId: string
): Promise<string> => {
  throw new Error(
    "open_render_surface_window 命令已移除。渲染窗口现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated list_render_windows command removed
 */
export const listRenderWindows = async (
  _sessionId: string
): Promise<RenderWindowContext[]> => {
  throw new Error(
    "list_render_windows 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated close_render_window command removed
 */
export const closeRenderWindow = async (_label: string): Promise<void> => {
  throw new Error(
    "close_render_window 命令已移除。渲染窗口现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated render_window_context command removed
 */
export const getRenderWindowContext = async (): Promise<RenderWindowContext | null> => {
  throw new Error(
    "render_window_context 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated bind_current_render_window_surface command removed
 */
export const bindCurrentRenderWindowSurface = async (_surfaceId: string): Promise<void> => {
  throw new Error(
    "bind_current_render_window_surface 命令已移除。Surface 绑定现在由 mrd-service 管理。"
  );
};

/**
 * @deprecated list_render_surfaces command removed
 */
export const listRenderSurfaces = async (
  _sessionId: string
): Promise<RenderSurfaceDescriptor[]> => {
  throw new Error(
    "list_render_surfaces 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};

/**
 * @deprecated create_render_surface command removed
 */
export const createRenderSurface = async (
  _sessionId: string,
  _name?: string
): Promise<RenderSurfaceDescriptor> => {
  throw new Error(
    "create_render_surface 命令已移除。Surface 管理现在由 mrd-service 处理。"
  );
};

/**
 * @deprecated select_current_render_surface command removed
 */
export const selectCurrentRenderSurface = async (
  _sessionId: string,
  _surfaceId: string
): Promise<void> => {
  throw new Error(
    "select_current_render_surface 命令已移除。Surface 选择现在由 mrd-service 处理。"
  );
};

/**
 * @deprecated current_render_surface command removed
 */
export const getCurrentRenderSurface = async (
  _sessionId: string
): Promise<string | null> => {
  throw new Error(
    "current_render_surface 命令已移除。请使用 ipc_session_snapshot 代替。"
  );
};
