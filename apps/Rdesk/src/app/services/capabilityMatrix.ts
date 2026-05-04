import type { EnvironmentSnapshot, ProbeSnapshot } from "../adapters/tauri";

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

export interface CapabilityCombinationRequest {
  capture?: string;
  captureSourceKind?: string;
  encoder?: string;
  decoder?: string;
  renderer?: string;
  memory?: string;
  transport?: string;
  allowCpuCopy?: boolean;
}

export interface CapabilityEvaluation {
  status: "ready" | "blocked" | "degraded" | "skipped";
  reasons: string[];
  requiredFallbacks: string[];
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
    id: "transport.media_profile_control_v1",
    domain: "transport",
    label: "Remote media profile control",
    status: "unknown",
    reason: "Requires service or peer probe",
  },
  {
    id: "render.d3d12_native",
    domain: "render",
    label: "D3D12 native renderer",
    status: "unimplemented",
    reason: "D3D12 is currently probe-only and not wired into mainline remote display",
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

export const BUILTIN_CAPABILITY_PROFILES: CapabilityProfile[] = [
  {
    id: "smoke.720p30",
    width: 1280,
    height: 720,
    fps: 30,
    bitrate_mbps: 8,
    codec: "h264",
    min_stable_fps_ratio: 0.4,
    required_capabilities: ["encode.openh264", "decode.software", "transport.quic_datagram"],
  },
  {
    id: "interactive.1080p60",
    width: 1920,
    height: 1080,
    fps: 60,
    bitrate_mbps: 20,
    codec: "h264",
    latency_budget_ms: 50,
    min_stable_fps_ratio: 0.6,
    required_capabilities: ["encode.nvenc_h264", "decode.nvdec", "render.d3d11"],
  },
  {
    id: "lan.2k144",
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 64,
    codec: "h264",
    latency_budget_ms: 35,
    min_stable_fps_ratio: 0.4,
    max_drop_ratio: 0.05,
    required_capabilities: [
      "encode.nvenc_h264",
      "decode.nvdec",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "quality.4k60",
    width: 3840,
    height: 2160,
    fps: 60,
    bitrate_mbps: 80,
    codec: "hevc",
    latency_budget_ms: 50,
    required_capabilities: ["encode.nvenc_hevc", "decode.nvdec", "render.d3d11"],
  },
  {
    id: "diagnostic.software",
    width: 1280,
    height: 720,
    fps: 30,
    bitrate_mbps: 6,
    codec: "h264",
    required_capabilities: ["encode.openh264", "decode.software", "render.webview"],
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
    profiles: BUILTIN_CAPABILITY_PROFILES,
    recent_profile_results: [],
  };
}

export function evaluateCapabilityCombination(
  request: CapabilityCombinationRequest,
  snapshot: CapabilitySnapshot
): CapabilityEvaluation {
  const reasons: string[] = [];
  const requiredFallbacks: string[] = [];
  let status: CapabilityEvaluation["status"] = "ready";

  if (
    request.encoder === "openh264" &&
    request.memory === "d3d11_shared" &&
    request.allowCpuCopy !== true
  ) {
    status = "blocked";
    reasons.push("OpenH264 requires CPU-backed input; insert a CPU copy step before using it.");
    requiredFallbacks.push("memory.cpu");
  }

  if (request.renderer === "d3d12_native") {
    status = "blocked";
    reasons.push("D3D12 native renderer is probe-only and is not wired into mainline remote display.");
    requiredFallbacks.push("render.d3d11");
  }

  if (request.renderer === "webview" && hasCapability(snapshot, "render.webview")) {
    if (status !== "blocked") {
      status = "degraded";
    }
    reasons.push("WebView render is a visual fallback, not native renderer parity.");
    requiredFallbacks.push("render.d3d11");
  }

  return { status, reasons, requiredFallbacks };
}

export function pickPreferredCaptureSourceKind(items: CapabilityItem[]): string | undefined {
  const candidates = items.filter(
    (item) => item.domain === "capture_source" && isSelectableCapability(item)
  );
  return (
    findCaptureSourceKind(candidates, "display_shared") ??
    findCaptureSourceKind(candidates, "display") ??
    findCaptureSourceKind(candidates, "window")
  );
}

export function getCapabilityProfile(profileId: string): CapabilityProfile | undefined {
  return BUILTIN_CAPABILITY_PROFILES.find((profile) => profile.id === profileId);
}

export function evaluateProfileSupport(
  profileId: string,
  snapshot: CapabilitySnapshot
): CapabilityEvaluation {
  const profile = getCapabilityProfile(profileId);
  if (!profile) {
    return {
      status: "blocked",
      reasons: [`Unknown capability profile: ${profileId}`],
      requiredFallbacks: [],
    };
  }

  const reasons = profile.required_capabilities.flatMap((capabilityId) => {
    const capability = snapshot.capabilities.find((item) => item.id === capabilityId);
    if (capability && isProfileCapabilityUsable(capability)) return [];
    const suffix = capability?.status ? ` (${capability.status})` : "";
    return [`Missing required capability: ${capabilityId}${suffix}`];
  });

  return {
    status: reasons.length > 0 ? "blocked" : "ready",
    reasons,
    requiredFallbacks: [],
  };
}

export function evaluateProfileProbe(
  profile: CapabilityProfile,
  probe: ProbeSnapshot
): ProfileProbeResult {
  if (probe.media_probe_valid !== true) {
    return {
      profile_id: profile.id,
      status: "failed",
      error: "Runtime media probe is not valid",
    };
  }

  const actual = {
    width: probe.media_probe_width ?? 0,
    height: probe.media_probe_height ?? 0,
    fps: probe.media_probe_target_fps ?? 0,
    bitrate_mbps: probe.media_probe_target_bitrate_mbps ?? 0,
  };
  const matches =
    actual.width === profile.width &&
    actual.height === profile.height &&
    actual.fps === profile.fps &&
    actual.bitrate_mbps === profile.bitrate_mbps;

  if (!matches) {
    return {
      profile_id: profile.id,
      status: "failed",
      error: `Runtime media profile mismatch: expected ${formatProfile(profile)}, got ${actual.width}x${actual.height} @ ${actual.fps} FPS / ${actual.bitrate_mbps} Mbps`,
    };
  }

  return {
    profile_id: profile.id,
    status: "passed",
    stable_fps: probe.current_fps ?? undefined,
    drop_ratio:
      probe.frames_received > 0 ? probe.frames_dropped / probe.frames_received : undefined,
  };
}

function hasCapability(snapshot: CapabilitySnapshot, id: string): boolean {
  return snapshot.capabilities.some((capability) => capability.id === id);
}

function findCaptureSourceKind(items: CapabilityItem[], kind: string): string | undefined {
  return items.some((item) => item.id === `capture_source.${kind}`) ? kind : undefined;
}

function isSelectableCapability(item: CapabilityItem): boolean {
  return ![
    "permission_missing",
    "driver_missing",
    "hardware_missing",
    "unimplemented",
    "unsupported",
  ].includes(item.status);
}

function isProfileCapabilityUsable(item: CapabilityItem): boolean {
  return item.status === "available" || item.status === "usable";
}

function formatProfile(profile: CapabilityProfile): string {
  return `${profile.width}x${profile.height} @ ${profile.fps} FPS / ${profile.bitrate_mbps} Mbps`;
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
