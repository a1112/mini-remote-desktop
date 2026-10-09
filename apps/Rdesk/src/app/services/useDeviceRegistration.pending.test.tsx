import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const status = vi.hoisted(() => vi.fn());
const native = vi.hoisted(() => vi.fn(() => true));
vi.mock("../adapters/tauri", () => ({
  ipcPublicServerStatus: status, ipcRegisterDevice: vi.fn(), registerDevice: vi.fn(),
  ipcBindPublicDevice: vi.fn(), ipcUnbindPublicDevice: vi.fn(),
}));
vi.mock("../utils/runtime", () => ({ isTauriRuntime: native }));
import { deviceService, useDeviceRegistration } from "./deviceService";

const waiting = {
  service_running: true, api_url: "https://175.178.16.90/rdesk/api/v1", api_reachable: true,
  device_registered: false, device_id: null, device_name: null,
  signaling_state: "disconnected", reconnect_attempt: 0, last_connected_at_ms: null, last_error: null,
};
const registered = { ...waiting, device_registered: true, device_id: "0123456789", device_name: "New PC" };

describe("first launch service registration", () => {
  beforeEach(() => {
    vi.useFakeTimers(); localStorage.clear(); native.mockReturnValue(true);
    status.mockReset().mockResolvedValue({ ok: true, value: waiting });
    Object.assign(deviceService, { deviceInfo: null, registrationError: null, bindingError: null, initPromise: null });
  });
  afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); });

  it("updates the first launch code after the resident service registers without account login", async () => {
    status.mockResolvedValueOnce({ ok: true, value: waiting }).mockResolvedValue({ ok: true, value: registered });
    const view = renderHook(() => useDeviceRegistration());
    await act(async () => { await Promise.resolve(); });
    expect(view.result.current.registrationError).toContain("自动登记");
    expect(localStorage.getItem("rdesk_access_token")).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(view.result.current.deviceId).toBe("0123456789");
    expect(view.result.current.isRegistered).toBe(true);
    expect(view.result.current.registrationError).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(status).toHaveBeenCalledTimes(2);
  });

  it("bounds retries for an unavailable first registration and leaves a readable retry message", async () => {
    const view = renderHook(() => useDeviceRegistration());
    await act(async () => { await vi.advanceTimersByTimeAsync(120000); });
    expect(status.mock.calls.length).toBeGreaterThan(1);
    expect(status.mock.calls.length).toBeLessThanOrEqual(31);
    const calls = status.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(120000); });
    expect(status).toHaveBeenCalledTimes(calls);
    expect(view.result.current.registrationError).toContain("重试");
    expect(view.result.current.registrationError).not.toContain("管理员");
  });

  it("stops pending registration checks when the native homepage unmounts", async () => {
    const view = renderHook(() => useDeviceRegistration());
    await act(async () => { await Promise.resolve(); });
    view.unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(status).toHaveBeenCalledTimes(1);
  });

  it("does not query native device status from a browser page", async () => {
    native.mockReturnValue(false);
    renderHook(() => useDeviceRegistration());
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(status).not.toHaveBeenCalled();
  });
  it("refreshes the code when the user retries after initial checks stop", async () => {
    const view = renderHook(() => useDeviceRegistration());
    await act(async () => { await vi.advanceTimersByTimeAsync(120000); });
    status.mockResolvedValue({ ok: true, value: registered });
    await act(async () => { await view.result.current.reregister(); });
    expect(view.result.current.deviceId).toBe("0123456789");
    expect(view.result.current.registrationError).toBeNull();
  });
  it("does not start another registration query after the initial sixty second window", async () => {
    let release!: (reply: unknown) => void;
    status.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    renderHook(() => useDeviceRegistration());
    await act(async () => { await vi.advanceTimersByTimeAsync(59000); });
    await act(async () => { release({ ok: true, value: waiting }); });
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(status).toHaveBeenCalledTimes(1);
  });
});
