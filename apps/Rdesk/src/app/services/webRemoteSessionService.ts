export type WebRemoteSession = {
  sessionId: string;
  targetDeviceId: string;
  targetDeviceName: string;
  targetOs: string;
  targetIp: string;
  transportKind: "webrtc";
  createdAt: number;
  mode: "web_to_local" | "web_to_peer";
};

const STORAGE_KEY = "rdesk_web_remote_sessions";

function readSessions(): Record<string, WebRemoteSession> {
  if (typeof window === "undefined") return {};
  try {
    return JSON.parse(window.sessionStorage.getItem(STORAGE_KEY) ?? "{}");
  } catch {
    return {};
  }
}

export function saveWebRemoteSession(session: WebRemoteSession): void {
  if (typeof window === "undefined") return;
  const sessions = readSessions();
  sessions[session.sessionId] = session;
  window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(sessions));
}

export function getWebRemoteSession(sessionId: string): WebRemoteSession | null {
  return readSessions()[sessionId] ?? null;
}
