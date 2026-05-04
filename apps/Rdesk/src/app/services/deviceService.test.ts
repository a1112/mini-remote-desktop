import { beforeEach, describe, expect, it, vi } from "vitest";

const mockIpcRegisterDevice = vi.hoisted(() => vi.fn());

vi.mock("../adapters/tauri", () => ({
  ipcRegisterDevice: mockIpcRegisterDevice,
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
    (deviceService as any).deviceInfo = null;
    (deviceService as any).initPromise = null;
    (window as any).__TAURI__ = {
      invoke: vi.fn().mockResolvedValue(hardwareInfo),
    };
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
});
