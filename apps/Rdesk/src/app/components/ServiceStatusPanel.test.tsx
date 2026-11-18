import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServiceStatusPanel } from "./ServiceStatusPanel";

const mocks = vi.hoisted(() => ({ getStatus: vi.fn(), getLanDiscovery: vi.fn() }));
vi.mock('../utils/runtime', () => ({ isTauriRuntime: () => true }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("../adapters/tauri/commands", () => ({
  ipcPublicServerStatus: mocks.getStatus,
  ipcLanDiscoverySnapshot: mocks.getLanDiscovery,
  shellGetStatus: vi.fn().mockResolvedValue({ ok: true, value: {} }),
  ipcRuntimeSnapshot: vi.fn().mockResolvedValue({ ok: true, value: { sessions: [] } }),
  getClientDiagnostics: vi.fn().mockResolvedValue({ ok: false, error: { message: "unavailable" } }),
}));

const connected = {
  service_running: true,
  api_url: "https://api.example/api/v1",
  api_reachable: true,
  device_registered: true,
  device_id: "0123456789",
  device_name: "Office PC",
  signaling_state: "authenticated",
  reconnect_attempt: 0,
  last_connected_at_ms: 1000,
  last_error: null,
};

const lanSnapshot = {
  enabled: true,
  running: true,
  discovery_port: 49700,
  instance_id: "local",
  peers: [],
};

describe("public server connection status", () => {
  beforeEach(() => {
    mocks.getStatus.mockReset().mockResolvedValue({ ok: true, value: connected });
    mocks.getLanDiscovery.mockReset().mockResolvedValue({ ok: true, value: lanSnapshot });
  });

  it.each([
    ["socket_bind_failed", "无法绑定局域网发现端口"],
    ["broadcast_setup_failed", "无法启用局域网广播"],
    ["probe_send_failed", "局域网探测发送失败"],
  ])("reports LAN error %s independently of public signaling", async (error_code, label) => {
    mocks.getLanDiscovery.mockResolvedValue({ ok: true, value: { ...lanSnapshot, error_code } });
    render(<ServiceStatusPanel />);

    expect(await screen.findByText("局域网发现异常")).toBeInTheDocument();
    expect(screen.getByText("公网服务器已连接")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it("keeps public authentication visible while LAN health cannot be confirmed", async () => {
    mocks.getLanDiscovery.mockResolvedValue({ ok: false, error: { message: "private diagnostic payload" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网服务器已连接")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText("状态未知")).toBeInTheDocument();
    expect(screen.queryByText("private diagnostic payload")).not.toBeInTheDocument();
  });

  it("distinguishes running discovery from a successfully sent probe", async () => {
    render(<ServiceStatusPanel />);
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(await screen.findByText("发现正在运行，等待探测")).toBeInTheDocument();
    mocks.getLanDiscovery.mockResolvedValue({ ok: true, value: { ...lanSnapshot, last_probe_ms: 1 } });
    await userEvent.click(screen.getByRole("button", { name: "刷新连接状态" }));
    expect(await screen.findByText("发现正在运行，已发送探测")).toBeInTheDocument();
  });

  it("shows public connection only after device signaling authentication", async () => {
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网服务器已连接")).toBeInTheDocument();
    expect(screen.queryByText(/pid|mrd-service online/i)).not.toBeInTheDocument();
  });

  it("does not present a reachable API as authenticated signaling", async () => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, signaling_state: "disabled" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网连接未启用")).toBeInTheDocument();
    expect(screen.queryByText("公网服务器已连接")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText("服务器接口可达")).toBeInTheDocument();
    expect(screen.getByText("信令连接未启用")).toBeInTheDocument();
  });

  it("shows waiting for device enrollment separately from API reachability", async () => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, device_registered: false, device_id: "lan-workstation", signaling_state: "disabled" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("等待设备登记")).toBeInTheDocument();
    expect(screen.queryByText("公网服务器已连接")).not.toBeInTheDocument();
  });

  it("drops an authenticated display after the next status request fails", async () => {
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网服务器已连接")).toBeInTheDocument();
    mocks.getStatus.mockResolvedValue({ ok: false, error: { message: "Bearer secret-token https://user:password@api.example/?token=secret" } });
    await userEvent.click(screen.getByRole("button", { name: "刷新连接状态" }));
    await waitFor(() => expect(screen.queryByText("公网服务器已连接")).not.toBeInTheDocument());
    expect(screen.getByText("无法读取连接状态")).toBeInTheDocument();
    expect(screen.queryByText(/secret-token|password|token=secret/)).not.toBeInTheDocument();
  });

  it("never renders secrets, file paths or URL credentials in connection details", async () => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, signaling_state: "backoff", api_url: "https://user:password@api.example/api/v1?token=top-secret", last_error: "transport failed with Bearer secret-token at C:\\Users\\private\\token.txt" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网连接已断开，正在重连")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText("api.example")).toBeInTheDocument();
    expect(screen.queryByText(/top-secret|secret-token|password|Users/)).not.toBeInTheDocument();
  });

  it.each([
    [{ service_running: false }, "本机后台服务未运行"],
    [{ api_url: null }, "公网服务器未配置"],
    [{ signaling_state: "connecting" }, "正在连接公网服务器"],
    [{ signaling_state: "stopped" }, "公网连接已停止"],
  ])("keeps lifecycle and connection phases distinct for %j", async (changes, label) => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, ...changes } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText(label)).toBeInTheDocument();
    expect(screen.queryByText("公网服务器已连接")).not.toBeInTheDocument();
  });
});
