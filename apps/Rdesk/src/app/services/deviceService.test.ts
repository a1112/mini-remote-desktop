import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mockIpcRegisterDevice = vi.hoisted(() => vi.fn());
const mockRegisterDevice = vi.hoisted(() => vi.fn());
const mockPublicStatus = vi.hoisted(() => vi.fn());
const mockBindPublicDevice = vi.hoisted(() => vi.fn());
const mockUnbindPublicDevice = vi.hoisted(() => vi.fn());

vi.mock("../adapters/tauri", () => ({
  ipcRegisterDevice: mockIpcRegisterDevice,
  registerDevice: mockRegisterDevice,
  ipcPublicServerStatus: mockPublicStatus,
  ipcBindPublicDevice: mockBindPublicDevice,
  ipcUnbindPublicDevice: mockUnbindPublicDevice,
}));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
}));

import { deviceService } from "./deviceService";

const hardwareInfo = {
  motherboard_serial: "MOCKUN3Q8K3Y",
  hostname: "MOCKUN3Q8K3Y",
  os_type: "windows",
  os_version: "Windows 11",
  cpu_info: {
    name: "CPU",
    vendor_id: "GenuineIntel",
    cores: 8,
  },
  total_memory_mb: 32768,
  gpu_info: [],
};

describe("deviceService", () => {
  beforeEach(() => {
    localStorage.clear();
    mockIpcRegisterDevice.mockReset();
    mockIpcRegisterDevice.mockResolvedValue({ ok: true, value: "registered" });
    mockRegisterDevice.mockReset();
    mockPublicStatus.mockReset();
    vi.spyOn(deviceService as any, "shouldUseServiceManagedRegistration").mockReturnValue(false);
    vi.spyOn(deviceService as any, "shouldUseServerRegistration").mockReturnValue(false);
    (deviceService as any).deviceInfo = null;
    (deviceService as any).initPromise = null;
    (deviceService as any).bindingError = null;
    (window as any).__TAURI__ = {
      invoke: vi.fn().mockResolvedValue(hardwareInfo),
    };
  });

  afterEach(() => vi.restoreAllMocks());

  const serverInfo = {
    device_id: "123456789",
    device_name: "My server device",
    access_token: "device-token",
    motherboard_serial: hardwareInfo.motherboard_serial,
    registered_at: "2026-05-03T00:00:00.000Z",
  };

  it("refreshes stored server devices with device authorization instead of a serial lookup", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    mockRegisterDevice.mockResolvedValue({ ok: true, value: { ...serverInfo, access_token: "renewed-token" } });
    const fetchSpy = vi.spyOn(globalThis, "fetch");

    const info = await deviceService.initialize();

    expect(mockRegisterDevice).toHaveBeenCalledWith(expect.objectContaining({
      motherboardSerial: hardwareInfo.motherboard_serial,
      deviceToken: "device-token",
    }));
    expect(info?.access_token).toBe("renewed-token");
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("preserves server identity and credentials when authenticated refresh is unavailable", async () => {
    vi.spyOn(deviceService as any, "shouldUseServerRegistration").mockReturnValue(true);
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    mockRegisterDevice.mockResolvedValue({ ok: false, error: { message: "连接服务器失败，请稍后重试" } });
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new Error("offline"));

    const info = await deviceService.initialize();

    expect(info).toEqual(serverInfo);
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!)).toEqual(serverInfo);
  });

  it("does not report LAN registration as success when server enrollment is required", async () => {
    vi.spyOn(deviceService as any, "shouldUseServerRegistration").mockReturnValue(true);
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("unauthorized", { status: 401 }));

    expect(await deviceService.initialize()).toBeNull();
    expect(localStorage.getItem("rdesk_device_info")).toBeNull();
    expect(mockRegisterDevice).not.toHaveBeenCalled();
  });

  it("enrolls explicitly and stores only the returned device credentials", async () => {
    const enrollmentToken = "a".repeat(43);
    mockRegisterDevice.mockResolvedValue({ ok: true, value: serverInfo });

    const info = await (deviceService as any).enroll(enrollmentToken, "Office PC");

    expect(info.device_id).toBe(serverInfo.device_id);
    expect(mockRegisterDevice).toHaveBeenCalledWith(expect.objectContaining({
      enrollmentToken,
      deviceName: "Office PC",
      cpuInfo: JSON.stringify(hardwareInfo.cpu_info),
      totalMemoryMb: hardwareInfo.total_memory_mb,
      gpuInfo: JSON.stringify(hardwareInfo.gpu_info),
    }));
    expect(localStorage.getItem("rdesk_device_info")).not.toContain(enrollmentToken);
    expect(mockIpcRegisterDevice).toHaveBeenCalledWith(serverInfo.device_id, serverInfo.device_name);
  });

  it("recovers an expired server credential through authenticated refresh without enrollment", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    mockRegisterDevice.mockResolvedValue({ ok: true, value: { ...serverInfo, access_token: "refreshed-token" } });
    const info = await (deviceService as any).recoverDeviceCredential("admin.rotated.token");
    expect(mockRegisterDevice).toHaveBeenCalledWith(expect.objectContaining({ deviceToken: "admin.rotated.token" }));
    expect(mockRegisterDevice.mock.calls[0]?.[0]?.enrollmentToken).toBeUndefined();
    expect(info.device_id).toBe(serverInfo.device_id);
    expect(info.access_token).toBe("refreshed-token");
  });

  it("waits for pending LAN initialization before persisting a server enrollment", async () => {
    let resolveHardware!: (value: typeof hardwareInfo) => void;
    (window.__TAURI__!.invoke as any).mockImplementationOnce(() => new Promise((resolve) => { resolveHardware = resolve; }));
    mockRegisterDevice.mockResolvedValue({ ok: true, value: serverInfo });
    const initializing = deviceService.initialize();
    const enrolling = deviceService.enroll("a".repeat(43));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mockRegisterDevice).not.toHaveBeenCalled();
    resolveHardware(hardwareInfo);
    await Promise.all([initializing, enrolling]);
    expect(deviceService.getDeviceId()).toBe(serverInfo.device_id);
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!).access_token).toBe(serverInfo.access_token);
  });

  it("rejects a recovered credential that would replace an existing server identity", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    mockRegisterDevice.mockResolvedValue({ ok: true, value: { ...serverInfo, device_id: "another-device" } });
    await expect((deviceService as any).recoverDeviceCredential("admin.rotated.token")).rejects.toThrow("设备身份不匹配");
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!)).toEqual(serverInfo);
  });

  it("preserves the old server credential when credential recovery is rejected", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    mockRegisterDevice.mockResolvedValue({ ok: false, error: { message: "设备凭据已失效，请向管理员申请更新设备凭据" } });
    await expect(deviceService.recoverDeviceCredential("expired.device.token")).rejects.toThrow("更新设备凭据");
    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!)).toEqual(serverInfo);
  });

  it("upgrades a local LAN identity using its administrator-issued server credential", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify({ ...serverInfo, device_id: "lan-local", access_token: "local-p2p" }));
    mockRegisterDevice.mockResolvedValue({ ok: true, value: serverInfo });
    expect((await deviceService.recoverDeviceCredential("admin.rotated.token")).device_id).toBe(serverInfo.device_id);
  });

  it("keeps server credentials during an unsuccessful explicit retry", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify(serverInfo));
    vi.spyOn(deviceService as any, "shouldUseServerRegistration").mockReturnValue(true);
    mockRegisterDevice.mockResolvedValue({ ok: false, error: { message: "设备认证已失效，请重新登记" } });
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("forbidden", { status: 403 }));

    await deviceService.reregister();

    expect(JSON.parse(localStorage.getItem("rdesk_device_info")!)).toEqual(serverInfo);
  });

  it("refreshes a stale local-only display name from the Tauri computer hostname", async () => {
    localStorage.setItem(
      "rdesk_device_info",
      JSON.stringify({
        device_id: "lan-MOCKUN3Q8K3Y",
        device_name: "开发服务器",
        access_token: "local-p2p",
        motherboard_serial: "MOCKUN3Q8K3Y",
        registered_at: "2026-05-03T00:00:00.000Z",
      })
    );

    const info = await deviceService.initialize();
    const stored = JSON.parse(localStorage.getItem("rdesk_device_info") ?? "{}");

    expect(info?.device_name).toBe("MOCKUN3Q8K3Y");
    expect(stored.device_name).toBe("MOCKUN3Q8K3Y");
    expect(mockIpcRegisterDevice).toHaveBeenCalledWith(
      "lan-MOCKUN3Q8K3Y",
      "MOCKUN3Q8K3Y"
    );
  });

  it("does not create a device identity when hardware information is unavailable", async () => {
    delete (window as any).__TAURI__;
    const info = await deviceService.initialize();
    expect(info).toBeNull();
    expect(localStorage.getItem("rdesk_device_info")).toBeNull();
  });
});

