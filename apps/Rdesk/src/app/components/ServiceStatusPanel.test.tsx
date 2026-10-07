import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServiceStatusPanel } from "./ServiceStatusPanel";

const mocks = vi.hoisted(() => ({ getStatus: vi.fn() }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: false }) }));
vi.mock("../adapters/tauri/commands", () => ({
  ipcPublicServerStatus: mocks.getStatus,
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

describe("public server connection status", () => {
  beforeEach(() => {
    mocks.getStatus.mockReset().mockResolvedValue({ ok: true, value: connected });
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

  it("reports a failed API probe before enrollment as a network problem", async () => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, device_registered: false, device_id: null, signaling_state: "disabled", api_reachable: false, last_error: "public_api_unreachable" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网服务器暂时不可达，正在重试")).toBeInTheDocument();
    expect(screen.queryByText("等待设备登记")).not.toBeInTheDocument();
  });

  it.each([
    ["public_auto_enrollment_pending", "正在领取设备码", "正在自动登记并领取设备码，请稍候。"],
    ["public_auto_enrollment_rate_limited", "自动登记暂未完成，正在重试", "设备登记请求较多，后台将稍后自动重试。"],
    ["public_auto_enrollment_identity_conflict", "等待恢复已有设备身份", "本机已有设备身份需要恢复，请使用设备恢复入口。"],
  ])("shows automatic enrollment %s without claiming an authenticated connection", async (code, label, message) => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, device_registered: false, device_id: null, signaling_state: "disabled", last_error: code } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText(label)).toBeInTheDocument();
    expect(screen.queryByText("公网服务器已连接")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText(message)).toBeInTheDocument();
  });

  it("preserves authenticated signaling when only the API probe is unreachable", async () => {
    mocks.getStatus.mockResolvedValue({ ok: true, value: { ...connected, api_reachable: false, last_error: "public_api_unreachable" } });
    render(<ServiceStatusPanel />);
    expect(await screen.findByText("公网服务器已连接")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "连接详情" }));
    expect(screen.getByText("服务器接口暂时不可达")).toBeInTheDocument();
    expect(screen.getByText("信令已认证在线")).toBeInTheDocument();
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
