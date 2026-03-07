import { invoke } from "@tauri-apps/api/tauri";

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

export const openRenderWindow = async (sessionId: string): Promise<string> =>
  invoke("open_render_window", {
    sessionId,
  });

export const openRenderSurfaceWindow = async (
  sessionId: string,
  surfaceId: string
): Promise<string> =>
  invoke("open_render_surface_window", {
    sessionId,
    surfaceId,
  });

export const listRenderWindows = async (
  sessionId: string
): Promise<RenderWindowContext[]> =>
  invoke<RenderWindowContext[]>("list_render_windows", {
    sessionId,
  });

export const closeRenderWindow = async (label: string): Promise<void> =>
  invoke("close_render_window", {
    label,
  });

export const getRenderWindowContext = async (): Promise<RenderWindowContext | null> =>
  invoke("render_window_context");

export const bindCurrentRenderWindowSurface = async (surfaceId: string): Promise<void> =>
  invoke("bind_current_render_window_surface", {
    surfaceId,
  });

export const listRenderSurfaces = async (
  sessionId: string
): Promise<RenderSurfaceDescriptor[]> =>
  invoke<RenderSurfaceDescriptor[]>("list_render_surfaces", {
    sessionId,
  });

export const createRenderSurface = async (
  sessionId: string,
  name?: string
): Promise<RenderSurfaceDescriptor> =>
  invoke<RenderSurfaceDescriptor>("create_render_surface", {
    sessionId,
    name,
  });

export const selectCurrentRenderSurface = async (
  sessionId: string,
  surfaceId: string
): Promise<void> =>
  invoke("select_current_render_surface", {
    sessionId,
    surfaceId,
  });

export const getCurrentRenderSurface = async (
  sessionId: string
): Promise<string | null> =>
  invoke<string | null>("current_render_surface", {
    sessionId,
  });
