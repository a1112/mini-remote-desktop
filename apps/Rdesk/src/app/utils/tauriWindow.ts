export type TauriWindowLike = {
  label?: string;
  startDragging: () => Promise<void> | void;
  minimize: () => Promise<void> | void;
  toggleMaximize: () => Promise<void> | void;
  isMaximized: () => Promise<boolean> | boolean;
  close: () => Promise<void> | void;
};

export const getTauriWindowLabel = async (): Promise<string | null> => {
  const win = await getTauriWindow();
  return typeof win?.label === "string" ? win.label : null;
};

const getTauriWindow = async (): Promise<TauriWindowLike | null> => {
  if (typeof window === "undefined") return null;

  try {
    // Tauri v2: @tauri-apps/api/webviewWindow
    const mod = await import("@tauri-apps/api/webviewWindow");
    // v2.10+ uses WebviewWindow.getCurrent() class method
    if ((mod as any)?.WebviewWindow?.getCurrent) {
      return (mod as any).WebviewWindow.getCurrent() as TauriWindowLike;
    }
    // v2.0-2.9 fallback
    if (typeof (mod as any)?.getCurrentWebviewWindow === "function") {
      return (mod as any).getCurrentWebviewWindow() as TauriWindowLike;
    }
  } catch {
    // Ignore missing API when not running in Tauri.
  }

  const w = window as any;
  const tauri = w.__TAURI__;
  // Tauri v2: webviewWindow API (global)
  if (tauri?.webviewWindow?.WebviewWindow?.getCurrent) {
    return tauri.webviewWindow.WebviewWindow.getCurrent();
  }
  // Fallback for older v2 versions
  if (tauri?.webviewWindow?.getCurrentWebviewWindow) {
    return tauri.webviewWindow.getCurrentWebviewWindow();
  }

  return null;
};

export const withTauriWindow = async (
  action: (appWindow: TauriWindowLike) => Promise<void> | void,
): Promise<void> => {
  const win = await getTauriWindow();
  if (!win) return;
  await action(win);
};
