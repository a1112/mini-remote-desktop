export type DeviceActionPreference = {
  favorite?: boolean;
  removed?: boolean;
};

export type DeviceActionPreferenceTarget = {
  deviceId: string;
  favorite: boolean;
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
      Boolean(preference.favorite || preference.removed)
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

function applyDevicePreferences<T extends DeviceActionPreferenceTarget>(devices: T[]): T[] {
  const preferences = readPreferences();
  return devices
    .filter((device) => !preferences[device.deviceId]?.removed)
    .map((device) => {
      const preference = preferences[device.deviceId];
      if (preference?.favorite === undefined) return device;
      return {
        ...device,
        favorite: preference.favorite,
      };
    });
}

export const deviceActionService = {
  applyDevicePreferences,
  getDevicePreference,
  markDeviceRemoved,
  setDeviceFavorite,
};
