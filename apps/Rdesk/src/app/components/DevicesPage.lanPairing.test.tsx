import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Monitor } from "lucide-react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import type { DeviceInGroup, NetworkGroup } from "../services/networkGroupService";
import type { Device } from "./deviceData";
import { DevicesPage } from "./DevicesPage";

const { deviceListState, launchLocalTest, launchRemote, runtime, groupState, getGroupDevices } = vi.hoisted(() => ({
  deviceListState: { devices: [] as Device[], loading: false, error: null as string | null },
  launchLocalTest: vi.fn(),
  launchRemote: vi.fn(),
  runtime: { native: true },
  groupState: { groups: [] as NetworkGroup[], signedIn: false },
  getGroupDevices: vi.fn(),
}));

vi.mock("../services/remoteDisplayLauncher", () => ({
  launchLocalRemoteDisplayTest: launchLocalTest,
  launchRemoteDisplayForDevice: launchRemote,
}));
vi.mock("../utils/runtime", () => ({ isTauriRuntime: () => runtime.native }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("./AuthContext", () => ({
  useAuth: () => ({
    isLoggedIn: groupState.signedIn,
    token: groupState.signedIn ? "account-token" : null,
    user: groupState.signedIn ? { id: "account" } : null,
  }),
}));
vi.mock("./deviceData", () => ({
  useDevices: () => ({
    ...deviceListState, refresh: vi.fn(),
    lastUpdated: null, currentDeviceId: "local-device",
  }),
}));
vi.mock("../hooks/useNetworkGroups", () => ({
  useNetworkGroups: () => ({ groups: groupState.groups, refresh: vi.fn() }),
}));
vi.mock("../services/networkGroupService", () => ({ networkGroupService: { getGroupDevices } }));

beforeEach(() => {
  groupState.groups = [];
  groupState.signedIn = false;
  getGroupDevices.mockReset();
});

function renderDevices(url = "/devices") {
  const router = createMemoryRouter([
    { path: "/devices", element: <DevicesPage /> },
    { path: "/devices/:id", element: <h1>设备详情</h1> },
    { path: "/session/:id", element: <h1>等待远程桌面</h1> },
    { path: "/display/:id", element: <h1>本机测试</h1> },
  ], { initialEntries: [url] });
  render(<RouterProvider router={router} />);
  return router;
}

const serverDevice: Device = {
  id: "server-device",
  name: "设计终端",
  deviceId: "0123456789",
  os: "Windows",
  icon: Monitor,
  status: "online",
  location: "",
  ping: null,
  lastSeen: "刚刚",
  cpu: null,
  ram: null,
  disk: null,
  ip: "10.0.0.12",
  group: "",
  favorite: false,
  discoverySources: ["server"],
  primarySource: "server",
  sourceLabel: "服务器",
  isLocal: false,
  p2pAvailable: false,
  serverAvailable: true,
};

const lanDevice: Device = {
  ...serverDevice,
  id: "lan-device",
  name: "渲染节点",
  deviceId: "lan-render",
  os: "Linux",
  ip: "192.168.10.2",
  discoverySources: ["lan_p2p"],
  primarySource: "lan_p2p",
  sourceLabel: "P2P 局域网",
  p2pAvailable: true,
  serverAvailable: false,
};

describe("normal device page first LAN pairing", () => {
  beforeEach(() => {
    deviceListState.devices = [];
    deviceListState.loading = false;
    deviceListState.error = null;
    getMockInvoke().mockResolvedValue({ type: "LanPairingCandidateList", candidates: [] });
  });
  it("only offers local tools in the installed app", () => {
    runtime.native = false;
    try {
      renderDevices();
      expect(screen.getByRole("heading", { name: "我的设备" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "局域网配对" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "测试窗口" })).not.toBeInTheDocument();
    } finally {
      runtime.native = true;
    }
  });

  it("offers an explicit pairing entry before any remote session is started", () => {
    renderDevices();
    expect(screen.getByRole("button", { name: "局域网配对" })).toBeInTheDocument();
  });

  it.each([
    { loading: true, error: null },
    { loading: false, error: "公网 API 不可达" },
  ])("keeps LAN pairing reachable while the device list is unavailable: %j", async (state) => {
    Object.assign(deviceListState, state);
    renderDevices();
    fireEvent.click(screen.getByRole("button", { name: "局域网配对" }));
    expect(await screen.findByText(/暂无可配对设备/)).toBeInTheDocument();
    expect(getMockInvoke()).toHaveBeenCalledWith("ipc_secure_remote", { request: { type: "ListLanPairingCandidates" } });
  });
});

describe("desktop device search and launch", () => {
  beforeEach(() => {
    deviceListState.devices = [serverDevice, lanDevice];
    deviceListState.loading = false;
    deviceListState.error = null;
    launchRemote.mockReset();
    launchLocalTest.mockReset();
  });

  it.each(["设计", "012 345 6789", "windows", "10.0.0.12"])("accepts a title bar query for %s", (query) => {
    renderDevices(`/devices?q=${encodeURIComponent(query)}`);
    expect(screen.getByRole("searchbox", { name: "搜索设备" })).toHaveValue(query);
    expect(screen.getByRole("article", { name: serverDevice.name })).toBeInTheDocument();
    expect(screen.queryByRole("article", { name: lanDevice.name })).not.toBeInTheDocument();
  });

  it("replaces only the query as the device list search changes", () => {
    const router = renderDevices("/devices?source=sidebar&q=unmatched");
    const search = screen.getByRole("searchbox", { name: "搜索设备" });
    expect(screen.getByRole("status")).toHaveTextContent("没有符合条件的设备");

    fireEvent.change(search, { target: { value: lanDevice.name } });
    expect(new URLSearchParams(router.state.location.search).get("q")).toBe(lanDevice.name);
    expect(router.state.historyAction).toBe("REPLACE");
    expect(screen.getByRole("article", { name: lanDevice.name })).toBeInTheDocument();

    fireEvent.change(search, { target: { value: "" } });
    expect(router.state.location.search).toBe("?source=sidebar");
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("keeps details keyboard-accessible for offline and disabled devices", async () => {
    deviceListState.devices = [
      { ...serverDevice, status: "offline" },
      { ...lanDevice, disabled: true },
    ];
    const router = renderDevices();
    const offline = screen.getByRole("article", { name: serverDevice.name });
    const disabled = screen.getByRole("article", { name: lanDevice.name });
    expect(within(offline).getByText("设备离线")).toBeInTheDocument();
    expect(within(disabled).getByText("设备已禁用")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /远程连接|P2P 连接/ })).not.toBeInTheDocument();

    const details = within(offline).getByRole("link", { name: `查看详情 ${serverDevice.name}` });
    details.focus();
    await userEvent.keyboard("{Enter}");
    expect(router.state.location.pathname).toBe("/devices/server-device");
    expect(launchRemote).not.toHaveBeenCalled();
  });

  it("allows only one launch at a time and reports a failure inline before retry", async () => {
    let failLaunch!: (error: Error) => void;
    launchRemote.mockImplementationOnce(() => new Promise((_, reject) => { failLaunch = reject; }));
    launchRemote.mockResolvedValueOnce({ mode: "route", sessionId: "lan-session" });
    const router = renderDevices();
    const serverConnect = screen.getByRole("button", { name: `远程连接 ${serverDevice.name}` });
    const lanConnect = screen.getByRole("button", { name: `P2P 连接 ${lanDevice.name}` });

    fireEvent.click(serverConnect);
    expect(launchRemote).toHaveBeenCalledWith(serverDevice.deviceId, expect.objectContaining({ transportKind: "webrtc", lanP2P: false }));
    expect(serverConnect).toHaveAttribute("aria-busy", "true");
    expect(lanConnect).toBeDisabled();
    expect(screen.getByRole("button", { name: "测试窗口" })).toBeDisabled();
    fireEvent.click(lanConnect);
    expect(launchRemote).toHaveBeenCalledTimes(1);

    await act(async () => { failLaunch(new Error("路由暂不可用")); });
    expect(screen.getByRole("alert")).toHaveTextContent("连接 设计终端 失败。路由暂不可用");
    expect(lanConnect).toBeEnabled();
    await userEvent.click(lanConnect);
    expect(launchRemote).toHaveBeenLastCalledWith(lanDevice.deviceId, expect.objectContaining({ transportKind: "quic", lanP2P: true }));
    expect(router.state.location.pathname).toBe("/session/lan-session");
  });

  it("presents the local card as a display test", async () => {
    deviceListState.devices = [{ ...serverDevice, id: "this-computer", deviceId: "local-device" }];
    launchLocalTest.mockResolvedValue({ mode: "route", sessionId: "local-preview" });
    const router = renderDevices();
    await userEvent.click(screen.getByRole("button", { name: `测试本机 ${serverDevice.name}` }));
    expect(launchLocalTest).toHaveBeenCalledOnce();
    expect(launchRemote).not.toHaveBeenCalled();
    expect(router.state.location.pathname).toBe("/display/local-preview");
  });

  it("opens the overflow menu with the keyboard and exposes unavailable management actions", async () => {
    const user = userEvent.setup();
    const router = renderDevices();
    const more = screen.getByRole("button", { name: `${serverDevice.name}的更多操作` });
    more.focus();
    await user.keyboard("{Enter}");
    const menu = await screen.findByRole("menu");
    const rename = within(menu).getByRole("menuitem", { name: "重命名 暂不可用" });
    expect(rename).toHaveAttribute("aria-disabled", "true");
    expect(within(menu).getByRole("menuitem", { name: "网络唤醒 暂不可用" })).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(rename);
    expect(router.state.location.pathname).toBe("/devices");
    expect(launchRemote).not.toHaveBeenCalled();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(more).toHaveFocus();

    await user.keyboard("{Enter}");
    await user.click(await screen.findByRole("menuitem", { name: "设备信息" }));
    expect(router.state.location.pathname).toBe("/devices/server-device");
    expect(router.state.location.search).toBe("?tab=info");
  });

  it("uses the same management availability in the context menu and still starts remote sessions", async () => {
    launchRemote.mockResolvedValue({ mode: "route", sessionId: "context-session" });
    const router = renderDevices();
    fireEvent.contextMenu(screen.getByRole("article", { name: lanDevice.name }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "重启 暂不可用" })).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "远程桌面" }));
    expect(launchRemote).toHaveBeenCalledWith(lanDevice.deviceId, expect.objectContaining({ lanP2P: true }));
    expect(router.state.location.pathname).toBe("/session/context-session");
  });
});

