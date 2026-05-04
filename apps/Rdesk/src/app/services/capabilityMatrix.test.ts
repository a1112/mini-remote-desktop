import { describe, expect, it } from "vitest";
import type { EnvironmentSnapshot } from "../adapters/tauri";
import {
  buildCapabilitySnapshotFromEnvironment,
  evaluateCapabilityCombination,
  pickPreferredCaptureSourceKind,
  type CapabilityItem,
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
