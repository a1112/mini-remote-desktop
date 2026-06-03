import { describe, expect, it } from "vitest";
import type {
  CapabilitySnapshot as IpcCapabilitySnapshot,
  EnvironmentSnapshot,
  ProbeSnapshot,
} from "../adapters/tauri";
import {
  buildCapabilitySnapshotFromIpc,
  buildCapabilitySnapshotFromEnvironment,
  evaluateCapabilityCombination,
  evaluateProfileProbe,
  evaluateProfileSupport,
  capabilityIdForLegacyOption,
  capabilityOptionState,
  environmentSnapshotFromCapabilitySnapshot,
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

const linuxEnvironment: EnvironmentSnapshot = {
  os_type: "linux",
  cpu_brand: "AMD",
  cpu_cores: 12,
  memory_gb: 32,
  gpu_info: "Mesa",
  available_captures: ["linux", "synthetic"],
  available_encoders: ["openh264"],
  available_decoders: ["software"],
  available_renderers: ["linux", "webview"],
  available_memory_modes: ["cpu"],
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
    expect(statusOf(snapshot, "capture.linux")).toBeUndefined();
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

  it("classifies optional FFmpeg legacy decoder capabilities as available", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...windowsEnvironment,
      available_decoders: ["ffmpeg_h264", "ffmpeg_hevc", "ffmpeg_vvc"],
    });

    expect(statusOf(snapshot, "decode.ffmpeg_h264")).toBe("available");
    expect(statusOf(snapshot, "decode.ffmpeg_hevc")).toBe("available");
    expect(statusOf(snapshot, "decode.ffmpeg_vvc")).toBe("available");
  });

  it("classifies Linux legacy capabilities as available on Linux", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(linuxEnvironment);

    expect(snapshot.platform).toBe("linux");
    expect(statusOf(snapshot, "capture.linux")).toBe("available");
    expect(statusOf(snapshot, "render.linux")).toBe("available");
  });

  it("marks wired VideoToolbox decode as available from legacy fallback", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...linuxEnvironment,
      os_type: "macos",
      available_captures: ["macos"],
      available_encoders: ["videotoolbox_h264"],
      available_decoders: ["videotoolbox"],
      available_renderers: ["macos"],
    });

    expect(statusOf(snapshot, "decode.videotoolbox")).toBe("available");
  });

  it("marks codec-specific VideoToolbox decode as available from legacy fallback", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...linuxEnvironment,
      os_type: "macos",
      available_captures: ["macos"],
      available_encoders: ["videotoolbox_h264", "videotoolbox_hevc"],
      available_decoders: ["videotoolbox_h264", "videotoolbox_hevc"],
      available_renderers: ["macos"],
    });

    expect(statusOf(snapshot, "decode.videotoolbox_h264")).toBe("available");
    expect(statusOf(snapshot, "decode.videotoolbox_hevc")).toBe("available");
    expect(capabilityOptionState(snapshot, "decoder", "videotoolbox")).toBe("selectable");
  });

  it("does not mark unwired NVENC AV1 encode as available from legacy fallback", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...windowsEnvironment,
      available_encoders: ["nvenc_av1"],
    });

    expect(statusOf(snapshot, "encode.nvenc_av1")).toBe("unimplemented");
  });

  it("does not expose unwired H.266/VVC software codec paths as selectable legacy capabilities", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...windowsEnvironment,
      available_encoders: ["software_vvc", "vvc_software", "software_h266", "h266_software"],
      available_decoders: ["software_vvc", "vvc_software", "software_h266", "h266_software"],
    });

    expect(statusOf(snapshot, "encode.software_vvc")).toBe("unimplemented");
    expect(statusOf(snapshot, "encode.vvc_software")).toBe("unimplemented");
    expect(statusOf(snapshot, "encode.software_h266")).toBe("unimplemented");
    expect(statusOf(snapshot, "encode.h266_software")).toBe("unimplemented");
    expect(statusOf(snapshot, "decode.software_vvc")).toBe("unimplemented");
    expect(statusOf(snapshot, "decode.vvc_software")).toBe("unimplemented");
    expect(statusOf(snapshot, "decode.software_h266")).toBe("unimplemented");
    expect(statusOf(snapshot, "decode.h266_software")).toBe("unimplemented");
    expect(capabilityOptionState(snapshot, "encoder", "software_vvc")).toBe("disabled");
    expect(capabilityOptionState(snapshot, "decoder", "software_vvc")).toBe("disabled");
    expect(environmentSnapshotFromCapabilitySnapshot(snapshot).available_encoders).not.toContain(
      "software_vvc"
    );
    expect(environmentSnapshotFromCapabilitySnapshot(snapshot).available_decoders).not.toContain(
      "software_vvc"
    );
  });

  it("seeds legacy fallback snapshots with the same built-in constraints as the service", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    expect(snapshot.constraints.map((constraint) => constraint.id)).toEqual([
      "openh264_requires_cpu_input",
      "d3d12_probe_only",
      "opengl_d3d11_shared_interop_hybrid",
      "webview_degraded_render",
    ]);
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

describe("buildCapabilitySnapshotFromIpc", () => {
  it("normalizes optional FFmpeg service and decoder capabilities", () => {
    const ipcSnapshot: IpcCapabilitySnapshot = {
      schema_version: 1,
      platform: "windows",
      service_version: "0.1.0",
      capabilities: [
        {
          id: "service.ffmpeg",
          domain: "service",
          label: "FFmpeg tools",
          status: "available",
          platform: "windows",
        },
        {
          id: "decode.ffmpeg_h264",
          domain: "decode",
          label: "FFmpeg H.264",
          status: "available",
          platform: "windows",
        },
        {
          id: "decode.ffmpeg_hevc",
          domain: "decode",
          label: "FFmpeg HEVC",
          status: "driver_missing",
          platform: "windows",
        },
      ],
      constraints: [],
      profiles: [],
      updated_at_ms: 1,
    };

    const snapshot = buildCapabilitySnapshotFromIpc(ipcSnapshot);

    expect(statusOf(snapshot, "service.ffmpeg")).toBe("available");
    expect(statusOf(snapshot, "decode.ffmpeg_h264")).toBe("available");
    expect(statusOf(snapshot, "decode.ffmpeg_hevc")).toBe("driver_missing");
  });

  it("preserves service-owned structured snapshot fields for UI evaluation", () => {
    const ipcSnapshot: IpcCapabilitySnapshot = {
      schema_version: 1,
      platform: "linux",
      service_version: "0.1.0",
      capabilities: [
        {
          id: "capture.pipewire",
          domain: "capture",
          label: "PipeWire capture",
          status: "available",
          platform: "linux",
        },
        {
          id: "transport.quic_datagram",
          domain: "transport",
          label: "QUIC datagram media",
          status: "usable",
          platform: "linux",
          fallback_ids: ["transport.webrtc"],
        },
      ],
      constraints: [
        {
          id: "openh264_requires_cpu_input",
          applies_to: ["encode.openh264", "memory.d3d11_shared"],
          status: "block",
          reason: "OpenH264 requires CPU-backed input",
          fallback_ids: ["memory.cpu"],
        },
      ],
      profiles: [
        {
          id: "smoke.720p30",
          width: 1280,
          height: 720,
          fps: 30,
          bitrate_mbps: 8,
          codec: "h264",
          required_capabilities: ["capture.pipewire", "transport.quic_datagram"],
        },
      ],
      updated_at_ms: 1_700_000_000_000,
    };

    const snapshot = buildCapabilitySnapshotFromIpc(ipcSnapshot);

    expect(snapshot.platform).toBe("linux");
    expect(snapshot.service_version).toBe("0.1.0");
    expect(snapshot.updated_at_ms).toBe(1_700_000_000_000);
    expect(statusOf(snapshot, "capture.pipewire")).toBe("available");
    expect(snapshot.constraints[0]?.status).toBe("block");
    expect(snapshot.profiles[0]?.required_capabilities).toEqual([
      "capture.pipewire",
      "transport.quic_datagram",
    ]);
    expect(snapshot.recent_profile_results).toEqual([]);

    const support = evaluateProfileSupport("smoke.720p30", snapshot);
    expect(support.status).toBe("ready");
  });

  it("keeps service-owned H.266/VVC software codec capabilities selectable when probed", () => {
    const ipcSnapshot: IpcCapabilitySnapshot = {
      schema_version: 1,
      platform: "windows",
      service_version: "0.1.0",
      capabilities: [
        {
          id: "encode.software_vvc",
          domain: "encode",
          label: "Software H.266/VVC encode",
          status: "available",
          platform: "windows",
        },
        {
          id: "decode.software_vvc",
          domain: "decode",
          label: "Software H.266/VVC decode",
          status: "supported",
          platform: "windows",
        },
      ],
      constraints: [],
      profiles: [],
      updated_at_ms: 1,
    };

    const snapshot = buildCapabilitySnapshotFromIpc(ipcSnapshot);
    const environment = environmentSnapshotFromCapabilitySnapshot(snapshot, windowsEnvironment);

    expect(capabilityOptionState(snapshot, "encoder", "software_vvc")).toBe("selectable");
    expect(capabilityOptionState(snapshot, "decoder", "software_vvc")).toBe("selectable");
    expect(environment.available_encoders).toContain("software_vvc");
    expect(environment.available_decoders).toContain("software_vvc");
  });
});

describe("evaluateCapabilityCombination", () => {
  it("accepts a Windows hardware capture-to-render path when every stage is exposed", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateCapabilityCombination(
      {
        capture: "dxgi",
        encoder: "nvenc_h264",
        decoder: "nvdec",
        renderer: "d3d11",
        memory: "d3d11_shared",
      },
      snapshot
    );

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("treats service-owned supported capabilities as runnable", () => {
    const snapshot: CapabilitySnapshot = {
      schema_version: 1,
      platform: "linux",
      capabilities: [
        {
          id: "capture.linux",
          domain: "capture",
          label: "Linux capture",
          status: "supported",
          platform: "linux",
        },
        {
          id: "encode.nvenc_h264",
          domain: "encode",
          label: "NVENC H.264",
          status: "supported",
          platform: "linux",
        },
        {
          id: "render.linux",
          domain: "render",
          label: "Linux renderer",
          status: "supported",
          platform: "linux",
        },
        {
          id: "memory.cpu",
          domain: "memory",
          label: "CPU memory",
          status: "supported",
          platform: "linux",
        },
        {
          id: "transport.webrtc",
          domain: "transport",
          label: "WebRTC",
          status: "supported",
          platform: "linux",
        },
      ],
      constraints: [],
      profiles: [],
      recent_profile_results: [],
    };

    const result = evaluateCapabilityCombination(
      {
        capture: "linux",
        encoder: "nvenc_h264",
        renderer: "linux",
        memory: "cpu",
        transport: "webrtc",
      },
      snapshot
    );

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("marks the Linux OpenH264 path runnable but degraded", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(linuxEnvironment);

    const result = evaluateCapabilityCombination(
      {
        capture: "linux",
        encoder: "openh264",
        decoder: "software",
        renderer: "linux",
        memory: "cpu",
      },
      snapshot
    );

    expect(result.status).toBe("degraded");
    expect(result.reasons.join(" ")).toContain("encode.openh264");
    expect(result.reasons.join(" ")).toContain("decode.software");
  });

  it("maps generic VideoToolbox decode to the H.264 decoder capability for H.264 encoders", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...linuxEnvironment,
      os_type: "macos",
      available_captures: ["macos"],
      available_encoders: ["videotoolbox_h264", "openh264"],
      available_decoders: ["videotoolbox_h264"],
      available_renderers: ["macos"],
    });

    const result = evaluateCapabilityCombination(
      {
        encoder: "videotoolbox_h264",
        decoder: "videotoolbox",
      },
      snapshot
    );

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("maps generic VideoToolbox decode to the HEVC decoder capability for HEVC encoders", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...linuxEnvironment,
      os_type: "macos",
      available_captures: ["macos"],
      available_encoders: ["videotoolbox_hevc"],
      available_decoders: ["videotoolbox_hevc"],
      available_renderers: ["macos"],
    });

    const result = evaluateCapabilityCombination(
      {
        encoder: "videotoolbox_hevc",
        decoder: "videotoolbox",
      },
      snapshot
    );

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("blocks generic VideoToolbox decode for HEVC when only H.264 decode is exposed", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...linuxEnvironment,
      os_type: "macos",
      available_captures: ["macos"],
      available_encoders: ["videotoolbox_hevc"],
      available_decoders: ["videotoolbox_h264"],
      available_renderers: ["macos"],
    });

    const result = evaluateCapabilityCombination(
      {
        encoder: "videotoolbox_hevc",
        decoder: "videotoolbox",
      },
      snapshot
    );

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("decode.videotoolbox_hevc");
  });

  it("blocks a Linux request for Windows-only NVIDIA hardware stages", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(linuxEnvironment);

    const result = evaluateCapabilityCombination(
      {
        capture: "linux",
        encoder: "nvenc_h264",
        decoder: "nvdec",
        renderer: "d3d11",
        memory: "d3d11_shared",
      },
      snapshot
    );

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("encode.nvenc_h264");
    expect(result.reasons.join(" ")).toContain("decode.nvdec");
    expect(result.reasons.join(" ")).toContain("render.d3d11");
  });

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

  it("applies service-owned block constraints to requested combinations", () => {
    const snapshot: CapabilitySnapshot = {
      ...withAvailableCapabilities(buildCapabilitySnapshotFromEnvironment(windowsEnvironment), [
        "render.d3d11",
        "memory.d3d11_shared",
      ]),
      constraints: [
        {
          id: "shared_memory_renderer_policy",
          applies_to: ["render.d3d11", "memory.d3d11_shared"],
          status: "block",
          reason: "Shared texture render path is temporarily disabled by policy",
          fallback_ids: ["memory.cpu"],
        },
      ],
    };

    const result = evaluateCapabilityCombination(
      {
        renderer: "d3d11",
        memory: "d3d11_shared",
      },
      snapshot
    );

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("temporarily disabled by policy");
    expect(result.requiredFallbacks).toContain("memory.cpu");
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

  it("allows OpenGL hybrid renderer with D3D11 shared memory", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment({
      ...windowsEnvironment,
      available_renderers: ["d3d11", "opengl", "webview"],
    });

    const result = evaluateCapabilityCombination(
      {
        renderer: "opengl",
        memory: "d3d11_shared",
      },
      snapshot
    );

    expect(result.status).not.toBe("blocked");
    expect(result.requiredFallbacks).not.toContain("memory.cpu");
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

describe("service capability option mapping", () => {
  it("maps UI renderer ids to service-owned renderer capability ids", () => {
    expect(capabilityIdForLegacyOption("renderer", "d3d12")).toBe("render.d3d12_native");
    expect(capabilityIdForLegacyOption("renderer", "d3d12_native")).toBe(
      "render.d3d12_native"
    );
  });

  it("keeps unavailable service capabilities out of legacy environment arrays", () => {
    const snapshot: CapabilitySnapshot = {
      schema_version: 1,
      platform: "windows",
      capabilities: [
        {
          id: "render.d3d11",
          domain: "render",
          label: "D3D11",
          status: "driver_missing",
          platform: "windows",
          reason: "D3D11 runtime probe failed",
        },
        {
          id: "render.opengl",
          domain: "render",
          label: "OpenGL",
          status: "supported",
          platform: "windows",
        },
      ],
      constraints: [],
      profiles: [],
      recent_profile_results: [],
    };

    const environment = environmentSnapshotFromCapabilitySnapshot(snapshot, windowsEnvironment);

    expect(environment.available_renderers).toEqual(["none", "opengl"]);
    expect(capabilityOptionState(snapshot, "renderer", "d3d11")).toBe("disabled");
    expect(capabilityOptionState(snapshot, "renderer", "opengl")).toBe("selectable");
  });
});

describe("capability profiles", () => {
  it("keeps built-in fallback profiles aligned with the service defaults", () => {
    const smoke = getCapabilityProfile("smoke.720p30");
    expect(smoke).toMatchObject({
      min_stable_fps_ratio: 0.8,
      max_drop_ratio: 0.02,
    });
    expect(smoke?.latency_budget_ms).toBeUndefined();
    expect(smoke?.required_capabilities).toEqual([
      "transport.loopback",
      "encode.openh264",
      "decode.software",
    ]);

    expect(getCapabilityProfile("interactive.1080p60")).toMatchObject({
      codec: "hevc",
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
    });
    expect(getCapabilityProfile("interactive.1080p60")?.latency_budget_ms).toBeUndefined();

    expect(getCapabilityProfile("compat.h264.1080p60")).toMatchObject({
      codec: "h264",
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
    });
    expect(getCapabilityProfile("compat.h264.1080p60")?.latency_budget_ms).toBeUndefined();

    const quality = getCapabilityProfile("quality.4k60");
    expect(quality).toMatchObject({
      min_stable_fps_ratio: 0.8,
      max_drop_ratio: 0.02,
    });
    expect(quality?.latency_budget_ms).toBeUndefined();
    expect(quality?.required_capabilities).toEqual([
      "encode.nvenc_hevc",
      "decode.nvdec_hevc",
      "media.hevc_main_420_8bit",
      "render.d3d11",
      "memory.d3d11_shared",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ]);
  });

  it("exposes a LAN 2K144 profile with required media capabilities", () => {
    const profile = getCapabilityProfile("lan.2k144");

    expect(profile).toMatchObject({
      id: "lan.2k144",
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 64,
      codec: "hevc",
    });
    expect(profile?.required_capabilities).toContain("encode.nvenc_hevc");
    expect(profile?.required_capabilities).toContain("decode.nvdec_hevc");
    expect(profile?.required_capabilities).toContain("media.hevc_main_420_8bit");
    expect(profile?.required_capabilities).toContain("transport.quic_datagram");
    expect(profile?.required_capabilities).toContain("transport.media_profile_control_v1");
  });

  it("exposes a native macOS 2K144 profile", () => {
    const profile = getCapabilityProfile("lan.macos.2k144");

    expect(profile).toMatchObject({
      id: "lan.macos.2k144",
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 80,
      codec: "h264",
    });
    expect(profile?.required_capabilities).toEqual([
      "capture.macos",
      "encode.videotoolbox_h264",
      "decode.videotoolbox_h264",
      "memory.cpu",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ]);
  });

  it("exposes a native macOS HEVC 2K144 profile tuned for VideoToolbox", () => {
    const profile = getCapabilityProfile("lan.macos.hevc.2k144");

    expect(profile).toMatchObject({
      id: "lan.macos.hevc.2k144",
      width: 2560,
      height: 1440,
      fps: 144,
      bitrate_mbps: 40,
      codec: "hevc",
    });
    expect(profile?.required_capabilities).toEqual([
      "capture.macos",
      "encode.videotoolbox_hevc",
      "decode.videotoolbox_hevc",
      "media.hevc_main_420_8bit",
      "memory.cpu",
      "transport.quic_datagram",
      "transport.media_profile_control_v1",
    ]);
  });

  it("exposes a native 1600p165 LAN profile for 16:10 high-refresh peers", () => {
    const profile = getCapabilityProfile("lan.1600p165");

    expect(profile).toMatchObject({
      id: "lan.1600p165",
      width: 2560,
      height: 1600,
      fps: 165,
      bitrate_mbps: 80,
      codec: "hevc",
    });
    expect(profile?.required_capabilities).toContain("encode.nvenc_hevc");
    expect(profile?.required_capabilities).toContain("decode.nvdec_hevc");
    expect(profile?.required_capabilities).toContain("media.hevc_main_420_8bit");
    expect(profile?.required_capabilities).toContain("transport.quic_datagram");
    expect(profile?.required_capabilities).toContain("transport.media_profile_control_v1");
  });

  it("marks a profile ready only when all required capabilities are available", () => {
    const snapshot = withAvailableCapabilities(
      buildCapabilitySnapshotFromEnvironment(windowsEnvironment),
      [
        "encode.nvenc_hevc",
        "decode.nvdec_hevc",
        "media.hevc_main_420_8bit",
        "transport.quic_datagram",
        "transport.media_profile_control_v1",
      ]
    );

    const result = evaluateProfileSupport("lan.2k144", snapshot);

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("marks the native macOS 2K144 profile ready on a complete macOS snapshot", () => {
    const snapshot = withAvailableCapabilities(
      buildCapabilitySnapshotFromEnvironment({
        ...linuxEnvironment,
        os_type: "macos",
        available_captures: ["macos", "synthetic"],
        available_encoders: ["videotoolbox_h264", "openh264"],
        available_decoders: ["videotoolbox_h264", "software"],
        available_renderers: ["macos", "webview"],
        available_memory_modes: ["cpu"],
      }),
      ["transport.quic_datagram", "transport.media_profile_control_v1"]
    );

    const result = evaluateProfileSupport("lan.macos.2k144", snapshot);

    expect(result.status).toBe("ready");
    expect(result.reasons).toEqual([]);
  });

  it("blocks a profile when a required capability is missing or not usable", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(windowsEnvironment);

    const result = evaluateProfileSupport("lan.2k144", snapshot);

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("transport.media_profile_control_v1");
  });

  it("applies service-owned constraints when evaluating profile support", () => {
    const snapshot: CapabilitySnapshot = {
      ...withAvailableCapabilities(buildCapabilitySnapshotFromEnvironment(windowsEnvironment), [
        "render.d3d11",
        "memory.d3d11_shared",
      ]),
      constraints: [
        {
          id: "profile_constraint",
          applies_to: ["render.d3d11", "memory.d3d11_shared"],
          status: "block",
          reason: "Profile is blocked by service policy",
          fallback_ids: ["memory.cpu"],
        },
      ],
      profiles: [
        {
          id: "blocked.profile",
          width: 1920,
          height: 1080,
          fps: 60,
          bitrate_mbps: 20,
          codec: "h264",
          required_capabilities: ["render.d3d11", "memory.d3d11_shared"],
        },
      ],
    };

    const result = evaluateProfileSupport("blocked.profile", snapshot);

    expect(result.status).toBe("blocked");
    expect(result.reasons.join(" ")).toContain("Profile is blocked by service policy");
  });

  it("marks software fallback profiles as degraded instead of unavailable", () => {
    const snapshot = buildCapabilitySnapshotFromEnvironment(linuxEnvironment);

    const result = evaluateProfileSupport("diagnostic.software", snapshot);

    expect(result.status).toBe("degraded");
    expect(result.reasons.join(" ")).toContain("encode.openh264");
    expect(result.reasons.join(" ")).toContain("decode.software");
    expect(result.reasons.join(" ")).toContain("render.webview");
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
