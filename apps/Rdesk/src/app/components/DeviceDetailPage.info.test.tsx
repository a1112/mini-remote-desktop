import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
  native: true,
}));

const remoteDisplayLauncherMock = vi.hoisted(() => ({
  launchRemoteApplicationForDevice: vi.fn(),
  launchRemoteDisplayForDevice: vi.fn(),
  prepareRemoteApplicationCatalogForDevice: vi.fn(),
}));

const tauriAdapterMock = vi.hoisted(() => ({
  ipcCancelFileTransfer: vi.fn(),
  ipcListDirectory: vi.fn(),
  ipcListFileTransferProviders: vi.fn(),
  ipcListFileTransfers: vi.fn(),
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
  ipcCancelFileTransfer: tauriAdapterMock.ipcCancelFileTransfer,
  ipcListDirectory: tauriAdapterMock.ipcListDirectory,
  ipcListFileTransferProviders: tauriAdapterMock.ipcListFileTransferProviders,
  ipcListFileTransfers: tauriAdapterMock.ipcListFileTransfers,
}));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => deviceDataMock.native,
}));

beforeEach(() => {
  deviceDataMock.devices = [device()];
  deviceDataMock.native = true;
  remoteDisplayLauncherMock.launchRemoteDisplayForDevice.mockReset();
  remoteDisplayLauncherMock.launchRemoteDisplayForDevice.mockResolvedValue({
    sessionId: "secure-session",
    windowLabel: null,
    mode: "route",
  });
  remoteDisplayLauncherMock.launchRemoteApplicationForDevice.mockReset();
  remoteDisplayLauncherMock.prepareRemoteApplicationCatalogForDevice.mockReset();
  tauriAdapterMock.ipcCancelFileTransfer.mockReset();
  tauriAdapterMock.ipcListDirectory.mockReset();
  tauriAdapterMock.ipcListFileTransferProviders.mockReset();
  tauriAdapterMock.ipcListFileTransfers.mockReset();
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
  tauriAdapterMock.ipcListFileTransfers.mockResolvedValue({
    ok: true,
    value: [],
  });
  tauriAdapterMock.ipcListFileTransferProviders.mockResolvedValue({
    ok: true,
    value: [
      {
        provider_kind: "mrd-local",
        display_name: "MRD local file transfer",
        status: "available",
        capabilities: ["service.file_transfer.local"],
        reason: null,
        handoff_hint: null,
      },
      {
        provider_kind: "r-file",
        display_name: "R-File external bridge",
        status: "unimplemented",
        capabilities: ["service.file_transfer.external_bridge"],
        reason: "reserved provider bridge",
        handoff_hint: {
          external_app: "R-File",
          bridge_service: "rfile-bridge",
          control_endpoint: "http://127.0.0.1:18100",
          data_endpoint: "http://127.0.0.1:18080",
          capabilities: [
            "rfile.bridge.session_v1",
            "rfile.watch.http_v1",
            "rfile.remote_mount.v1",
          ],
        },
      },
    ],
  });
  tauriAdapterMock.ipcCancelFileTransfer.mockResolvedValue({
    ok: true,
    value: {
      transfer_id: "file-transfer-running",
      status: "cancelled",
      source_device_id: "agent-device",
      target_device_id: "peer-device",
      transport_kind: "local",
      total_entries: 2,
      copied_entries: 1,
      total_bytes: 4096,
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
    deviceDataMock.devices = [device({ isLocal: true })];
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

  it("opens the folder selected from the menu and refreshes the actual service path", async () => {
    deviceDataMock.devices = [device({ isLocal: true })];
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    const folder = await screen.findByRole("button", { name: /ServiceDownloads/ });
    const folderPath = "C:\\Users\\tester\\Downloads";
    const parentPath = "C:\\Users\\tester";
    const listing = { ok: true, value: { path: folderPath, parent_path: parentPath, entries: [] } };
    tauriAdapterMock.ipcListDirectory.mockResolvedValueOnce(listing);
    fireEvent.contextMenu(folder);
    await user.click(await screen.findByRole("menuitem", { name: "打开" }));
    expect(tauriAdapterMock.ipcListDirectory).toHaveBeenLastCalledWith(folderPath);
    expect(await screen.findByText(folderPath)).toBeInTheDocument();

    tauriAdapterMock.ipcListDirectory.mockResolvedValueOnce(listing);
    await user.click(screen.getByRole("button", { name: "刷新目录" }));
    expect(tauriAdapterMock.ipcListDirectory).toHaveBeenLastCalledWith(folderPath);
    await user.click(screen.getByRole("button", { name: "上级目录" }));
    expect(tauriAdapterMock.ipcListDirectory).toHaveBeenLastCalledWith(parentPath);
  });

  it("shows only real file actions and uses the service metadata for properties and copy path", async () => {
    deviceDataMock.devices = [device({ isLocal: true })];
    const user = userEvent.setup();
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes><Route path="/devices/:id" element={<DeviceDetailPage />} /></Routes>
      </MemoryRouter>
    );

    const path = "C:\\Users\\tester\\service-report.txt";
    const file = await screen.findByRole("button", { name: /service-report.txt/ });
    fireEvent.contextMenu(file);
    const menu = within(await screen.findByRole("menu"));
    expect(menu.getAllByRole("menuitem")).toHaveLength(2);
    await user.click(menu.getByRole("menuitem", { name: "复制路径" }));
    expect(await navigator.clipboard.readText()).toBe(path);
    expect(await screen.findByText("已复制文件路径")).toBeInTheDocument();

    await user.click(file);
    const properties = within(screen.getByRole("region", { name: "文件属性" }));
    expect(properties.getByText(path)).toBeInTheDocument();
    expect(properties.getByText("2 KB")).toBeInTheDocument();
    expect(properties.getByText("否")).toBeInTheDocument();
  });

  it("does not display local service files as a remote directory", async () => {
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes><Route path="/devices/:id" element={<DeviceDetailPage />} /></Routes>
      </MemoryRouter>
    );
    await waitFor(() => expect(screen.getByText(/远程目录浏览尚未接入服务/)).toBeInTheDocument());
    expect(screen.queryByText("service-report.txt")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "添加设备" })).not.toBeInTheDocument();
    expect(tauriAdapterMock.ipcListDirectory).not.toHaveBeenCalled();
  });

  it("does not request local file capabilities from the browser", () => {
    deviceDataMock.native = false;
    deviceDataMock.devices = [device({ isLocal: true })];
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes><Route path="/devices/:id" element={<DeviceDetailPage />} /></Routes>
      </MemoryRouter>
    );
    expect(screen.getByText(/网页端暂不支持文件操作/)).toBeInTheDocument();
    expect(tauriAdapterMock.ipcListDirectory).not.toHaveBeenCalled();
    expect(tauriAdapterMock.ipcListFileTransfers).not.toHaveBeenCalled();
    expect(tauriAdapterMock.ipcListFileTransferProviders).not.toHaveBeenCalled();
  });

  it("shows local and reserved file transfer providers in the file transfer tab", async () => {
    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(tauriAdapterMock.ipcListFileTransferProviders).toHaveBeenCalled();
    });
    expect(screen.getByText("传输 Provider")).toBeInTheDocument();
    expect(screen.getByText("MRD local file transfer")).toBeInTheDocument();
    expect(screen.getByText("R-File external bridge")).toBeInTheDocument();
    expect(screen.getByText("预留")).toBeInTheDocument();
    expect(screen.getByText("rfile-bridge")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:18100")).toBeInTheDocument();
  });

  it("shows service-owned file transfer task snapshots in the file transfer tab", async () => {
    tauriAdapterMock.ipcListFileTransfers.mockResolvedValue({
      ok: true,
      value: [
        {
          transfer_id: "file-transfer-1",
          status: "completed",
          source_device_id: "agent-device",
          target_device_id: "peer-device",
          transport_kind: "local",
          provider_kind: "mrd-local",
          provider_capabilities: ["service.file_transfer.local"],
          total_entries: 3,
          copied_entries: 3,
          total_bytes: 3072,
          copied_bytes: 3072,
          error: null,
          entries: [],
        },
      ],
    });

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(tauriAdapterMock.ipcListFileTransfers).toHaveBeenCalled();
    });
    expect(screen.getByText("传输任务")).toBeInTheDocument();
    expect(screen.getByText("file-transfer-1")).toBeInTheDocument();
    expect(screen.getByText("完成 3/3")).toBeInTheDocument();
    expect(screen.getByText("3 KB / 3 KB")).toBeInTheDocument();
    expect(screen.getAllByText("mrd-local").length).toBeGreaterThan(0);
  });

  it("cancels a running service-owned file transfer task from the file transfer tab", async () => {
    tauriAdapterMock.ipcListFileTransfers.mockResolvedValue({
      ok: true,
      value: [
        {
          transfer_id: "file-transfer-running",
          status: "running",
          source_device_id: "agent-device",
          target_device_id: "peer-device",
          transport_kind: "local",
          total_entries: 2,
          copied_entries: 1,
          total_bytes: 4096,
          copied_bytes: 2048,
          error: null,
          entries: [],
        },
      ],
    });
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/devices/agent-device?tab=files"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByText("运行中 1/2")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "取消 file-transfer-running" }));

    expect(tauriAdapterMock.ipcCancelFileTransfer).toHaveBeenCalledWith(
      "file-transfer-running"
    );
    await waitFor(() => {
      expect(screen.getByText("已取消 1/2")).toBeInTheDocument();
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

  it("routes an acknowledged secure request without presenting it as connected", async () => {
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/devices/agent-device"]}>
        <Routes>
          <Route path="/devices/:id" element={<DeviceDetailPage />} />
          <Route
            path="/session/:id"
            element={<div data-testid="secure-session-route">authorizing</div>}
          />
        </Routes>
      </MemoryRouter>
    );

    await user.click(screen.getByRole("button", { name: "发起远程连接" }));

    expect(await screen.findByTestId("secure-session-route")).toHaveTextContent(
      "authorizing",
    );
    expect(remoteDisplayLauncherMock.launchRemoteDisplayForDevice).toHaveBeenCalledWith(
      "agent-device",
      expect.objectContaining({ transportKind: "quic", lanP2P: true }),
    );
    expect(screen.queryByText("Native remote window active")).not.toBeInTheDocument();
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
