import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteSessionSnapshot } from "../adapters/tauri/types";
import { recordConnectionSnapshot } from "../services/connectionHistoryService";
import { ConnectionsPage } from "./ConnectionsPage";

vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("./deviceData", () => ({
  useDevices: () => ({ devices: [], loading: false, error: null }),
}));

const snapshot = (state: RemoteSessionSnapshot["presentation_state"]): RemoteSessionSnapshot => ({
  session_id: "verified-session",
  role: "controller",
  peer_device_id: "real-peer-42",
  peer_key_id: "key",
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

describe("ConnectionsPage", () => {
  beforeEach(() => localStorage.clear());

  it("starts empty and updates from a confirmed streaming session", () => {
    render(<ConnectionsPage />);
    expect(screen.getByText("暂无真实连接记录")).toBeInTheDocument();
    expect(screen.queryByText("办公室电脑")).not.toBeInTheDocument();

    act(() => recordConnectionSnapshot(snapshot("streaming"), 60_000));
    expect(screen.queryByText("暂无真实连接记录")).not.toBeInTheDocument();
    expect(screen.getAllByText("real-peer-42").length).toBeGreaterThan(0);

    act(() => recordConnectionSnapshot(snapshot("closed"), 360_000));
    expect(screen.getByText("5分钟")).toBeInTheDocument();
  });
});
