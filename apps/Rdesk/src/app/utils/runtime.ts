export const isTauriRuntime = (): boolean => {
  if (typeof window === "undefined") return false;
  const w = window as any;
  if (w.__TAURI__ || w.__TAURI_INTERNALS__) return true;
  const protocol = window.location.protocol;
  if (protocol === "tauri:" || protocol === "asset:") return true;
  const host = window.location.host;
  if (host === "tauri.localhost") return true;
  const ua = navigator.userAgent?.toLowerCase();
  return Boolean(ua && ua.includes("tauri"));
};
