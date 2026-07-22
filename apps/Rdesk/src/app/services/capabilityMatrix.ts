import type {
  CapabilitySnapshot as IpcCapabilitySnapshot,
  EnvironmentSnapshot,
  ProbeSnapshot,
} from "../adapters/tauri";

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

export interface CapabilityItem {
  id: string;
  domain: CapabilityDomain;
  label: string;
  status: CapabilityStatus;
  platform: CapabilityPlatform;
  reason?: string;
  detail?: string;
  requires?: string[];
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
  codec: string;
  codec_profile?: string;
  bit_depth?: number;
  chroma_subsampling?: string;
  pixel_format?: string;
  hdr_enabled?: boolean;
  color_mode?: string;
  color_pipeline?: string;
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
  service_version?: string;
  capabilities: CapabilityItem[];
  constraints: CapabilityConstraint[];
  profiles: CapabilityProfile[];
  recent_profile_results: ProfileProbeResult[];
  updated_at_ms?: number;
}

export type CapabilitySourceState = "service" | "legacyFallback" | "unavailable";

export type CapabilityOptionState = "selectable" | "degraded" | "disabled";

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
  "capture.macos": "available",
  "capture.linux": "available",
  "capture.synthetic": "available",
  "encode.nvenc_h264": "available",
  "encode.nvenc_hevc": "available",
  "encode.nvenc_hevc_main10": "available",
  "encode.nvenc_av1": "unimplemented",
  "encode.openh264": "degraded",
  "encode.software_vvc": "unimplemented",
  "encode.vvc_software": "unimplemented",
  "encode.software_h266": "unimplemented",
  "encode.h266_software": "unimplemented",
  "encode.videotoolbox_h264": "available",
  "encode.videotoolbox_hevc": "available",
  "decode.nvdec": "available",
  "decode.nvdec_hevc": "available",
  "decode.nvdec_hevc_main10": "available",
  "decode.software": "degraded",
  "decode.software_vvc": "unimplemented",
  "decode.vvc_software": "unimplemented",
  "decode.software_h266": "unimplemented",
  "decode.h266_software": "unimplemented",
  "decode.ffmpeg_h264": "available",
  "decode.ffmpeg_hevc": "available",
  "decode.ffmpeg_vvc": "available",
  "decode.linux_h264": "available",
  "decode.linux_hevc": "available",
  "decode.linux_hevc_main10": "available",
  "decode.videotoolbox": "available",
  "decode.videotoolbox_h264": "available",
  "decode.videotoolbox_hevc": "available",
  "render.d3d11": "available",
  "render.opengl": "supported",
  "render.linux": "available",
  "render.macos": "available",
  "render.webview": "degraded",
  "memory.cpu": "available",
  "memory.d3d11_shared": "available",
  "service.ffmpeg": "available",
  "media.hevc_main_420_8bit": "supported",
  "media.hevc_main10_420_10bit": "supported",
  "media.color_mode_v1": "supported",
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
    id: "transport.loopback",
    domain: "transport",
    label: "In-process loopback transport",
    status: "available",
    reason: "Local harness transport is available without peer discovery",
  },
  {
    id: "transport.webrtc",
    domain: "transport",
    label: "WebRTC RTP media transport",
    status: "available",
    reason: "Local harness WebRTC RTP path is available",
  },
  {
    id: "transport.quic",
    domain: "transport",
    label: "QUIC media transport",
    status: "available",
    reason: "Local harness QUIC path is available",
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
    id: "render.opengl",
    domain: "render",
    label: "OpenGL renderer",
    status: "unknown",
    reason: "Requires platform renderer probe or service capability snapshot",
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
  {
    id: "media.hevc_main_420_8bit",
    domain: "service",
    label: "HEVC Main 8-bit 4:2:0",
    status: "supported",
    reason: "LAN high-performance HEVC profile metadata; encoder and decoder capabilities still gate runtime use",
  },
  {
    id: "media.hevc_main10_420_10bit",
    domain: "service",
    label: "HEVC Main10 10-bit 4:2:0",
    status: "supported",
    reason:
      "LAN HEVC Main10 profile metadata; NVENC Main10 encode and Main10 decode capabilities still gate runtime use",
  },
  {
    id: "media.color_mode_v1",
    domain: "service",
    label: "GPU color mode transform",
    status: "supported",
    reason:
      "LAN color mode profile metadata and GPU-side transform contract for full, grayscale, monochrome, and low-chroma modes",
  },
];

