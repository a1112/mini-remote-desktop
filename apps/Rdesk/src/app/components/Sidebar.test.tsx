import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Monitor } from "lucide-react";
import { MemoryRouter, useLocation } from "react-router";
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

const actionServiceMock = vi.hoisted(() => ({
  setDeviceFavorite: vi.fn(),
  markDeviceRemoved: vi.fn(),
  setDeviceDisabled: vi.fn(),
  wakeOnLan: vi.fn(),
}));

const sessionServiceMock = vi.hoisted(() => ({
  listSessions: vi.fn(),
  stopSession: vi.fn(),
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

vi.mock("../services/deviceActionService", () => ({
  deviceActionService: {
    setDeviceFavorite: actionServiceMock.setDeviceFavorite,
    markDeviceRemoved: actionServiceMock.markDeviceRemoved,
    setDeviceDisabled: actionServiceMock.setDeviceDisabled,
    wakeOnLan: actionServiceMock.wakeOnLan,
  },
}));

vi.mock("../services/ipcSessionService", () => ({
  listSessions: sessionServiceMock.listSessions,
  stopSession: sessionServiceMock.stopSession,
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

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{`${location.pathname}${location.search}`}</div>;
}

function renderSidebar() {
  render(
    <MemoryRouter>
      <Sidebar
        collapsed={false}
        onOpenConnections={vi.fn()}
        onOpenSettings={vi.fn()}
      />
      <LocationProbe />
    </MemoryRouter>
  );
}

function openDeviceMenu() {
  fireEvent.contextMenu(screen.getByText("Agent PC"));
}

describe("Sidebar device actions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    actionServiceMock.setDeviceFavorite.mockReturnValue({
      deviceId: "agent-device",
      favorite: true,
      removed: false,
    });
    actionServiceMock.markDeviceRemoved.mockReturnValue({
      deviceId: "agent-device",
      favorite: false,
      removed: true,
    });
    actionServiceMock.setDeviceDisabled.mockReturnValue({
      deviceId: "agent-device",
      disabled: true,
    });
    actionServiceMock.wakeOnLan.mockResolvedValue({
      device_id: "agent-device",
      mac_address: "AA:BB:CC:DD:EE:FF",
      broadcast_addr: "255.255.255.255:9",
      packet_bytes: 102,
    });
    sessionServiceMock.listSessions.mockResolvedValue([]);
    sessionServiceMock.stopSession.mockResolvedValue("session-1");
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

  it("enables implemented device menu entries and keeps unsupported operations disabled", () => {
    renderSidebar();

    openDeviceMenu();

    expect(screen.getByRole("button", { name: "文件传输" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "远程终端" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "收藏设备" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "禁用设备" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "断开连接" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "移除设备" })).not.toBeDisabled();

    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));

    expect(screen.getByRole("button", { name: "重启" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "关机" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Wake-on-LAN" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "设备信息" })).not.toBeDisabled();
  });

  it("sends Wake-on-LAN for an offline device with a known MAC address", async () => {
    deviceDataMock.devices = [
      device({
        status: "offline",
        macAddress: "AA:BB:CC:DD:EE:FF",
      }),
    ];
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    const wakeButton = screen.getByRole("button", { name: "Wake-on-LAN" });
    expect(wakeButton).not.toBeDisabled();

    await user.click(wakeButton);

    expect(actionServiceMock.wakeOnLan).toHaveBeenCalledWith({
      deviceId: "agent-device",
      macAddress: "AA:BB:CC:DD:EE:FF",
      broadcastAddr: undefined,
    });
    await waitFor(() => {
      expect(screen.getByText("已发送唤醒包：Agent PC")).toBeInTheDocument();
    });
  });

  it("navigates to existing detail routes for file transfer and device info", async () => {
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "文件传输" }));
    expect(screen.getByTestId("location")).toHaveTextContent("/devices/agent-device?tab=files");

    openDeviceMenu();
    fireEvent.mouseEnter(screen.getByRole("button", { name: "管理" }));
    await user.click(screen.getByRole("button", { name: "设备信息" }));
    expect(screen.getByTestId("location")).toHaveTextContent("/devices/agent-device?tab=info");
  });

  it("toggles favorite state through local device action preferences", async () => {
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "收藏设备" }));

    expect(actionServiceMock.setDeviceFavorite).toHaveBeenCalledWith("agent-device", true);
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已收藏：Agent PC")).toBeInTheDocument();
    });
  });

  it("marks non-local devices as removed through local device action preferences", async () => {
    Object.defineProperty(window, "confirm", {
      configurable: true,
      writable: true,
      value: vi.fn(() => true),
    });
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "移除设备" }));

    expect(actionServiceMock.markDeviceRemoved).toHaveBeenCalledWith("agent-device");
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已移除：Agent PC")).toBeInTheDocument();
    });
  });

  it("disables a non-local device through local device action preferences", async () => {
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "禁用设备" }));

    expect(actionServiceMock.setDeviceDisabled).toHaveBeenCalledWith("agent-device", true);
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已禁用：Agent PC")).toBeInTheDocument();
    });
  });

  it("re-enables a locally disabled device from the same device menu", async () => {
    actionServiceMock.setDeviceDisabled.mockReturnValue({
      deviceId: "agent-device",
      disabled: false,
    });
    deviceDataMock.devices = [
      device({
        disabled: true,
        status: "offline",
      }),
    ];
    const user = userEvent.setup();

    renderSidebar();
    openDeviceMenu();

    await user.click(screen.getByRole("button", { name: "解除禁用" }));

    expect(actionServiceMock.setDeviceDisabled).toHaveBeenCalledWith("agent-device", false);
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已解除禁用：Agent PC")).toBeInTheDocument();
    });
  });

  it("disconnects the active peer session for the selected device", async () => {
    sessionServiceMock.listSessions.mockResolvedValue([
      {
        session_id: "session-1",
        role: "controller",
        state: "streaming",
        transport_kind: "quic",
        peer_device_id: "agent-device",
        last_error: null,
        sender_active: false,
        receiver_active: true,
      },
    ]);
    const user = userEvent.setup();

    renderSidebar();

    await waitFor(() => {
      expect(sessionServiceMock.listSessions).toHaveBeenCalled();
    });
    openDeviceMenu();
    const disconnect = screen.getByRole("button", { name: "断开连接" });
    expect(disconnect).not.toBeDisabled();

    await user.click(disconnect);

    expect(sessionServiceMock.stopSession).toHaveBeenCalledWith("session-1");
    expect(deviceDataMock.refresh).toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByText("已断开连接：Agent PC")).toBeInTheDocument();
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
