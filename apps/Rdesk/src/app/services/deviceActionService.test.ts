import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ipcGetDevicePreferences,
  ipcUpdateDevicePreference,
} from "../adapters/tauri";
import { deviceActionService } from "./deviceActionService";

vi.mock("../adapters/tauri", () => ({
  ipcGetDevicePreferences: vi.fn(),
  ipcRequestRemoteDevicePowerAction: vi.fn(),
  ipcUpdateDevicePreference: vi.fn(),
  ipcWakeOnLan: vi.fn(),
}));

const ipcGetDevicePreferencesMock = vi.mocked(ipcGetDevicePreferences);
const ipcUpdateDevicePreferenceMock = vi.mocked(ipcUpdateDevicePreference);

describe("deviceActionService", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  it("updates favorites through service IPC and caches the returned preference", async () => {
    ipcUpdateDevicePreferenceMock.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        favorite: true,
        disabled: false,
        removed: false,
      },
    });

    const preference = await deviceActionService.setDeviceFavorite("agent-device", true);

    expect(ipcUpdateDevicePreferenceMock).toHaveBeenCalledWith("agent-device", {
      favorite: true,
    });
    expect(preference).toEqual({
      favorite: true,
      disabled: undefined,
      removed: undefined,
    });
    expect(deviceActionService.applyDevicePreferences([
      {
        deviceId: "agent-device",
        favorite: false,
        status: "online",
      },
    ])).toMatchObject([{ favorite: true }]);
  });

  it("syncs service-owned preferences before applying them to device lists", async () => {
    ipcGetDevicePreferencesMock.mockResolvedValue({
      ok: true,
      value: [
        {
          device_id: "disabled-device",
          favorite: false,
          disabled: true,
          removed: false,
        },
        {
          device_id: "removed-device",
          favorite: false,
          disabled: false,
          removed: true,
        },
      ],
    });

    await deviceActionService.refreshDevicePreferences();

    const devices = deviceActionService.applyDevicePreferences([
      {
        deviceId: "disabled-device",
        favorite: false,
        status: "online",
      },
      {
        deviceId: "removed-device",
        favorite: false,
        status: "online",
      },
    ]);

    expect(ipcGetDevicePreferencesMock).toHaveBeenCalled();
    expect(devices).toEqual([
      {
        deviceId: "disabled-device",
        favorite: false,
        disabled: true,
        status: "offline",
      },
    ]);
  });

  it("sends explicit false when re-enabling a disabled device through service IPC", async () => {
    ipcUpdateDevicePreferenceMock.mockResolvedValue({
      ok: true,
      value: {
        device_id: "agent-device",
        favorite: false,
        disabled: false,
        removed: false,
      },
    });

    await deviceActionService.setDeviceDisabled("agent-device", false);

    expect(ipcUpdateDevicePreferenceMock).toHaveBeenCalledWith("agent-device", {
      disabled: false,
    });
  });
});