const BUILTIN_CAPABILITY_CONSTRAINTS: CapabilityConstraint[] = [
  {
    id: "openh264_requires_cpu_input",
    applies_to: ["encode.openh264", "memory.d3d11_shared"],
    status: "requires_copy",
    reason: "OpenH264 requires CPU-backed input unless an explicit copy step is inserted.",
    fallback_ids: ["memory.cpu"],
  },
  {
    id: "d3d12_probe_only",
    applies_to: ["render.d3d12_native"],
    status: "block",
    reason: "D3D12 native renderer is probe-only and not wired as mainline remote display.",
    fallback_ids: ["render.d3d11", "render.webview"],
  },
  {
    id: "opengl_d3d11_shared_interop_hybrid",
    applies_to: ["render.opengl", "memory.d3d11_shared"],
    status: "degrade",
    reason:
      "OpenGL accepts D3D11 shared NV12 through WGL/DX interop when available and readback fallback otherwise; D3D11 native remains preferred for parity.",
    fallback_ids: ["render.d3d11"],
  },
  {
    id: "webview_degraded_render",
    applies_to: ["render.webview"],
    status: "degrade",
    reason: "WebView render is a visual fallback, not native renderer parity.",
  },
];

const SDR8_FULL_420_8 = {
  codec_profile: "main",
  bit_depth: 8,
  chroma_subsampling: "4:2:0",
  pixel_format: "nv12",
  hdr_enabled: false,
  color_mode: "full",
  color_pipeline: "sdr8",
} satisfies Pick<
  CapabilityProfile,
  | "codec_profile"
  | "bit_depth"
  | "chroma_subsampling"
  | "pixel_format"
  | "hdr_enabled"
  | "color_mode"
  | "color_pipeline"
>;

const HDR_MAIN10_420_10 = {
  codec_profile: "main10",
  bit_depth: 10,
  chroma_subsampling: "4:2:0",
  pixel_format: "p010",
  hdr_enabled: true,
  color_mode: "full",
  color_pipeline: "hdr_main10",
} satisfies Pick<
  CapabilityProfile,
  | "codec_profile"
  | "bit_depth"
  | "chroma_subsampling"
  | "pixel_format"
  | "hdr_enabled"
  | "color_mode"
  | "color_pipeline"
>;

export const BUILTIN_CAPABILITY_PROFILES: CapabilityProfile[] = [
  {
    id: "smoke.720p30",
    width: 1280,
    height: 720,
    fps: 30,
    bitrate_mbps: 8,
    codec: "h264",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: ["transport.loopback", "encode.openh264", "decode.software"],
  },
  {
    id: "interactive.1080p60",
    width: 1920,
    height: 1080,
    fps: 60,
    bitrate_mbps: 20,
    codec: "hevc",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "encode.nvenc_hevc",
      "decode.nvdec_hevc",
      "media.hevc_main_420_8bit",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "compat.h264.1080p60",
    width: 1920,
    height: 1080,
    fps: 60,
    bitrate_mbps: 20,
    codec: "h264",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
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
    id: "lan.2k144",
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 64,
    codec: "hevc",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "encode.nvenc_hevc",
      "decode.nvdec_hevc",
      "media.hevc_main_420_8bit",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "lan.2k144.main10",
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 80,
    codec: "hevc",
    ...HDR_MAIN10_420_10,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "encode.nvenc_hevc_main10",
      "decode.nvdec_hevc_main10",
      "media.hevc_main10_420_10bit",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "lan.macos.2k144",
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 80,
    codec: "h264",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "capture.macos",
      "encode.videotoolbox_h264",
      "decode.videotoolbox_h264",
      "memory.cpu",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "lan.macos.hevc.2k144",
    width: 2560,
    height: 1440,
    fps: 144,
    bitrate_mbps: 40,
    codec: "hevc",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "capture.macos",
      "encode.videotoolbox_hevc",
      "decode.videotoolbox_hevc",
      "media.hevc_main_420_8bit",
      "memory.cpu",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "lan.1600p165",
    width: 2560,
    height: 1600,
    fps: 165,
    bitrate_mbps: 80,
    codec: "hevc",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "encode.nvenc_hevc",
      "decode.nvdec_hevc",
      "media.hevc_main_420_8bit",
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
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "encode.nvenc_hevc",
      "decode.nvdec_hevc",
      "media.hevc_main_420_8bit",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ],
  },
  {
    id: "diagnostic.software",
    width: 1280,
    height: 720,
    fps: 30,
    bitrate_mbps: 6,
    codec: "h264",
    ...SDR8_FULL_420_8,
    min_stable_fps_ratio: 0.8,
    max_drop_ratio: 0.02,
    required_capabilities: [
      "capture.synthetic",
      "encode.openh264",
      "decode.software",
      "render.webview",
    ],
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
    constraints: cloneCapabilityConstraints(BUILTIN_CAPABILITY_CONSTRAINTS),
    profiles: BUILTIN_CAPABILITY_PROFILES,
    recent_profile_results: [],
  };
}

