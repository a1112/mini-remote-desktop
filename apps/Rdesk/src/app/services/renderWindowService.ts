import { invoke } from "@tauri-apps/api/tauri";

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
