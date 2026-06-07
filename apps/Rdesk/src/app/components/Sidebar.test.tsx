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
  setDeviceFavorite: vi.fn(),
  setDeviceDisabled: vi.fn(),
  removeDeviceLocally: vi.fn(),
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
  disconnectDeviceSessions: vi.fn(),
  ipcRequestDeviceAction: vi.fn(),
  ipcDeviceDetail: vi.fn(),
}));
const layoutActionMock = vi.hoisted(() => ({
  openTransfers: vi.fn(),
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
  setDeviceFavorite: deviceDataMock.setDeviceFavorite,
  setDeviceDisabled: deviceDataMock.setDeviceDisabled,
  removeDeviceLocally: deviceDataMock.removeDeviceLocally,
}));

vi.mock("../services/deviceService", () => ({
  deviceService: {
    renameDevice: serviceMock.renameDevice,
    unbindDevice: serviceMock.unbindDevice,
  },
}));

vi.mock("../services/ipcSessionService", () => ({
  disconnectDeviceSessions: serviceMock.disconnectDeviceSessions,
}));

vi.mock("../adapters/tauri/commands", () => ({
  ipcRequestDeviceAction: serviceMock.ipcRequestDeviceAction,
  ipcDeviceDetail: serviceMock.ipcDeviceDetail,
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
  disabled: false,
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
        onOpenTransfers={layoutActionMock.openTransfers}
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

  it("disables only device menu entries that do not have a real implementation", () => {
    renderSidebar();

    openDeviceMenu();

    expect(screen.getByRole("button", { name: "文件传输" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "远程终端" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "收藏设备" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "禁用设备" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "断开连接" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "移除设备" })).toBeEnabled();

    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));

    expect(screen.getByRole("button", { name: "重启" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "关机" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Wake-on-LAN" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "设备信息" })).toBeEnabled();
  });

  it("opens the service-backed transfer panel from an online device menu", async () => {
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "文件传输" }));

    expect(layoutActionMock.openTransfers).toHaveBeenCalled();
  });

  it("persists favorite, disabled, and removed state through deviceData actions", async () => {
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();
    await user.click(screen.getByRole("button", { name: "收藏设备" }));

    expect(deviceDataMock.setDeviceFavorite).toHaveBeenCalledWith("agent-device", true);
    expect(deviceDataMock.refresh).toHaveBeenCalled();

    openDeviceMenu();
    await user.click(screen.getByRole("button", { name: "禁用设备" }));

    expect(deviceDataMock.setDeviceDisabled).toHaveBeenCalledWith("agent-device", true);

    openDeviceMenu();
    await user.click(screen.getByRole("button", { name: "移除设备" }));

    expect(deviceDataMock.removeDeviceLocally).toHaveBeenCalledWith("agent-device");
  });

  it("requests service-owned disconnect for the selected device", async () => {
    serviceMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "disconnect",
        accepted: true,
        supported: true,
        message: "Disconnected 2 active session(s).",
      },
    });
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "断开连接" }));

    expect(serviceMock.ipcRequestDeviceAction).toHaveBeenCalledWith(
      "agent-device",
      "disconnect"
    );
    await waitFor(() => {
      expect(screen.getByText(/断开连接：Agent PC/)).toBeInTheDocument();
    });
  });

  it("requests service-owned terminal and power actions for a selected device", async () => {
    serviceMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "remote_terminal",
        accepted: false,
        supported: false,
        message: "Remote terminal requires a service-owned command channel and consent flow.",
      },
    });
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();
    await user.click(screen.getByRole("button", { name: "远程终端" }));

    expect(serviceMock.ipcRequestDeviceAction).toHaveBeenCalledWith(
      "agent-device",
      "remote_terminal"
    );
    await waitFor(() => {
      expect(screen.getByText(/远程终端：Agent PC/)).toBeInTheDocument();
    });

    serviceMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "wake_on_lan",
        accepted: true,
        supported: false,
        message: "Wake-on-LAN provider is reserved; no MAC address binding is available yet.",
      },
    });

    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    await user.click(screen.getByRole("button", { name: "Wake-on-LAN" }));

    expect(serviceMock.ipcRequestDeviceAction).toHaveBeenLastCalledWith(
      "agent-device",
      "wake_on_lan"
    );
  });

  it("requests service-owned restart and shutdown from the management submenu", async () => {
    serviceMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "restart",
        accepted: false,
        supported: false,
        message: "Power management provider is reserved.",
      },
    });
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    await user.click(screen.getByRole("button", { name: "重启" }));

    expect(serviceMock.ipcRequestDeviceAction).toHaveBeenCalledWith(
      "agent-device",
      "restart"
    );
    await waitFor(() => {
      expect(screen.getByText(/重启：Agent PC/)).toBeInTheDocument();
    });

    serviceMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "shutdown",
        accepted: false,
        supported: false,
        message: "Power management provider is reserved.",
      },
    });

    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    await user.click(screen.getByRole("button", { name: "关机" }));

    expect(serviceMock.ipcRequestDeviceAction).toHaveBeenLastCalledWith(
      "agent-device",
      "shutdown"
    );
    await waitFor(() => {
      expect(screen.getByText(/关机：Agent PC/)).toBeInTheDocument();
    });
  });

  it("requests service-owned device detail before opening device info", async () => {
    serviceMock.ipcDeviceDetail.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        device_name: "Agent PC",
        is_local: false,
        is_online: true,
        is_lan_peer: true,
        is_paired: false,
        transports: ["quic"],
        media_capabilities: ["control.keyboard_mouse"],
      },
    });
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    await user.click(screen.getByRole("button", { name: "设备信息" }));

    expect(serviceMock.ipcDeviceDetail).toHaveBeenCalledWith("agent-device");
    await waitFor(() => {
      expect(screen.getByText("设备信息：Agent PC")).toBeInTheDocument();
    });
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
