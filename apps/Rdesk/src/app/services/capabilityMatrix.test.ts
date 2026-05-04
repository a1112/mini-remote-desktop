import { describe, expect, it } from "vitest";
import type { EnvironmentSnapshot, ProbeSnapshot } from "../adapters/tauri";
import {
  buildCapabilitySnapshotFromEnvironment,
  evaluateCapabilityCombination,
  evaluateProfileProbe,
  evaluateProfileSupport,
  getCapabilityProfile,
  pickPreferredCaptureSourceKind,
  type CapabilityItem,
  type CapabilitySnapshot,
} from "./capabilityMatrix";

const windowsEnvironment: EnvironmentSnapshot = {
  os_type: "windows",
  cpu_brand: "Intel",
  cpu_cores: 16,
  memory_gb: 32,
  gpu_info: "NVIDIA RTX",
  available_captures: ["dxgi", "winrt", "synthetic"],
  available_encoders: ["nvenc_h264", "openh264"],
  available_decoders: ["nvdec", "software"],
  available_renderers: ["d3d11", "webview"],
  available_memory_modes: ["cpu", "d3d11_shared"],
};

function statusOf(snapshot: ReturnType<typeof buildCapabilitySnapshotFromEnvironment>, id: string) {
  return snapshot.capabilities.find((capability) => capability.id === id)?.status;
}

describe("buildCapabilitySnapshotFromEnvironment", () => {
  it("converts legacy environment arrays into structured capability items", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    expect(snapshot.schema_version).toBe(1);
    expect(snapshot.platform).toBe("windows");
    expect(statusOf(snapshot, "capture.dxgi")).toBe("available");
    expect(statusOf(snapshot, "encode.nvenc_h264")).toBe("available");
    expect(statusOf(snapshot, "decode.nvdec")).toBe("available");
    expect(statusOf(snapshot, "render.d3d11")).toBe("available");
    expect(statusOf(snapshot, "memory.d3d11_shared")).toBe("available");
  });

  it("includes all product capability domains needed by the matrix", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);
    const domains = new Set(snapshot.capabilities.map((capability) => capability.domain));

    expect(domains).toEqual(
      new Set([
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
      ])
    );
  });

  it("marks known fallback capabilities as degraded when they are usable but not preferred", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    expect(statusOf(snapshot, "encode.openh264")).toBe("degraded");
    expect(statusOf(snapshot, "decode.software")).toBe("degraded");
    expect(statusOf(snapshot, "render.webview")).toBe("degraded");
  });

  it("preserves unknown legacy values instead of dropping them", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...windowsEnvironment,
      available_captures: ["dxgi", "experimental_capture"],
    });

    const unknown = snapshot.capabilities.find(
      (capability) => capability.id === "capture.experimental_capture"
    );
    expect(unknown).toMatchObject({
      domain: "capture",
      status: "unknown",
      reason: "Unknown legacy capability",
    });
  });
});

describe("evaluateCapabilityCombination", () => {
  it("blocks OpenH264 with D3D11 shared memory unless a CPU copy step is declared", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateCapabilityCombination(
      {
        encoder: "openh264",
        memory: "d3d11_shared",
      },
      snapshot
    );

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("CPU-backed input");
  });

  it("blocks D3D12 native as a mainline remote display renderer until it is wired", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateCapabilityCombination(
      {
        renderer: "d3d12_native",
      },
      snapshot
    );

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("D3D12 native renderer is probe-only");
  });

  it("marks WebView rendering as degraded instead of native renderer parity", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateCapabilityCombination(
      {
        renderer: "webview",
      },
      snapshot
    );

    expect(result.status).toBe("degraded");
    expect(result.reasons.join(" ")).toContain("WebView render is a visual fallback");
  });

  it("prefers shared display capture source over copy display and window", () => {
    const sources: CapabilityItem[] = [
      {
        id: "capture_source.window",
        domain: "capture_source",
        label: "Window",
        status: "available",
        platform: "windows",
      },
      {
        id: "capture_source.display",
        domain: "capture_source",
        label: "Display copy",
        status: "available",
        platform: "windows",
      },
      {
        id: "capture_source.display_shared",
        domain: "capture_source",
        label: "Display shared",
        status: "available",
        platform: "windows",
      },
    ];

    expect(pickPreferredCaptureSourceKind(sources)).toBe("display_shared");
  });
});

describe("capability profiles", () => {
  it("exposes a LAN 2K144 profile with required media capabilities", () => {
    const profile = getCapabilityProfile("lan.2k144");

    expect(profile).toMatchObject({
      id: "lan.2k144",
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 64,
      codec: "h264",
    });
    expect(profile?.required_capabilities).toContain("transport.quic_datagram");
    expect(profile?.required_capabilities).toContain("transport.media_profile_control_v1");
  });

  it("marks a profile ready only when all required capabilities are available", () => {
    const snapshot = withAvailableCapabilities(
      buildCapabilitySnapshotFromEnvironment(windowsEnvironment),
      ["transport.quic_datagram", "transport.media_profile_control_v1"]
    );

    const result = evaluateProfileSupport("lan.2k144", snapshot);

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("blocks a profile when a required capability is missing or not usable", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateProfileSupport("lan.2k144", snapshot);

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("transport.media_profile_control_v1");
  });

  it("fails runtime profile probe when negotiated media does not match the requested profile", () => {
    const profile = getCapabilityProfile("lan.2k144");
    const probe: ProbeSnapshot = {
      session_id: "session-1",
      frames_received: 30,
      frames_decoded: 30,
      frames_dropped: 0,
      current_fps: 144,
      bitrate_mbps: 64,
      media_probe_valid: true,
      media_probe_format: "compressed_test_pattern",
      media_probe_width: 1920,
      media_probe_height: 1080,
      media_probe_target_fps: 60,
      media_probe_target_bitrate_mbps: 20,
      media_probe_payload_bytes: 1000,
      last_error: null,
    };

    const result = evaluateProfileProbe(profile!, probe);

    expect(result.status).toBe("failed");
    expect(result.error).toContain("expected 2560x1440 @ 144 FPS / 64 Mbps");
  });
});

function withAvailableCapabilities(
  snapshot: CapabilitySnapshot,
  ids: string[]
): CapabilitySnapshot {
  const capabilities = snapshot.capabilities.map((capability) =>
    ids.includes(capability.id)
      ? { ...capability, status: "available" as const, reason: undefined }
      : capability
  );

  for (const id of ids) {
    if (capabilities.some((capability) => capability.id === id)) continue;
    const [domain] = id.split(".");
    capabilities.push({
      id,
      domain: domain as CapabilityItem["domain"],
      label: id,
      status: "available",
      platform: snapshot.platform,
    });
  }

  return { ...snapshot, capabilities };
}