export function buildCapabilitySnapshotFromIpc(snapshot: IpcCapabilitySnapshot): CapabilitySnapshot {
  const platform = normalizePlatformValue(snapshot.platform);
  return {
    schema_version: 1,
    platform,
    service_version: snapshot.service_version,
    capabilities: snapshot.capabilities.map((item) => ({
      id: item.id,
      domain: normalizeDomainValue(item.domain),
      label: item.label,
      status: normalizeStatusValue(item.status),
      platform: normalizePlatformValue(item.platform),
      ...(item.reason ? { reason: item.reason } : {}),
      ...(item.detail ? { detail: item.detail } : {}),
      ...(item.requires?.length ? { requires: item.requires } : {}),
      ...(item.conflicts_with?.length ? { conflicts_with: item.conflicts_with } : {}),
      ...(item.depends_on?.length ? { depends_on: item.depends_on } : {}),
      ...(item.fallback_ids?.length ? { fallback_ids: item.fallback_ids } : {}),
      ...(item.last_probe_time_ms ? { last_probe_time_ms: item.last_probe_time_ms } : {}),
    })),
    constraints: snapshot.constraints.map((constraint) => ({
      id: constraint.id,
      applies_to: constraint.applies_to,
      status: normalizeConstraintStatusValue(constraint.status),
      reason: constraint.reason,
      ...(constraint.fallback_ids?.length ? { fallback_ids: constraint.fallback_ids } : {}),
    })),
    profiles:
      snapshot.profiles.length > 0
        ? snapshot.profiles.map((profile) => ({
            id: profile.id,
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            bitrate_mbps: profile.bitrate_mbps,
            codec: profile.codec,
            ...(profile.codec_profile ? { codec_profile: profile.codec_profile } : {}),
            ...(profile.bit_depth ? { bit_depth: profile.bit_depth } : {}),
            ...(profile.chroma_subsampling
              ? { chroma_subsampling: profile.chroma_subsampling }
              : {}),
            ...(profile.pixel_format ? { pixel_format: profile.pixel_format } : {}),
            ...(profile.hdr_enabled !== undefined && profile.hdr_enabled !== null
              ? { hdr_enabled: profile.hdr_enabled }
              : {}),
            ...(profile.color_mode ? { color_mode: profile.color_mode } : {}),
            ...(profile.color_pipeline ? { color_pipeline: profile.color_pipeline } : {}),
            ...(profile.latency_budget_ms ? { latency_budget_ms: profile.latency_budget_ms } : {}),
            ...(profile.min_stable_fps_ratio
              ? { min_stable_fps_ratio: profile.min_stable_fps_ratio }
              : {}),
            ...(profile.max_drop_ratio ? { max_drop_ratio: profile.max_drop_ratio } : {}),
            required_capabilities: profile.required_capabilities,
          }))
        : BUILTIN_CAPABILITY_PROFILES,
    recent_profile_results: [],
    updated_at_ms: snapshot.updated_at_ms,
  };
}

