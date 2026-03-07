import { invoke } from "@tauri-apps/api/tauri";

export const openRenderWindow = async (sessionId: string): Promise<string> =>
  invoke("open_render_window", {
    sessionId,
  });
