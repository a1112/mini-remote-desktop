import { ipcWakeOnLan, type WakeOnLanSent } from "../adapters/tauri";

export type DeviceActionPreference = {
  favorite?: boolean;
  disabled?: boolean;
  removed?: boolean;
};

export type DeviceActionPreferenceTarget = {
  deviceId: string;
  favorite: boolean;
  disabled?: boolean;
  status: "online" | "offline";
};

const DEVICE_ACTION_PREFERENCES_KEY = "rdesk_device_action_preferences";

function readPreferences(): Record<string, DeviceActionPreference> {
  if (typeof localStorage === "undefined") return {};
  try {
    const raw = localStorage.getItem(DEVICE_ACTION_PREFERENCES_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, DeviceActionPreference>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writePreferences(preferences: Record<string, DeviceActionPreference>) {
  if (typeof localStorage === "undefined") return;
  const compact = Object.fromEntries(
    Object.entries(preferences).filter(([, preference]) =>
      Boolean(
        preference.favorite !== undefined ||
          preference.disabled ||
          preference.removed
      )
    )
  );
  if (Object.keys(compact).length === 0) {
    localStorage.removeItem(DEVICE_ACTION_PREFERENCES_KEY);
    return;
  }
  localStorage.setItem(DEVICE_ACTION_PREFERENCES_KEY, JSON.stringify(compact));
}

function getDevicePreference(deviceId: string): DeviceActionPreference {
  return readPreferences()[deviceId] ?? {};
}

function setDeviceFavorite(deviceId: string, favorite: boolean): DeviceActionPreference {
  const preferences = readPreferences();
  preferences[deviceId] = {
    ...preferences[deviceId],
    favorite,
  };
  writePreferences(preferences);
  return getDevicePreference(deviceId);
}

function markDeviceRemoved(deviceId: string): DeviceActionPreference {
  const preferences = readPreferences();
  preferences[deviceId] = {
    ...preferences[deviceId],
    removed: true,
  };
  writePreferences(preferences);
  return getDevicePreference(deviceId);
}

function setDeviceDisabled(deviceId: string, disabled: boolean): DeviceActionPreference {
  const preferences = readPreferences();
  preferences[deviceId] = {
    ...preferences[deviceId],
    disabled: disabled ? true : undefined,
  };
  writePreferences(preferences);
  return getDevicePreference(deviceId);
}

function applyDevicePreferences<T extends DeviceActionPreferenceTarget>(devices: T[]): T[] {
  const preferences = readPreferences();
  return devices
    .filter((device) => !preferences[device.deviceId]?.removed)
    .map((device) => {
      const preference = preferences[device.deviceId];
      if (!preference) return device;
      const disabled = preference.disabled === true;
      if (preference.favorite === undefined && !disabled) return device;
      return {
        ...device,
        favorite: preference.favorite ?? device.favorite,
        disabled,
        status: disabled ? "offline" : device.status,
      } as T;
    });
}

async function wakeOnLan(params: {
  deviceId: string;
  macAddress: string;
  broadcastAddr?: string | null;
}): Promise<WakeOnLanSent> {
  const result = await ipcWakeOnLan(params);
  if (!result.ok) {
    throw new Error(result.error.message);
  }
  return result.value;
}

export const deviceActionService = {
  applyDevicePreferences,
  getDevicePreference,
  markDeviceRemoved,
  setDeviceDisabled,
  setDeviceFavorite,
  wakeOnLan,
};