export function evaluateCapabilityCombination(
  request: CapabilityCombinationRequest,
  snapshot: CapabilitySnapshot
): CapabilityEvaluation {
  const reasons: string[] = [];
  const requiredFallbacks: string[] = [];
  let status: CapabilityEvaluation["status"] = "ready";
  const requestedIds = requestedCapabilityIds(request);

  for (const capabilityId of requestedIds) {
    const capability = snapshot.capabilities.find((item) => item.id === capabilityId);
    if (!capability) {
      status = "blocked";
      reasons.push(`Requested capability not exposed: ${capabilityId}`);
      continue;
    }

    if (capability.status === "degraded") {
      if (status !== "blocked") {
        status = "degraded";
      }
      reasons.push(`Requested capability ${capabilityId} is degraded on this platform.`);
      continue;
    }

    if (!isProfileCapabilityUsable(capability)) {
      status = "blocked";
      reasons.push(`Requested capability ${capabilityId} is ${capability.status}.`);
    }
  }

  status = applyCapabilityConstraints(
    snapshot.constraints,
    requestedIds,
    status,
    reasons,
    requiredFallbacks,
    request.allowCpuCopy === true
  );

  if (
    request.encoder === "openh264" &&
    request.memory === "d3d11_shared" &&
    request.allowCpuCopy !== true &&
    !hasApplyingConstraint(snapshot.constraints, requestedIds, "openh264_requires_cpu_input")
  ) {
    status = "blocked";
    reasons.push("OpenH264 requires CPU-backed input; insert a CPU copy step before using it.");
    appendFallbacks(requiredFallbacks, ["memory.cpu"]);
  }

  if (
    request.renderer === "d3d12_native" &&
    !hasApplyingConstraint(snapshot.constraints, requestedIds, "d3d12_probe_only")
  ) {
    status = "blocked";
    reasons.push("D3D12 native renderer is probe-only and is not wired into mainline remote display.");
    appendFallbacks(requiredFallbacks, ["render.d3d11"]);
  }

  if (
    request.renderer === "opengl" &&
    request.memory === "d3d11_shared" &&
    !hasApplyingConstraint(
      snapshot.constraints,
      requestedIds,
      "opengl_d3d11_shared_interop_hybrid"
    )
  ) {
    if (status !== "blocked") {
      status = "degraded";
    }
    reasons.push(
      "OpenGL uses WGL/DX interop when available, with D3D11 readback fallback; use D3D11 native for highest throughput."
    );
  }

  if (
    request.renderer === "webview" &&
    hasCapability(snapshot, "render.webview") &&
    !hasApplyingConstraint(snapshot.constraints, requestedIds, "webview_degraded_render")
  ) {
    if (status !== "blocked") {
      status = "degraded";
    }
    reasons.push("WebView render is a visual fallback, not native renderer parity.");
    appendFallbacks(requiredFallbacks, ["render.d3d11"]);
  }

  return { status, reasons, requiredFallbacks };
}

export function capabilityStatusIsSelectable(status: CapabilityStatus): boolean {
  return status === "available" || status === "supported" || status === "usable";
}

export function capabilityStatusIsVisibleByDefault(status: CapabilityStatus): boolean {
  return capabilityStatusIsSelectable(status) || status === "degraded";
}

export function capabilityIdForLegacyOption(
  dimensionId: string,
  optionId: string
): string | null {
  switch (dimensionId) {
    case "capture":
      return `capture.${optionId}`;
    case "encoder":
      return optionId === "none" ? null : `encode.${optionId}`;
    case "decoder":
      return optionId === "none" ? null : `decode.${optionId}`;
    case "transport":
      return `transport.${optionId}`;
    case "renderer":
      if (optionId === "renderer_none" || optionId === "none") return null;
      return optionId === "d3d12" || optionId === "d3d12_native"
        ? "render.d3d12_native"
        : `render.${optionId}`;
    case "memory":
      return `memory.${optionId}`;
    default:
      return null;
  }
}

export function capabilityForOption(
  snapshot: CapabilitySnapshot | null | undefined,
  dimensionId: string,
  optionId: string
): CapabilityItem | null {
  const capabilityId = capabilityIdForLegacyOption(dimensionId, optionId);
  if (!snapshot || !capabilityId) return null;
  const capabilityIds = capabilityIdsForLegacyOption(dimensionId, optionId);
  const matches = capabilityIds
    .map((id) => snapshot.capabilities.find((item) => item.id === id))
    .filter((item): item is CapabilityItem => item !== undefined);
  return matches.find((item) => capabilityStatusIsSelectable(item.status)) ?? matches[0] ?? null;
}

