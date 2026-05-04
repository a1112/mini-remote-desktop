import type { EnvironmentSnapshot } from "../adapters/tauri";

export type CapabilityStatus =
  | "supported"
  | "available"
  | "usable"
  | "degraded"
  | "permission_missing"
  | "driver_missing"
  | "hardware_missing"
  | "unimplemented"
  | "unsupported"
  | "unknown";

export type CapabilityDomain =
  | "capture"
  | "capture_source"
  | "encode"
  | "decode"
  | "render"
  | "memory"
  | "transport"
  | "control"
  | "audio"
  | "service"
  | "security";

export type CapabilityPlatform =
  | "windows"
  | "macos"
  | "linux"
  | "android"
  | "ios"
  | "web"
  | "unknown";

export interface CapabilityRequirement {
  id: string;
  reason?: string;
}

export interface CapabilityItem {
  id: string;
  domain: CapabilityDomain;
  label: string;
  status: CapabilityStatus;
  platform: CapabilityPlatform;
  reason?: string;
  detail?: string;
  requires?: CapabilityRequirement[];
  conflicts_with?: string[];
  depends_on?: string[];
  fallback_ids?: string[];
  last_probe_time_ms?: number;
}

export interface CapabilityConstraint {
  id: string;
  applies_to: string[];
  status: "allow" | "block" | "degrade" | "requires_copy" | "requires_probe";
  reason: string;
  fallback_ids?: string[];
}

export interface CapabilityProfile {
  id: string;
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
  codec: "h264" | "hevc" | "av1";
  latency_budget_ms?: number;
  min_stable_fps_ratio?: number;
  max_drop_ratio?: number;
  required_capabilities: string[];
}

export interface ProfileProbeResult {
  profile_id: string;
  status: "passed" | "failed" | "degraded" | "skipped";
  first_frame_ms?: number;
  stable_fps?: number;
  perceived_latency_p95_ms?: number;
  source_wait_p95_ms?: number;
  encode_p95_ms?: number;
  decode_p95_ms?: number;
  render_p95_ms?: number;
  drop_ratio?: number;
  error?: string;
}

export interface CapabilitySnapshot {
  schema_version: 1;
  platform: CapabilityPlatform;
  capabilities: CapabilityItem[];
  constraints: CapabilityConstraint[];
  profiles: CapabilityProfile[];
  recent_profile_results: ProfileProbeResult[];
}

type LegacyCapabilityKey =
  | "available_captures"
  | "available_encoders"
  | "available_decoders"
  | "available_renderers"
  | "available_memory_modes";

const LEGACY_DOMAIN_BY_KEY: Record<LegacyCapabilityKey, CapabilityDomain> = {
  available_captures: "capture",
  available_encoders: "encode",
  available_decoders: "decode",
  available_renderers: "render",
  available_memory_modes: "memory",
};

const KNOWN_STATUS_BY_ID: Record<string, CapabilityStatus> = {
  "capture.dxgi": "available",
  "capture.winrt": "available",
  "capture.synthetic": "available",
  "encode.nvenc_h264": "available",
  "encode.nvenc_hevc": "available",
  "encode.nvenc_hevc_main10": "available",
  "encode.nvenc_av1": "available",
  "encode.openh264": "degraded",
  "decode.nvdec": "available",
  "decode.software": "degraded",
  "decode.videotoolbox": "available",
  "render.d3d11": "available",
  "render.macos": "available",
  "render.webview": "degraded",
  "memory.cpu": "available",
  "memory.d3d11_shared": "available",
};

const DOMAIN_BASELINE_ITEMS: Array<Omit<CapabilityItem, "platform">> = [
  {
    id: "capture_source.display_shared",
    domain: "capture_source",
    label: "Shared display capture source",
    status: "unknown",
    reason: "Requires remote source enumeration",
  },
  {
    id: "transport.quic_datagram",
    domain: "transport",
    label: "QUIC datagram media transport",
    status: "unknown",
    reason: "Requires service or peer probe",
  },
  {
    id: "control.keyboard_mouse",
    domain: "control",
    label: "Keyboard and mouse input",
    status: "unknown",
    reason: "Requires platform input injection probe",
  },
  {
    id: "audio.loopback",
    domain: "audio",
    label: "System audio capture",
    status: "unknown",
    reason: "Not probed by legacy environment snapshot",
  },
  {
    id: "service.tray",
    domain: "service",
    label: "Service-owned tray lifecycle",
    status: "unknown",
    reason: "Requires mrd-service shell probe",
  },
  {
    id: "security.pairing",
    domain: "security",
    label: "Pairing and consent",
    status: "unknown",
    reason: "Requires session policy snapshot",
  },
];

export function buildCapabilitySnapshotFromEnvironment(
  environment: EnvironmentSnapshot
): CapabilitySnapshot {
  const platform = normalizePlatform(environment.os_type);
  return {
    schema_version: 1,
    platform,
    capabilities: [
      ...buildLegacyCapabilities(environment, platform),
      ...DOMAIN_BASELINE_ITEMS.map((item) => ({ ...item, platform })),
    ],
    constraints: [],
    profiles: [],
    recent_profile_results: [],
  };
}

function buildLegacyCapabilities(
  environment: EnvironmentSnapshot,
  platform: CapabilityPlatform
): CapabilityItem[] {
  const items: CapabilityItem[] = [];
  addLegacyItems(items, environment.available_captures ?? [], "available_captures", platform);
  addLegacyItems(items, environment.available_encoders, "available_encoders", platform);
  addLegacyItems(items, environment.available_decoders, "available_decoders", platform);
  addLegacyItems(items, environment.available_renderers ?? [], "available_renderers", platform);
  addLegacyItems(items, environment.available_memory_modes ?? [], "available_memory_modes", platform);
  return items;
}

function addLegacyItems(
  items: CapabilityItem[],
  values: string[],
  key: LegacyCapabilityKey,
  platform: CapabilityPlatform
): void {
  const domain = LEGACY_DOMAIN_BY_KEY[key];
  for (const value of values) {
    const id = `${domain}.${value}`;
    const knownStatus = KNOWN_STATUS_BY_ID[id];
    items.push({
      id,
      domain,
      label: value,
      status: knownStatus ?? "unknown",
      platform,
      ...(knownStatus ? {} : { reason: "Unknown legacy capability" }),
    });
  }
}

function normalizePlatform(osType: string | undefined): CapabilityPlatform {
  const normalized = osType?.toLowerCase() ?? "";
  if (normalized.includes("windows")) return "windows";
  if (normalized.includes("mac") || normalized.includes("darwin")) return "macos";
  if (normalized.includes("linux")) return "linux";
  if (normalized.includes("android")) return "android";
  if (normalized.includes("ios")) return "ios";
  if (normalized.includes("web")) return "web";
  return "unknown";
}
