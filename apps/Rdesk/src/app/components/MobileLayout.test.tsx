import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MobileLayout } from "./MobileLayout";

const mocks = vi.hoisted(() => ({
  launch: vi.fn(),
  history: [] as Array<{ sessionId: string; peerDeviceId: string; role: "controller"; startedAt: number; endedAt: number | null }>,
  devices: [] as Array<Record<string, unknown>>,
  native: true,
  isLoggedIn: false,
}));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => mocks.native }));

vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: true, theme: "dark", setTheme: vi.fn() }) }));
vi.mock("./AuthContext", () => ({ useAuth: () => ({ isLoggedIn: mocks.isLoggedIn, user: null, logout: vi.fn() }) }));
vi.mock("./deviceData", () => ({ useDevices: () => ({ devices: mocks.devices, loading: false, error: null, refresh: vi.fn() }) }));
vi.mock("../services/deviceService", () => ({ useDeviceRegistration: () => ({ deviceId: "123456789", deviceName: "本机" }) }));
vi.mock("../services/connectionHistoryService", () => ({ useConnectionHistory: () => mocks.history }));
vi.mock("../services/remoteDisplayLauncher", () => ({ launchRemoteDisplayForDevice: mocks.launch }));
vi.mock("./ServiceStatusPanel", () => ({ ServiceStatusPanel: () => <div>公网连接状态</div> }));

function renderMobile(path = "/") {
  return render(<MemoryRouter initialEntries={[path]}><Routes>
    <Route path="/session/:id" element={<div>会话路由已打开</div>} />
    <Route path="/browser-session/:id" element={<div>网页会话已打开</div>} />
    <Route path="*" element={<MobileLayout onOpenAuth={vi.fn()} />} />
  </Routes></MemoryRouter>);
}

