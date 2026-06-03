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
  nvenc_h264: "NVIDIA",
  nvenc_hevc: "NVIDIA",
  nvenc_hevc_main10: "NVIDIA",
  nvenc_av1: "NVIDIA",
  software_vvc: "VVenC",
  nvdec: "NVIDIA",
  linux_h264: "Linux",
  linux_hevc: "Linux",
  linux_hevc_main10: "Linux",
  macos: "macOS",
  metal: "macOS",
  videotoolbox_h264: "macOS",
  videotoolbox_hevc: "macOS",
  videotoolbox: "macOS",
  linux: "Linux",
  pipewire: "Linux",
  x11: "Linux",
  synthetic: "通用",
  openh264: "通用",
  software: "通用",
  none: "通用",
  loopback: "通用",
  quic: "通用",
  webrtc: "通用",
  d3d12: "Windows",
  opengl: "通用",
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
  return capabilityAliases(key, id).some((alias) => values.includes(alias));
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

export function chooseDecoderCapabilityForConfig<T extends string>(
  candidates: T[],
  capabilities: EnvironmentSnapshot | null,
  encoderId?: string | null,
  fallback?: T
): T {
  const match = candidates.find((id) =>
    decoderCapabilityAvailableForConfig(capabilities, id, encoderId)
  );
  if (match) return match;
  if (fallback !== undefined) return fallback;
  return candidates[0] as T;
}

export function decoderCapabilityAvailableForConfig(
  capabilities: EnvironmentSnapshot | null,
  decoderId: string | null | undefined,
  encoderId?: string | null,
  fallback = false
): boolean {
  if (!decoderId || decoderId === "none") return true;
  if (decoderId !== "videotoolbox") {
    return capabilityAvailable(capabilities, "available_decoders", decoderId, fallback);
  }
  return videoToolboxDecoderAvailableForEncoder(capabilities, encoderId, fallback);
}

export function videoToolboxDecoderAvailableForEncoder(
  capabilities: EnvironmentSnapshot | null,
  encoderId?: string | null,
  fallback = false
): boolean {
  if (encoderId === "videotoolbox_hevc" || encoderId === "hevc") {
    return capabilityAvailable(capabilities, "available_decoders", "videotoolbox_hevc", fallback);
  }
  return capabilityAvailable(capabilities, "available_decoders", "videotoolbox_h264", fallback);
}

function capabilityAliases(key: CapabilityKey, id: string): string[] {
  if (key === "available_decoders" && id === "videotoolbox") {
    return [
      "videotoolbox",
      "videotoolbox_h264",
      "videotoolbox_hevc",
      "decode.videotoolbox_h264",
      "decode.videotoolbox_hevc",
    ];
  }
  if (key === "available_decoders" && id === "videotoolbox_h264") {
    return [
      "videotoolbox_h264",
      "decode.videotoolbox_h264",
      "videotoolbox",
      "decode.videotoolbox",
    ];
  }
  if (key === "available_decoders" && id === "videotoolbox_hevc") {
    return [
      "videotoolbox_hevc",
      "decode.videotoolbox_hevc",
      "videotoolbox",
      "decode.videotoolbox",
    ];
  }
  return [id];
}
