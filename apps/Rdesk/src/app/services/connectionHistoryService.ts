import { useEffect, useState } from "react";
import type { RemoteSessionSnapshot } from "../adapters/tauri/types";

export interface ConnectionHistoryEntry {
  sessionId: string;
  peerDeviceId: string;
  role: "controller" | "agent";
  startedAt: number;
  endedAt: number | null;
}

const STORAGE_KEY = "rdesk_connection_history_v1";
const CHANGE_EVENT = "rdesk_connection_history_changed";
const MAX_ENTRIES = 500;

function validEntry(value: unknown): value is ConnectionHistoryEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Partial<ConnectionHistoryEntry>;
  return typeof entry.sessionId === "string" && entry.sessionId.length > 0
    && typeof entry.peerDeviceId === "string" && entry.peerDeviceId.length > 0
    && (entry.role === "controller" || entry.role === "agent")
    && typeof entry.startedAt === "number" && Number.isFinite(entry.startedAt)
    && (entry.endedAt === null || typeof entry.endedAt === "number" && Number.isFinite(entry.endedAt));
}

export function getConnectionHistory(): ConnectionHistoryEntry[] {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(stored)
      ? stored.filter(validEntry).sort((a, b) => b.startedAt - a.startedAt).slice(0, MAX_ENTRIES)
      : [];
  } catch {
    return [];
  }
}

function save(entries: ConnectionHistoryEntry[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries.slice(0, MAX_ENTRIES)));
    window.dispatchEvent(new Event(CHANGE_EVENT));
  } catch {
    // A disabled or full local store should not interrupt a remote session.
  }
}

export function recordConnectionSnapshot(snapshot: RemoteSessionSnapshot, observedAt = Date.now()): void {
  const entries = getConnectionHistory();
  const existing = entries.find((entry) => entry.sessionId === snapshot.session_id);
  if (snapshot.presentation_state === "streaming") {
    if (!existing) {
      save([{ sessionId: snapshot.session_id, peerDeviceId: snapshot.peer_device_id,
        role: snapshot.role, startedAt: observedAt, endedAt: null }, ...entries]);
    }
  } else if (existing && ["closed", "failed", "denied"].includes(snapshot.presentation_state)) {
    recordConnectionClosed(snapshot.session_id, observedAt);
  }
}

export function recordConnectionClosed(sessionId: string, observedAt = Date.now()): void {
  const entries = getConnectionHistory();
  if (!entries.some((entry) => entry.sessionId === sessionId && entry.endedAt === null)) return;
  save(entries.map((entry) => entry.sessionId === sessionId && entry.endedAt === null
    ? { ...entry, endedAt: Math.max(observedAt, entry.startedAt) } : entry));
}

export function clearConnectionHistory(): void {
  save([]);
}

export function useConnectionHistory(): ConnectionHistoryEntry[] {
  const [entries, setEntries] = useState(getConnectionHistory);
  useEffect(() => {
    const refresh = () => setEntries(getConnectionHistory());
    window.addEventListener(CHANGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(CHANGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, []);
  return entries;
}