export function capabilityOptionState(
  snapshot: CapabilitySnapshot | null | undefined,
  dimensionId: string,
  optionId: string
): CapabilityOptionState {
  const capabilityId = capabilityIdForLegacyOption(dimensionId, optionId);
  if (!capabilityId) return "selectable";
  const capability = capabilityForOption(snapshot, dimensionId, optionId);
  if (!capability) return snapshot ? "disabled" : "selectable";
  if (capabilityStatusIsSelectable(capability.status)) return "selectable";
  if (capability.status === "degraded") return "degraded";
  return "disabled";
}

export function shouldShowCapabilityOptionForSnapshot(
  snapshot: CapabilitySnapshot | null | undefined,
  dimensionId: string,
  optionId: string,
  showUnavailable: boolean
): boolean {
  if (!snapshot) return true;
  const capabilityId = capabilityIdForLegacyOption(dimensionId, optionId);
  if (!capabilityId) return true;
  const capability = capabilityForOption(snapshot, dimensionId, optionId);
  if (!capability) return showUnavailable;
  return showUnavailable || capabilityStatusIsVisibleByDefault(capability.status);
}

function capabilityIdsForLegacyOption(dimensionId: string, optionId: string): string[] {
  const capabilityId = capabilityIdForLegacyOption(dimensionId, optionId);
  if (!capabilityId) return [];
  if (dimensionId === "decoder" && optionId === "videotoolbox") {
    return [capabilityId, "decode.videotoolbox_h264", "decode.videotoolbox_hevc"];
  }
  return [capabilityId];
}

