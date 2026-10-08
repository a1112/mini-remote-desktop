/** Device codes stay strings so a leading zero survives display, copy and IPC. */
export function normalizeDeviceCode(value: string): string {
  return value.replace(/\s/g, "");
}

export function formatDeviceCode(value: string | null | undefined): string {
  if (!value) return "设备码未就绪";
  const code = normalizeDeviceCode(value);
  if (/^\d{10}$/.test(code) || /^\d{9}$/.test(code)) {
    return `${code.slice(0, 3)} ${code.slice(3, 6)} ${code.slice(6)}`;
  }
  return code;
}

/** Older server codes and LAN identities remain usable during the migration. */
export function parseRemoteDeviceInput(value: string): {
  deviceId: string;
  kind: "current" | "legacy";
} | null {
  const code = normalizeDeviceCode(value);
  if (/^\d{9}$/.test(code)) return { deviceId: code, kind: "current" };
  if (/^\d{10}$/.test(code)) return { deviceId: code, kind: "legacy" };
  if (/[A-Za-z]/.test(code) && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,255}$/.test(code)) {
    return { deviceId: code, kind: "legacy" };
  }
  return null;
}

export const DEVICE_CODE_INPUT_ERROR = "请输入 9 位数字设备码，也可使用已有的旧设备码或局域网标识";

export function deviceCodeLabel(value: string | null | undefined): string {
  const code = normalizeDeviceCode(value ?? "");
  if (/^\d{9}$/.test(code)) return "9 位设备码";
  if (/^\d{10}$/.test(code)) return "已有 10 位设备码";
  return "当前设备标识";
}
