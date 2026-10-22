export const DEFAULT_PUBLIC_API_URL = "https://175.178.16.90/rdesk/api/v1";
export const SERVER_API_URL = (
  (import.meta as any).env?.VITE_RDESK_SERVER_URL ?? DEFAULT_PUBLIC_API_URL
).replace(/\/+$/, "");
export const DEVICE_REGISTRATION_MODE = (
  (import.meta as any).env?.VITE_RDESK_DEVICE_REGISTRATION ?? "server"
).toLowerCase();