export function environmentSnapshotFromCapabilitySnapshot(
  snapshot: CapabilitySnapshot,
  fallback?: EnvironmentSnapshot | null
): EnvironmentSnapshot {
  const valuesByKey: Record<LegacyCapabilityKey, string[]> = {
    available_captures: [],
    available_encoders: [],
    available_decoders: ["none"],
    available_renderers: ["none"],
    available_memory_modes: [],
  };

  for (const capability of snapshot.capabilities) {
    if (!capabilityStatusIsVisibleByDefault(capability.status)) continue;
    const [domain, ...rest] = capability.id.split(".");
    const value = rest.join(".");
    if (!value) continue;
    if (domain === "capture") valuesByKey.available_captures.push(value);
    if (domain === "encode") valuesByKey.available_encoders.push(value);
    if (domain === "decode") valuesByKey.available_decoders.push(value);
    if (domain === "render") {
      valuesByKey.available_renderers.push(value === "d3d12_native" ? "d3d12" : value);
    }
    if (domain === "memory") valuesByKey.available_memory_modes.push(value);
  }

  return {
    os_type: snapshot.platform,
    cpu_brand: fallback?.cpu_brand ?? "mrd-service capability snapshot",
    cpu_cores: fallback?.cpu_cores ?? 0,
    memory_gb: fallback?.memory_gb ?? 0,
    gpu_info: fallback?.gpu_info ?? "Reported by mrd-service",
    available_captures: unique(valuesByKey.available_captures),
    available_encoders: unique(valuesByKey.available_encoders),
    available_decoders: unique(valuesByKey.available_decoders),
    available_renderers: unique(valuesByKey.available_renderers),
    available_memory_modes: unique(valuesByKey.available_memory_modes),
  };
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
  const profile =
    snapshot.profiles.find((candidate) => candidate.id === profileId) ??
    getCapabilityProfile(profileId);
  if (!profile) {
    return {
      status: "blocked",
      reasons: [`Unknown capability profile: ${profileId}`],
      requiredFallbacks: [],
    };
  }

  const reasons: string[] = [];
  const requiredFallbacks: string[] = [];
  let status: CapabilityEvaluation["status"] = "ready";

  for (const capabilityId of profile.required_capabilities) {
    const capability = snapshot.capabilities.find((item) => item.id === capabilityId);
    if (!capability) {
      status = "blocked";
      reasons.push(`Missing required capability: ${capabilityId}`);
      continue;
    }

    if (capability.status === "degraded") {
      if (status !== "blocked") {
        status = "degraded";
      }
      reasons.push(`Required capability ${capabilityId} is degraded on this platform.`);
      continue;
    }

    if (!isProfileCapabilityUsable(capability)) {
      status = "blocked";
      reasons.push(`Missing required capability: ${capabilityId} (${capability.status})`);
    }
  }

  status = applyCapabilityConstraints(
    snapshot.constraints,
    profile.required_capabilities,
    status,
    reasons,
    requiredFallbacks,
    false
  );

  return {
    status,
    reasons,
    requiredFallbacks,
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
    codec: normalizeProbeCodec(probe.media_probe_format),
  };
  const expectedCodec = normalizeProfileCodec(profile.codec);
  const matches =
    actual.width === profile.width &&
    actual.height === profile.height &&
    actual.fps === profile.fps &&
    actual.bitrate_mbps === profile.bitrate_mbps &&
    (actual.codec === undefined || actual.codec === expectedCodec);

  if (!matches) {
    return {
      profile_id: profile.id,
      status: "failed",
      error: `Runtime media profile mismatch: expected ${formatProfile(profile)}, got ${formatActualProfile(actual)}`,
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

function unique(values: string[]): string[] {
  return values.filter((value, index, all) => all.indexOf(value) === index);
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
  return item.status === "supported" || item.status === "available" || item.status === "usable";
}

function formatProfile(profile: CapabilityProfile): string {
  const codecParts = [normalizeProfileCodec(profile.codec), profile.codec_profile]
    .filter(Boolean)
    .join("/");
  const metadataParts = [
    profile.bit_depth ? `${profile.bit_depth}-bit` : null,
    profile.chroma_subsampling,
    profile.pixel_format,
    profile.hdr_enabled === true ? "HDR" : null,
    profile.color_mode ? `color=${profile.color_mode}` : null,
    profile.color_pipeline ? `pipeline=${profile.color_pipeline}` : null,
  ].filter(Boolean);
  const metadata = metadataParts.length > 0 ? ` / ${metadataParts.join(" / ")}` : "";
  return `${profile.width}x${profile.height} @ ${profile.fps} FPS / ${
    profile.bitrate_mbps
  } Mbps / ${codecParts}${metadata}`;
}

function formatActualProfile(profile: {
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
  codec?: CapabilityProfile["codec"];
}): string {
  const codec = profile.codec ? ` / ${profile.codec}` : "";
  return `${profile.width}x${profile.height} @ ${profile.fps} FPS / ${
    profile.bitrate_mbps
  } Mbps${codec}`;
}

function normalizeProfileCodec(codec: string): CapabilityProfile["codec"] {
  const normalized = normalizeCodecKey(codec);
  if (normalized === "hevc" || normalized === "h265") return "hevc";
  if (normalized === "av1") return "av1";
  return "h264";
}

function normalizeProbeCodec(
  format: string | null | undefined
): CapabilityProfile["codec"] | undefined {
  const normalized = normalizeCodecKey(format ?? "");
  if (!normalized) return undefined;
  if (normalized.includes("hevc") || normalized.includes("h265")) return "hevc";
  if (normalized.includes("av1")) return "av1";
  if (normalized.includes("h264")) return "h264";
  return undefined;
}

function normalizeCodecKey(value: string): string {
  return value.trim().toLowerCase().replace(/\./g, "");
}

function requestedCapabilityIds(request: CapabilityCombinationRequest): string[] {
  const ids: string[] = [];

  if (request.capture) ids.push(`capture.${request.capture}`);
  if (request.encoder) ids.push(`encode.${request.encoder}`);
  const decoderCapabilityId = requestedDecoderCapabilityId(request);
  if (decoderCapabilityId) ids.push(decoderCapabilityId);
  if (request.renderer && request.renderer !== "none") {
    ids.push(
      request.renderer === "d3d12_native" ? "render.d3d12_native" : `render.${request.renderer}`
    );
  }
  if (request.memory) ids.push(`memory.${request.memory}`);
  if (request.transport) ids.push(`transport.${request.transport}`);

  return ids;
}

function requestedDecoderCapabilityId(request: CapabilityCombinationRequest): string | null {
  if (!request.decoder || request.decoder === "none") return null;
  if (request.decoder !== "videotoolbox") return `decode.${request.decoder}`;
  if (request.encoder === "videotoolbox_hevc" || request.encoder === "hevc") {
    return "decode.videotoolbox_hevc";
  }
  if (
    request.encoder === "videotoolbox_h264" ||
    request.encoder === "openh264" ||
    request.encoder === "h264"
  ) {
    return "decode.videotoolbox_h264";
  }
  return "decode.videotoolbox";
}

function applyCapabilityConstraints(
  constraints: CapabilityConstraint[],
  requestedIds: string[],
  status: CapabilityEvaluation["status"],
  reasons: string[],
  requiredFallbacks: string[],
  allowCopy: boolean
): CapabilityEvaluation["status"] {
  let currentStatus = status;
  for (const constraint of constraints) {
    if (!constraintAppliesToIds(constraint, requestedIds)) continue;
    appendFallbacks(requiredFallbacks, constraint.fallback_ids);

    if (constraint.status === "allow") continue;

    if (constraint.status === "degrade") {
      if (currentStatus !== "blocked" && currentStatus !== "skipped") {
        currentStatus = "degraded";
      }
      reasons.push(constraint.reason);
      continue;
    }

    if (constraint.status === "requires_probe") {
      if (currentStatus !== "blocked") {
        currentStatus = "skipped";
      }
      reasons.push(constraint.reason);
      continue;
    }

    if (constraint.status === "requires_copy") {
      if (allowCopy) {
        if (currentStatus !== "blocked" && currentStatus !== "skipped") {
          currentStatus = "degraded";
        }
      } else {
        currentStatus = "blocked";
      }
      reasons.push(constraint.reason);
      continue;
    }

    currentStatus = "blocked";
    reasons.push(constraint.reason);
  }
  return currentStatus;
}

function constraintAppliesToIds(constraint: CapabilityConstraint, requestedIds: string[]): boolean {
  return constraint.applies_to.every((target) =>
    requestedIds.some((id) => capabilityIdMatchesTarget(id, target))
  );
}

function hasApplyingConstraint(
  constraints: CapabilityConstraint[],
  requestedIds: string[],
  constraintId: string
): boolean {
  return constraints.some(
    (constraint) =>
      constraint.id === constraintId && constraintAppliesToIds(constraint, requestedIds)
  );
}

function capabilityIdMatchesTarget(id: string, target: string): boolean {
  return id === target || id.startsWith(target.endsWith(".") ? target : `${target}.`);
}

function appendFallbacks(requiredFallbacks: string[], fallbackIds: string[] | undefined): void {
  for (const fallbackId of fallbackIds ?? []) {
    if (!requiredFallbacks.includes(fallbackId)) {
      requiredFallbacks.push(fallbackId);
    }
  }
}

function cloneCapabilityConstraints(constraints: CapabilityConstraint[]): CapabilityConstraint[] {
  return constraints.map((constraint) => ({
    ...constraint,
    applies_to: [...constraint.applies_to],
    ...(constraint.fallback_ids ? { fallback_ids: [...constraint.fallback_ids] } : {}),
  }));
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

function normalizePlatformValue(value: string | undefined): CapabilityPlatform {
  return normalizePlatform(value);
}

function normalizeDomainValue(value: string): CapabilityDomain {
  return isCapabilityDomain(value) ? value : "service";
}

function normalizeStatusValue(value: string): CapabilityStatus {
  return isCapabilityStatus(value) ? value : "unknown";
}

function normalizeConstraintStatusValue(value: string): CapabilityConstraint["status"] {
  return isCapabilityConstraintStatus(value) ? value : "requires_probe";
}

function isCapabilityDomain(value: string): value is CapabilityDomain {
  return [
    "capture",
    "capture_source",
    "encode",
    "decode",
    "render",
    "memory",
    "transport",
    "control",
    "audio",
    "service",
    "security",
  ].includes(value);
}

function isCapabilityStatus(value: string): value is CapabilityStatus {
  return [
    "supported",
    "available",
    "usable",
    "degraded",
    "permission_missing",
    "driver_missing",
    "hardware_missing",
    "unimplemented",
    "unsupported",
    "unknown",
  ].includes(value);
}

function isCapabilityConstraintStatus(value: string): value is CapabilityConstraint["status"] {
  return ["allow", "block", "degrade", "requires_copy", "requires_probe"].includes(value);
}
