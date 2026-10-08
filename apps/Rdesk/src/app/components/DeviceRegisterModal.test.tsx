import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PublicServerStatus } from "../adapters/tauri/types";

const mocks = vi.hoisted(() => ({ status: vi.fn(), bind: vi.fn(), register: vi.fn(), hardware: vi.fn(), bootstrap: vi.fn() }));
vi.mock("../adapters/tauri", () => ({
  ipcPublicServerStatus: mocks.status,
  ipcBindPublicDevice: mocks.bind,
  ipcUnbindPublicDevice: vi.fn(),
  ipcRegisterDevice: vi.fn(),
  registerDevice: mocks.register,
  getHardwareInfo: mocks.hardware,
  serviceBootstrapIfNeeded: mocks.bootstrap,
}));
vi.mock("../utils/runtime", () => ({ isTauriRuntime: () => true }));

import { DeviceRegisterModal } from "./DeviceRegisterModal";
import { deviceService, useDeviceRegistration } from "../services/deviceService";

const registered: PublicServerStatus = {
  service_running: true, api_url: "https://175.178.16.90/rdesk/api/v1", api_reachable: true,
  device_registered: true, device_id: "012345678", device_name: "Office Mac",
  signaling_state: "authenticated", reconnect_attempt: 0, last_connected_at_ms: 1000, last_error: null,
};
const pending: PublicServerStatus = {
  ...registered, device_registered: false, device_id: null, signaling_state: "disabled",
  last_error: "public_auto_enrollment_pending",
};

function HomeWithDialog({ onSuccess }: { onSuccess: () => void }) {
  const identity = useDeviceRegistration();
  return <><span>{identity.deviceId}</span><DeviceRegisterModal isOpen onClose={vi.fn()} onSuccess={onSuccess} /></>;
}

describe("automatic device registration", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    mocks.status.mockReset().mockResolvedValue({ ok: true, value: registered });
    mocks.bind.mockReset().mockResolvedValue({ ok: true });
    mocks.register.mockReset();
    mocks.hardware.mockReset();
    mocks.bootstrap.mockReset().mockResolvedValue({ ok: true, value: true });
    (deviceService as any).deviceInfo = null;
    (deviceService as any).registrationError = null;
    (deviceService as any).bindingError = null;
  });
  afterEach(() => { cleanup(); vi.useRealTimers(); });

  it("automatically changes from pending to a nine digit code without credential input", async () => {
    mocks.status.mockResolvedValueOnce({ ok: true, value: pending });
    const onSuccess = vi.fn();
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} onSuccess={onSuccess} />);
    await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("正在自动登记并领取设备码");
    expect(screen.getByRole("progressbar", { name: "设备登记进度" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "启动后台服务" })).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("设备登记码")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("新设备凭据")).not.toBeInTheDocument();

    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(screen.getByRole("status")).toHaveTextContent("设备已自动登记");
    expect(screen.getByText("012 345 678")).toBeInTheDocument();
    expect(screen.getByText("9 位设备码")).toBeInTheDocument();
    expect(onSuccess).toHaveBeenCalledOnce();
    expect(mocks.register).not.toHaveBeenCalled();
    expect(mocks.hardware).not.toHaveBeenCalled();
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!).access_token).toBe("service-managed");
  });

  it("offers immediate retry after a transient failure while sharing one poller with the home page", async () => {
    mocks.status.mockResolvedValueOnce({ ok: false, error: { message: "private device credential" } });
    const onSuccess = vi.fn();
    const { unmount } = render(<HomeWithDialog onSuccess={onSuccess} />);
    await act(async () => {});
    expect(mocks.status).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("status")).toHaveTextContent("无法连接本机后台服务");
    expect(screen.queryByText("private device credential")).not.toBeInTheDocument();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "立即重试" })); });
    expect(mocks.status).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("status")).toHaveTextContent("设备已自动登记");
    expect(onSuccess).toHaveBeenCalledOnce();

    await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
    expect(mocks.status).toHaveBeenCalledTimes(4);
    expect(onSuccess).toHaveBeenCalledOnce();
    unmount();
    await vi.advanceTimersByTimeAsync(6000);
    expect(mocks.status).toHaveBeenCalledTimes(4);
  });

  it("does not show cached device metadata as completed enrollment after an IPC failure", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify({
      device_id: "012345678", device_name: "Old Mac", access_token: "service-managed",
      motherboard_serial: "service-managed", registered_at: "old",
    }));
    mocks.status.mockResolvedValue({ ok: false, error: { message: "unreachable" } });
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("无法连接本机后台服务");
    expect(screen.queryByText("设备已自动登记")).not.toBeInTheDocument();
  });

  it("keeps missing IPC visible without a registration spinner and starts only on request", async () => {
    mocks.status.mockResolvedValue({ ok: false, error: { message: "socket unavailable at private path" } });
    mocks.bootstrap.mockResolvedValueOnce({ ok: false, error: { message: "private keychain startup detail" } });
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("自动登记无法继续");
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动后台服务" })).toBeEnabled();

    await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(mocks.status).toHaveBeenCalledTimes(3);
    expect(mocks.bootstrap).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "启动后台服务" })); });
    expect(mocks.bootstrap).toHaveBeenCalledOnce();
    expect(screen.getByRole("status")).toHaveTextContent("后台服务尚未就绪");
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.queryByText(/private/)).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();

    mocks.status.mockResolvedValueOnce({ ok: true, value: pending }).mockResolvedValue({ ok: true, value: registered });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "启动后台服务" })); });
    expect(screen.getByRole("status")).toHaveTextContent("正在自动登记并领取设备码");
    expect(screen.getByRole("progressbar")).toBeInTheDocument();
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(screen.getByRole("status")).toHaveTextContent("设备已自动登记");
  });

  it("offers service startup instead of registration progress when the native service reports stopped", async () => {
    mocks.status.mockResolvedValue({ ok: true, value: { ...pending, service_running: false } });
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("本机后台服务未运行，自动登记尚未开始");
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动后台服务" })).toBeEnabled();
  });

  it("shows a configuration mismatch instead of completion for a different native server", async () => {
    mocks.status.mockResolvedValue({ ok: true, value: { ...registered, api_url: "https://other.example/rdesk/api/v1" } });
    localStorage.setItem("rdesk_access_token", "private.login.token");
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByRole("status")).toHaveTextContent("服务器与客户端配置不一致");
    expect(screen.queryByText("设备已自动登记")).not.toBeInTheDocument();
    expect(mocks.bind).not.toHaveBeenCalled();
  });

  it("starts no polling while the registration dialog is closed", async () => {
    render(<DeviceRegisterModal isOpen={false} onClose={vi.fn()} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
    expect(mocks.status).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
