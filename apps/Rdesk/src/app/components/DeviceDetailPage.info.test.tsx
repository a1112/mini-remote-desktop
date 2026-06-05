import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Monitor } from "lucide-react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

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

const deviceDataMock = vi.hoisted(() => ({
  devices: [] as Device[],
}));

const remoteDisplayLauncherMock = vi.hoisted(() => ({
  launchRemoteApplicationForDevice: vi.fn(),
  launchRemoteDisplayForDevice: vi.fn(),
  prepareRemoteApplicationCatalogForDevice: vi.fn(),
}));

vi.mock("./deviceData", () => ({
  useDevices: () => ({
    devices: deviceDataMock.devices,
    loading: false,
  }),
  useDeviceById: (id: string | undefined) =>
    deviceDataMock.devices.find((item) => item.id === id),
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
  launchRemoteApplicationForDevice: remoteDisplayLauncherMock.launchRemoteApplicationForDevice,
  launchRemoteDisplayForDevice: remoteDisplayLauncherMock.launchRemoteDisplayForDevice,
  prepareRemoteApplicationCatalogForDevice:
    remoteDisplayLauncherMock.prepareRemoteApplicationCatalogForDevice,
}));

vi.mock("../services/ipcSessionService", () => ({
  getProbeSnapshot: vi.fn(),
  getSessionSnapshot: vi.fn(),
  stopSession: vi.fn(),
}));

beforeEach(() => {
  deviceDataMock.devices = [device()];
  remoteDisplayLauncherMock.launchRemoteDisplayForDevice.mockReset();
  remoteDisplayLauncherMock.launchRemoteApplicationForDevice.mockReset();
  remoteDisplayLauncherMock.prepareRemoteApplicationCatalogForDevice.mockReset();
});

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

  it("blocks file transfer for disabled devices even when the device record is online", () => {
    deviceDataMock.devices = [device({ disabled: true, status: "online" })];

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    expect(screen.getByText("设备已禁用，无法传输文件")).toBeInTheDocument();
    expect(screen.queryByText("选择设备以开始传输")).not.toBeInTheDocument();
  });

  it("shows remote launch failures inline without a blocking browser alert", async () => {
    remoteDisplayLauncherMock.launchRemoteDisplayForDevice.mockRejectedValue(
      new Error("service route unavailable")
    );
    const alertSpy = vi.fn();
    Object.defineProperty(window, "alert", {
      configurable: true,
      writable: true,
      value: alertSpy,
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/devices/agent-device"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await user.click(screen.getByRole("button", { name: "发起远程连接" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("service route unavailable");
    });
    expect(alertSpy).not.toHaveBeenCalled();

    delete (window as unknown as Record<string, unknown>).alert;
  });
});
