import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Monitor } from "lucide-react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DeviceDetailPage } from "./DeviceDetailPage";
import type { Device } from "./deviceData";

const deviceDataMock = vi.hoisted(() => ({
  devices: [] as Device[],
}));

const launcherMock = vi.hoisted(() => ({
  launchRemoteDisplayForDevice: vi.fn(),
  launchRemoteApplicationForDevice: vi.fn(),
  prepareRemoteApplicationCatalogForDevice: vi.fn(),
}));

const sessionMock = vi.hoisted(() => ({
  getSessionSnapshot: vi.fn(),
  getProbeSnapshot: vi.fn(),
  stopSession: vi.fn(),
}));

const commandMock = vi.hoisted(() => ({
  ipcRequestDeviceAction: vi.fn(),
}));

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({
    isDark: false,
    theme: "light",
    setTheme: vi.fn(),
  }),
}));

vi.mock("./DetailBarContext", () => ({
  useDetailBar: () => ({
    collapsed: false,
    payload: null,
    collapse: vi.fn(),
    expand: vi.fn(),
    reset: vi.fn(),
  }),
}));

vi.mock("./deviceData", () => ({
  useDevices: () => ({ devices: deviceDataMock.devices, loading: false }),
  useDeviceById: (id: string | undefined, devices: Device[]) =>
    devices.find((device) => device.id === id),
}));

vi.mock("../services/remoteDisplayLauncher", () => ({
  launchRemoteDisplayForDevice: launcherMock.launchRemoteDisplayForDevice,
  launchRemoteApplicationForDevice: launcherMock.launchRemoteApplicationForDevice,
  prepareRemoteApplicationCatalogForDevice:
    launcherMock.prepareRemoteApplicationCatalogForDevice,
}));

vi.mock("../services/ipcSessionService", () => ({
  getSessionSnapshot: sessionMock.getSessionSnapshot,
  getProbeSnapshot: sessionMock.getProbeSnapshot,
  stopSession: sessionMock.stopSession,
}));

vi.mock("../adapters/tauri/commands", () => ({
  ipcRequestDeviceAction: commandMock.ipcRequestDeviceAction,
}));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
}));

const device = (overrides: Partial<Device>): Device => ({
  id: "agent-device",
  name: "Agent PC",
  deviceId: "agent-device",
  os: "Windows",
  icon: Monitor,
  status: "online",
  location: "LAN",
  ping: 3,
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

function renderDetailPage() {
  render(
    <MemoryRouter initialEntries={["/devices/agent-device"]}>
      <Routes>
        <Route path="/devices/:id" element={<DeviceDetailPage />} />
      </Routes>
    </MemoryRouter>
  );
}

describe("DeviceDetailPage remote toolbar actions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    deviceDataMock.devices = [device({})];
    launcherMock.launchRemoteDisplayForDevice.mockResolvedValue({
      sessionId: "session-1",
      windowLabel: null,
      mode: "native_window",
    });
    sessionMock.getSessionSnapshot.mockResolvedValue({ state: "streaming" });
    sessionMock.getProbeSnapshot.mockResolvedValue({
      current_fps: 60,
      bitrate_mbps: 20,
      frames_decoded: 12,
    });
    commandMock.ipcRequestDeviceAction.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        action: "restart",
        accepted: false,
        supported: false,
        message: "Power management provider is reserved.",
      },
    });
  });

  it("routes toolbar restart through the service-owned device action API", async () => {
    const user = userEvent.setup();

    renderDetailPage();
    await user.click(screen.getByRole("button", { name: "发起远程连接" }));
    await screen.findByRole("button", { name: "重启" });

    await user.click(screen.getByRole("button", { name: "重启" }));

    await waitFor(() => {
      expect(commandMock.ipcRequestDeviceAction).toHaveBeenCalledWith(
        "agent-device",
        "restart"
      );
    });
  });
});
