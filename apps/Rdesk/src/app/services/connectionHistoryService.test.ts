import { beforeEach, describe, expect, it } from "vitest";
import type { RemoteSessionSnapshot } from "../adapters/tauri/types";
import {
  clearConnectionHistory,
  getConnectionHistory,
  recordConnectionClosed,
  recordConnectionSnapshot,
} from "./connectionHistoryService";

const snapshot = (state: RemoteSessionSnapshot["presentation_state"]): RemoteSessionSnapshot => ({
  session_id: "session-1",
  role: "controller",
  peer_device_id: "real-device-1",
  peer_key_id: "key-1",
  access_mode: "attended",
  authorization_state: "granted",
  route_state: "connected",
  media_state: "streaming",
  presentation_state: state,
  requested_scopes: ["screen.view"],
  granted_scopes: ["screen.view"],
  policy_revision: "1",
  created_at_ms: 1,
  updated_at_ms: 2,
});

describe("connection history", () => {
  beforeEach(() => localStorage.clear());

  it("records only authoritative streaming sessions and deduplicates updates", () => {
    recordConnectionSnapshot(snapshot("connecting"), 1000);
    expect(getConnectionHistory()).toEqual([]);

    recordConnectionSnapshot(snapshot("streaming"), 2000);
    recordConnectionSnapshot(snapshot("streaming"), 3000);
    expect(getConnectionHistory()).toEqual([{
      sessionId: "session-1",
      peerDeviceId: "real-device-1",
      role: "controller",
      startedAt: 2000,
      endedAt: null,
    }]);

    recordConnectionSnapshot(snapshot("closed"), 4000);
    expect(getConnectionHistory()[0]?.endedAt).toBe(4000);
  });

  it("ignores invalid persisted data", () => {
    localStorage.setItem("rdesk_connection_history_v1", JSON.stringify([{ sessionId: "fake" }]));
    expect(getConnectionHistory()).toEqual([]);
    clearConnectionHistory();
    expect(getConnectionHistory()).toEqual([]);
  });

  it("ends a previously streamed session from a service close event", () => {
    recordConnectionClosed("unknown", 5000);
    expect(getConnectionHistory()).toEqual([]);
    recordConnectionSnapshot(snapshot("streaming"), 2000);
    recordConnectionClosed("session-1", 5000);
    expect(getConnectionHistory()[0]?.endedAt).toBe(5000);
  });
});
