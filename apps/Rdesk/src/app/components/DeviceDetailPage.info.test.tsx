import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

const tauriAdapterMock = vi.hoisted(() => ({
  ipcListDirectory: vi.fn(),
  ipcStartFileTransfer: vi.fn(),
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
  stopSession: vi.fn(() => Promise.resolve()),
}));

vi.mock("../adapters/tauri", () => ({
  ipcListDirectory: tauriAdapterMock.ipcListDirectory,
  ipcStartFileTransfer: tauriAdapterMock.ipcStartFileTransfer,
}));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
}));

beforeEach(() => {
  deviceDataMock.devices = [device()];
  remoteDisplayLauncherMock.launchRemoteDisplayForDevice.mockReset();
  remoteDisplayLauncherMock.launchRemoteApplicationForDevice.mockReset();
  remoteDisplayLauncherMock.prepareRemoteApplicationCatalogForDevice.mockReset();
  tauriAdapterMock.ipcListDirectory.mockReset();
  tauriAdapterMock.ipcStartFileTransfer.mockReset();
  tauriAdapterMock.ipcListDirectory.mockResolvedValue({
    ok: true,
    value: {
      path: "C:\\Users\\tester",
      parent_path: "C:\\Users",
      entries: [
        {
          name: "ServiceDownloads",
          path: "C:\\Users\\tester\\Downloads",
          kind: "directory",
          size_bytes: null,
          modified_ms: 1776000000000,
          readonly: false,
        },
        {
          name: "service-report.txt",
          path: "C:\\Users\\tester\\service-report.txt",
          kind: "file",
          size_bytes: 2048,
          modified_ms: 1776000000000,
          readonly: false,
        },
      ],
    },
  });
  tauriAdapterMock.ipcStartFileTransfer.mockResolvedValue({
    ok: true,
    value: {
      transfer_id: "file-transfer-1",
      status: "completed",
      source_device_id: "agent-device",
      target_device_id: "peer-device",
      transport_kind: "local",
      total_entries: 1,
      copied_entries: 1,
      total_bytes: 2048,
      copied_bytes: 2048,
      error: null,
      entries: [],
    },
  });
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

  it("renders service directory entries in the file transfer tab", async () => {
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText("ServiceDownloads")).toBeInTheDocument();
    });
    expect(screen.getByText("service-report.txt")).toBeInTheDocument();
    expect(tauriAdapterMock.ipcListDirectory).toHaveBeenCalledWith(null);
  });

  it("starts a service-owned file transfer when a file is dropped onto another device pane", async () => {
    deviceDataMock.devices = [
      device(),
      device({
        id: "peer-device",
        deviceId: "peer-device",
        name: "Peer PC",
        ip: "192.168.1.3",
        favorite: false,
      }),
    ];
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText("service-report.txt")).toBeInTheDocument();
    });

    await user.click(screen.getAllByRole("button", { name: "添加设备" })[0]!);
    await user.click(screen.getByRole("button", { name: /Peer PC/ }));

    await waitFor(() => {
      expect(screen.getAllByText("service-report.txt").length).toBeGreaterThan(1);
    });

    const dragStore: Record<string, string> = {};
    const dataTransfer = {
      effectAllowed: "",
      dropEffect: "",
      setData: vi.fn((key: string, value: string) => {
        dragStore[key] = value;
      }),
      getData: vi.fn((key: string) => dragStore[key] ?? ""),
    };

    fireEvent.dragStart(screen.getAllByText("service-report.txt")[0]!, { dataTransfer });
    fireEvent.drop(screen.getAllByText("Peer PC")[0]!, { dataTransfer });

    await waitFor(() => {
      expect(tauriAdapterMock.ipcStartFileTransfer).toHaveBeenCalledWith({
        source_device_id: "agent-device",
        target_device_id: "peer-device",
        entries: [
          {
            source_path: "C:\\Users\\tester\\service-report.txt",
            file_name: "service-report.txt",
            kind: "file",
          },
        ],
        target_path: "C:\\Users\\tester",
        conflict_policy: "rename",
        transport_hint: "local",
      });
    });
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

  it("does not offer non-terminal windows from the remote terminal route", async () => {
    remoteDisplayLauncherMock.prepareRemoteApplicationCatalogForDevice.mockResolvedValue({
      sessionId: "terminal-catalog-session",
      sources: [
        {
          id: "notepad-window",
          platform: "windows",
          source_kind: "window",
          title: "notes.txt - Notepad",
          class_name: "Notepad",
          width: 1280,
          height: 720,
          process_id: 42,
          app_name: "Notepad",
        },
      ],
      windows: [
        {
          id: "notepad-window",
          platform: "windows",
          source_kind: "window",
          title: "notes.txt - Notepad",
          class_name: "Notepad",
          width: 1280,
          height: 720,
          process_id: 42,
          app_name: "Notepad",
        },
      ],
      displays: [],
    });

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=terminal"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText("未发现远程终端窗口")).toBeInTheDocument();
    });
    expect(screen.queryByText("Notepad")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "打开应用" })).not.toBeInTheDocument();
  });
});
