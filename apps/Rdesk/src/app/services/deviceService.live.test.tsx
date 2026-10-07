import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PublicServerStatus } from "../adapters/tauri/types";

const mocks = vi.hoisted(() => ({ status: vi.fn(), bind: vi.fn(), register: vi.fn() }));
vi.mock("../adapters/tauri", () => ({
  ipcPublicServerStatus: mocks.status,
  ipcBindPublicDevice: mocks.bind,
  ipcUnbindPublicDevice: vi.fn(),
  ipcRegisterDevice: vi.fn(),
  registerDevice: mocks.register,
}));
vi.mock("../utils/runtime", () => ({ isTauriRuntime: () => true }));

import { deviceService, useDeviceRegistration, usePublicServerStatus } from "./deviceService";

const registered: PublicServerStatus = {
  service_running: true,
  api_url: "https://175.178.16.90/rdesk/api/v1",
  api_reachable: true,
  device_registered: true,
  device_id: "0123456789",
  device_name: "Office PC",
  signaling_state: "authenticated",
  reconnect_attempt: 0,
  last_connected_at_ms: 1000,
  last_error: null,
};
const unregistered: PublicServerStatus = { ...registered, device_registered: false, device_id: null, signaling_state: "disabled" };

describe("live service-managed device identity", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    mocks.status.mockReset().mockResolvedValue({ ok: true, value: registered });
    mocks.bind.mockReset().mockResolvedValue({ ok: true });
    mocks.register.mockReset();
    (deviceService as any).deviceInfo = null;
    (deviceService as any).registrationError = null;
    (deviceService as any).bindingError = null;
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("recovers the device code automatically after the initial IPC read fails", async () => {
    mocks.status.mockResolvedValueOnce({ ok: false, error: { message: "private transport detail" } });
    const { result } = renderHook(() => ({ identity: useDeviceRegistration(), connection: usePublicServerStatus() }));
    await act(async () => {});
    expect(result.current.identity.deviceId).toBeNull();
    expect(result.current.identity.registrationError).toContain("正在自动重试");
    expect(result.current.connection.failed).toBe(true);

    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(result.current.identity.deviceId).toBe("0123456789");
    expect(result.current.identity.registrationError).toBeNull();
    expect(result.current.connection.status?.signaling_state).toBe("authenticated");
    expect(mocks.register).not.toHaveBeenCalled();
  });

  it("shares one poll across multiple identity/status subscribers and binds only when identity first appears", async () => {
    mocks.status.mockResolvedValueOnce({ ok: true, value: unregistered });
    localStorage.setItem("rdesk_access_token", "private.user.credential");
    const { result, unmount } = renderHook(() => ({
      first: useDeviceRegistration(), second: useDeviceRegistration(), connection: usePublicServerStatus(),
    }));
    await act(async () => {});
    expect(mocks.status).toHaveBeenCalledTimes(1);
    expect(result.current.first.deviceId).toBeNull();
    expect(result.current.first.registrationError).toContain("设备尚未登记");
    expect(mocks.bind).not.toHaveBeenCalled();

    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(result.current.first.deviceId).toBe("0123456789");
    expect(result.current.second.deviceId).toBe("0123456789");
    expect(mocks.status).toHaveBeenCalledTimes(2);
    expect(mocks.bind).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem("rdesk_device_info")).not.toContain("private.user.credential");

    await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
    expect(mocks.status).toHaveBeenCalledTimes(4);
    expect(mocks.bind).toHaveBeenCalledTimes(1);
    unmount();
    await vi.advanceTimersByTimeAsync(6000);
    expect(mocks.status).toHaveBeenCalledTimes(4);
  });

  it("explicit identity refresh requests backend state and updates every subscriber", async () => {
    mocks.status.mockResolvedValueOnce({ ok: true, value: unregistered });
    const { result } = renderHook(() => ({ first: useDeviceRegistration(), second: useDeviceRegistration(), connection: usePublicServerStatus() }));
    await act(async () => {});
    await act(async () => { await result.current.first.refresh(); });
    expect(mocks.status).toHaveBeenCalledTimes(2);
    expect(result.current.first.deviceId).toBe("0123456789");
    expect(result.current.second.deviceId).toBe("0123456789");
    expect(result.current.connection.status?.device_id).toBe("0123456789");
  });

  it("keeps cached code as metadata while failed connectivity cannot remain authenticated", async () => {
    const { result } = renderHook(() => ({ identity: useDeviceRegistration(), connection: usePublicServerStatus() }));
    await act(async () => {});
    mocks.status.mockRejectedValueOnce(new Error("Bearer private.transport.secret"));
    await act(async () => { await result.current.connection.refresh(); });
    expect(result.current.identity.deviceId).toBe("0123456789");
    expect(result.current.connection.status).toBeNull();
    expect(result.current.connection.failed).toBe(true);
    expect(result.current.identity.registrationError).not.toContain("private");
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!).access_token).toBe("service-managed");
  });

  it("reports API network failure distinctly from missing enrollment", async () => {
    mocks.status.mockResolvedValue({ ok: true, value: { ...unregistered, api_reachable: false, last_error: "public_api_unreachable" } });
    const { result } = renderHook(() => useDeviceRegistration());
    await act(async () => {});
    expect(result.current.registrationError).toContain("无法连接公网服务器");
    expect(result.current.registrationError).not.toContain("设备登记码");
    expect(result.current.deviceId).toBeNull();
  });

  it("deduplicates simultaneous manual refreshes while an IPC request is pending", async () => {
    let resolveStatus!: (value: { ok: true; value: PublicServerStatus }) => void;
    mocks.status.mockImplementationOnce(() => new Promise((resolve) => { resolveStatus = resolve; }));
    const { result } = renderHook(() => ({ identity: useDeviceRegistration(), connection: usePublicServerStatus() }));
    const first = result.current.identity.refresh();
    const second = result.current.connection.refresh();
    expect(mocks.status).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveStatus({ ok: true, value: registered });
      await Promise.all([first, second]);
    });
    expect(result.current.identity.deviceId).toBe("0123456789");
    expect(mocks.status).toHaveBeenCalledTimes(1);
  });
});