describe("service-managed public device identity", () => {
  const registeredStatus = {
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
  beforeEach(() => {
    localStorage.clear();
    mockIpcRegisterDevice.mockReset();
    mockIpcRegisterDevice.mockResolvedValue({ ok: true, value: "registered" });
    mockRegisterDevice.mockReset();
    mockPublicStatus.mockReset().mockResolvedValue({ ok: true, value: registeredStatus });
    mockBindPublicDevice.mockReset().mockResolvedValue({ ok: true });
    mockUnbindPublicDevice.mockReset().mockResolvedValue({ ok: true });
    (deviceService as any).deviceInfo = null;
    (deviceService as any).initPromise = null;
    (deviceService as any).bindingError = null;
    (window as any).__TAURI__ = { invoke: vi.fn().mockResolvedValue(hardwareInfo) };
  });
  afterEach(() => vi.restoreAllMocks());

  it("restores the resident identity without retrieving or refreshing a device JWT", async () => {
    const info = await deviceService.initialize();
    expect(info?.device_id).toBe("0123456789");
    expect(info?.device_name).toBe("Office PC");
    expect(info?.access_token).toBe("service-managed");
    expect(mockRegisterDevice).not.toHaveBeenCalled();
    expect(mockIpcRegisterDevice).not.toHaveBeenCalled();
    expect(deviceService.getAccessToken()).toBeNull();
  });

  it("does not present an old local-only identity as public registration", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify({ device_id: "lan-old", device_name: "Old PC", access_token: "local-p2p", motherboard_serial: "old", registered_at: "old" }));
    mockPublicStatus.mockResolvedValue({ ok: true, value: { ...registeredStatus, device_registered: false, device_id: "lan-old" } });
    expect(await deviceService.initialize()).toBeNull();
    expect(deviceService.getDeviceId()).toBeNull();
    expect(deviceService.getRegistrationError()).toContain("设备登记码");
    expect(mockIpcRegisterDevice).not.toHaveBeenCalled();
  });

  it("stores metadata after enrollment and skips the privileged legacy registration pipe", async () => {
    mockRegisterDevice.mockResolvedValue({ ok: true, value: { device_id: "0123456789", device_name: "Office PC", access_token: "service-managed" } });
    const enrollmentToken = "a".repeat(43);
    const info = await deviceService.enroll(enrollmentToken, "Office PC");
    expect(info.access_token).toBe("service-managed");
    expect(mockIpcRegisterDevice).not.toHaveBeenCalled();
    expect(localStorage.getItem("rdesk_device_info")).not.toContain(enrollmentToken);
    expect(mockRegisterDevice).toHaveBeenCalledWith(expect.objectContaining({ enrollmentToken, apiBase: "https://175.178.16.90/rdesk/api/v1" }));
  });

  it("does not send the service-managed placeholder back as a credential on initialization", async () => {
    localStorage.setItem("rdesk_device_info", JSON.stringify({ device_id: "0123456789", device_name: "Office PC", access_token: "service-managed", motherboard_serial: "service-managed", registered_at: "old" }));
    await deviceService.initialize();
    expect(mockRegisterDevice).not.toHaveBeenCalled();
  });

  it("binds an existing device on login using only the user credential over native IPC", async () => {
    await deviceService.initialize();
    localStorage.setItem("rdesk_access_token", "user.access.token");
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    expect((await deviceService.bindDevice("user-1")).success).toBe(true);
    expect(mockBindPublicDevice).toHaveBeenCalledWith("user.access.token");
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("binds after restoration when the user logged in before initialization", async () => {
    localStorage.setItem("rdesk_access_token", "user.access.token");
    await deviceService.initialize();
    expect(mockBindPublicDevice).toHaveBeenCalledWith("user.access.token");
    expect(localStorage.getItem("rdesk_device_info")).not.toContain("user.access.token");
  });

  it.each(["enroll", "recoverDeviceCredential"] as const)("binds after %s when already logged in", async (operation) => {
    mockPublicStatus.mockResolvedValue({ ok: true, value: { ...registeredStatus, device_registered: false } });
    await deviceService.initialize();
    localStorage.setItem("rdesk_access_token", "user.access.token");
    mockRegisterDevice.mockResolvedValue({ ok: true, value: { device_id: registeredStatus.device_id, device_name: "Office PC", access_token: "service-managed" } });
    await deviceService[operation](operation === "enroll" ? "a".repeat(43) : "device.rotated.token");
    expect(mockBindPublicDevice).toHaveBeenCalledWith("user.access.token");
  });

  it("unbinds only the current resident device through native IPC", async () => {
    await deviceService.initialize();
    localStorage.setItem("rdesk_access_token", "user.access.token");
    expect(await deviceService.unbindDevice("user-1", "another-device")).toBe(false);
    expect(mockUnbindPublicDevice).not.toHaveBeenCalled();
    expect(await deviceService.unbindDevice("user-1")).toBe(true);
    expect(mockUnbindPublicDevice).toHaveBeenCalledWith("user.access.token");
  });

  it("preserves registration and exposes a readable binding failure without storing credentials", async () => {
    await deviceService.initialize();
    localStorage.setItem("rdesk_access_token", "user.access.token");
    mockBindPublicDevice.mockResolvedValue({ ok: false, error: { message: "登录已过期，请重新登录后重试" } });
    expect((await deviceService.bindDevice("user-1")).success).toBe(false);
    expect(deviceService.getRegistrationError()).toContain("登录已过期");
    expect(deviceService.getDeviceId()).toBe(registeredStatus.device_id);
    expect(localStorage.getItem("rdesk_device_info")).not.toContain("user.access.token");
  });
});
