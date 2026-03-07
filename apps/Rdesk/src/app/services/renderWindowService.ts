import { invoke } from "@tauri-apps/api/tauri";

export type RenderWindowContext = {
  label: string;
  session_id: string;
  session_window_count: number;
};

export const openRenderWindow = async (sessionId: string): Promise<string> =>
  invoke("open_render_window", {
    sessionId,
  });

export const listRenderWindows = async (sessionId: string): Promise<string[]> =>
  invoke("list_render_windows", {
    sessionId,
  });

export const closeRenderWindow = async (label: string): Promise<void> =>
  invoke("close_render_window", {
    label,
  });

export const getRenderWindowContext = async (): Promise<RenderWindowContext | null> =>
  invoke("render_window_context");