describe("mobile pages", () => {
  beforeEach(() => {
    mocks.history = [];
    mocks.native = true;
    mocks.isLoggedIn = false;
    mocks.devices = [];
    mocks.launch.mockReset().mockResolvedValue({ sessionId: "real-session", mode: "route" });
  });

  it("shows truthful empty states with no discovered devices or history", async () => {
    const user = userEvent.setup();
    renderMobile();
    expect(screen.getByText("暂无连接记录")).toBeInTheDocument();
    expect(screen.getByText("123 456 789")).toBeInTheDocument();
    expect(screen.getByText("公网连接状态")).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "设备" }));
    expect(screen.getByText("暂无已发现设备")).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "记录" }));
    expect(screen.getByText("暂无连接记录")).toBeInTheDocument();
  });

  it('shows zero-install browser controller mode and navigates to its dedicated session page', async () => {
    mocks.native = false;
    mocks.launch.mockResolvedValue({ sessionId: 'browser-session', mode: 'route', routePath: '/browser-session/browser-session' });
    renderMobile();
    expect(screen.getByText('网页控制端')).toBeInTheDocument();
    expect(screen.queryByText('123 456 789')).not.toBeInTheDocument();
    await userEvent.type(screen.getByPlaceholderText('输入 10 位设备码'), '0123456789');
    await userEvent.type(screen.getByLabelText('远端临时密码'), 'ABCD2345');
    await userEvent.click(screen.getByRole('button', { name: '立即连接' }));
    expect(mocks.launch).toHaveBeenCalledWith('0123456789', expect.objectContaining({ temporaryPassword: 'ABCD2345' }));
    expect(await screen.findByText('网页会话已打开')).toBeInTheDocument();
  });

  it("prefills a recent guest target, focuses the password, and waits for submission", async () => {
    mocks.native = false;
    mocks.history = [{ sessionId: "s1", peerDeviceId: "0123456789", role: "controller", startedAt: 1_000, endedAt: 2_000 }];
    let finishLaunch!: (value: unknown) => void;
    mocks.launch.mockImplementationOnce(() => new Promise((resolve) => { finishLaunch = resolve; }));
    const user = userEvent.setup();
    renderMobile();

    const recent = screen.getByRole("button", { name: /0123456789/ });
    await user.click(recent);
    expect(mocks.launch).not.toHaveBeenCalled();
    expect(screen.getByLabelText("远程设备码")).toHaveValue("0123456789");
    const password = screen.getByLabelText("远端临时密码");
    expect(password).toHaveFocus();
    expect(password).toHaveValue("");
    await user.type(password, "ABCD2345");
    await user.click(screen.getByRole("button", { name: "立即连接" }));
    expect(recent).toBeDisabled();
    expect(mocks.launch).toHaveBeenCalledWith("0123456789", expect.objectContaining({ temporaryPassword: "ABCD2345" }));

    await act(async () => finishLaunch({ sessionId: "guest-retry", mode: "route", routePath: "/browser-session/guest-retry" }));
    expect(await screen.findByText("网页会话已打开")).toBeInTheDocument();
  });

  it("shows a signed-in recent connection error and allows account retry", async () => {
    mocks.native = false;
    mocks.isLoggedIn = true;
    mocks.history = [{ sessionId: "s1", peerDeviceId: "0123456789", role: "controller", startedAt: 1_000, endedAt: 2_000 }];
    mocks.launch.mockRejectedValueOnce(new Error("远端设备当前不可达"));
    mocks.launch.mockResolvedValueOnce({ sessionId: "account-retry", mode: "route", routePath: "/browser-session/account-retry" });
    const user = userEvent.setup();
    renderMobile();

    const recent = within(screen.getByRole("region", { name: "最近连接" }));
    const connect = recent.getByRole("button", { name: /0123456789/ });
    await user.click(connect);
    expect(mocks.launch).toHaveBeenCalledWith("0123456789", expect.objectContaining({ routePreference: "auto" }));
    expect(mocks.launch.mock.calls[0]?.[1]).not.toHaveProperty("temporaryPassword");
    expect(await recent.findByRole("alert")).toHaveTextContent("远端设备当前不可达");

    await user.click(connect);
    expect(await screen.findByText("网页会话已打开")).toBeInTheDocument();
  });

  it("starts an authenticated session for the entered device ID", async () => {
    const user = userEvent.setup();
    renderMobile();
    await user.type(screen.getByPlaceholderText("输入 10 位设备码"), "900 123 456");
    await user.click(screen.getByRole("button", { name: "发起连接" }));
    expect(mocks.launch).toHaveBeenCalledWith("900123456", expect.objectContaining({ routePreference: "auto" }));
    expect(await screen.findByText("会话路由已打开")).toBeInTheDocument();
  });

  it("preserves leading zeroes in a ten digit remote device code", async () => {
    const user = userEvent.setup();
    renderMobile();
    await user.type(screen.getByPlaceholderText("输入 10 位设备码"), "012 345 6789");
    await user.click(screen.getByRole("button", { name: "发起连接" }));
    expect(mocks.launch).toHaveBeenCalledWith("0123456789", expect.objectContaining({ routePreference: "auto" }));
    expect(await screen.findByText("会话路由已打开")).toBeInTheDocument();
  });

  it("rejects malformed input while preserving legacy LAN identifiers", async () => {
    const user = userEvent.setup();
    renderMobile();
    const input = screen.getByPlaceholderText("输入 10 位设备码");
    await user.type(input, "0123456789/private");
    await user.click(screen.getByRole("button", { name: "发起连接" }));
    expect(mocks.launch).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("请输入 10 位数字设备码");
    await user.clear(input);
    await user.type(input, "lan-LCXACE");
    await user.click(screen.getByRole("button", { name: "发起连接" }));
    expect(mocks.launch).toHaveBeenCalledWith("lan-LCXACE", expect.anything());
  });

  it("lists real history and devices without fabricating entries", async () => {
    const user = userEvent.setup();
    mocks.history = [{ sessionId: "s1", peerDeviceId: "remote-1", role: "controller", startedAt: 1_000, endedAt: 2_000 }];
    mocks.devices = [{ id: "remote-1", deviceId: "remote-1", name: "工作电脑", status: "online", isLocal: false, sourceLabel: "P2P 局域网" }];
    renderMobile("/connections");
    expect(screen.getByText("工作电脑")).toBeInTheDocument();
    expect(screen.queryByText("暂无连接记录")).not.toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "设备" }));
    expect(screen.getByRole("link", { name: /工作电脑/ })).toBeInTheDocument();
    expect(screen.queryByText("暂无已发现设备")).not.toBeInTheDocument();
  });
});
