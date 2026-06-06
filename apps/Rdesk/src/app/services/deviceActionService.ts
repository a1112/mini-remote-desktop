import {
  ipcGetDevicePreferences,
  ipcRequestRemoteDevicePowerAction,
  ipcUpdateDevicePreference,
  ipcWakeOnLan,
  type DevicePreference,
  type RemoteDevicePowerAction,
  type RemoteDevicePowerActionAccepted,
  type WakeOnLanSent,
} from "../adapters/tauri";

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

function devicePreferenceToActionPreference(
  preference: DevicePreference
): DeviceActionPreference {
  return {
    favorite: preference.favorite,
    disabled: preference.disabled ? true : undefined,
    removed: preference.removed ? true : undefined,
  };
}

function cacheServicePreference(preference: DevicePreference): DeviceActionPreference {
  const preferences = readPreferences();
  preferences[preference.device_id] = devicePreferenceToActionPreference(preference);
  writePreferences(preferences);
  return getDevicePreference(preference.device_id);
}

function replaceCachedServicePreferences(preferences: DevicePreference[]) {
  writePreferences(
    Object.fromEntries(
      preferences.map((preference) => [
        preference.device_id,
        devicePreferenceToActionPreference(preference),
      ])
    )
  );
}

function setLocalDevicePreference(
  deviceId: string,
  update: DeviceActionPreference
): DeviceActionPreference {
  const preferences = readPreferences();
  preferences[deviceId] = {
    ...preferences[deviceId],
    ...update,
  };
  writePreferences(preferences);
  return getDevicePreference(deviceId);
}

async function updateDevicePreference(
  deviceId: string,
  update: DeviceActionPreference
): Promise<DeviceActionPreference> {
  const result = await ipcUpdateDevicePreference(deviceId, update);
  if (result.ok) {
    return cacheServicePreference(result.value);
  }
  return setLocalDevicePreference(deviceId, update);
}

async function refreshDevicePreferences(): Promise<Record<string, DeviceActionPreference>> {
  const result = await ipcGetDevicePreferences();
  if (result.ok) {
    replaceCachedServicePreferences(result.value);
  }
  return readPreferences();
}

async function setDeviceFavorite(
  deviceId: string,
  favorite: boolean
): Promise<DeviceActionPreference> {
  return updateDevicePreference(deviceId, { favorite });
}

async function markDeviceRemoved(deviceId: string): Promise<DeviceActionPreference> {
  return updateDevicePreference(deviceId, { removed: true });
}

async function setDeviceDisabled(
  deviceId: string,
  disabled: boolean
): Promise<DeviceActionPreference> {
  return updateDevicePreference(deviceId, { disabled });
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

async function requestRemoteDevicePowerAction(params: {
  deviceId: string;
  action: RemoteDevicePowerAction;
}): Promise<RemoteDevicePowerActionAccepted> {
  const result = await ipcRequestRemoteDevicePowerAction(params);
  if (!result.ok) {
    throw new Error(result.error.message);
  }
  return result.value;
}

export const deviceActionService = {
  applyDevicePreferences,
  getDevicePreference,
  markDeviceRemoved,
  refreshDevicePreferences,
  requestRemoteDevicePowerAction,
  setDeviceDisabled,
  setDeviceFavorite,
  wakeOnLan,
};
