import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MobileLayout } from "./MobileLayout";

const mocks = vi.hoisted(() => ({
  launch: vi.fn(),
  history: [] as Array<{ sessionId: string; peerDeviceId: string; role: "controller"; startedAt: number; endedAt: number | null }>,
  devices: [] as Array<Record<string, unknown>>,
}));

vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: true, theme: "dark", setTheme: vi.fn() }) }));
vi.mock("./AuthContext", () => ({ useAuth: () => ({ isLoggedIn: false, user: null, logout: vi.fn() }) }));
vi.mock("./deviceData", () => ({ useDevices: () => ({ devices: mocks.devices, loading: false, error: null, refresh: vi.fn() }) }));
vi.mock("../services/deviceService", () => ({ useDeviceRegistration: () => ({ deviceId: "123456789", deviceName: "本机" }) }));
vi.mock("../services/connectionHistoryService", () => ({ useConnectionHistory: () => mocks.history }));
vi.mock("../services/remoteDisplayLauncher", () => ({ launchRemoteDisplayForDevice: mocks.launch }));

function renderMobile(path = "/") {
  return render(<MemoryRouter initialEntries={[path]}><Routes>
    <Route path="/session/:id" element={<div>会话路由已打开</div>} />
    <Route path="*" element={<MobileLayout onOpenAuth={vi.fn()} />} />
  </Routes></MemoryRouter>);
}

describe("mobile pages", () => {
  beforeEach(() => {
    mocks.history = [];
    mocks.devices = [];
    mocks.launch.mockReset().mockResolvedValue({ sessionId: "real-session", mode: "route" });
  });

  it("shows truthful empty states with no discovered devices or history", async () => {
    const user = userEvent.setup();
    renderMobile();
    expect(screen.getByText("暂无连接记录")).toBeInTheDocument();
    expect(screen.getByText("123456789")).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "设备" }));
    expect(screen.getByText("暂无已发现设备")).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "记录" }));
    expect(screen.getByText("暂无连接记录")).toBeInTheDocument();
  });

  it("starts an authenticated session for the entered device ID", async () => {
    const user = userEvent.setup();
    renderMobile();
    await user.type(screen.getByPlaceholderText("输入设备 ID"), "900 123 456");
    await user.click(screen.getByRole("button", { name: "发起连接" }));
    expect(mocks.launch).toHaveBeenCalledWith("900123456", expect.objectContaining({ routePreference: "auto" }));
    expect(await screen.findByText("会话路由已打开")).toBeInTheDocument();
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
