import { describe, expect, it } from "vitest";
import type { EnvironmentSnapshot } from "../adapters/tauri";
import { buildCapabilitySnapshotFromEnvironment } from "./capabilityMatrix";

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