describe("network group device membership", () => {
  const designGroup: NetworkGroup = {
    id: "design-group",
    user_id: "account",
    name: "设计分组",
    is_enabled: true,
    device_count: 1,
    online_device_count: 1,
    created_at: "2026-11-18T00:00:00Z",
    updated_at: "2026-11-18T00:00:00Z",
  };
  const renderGroup = { ...designGroup, id: "render-group", name: "渲染分组" };
  const member = (device: Device, enabled = true): DeviceInGroup => ({
    id: `association-${device.id}`,
    device_id: device.deviceId,
    name: device.name,
    status: device.status,
    ip: device.ip,
    is_enabled: enabled,
  });

  beforeEach(() => {
    groupState.signedIn = true;
    groupState.groups = [designGroup, renderGroup];
    Object.assign(deviceListState, { devices: [serverDevice, lanDevice], loading: false, error: null });
  });

  it("filters by the public device code and combines the group with text search", async () => {
    getGroupDevices.mockResolvedValue([member(serverDevice, false)]);
    const user = userEvent.setup();
    renderDevices();
    expect(screen.getAllByRole("article")).toHaveLength(2);
    expect(getGroupDevices).not.toHaveBeenCalled();

    const filter = screen.getByRole("combobox", { name: "按网络分组筛选" });
    await user.selectOptions(filter, designGroup.id);
    expect(getGroupDevices).toHaveBeenCalledWith(designGroup.id);
    const card = await screen.findByRole("article", { name: serverDevice.name });
    expect(within(card).getByText("设备在此分组中已禁用")).toBeInTheDocument();
    expect(screen.queryByRole("article", { name: lanDevice.name })).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole("searchbox"), { target: { value: lanDevice.name } });
    expect(screen.getByRole("status")).toHaveTextContent("没有符合条件的设备");
    await user.selectOptions(filter, "");
    expect(screen.getByRole("article", { name: lanDevice.name })).toBeInTheDocument();
  });

  it("ignores late membership results for a group that is no longer selected", async () => {
    let resolveDesign!: (value: DeviceInGroup[]) => void;
    getGroupDevices.mockImplementationOnce(() => new Promise((resolve) => { resolveDesign = resolve; }));
    getGroupDevices.mockResolvedValueOnce([member(lanDevice)]);
    const user = userEvent.setup();
    renderDevices();

    const filter = screen.getByRole("combobox", { name: "按网络分组筛选" });
    await user.selectOptions(filter, designGroup.id);
    expect(screen.getByRole("status")).toHaveTextContent("正在读取分组设备");
    expect(screen.queryByRole("article")).not.toBeInTheDocument();
    await user.selectOptions(filter, renderGroup.id);
    expect(await screen.findByRole("article", { name: lanDevice.name })).toBeInTheDocument();
    await act(async () => { resolveDesign([member(serverDevice)]); });
    expect(screen.queryByRole("article", { name: serverDevice.name })).not.toBeInTheDocument();
  });

  it("reports membership failure and retries without showing unrelated devices", async () => {
    getGroupDevices.mockRejectedValueOnce(new Error("获取分组设备失败: 503"));
    getGroupDevices.mockResolvedValueOnce([]);
    const user = userEvent.setup();
    renderDevices();
    await user.selectOptions(screen.getByRole("combobox", { name: "按网络分组筛选" }), designGroup.id);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("获取分组设备失败: 503");
    expect(screen.queryByRole("article")).not.toBeInTheDocument();

    await user.click(within(alert).getByRole("button", { name: "重试" }));
    expect(await screen.findByText("此分组暂无可显示的设备")).toBeInTheDocument();
    expect(getGroupDevices).toHaveBeenCalledTimes(2);
  });
});
