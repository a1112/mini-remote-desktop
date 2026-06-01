import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Monitor } from "lucide-react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { Sidebar } from "./Sidebar";
import type { Device } from "./deviceData";

const deviceDataMock = vi.hoisted(() => ({
  devices: [] as Device[],
  currentDeviceId: "local-device",
  refresh: vi.fn(),
}));

const authMock = vi.hoisted(() => ({
  value: {
    isLoggedIn: false,
    user: null as { id: string; username: string; role: string } | null,
    token: null as string | null,
    login: vi.fn(),
    logout: vi.fn(),
  },
}));

const serviceMock = vi.hoisted(() => ({
  renameDevice: vi.fn(),
  unbindDevice: vi.fn(),
}));

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({
    isDark: false,
    theme: "light",
    setTheme: vi.fn(),
  }),
}));

vi.mock("./AuthContext", () => ({
  useAuth: () => authMock.value,
}));

vi.mock("./deviceData", () => ({
  useDevices: () => ({
    devices: deviceDataMock.devices,
    refresh: deviceDataMock.refresh,
    currentDeviceId: deviceDataMock.currentDeviceId,
  }),
}));

vi.mock("../services/deviceService", () => ({
  deviceService: {
    renameDevice: serviceMock.renameDevice,
    unbindDevice: serviceMock.unbindDevice,
  },
}));

const device = (overrides: Partial<Device>): Device => ({
  id: "agent-device",
  name: "Agent PC",
  deviceId: "agent-device",
  os: "Windows",
  icon: Monitor,
  status: "online",
  location: "LAN",
  ping: 1,
  lastSeen: "刚刚",
  cpu: null,
  ram: null,
  disk: null,
  ip: "192.168.1.2",
  group: "LAN P2P",
  favorite: false,
  discoverySources: ["lan_p2p", "server"],
  primarySource: "lan_p2p",
  sourceLabel: "P2P 局域网 / 服务器",
  isLocal: false,
  p2pAvailable: true,
  serverAvailable: true,
  ...overrides,
});

function renderSidebar() {
  render(
    <MemoryRouter>
      <Sidebar
        collapsed={false}
        onOpenConnections={vi.fn()}
        onOpenSettings={vi.fn()}
      />
    </MemoryRouter>
  );
}

function openDeviceMenu() {
  fireEvent.contextMenu(screen.getByText("Agent PC"));
}

describe("Sidebar device actions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    deviceDataMock.devices = [device({})];
    deviceDataMock.currentDeviceId = "local-device";
    authMock.value = {
      isLoggedIn: false,
      user: null,
      token: null,
      login: vi.fn(),
      logout: vi.fn(),
    };
  });

  it("disables device menu entries that do not have a real implementation", () => {
    renderSidebar();

    openDeviceMenu();

    expect(screen.getByRole("button", { name: "文件传输" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "远程终端" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "收藏设备" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "禁用设备" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "断开连接" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "移除设备" })).toBeDisabled();

    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));

    expect(screen.getByRole("button", { name: "重启" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "关机" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Wake-on-LAN" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "设备信息" })).toBeDisabled();
  });

  it("unbinds the selected device through the device service for a logged-in user", async () => {
    authMock.value = {
      isLoggedIn: true,
      user: { id: "user-1", username: "admin", role: "admin" },
      token: "token-1",
      login: vi.fn(),
      logout: vi.fn(),
    };
    serviceMock.unbindDevice.mockResolvedValue(true);
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "退出绑定" }));

    expect(serviceMock.unbindDevice).toHaveBeenCalledWith("user-1", "agent-device");
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已退出绑定：Agent PC")).toBeInTheDocument();
    });
  });
});
