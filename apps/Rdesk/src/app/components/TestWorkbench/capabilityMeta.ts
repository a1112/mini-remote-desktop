import type { EnvironmentSnapshot } from "../../adapters/tauri/types";

type CapabilityKey =
  | "available_captures"
  | "available_encoders"
  | "available_decoders"
  | "available_renderers"
  | "available_memory_modes";

const PLATFORM_TAGS: Record<string, string> = {
  dxgi: "Windows",
  winrt: "Windows",
  d3d11: "Windows",
  d3d11_shared: "Windows",
  nvenc_h264: "Windows/NVIDIA",
  nvenc_hevc: "Windows/NVIDIA",
  nvenc_hevc_main10: "Windows/NVIDIA",
  nvenc_av1: "Windows/NVIDIA",
  nvdec: "Windows/NVIDIA",
  macos: "macOS",
  metal: "macOS",
  videotoolbox_h264: "macOS",
  videotoolbox: "macOS",
  synthetic: "通用",
  openh264: "通用",
  software: "通用",
  none: "通用",
  loopback: "通用",
  quic: "通用",
  webrtc: "通用",
  d3d12: "Windows",
  opengl: "Windows",
  webview: "通用",
};

export function capabilityTag(id: string): string {
  return PLATFORM_TAGS[id] ?? "通用";
}

export function capabilityAvailable(
  capabilities: EnvironmentSnapshot | null,
  key: CapabilityKey,
  id: string,
  fallback = false
): boolean {
  const values = capabilities?.[key];
  if (!values) return fallback;
  return values.includes(id);
}

export function unavailableText(
  capabilities: EnvironmentSnapshot | null,
  key: CapabilityKey,
  id: string
): string | null {
  if (capabilityAvailable(capabilities, key, id)) return null;
  if (!capabilities) return "检测中";
  return "当前平台不可用";
}

export function chooseCapability<T extends string>(
  candidates: T[],
  capabilities: EnvironmentSnapshot | null,
  key: CapabilityKey,
  fallback?: T
): T {
  const match = candidates.find((id) => capabilityAvailable(capabilities, key, id));
  if (match) return match;
  if (fallback !== undefined) return fallback;
  return candidates[0] as T;
}
