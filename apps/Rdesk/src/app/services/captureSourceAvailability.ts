import type { CaptureSource } from "./ipcSessionService";

const UNAVAILABLE_CAPTURE_SOURCE_CLASS_NAMES = new Set([
  "screencapturekitdisplayunavailable",
  "screencapturekitwindowunavailable",
]);

export function captureSourceAvailableForAutoSelect(source: CaptureSource): boolean {
  const className = source.class_name.trim().toLowerCase();
  return !UNAVAILABLE_CAPTURE_SOURCE_CLASS_NAMES.has(className);
}
