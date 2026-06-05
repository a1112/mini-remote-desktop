import { render, screen } from "@testing-library/react";
import { Monitor } from "lucide-react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { DeviceDetailPage } from "./DeviceDetailPage";
import type { Device } from "./deviceData";

const device = (overrides: Partial<Device> = {}): Device => ({
  id: "agent-device",
  name: "Agent PC",
  deviceId: "agent-device",
  os: "Windows",
  icon: Monitor,
  status: "online",
  location: "LAN",
  ping: 7,
  lastSeen: "刚刚",
  cpu: null,
  ram: null,
  disk: null,
  ip: "192.168.1.2",
  group: "LAN P2P",
  favorite: true,
  discoverySources: ["lan_p2p", "server"],
  primarySource: "lan_p2p",
  sourceLabel: "P2P 局域网 / 服务器",
  isLocal: false,
  p2pAvailable: true,
  serverAvailable: true,
  ...overrides,
});

const devices = [device()];

vi.mock("./deviceData", () => ({
  useDevices: () => ({
    devices,
    loading: false,
  }),
  useDeviceById: (id: string | undefined) => devices.find((item) => item.id === id),
}));

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({ isDark: false }),
}));

vi.mock("./DetailBarContext", () => ({
  useDetailBar: () => ({
    collapsed: false,
    payload: null,
    collapse: vi.fn(),
    reset: vi.fn(),
  }),
}));

vi.mock("../services/remoteDisplayLauncher", () => ({
  launchRemoteApplicationForDevice: vi.fn(),
  launchRemoteDisplayForDevice: vi.fn(),
  prepareRemoteApplicationCatalogForDevice: vi.fn(),
}));

vi.mock("../services/ipcSessionService", () => ({
  getProbeSnapshot: vi.fn(),
  getSessionSnapshot: vi.fn(),
  stopSession: vi.fn(),
}));

describe("DeviceDetailPage info tab", () => {
  it("renders real device metadata from the sidebar info route", () => {
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=info"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    expect(screen.getByRole("button", { name: "设备信息" })).toHaveClass("text-blue-600");
    expect(screen.getByText("设备 ID")).toBeInTheDocument();
    expect(screen.getAllByText("agent-device").length).toBeGreaterThan(0);
    expect(screen.getAllByText("P2P 局域网 / 服务器").length).toBeGreaterThan(0);
    expect(screen.getByText("P2P 可用")).toBeInTheDocument();
    expect(screen.getByText("服务器可用")).toBeInTheDocument();
  });
});
