import { describe, expect, it, vi } from "vitest";
import {
  runLanE2EAutomation,
  type LanE2EAutomationCommands,
} from "./lanE2eAutomationService";

function ok<T>(value: T) {
  return { ok: true as const, value };
}

function err(message: string) {
  return { ok: false as const, error: { message } };
}

const DEFAULT_REQUESTED_PROFILE = {
  width: 2560,
  height: 1600,
  fps: 165,
  bitrate_mbps: 80,
  codec: "hevc",
  codec_profile: "main",
  bit_depth: 8,
  chroma_subsampling: "4:2:0",
  pixel_format: "nv12",
  hdr_enabled: false,
};

const H264_FALLBACK_REQUESTED_PROFILE = {
  width: 2560,
  height: 1600,
  fps: 165,
  bitrate_mbps: 80,
  codec: "h264",
};

const MACOS_HEVC_2K144_REQUESTED_PROFILE = {
  width: 2560,
  height: 1440,
  fps: 144,
  bitrate_mbps: 40,
  codec: "hevc",
  codec_profile: "main",
  bit_depth: 8,
  chroma_subsampling: "4:2:0",
  pixel_format: "nv12",
  hdr_enabled: false,
};

const DEFAULT_CAPTURE_SOURCES = [
  {
    id: "window-codex",
    platform: "windows",
    source_kind: "window",
    title: "Codex",
    class_name: "Chrome_WidgetWin_1",
    width: 1280,
    height: 720,
    process_id: 100,
    app_name: "Codex",
  },
  {
    id: "display-shared",
    platform: "windows",
    source_kind: "display_shared",
    title: "DISPLAY1",
    class_name: "Monitor",
    width: 2560,
    height: 1600,
    process_id: 0,
    app_name: null,
  },
  {
    id: "display",
    platform: "windows",
    source_kind: "display",
    title: "DISPLAY1",
    class_name: "Monitor",
    width: 2560,
    height: 1600,
    process_id: 0,
    app_name: null,
  },
];

const DEFAULT_ATTACHED_SURFACE = {
  surface_id: "surface-1",
  backend: "d3d11",
  window_handle: 1234,
};

function createCommands(
  overrides: Partial<LanE2EAutomationCommands> = {}
): LanE2EAutomationCommands {
  return {
    serviceBootstrapIfNeeded: vi.fn().mockResolvedValue(ok(true)),
    serviceWaitForHealthy: vi.fn().mockResolvedValue(ok(true)),
    ipcRuntimeSnapshot: vi.fn().mockResolvedValue(
      ok({
        device_id: "controller-device",
        is_registered: true,
        sessions: [],
      })
    ),
    getHardwareInfo: vi.fn().mockResolvedValue(
      ok({
        motherboard_serial: "MB-LOCAL-1234",
        hostname: "Controller PC",
        os_type: "windows",
        os_version: "Windows",
        cpu_info: {
          name: "CPU",
          vendor_id: "GenuineIntel",
          cores: 8,
        },
        total_memory_mb: 16384,
        gpu_info: [],
      })
    ),
    ipcRegisterDevice: vi.fn().mockResolvedValue(ok("lan-MBLOCAL1234")),
    ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        running: true,
        discovery_port: 37777,
        instance_id: "controller-instance",
        last_probe_ms: 10,
        peers: [
          {
            device_id: "agent-device",
            device_name: "Agent PC",
            device_type: "desktop",
            ip: "192.168.1.24",
            discovery_port: 37777,
            p2p_control_addr: "192.168.1.24:37778",
            transports: [
              "quic",
              "quic_datagram",
              "quic_datagram_2k144",
              "quic_datagram_media_v2",
              "quic_datagram_media_v3",
              "media_profile_control_v1",
            ],
            protocol_version: 1,
            service_build_id: "test-build",
            media_protocol_version: 3,
            media_capabilities: [
              "quic_datagram_media_v3",
              "dxgi_capture",
              "nvenc_hevc",
              "nvdec_hevc",
              "media.hevc_main_420_8bit",
              "nvenc_h264",
              "nvdec",
              "d3d11_native_render",
            ],
            age_ms: 20,
            p2p_available: true,
          },
        ],
      })
    ),
    ipcStartLanRemoteSession: vi.fn().mockResolvedValue(ok("session-started")),
    ipcUpdateMediaProfile: vi.fn().mockResolvedValue(ok({ status: "selected" })),
    ipcConfigureMediaAdaptation: vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        state: "configured",
        ladder_index: 0,
        current_profile: DEFAULT_REQUESTED_PROFILE,
        target_profile: DEFAULT_REQUESTED_PROFILE,
        last_reason: "configured",
        last_change_ms: 1_700_000_000_000,
        observed_fps: 0,
        drop_ratio: 0,
        queue_depth: 0,
      })
    ),
    ipcListRemoteCaptureSources: vi.fn().mockResolvedValue(ok(DEFAULT_CAPTURE_SOURCES)),
    ipcSelectRemoteCaptureSource: vi.fn().mockResolvedValue(
      ok({
        session_id: "lan-e2e-test-session",
        source: DEFAULT_CAPTURE_SOURCES[2],
        status: "selected",
        reason: null,
      })
    ),
    ipcListRemoteDisplayModes: vi.fn().mockResolvedValue(
      ok([
        {
          id: "mode-current",
          source_id: "display-shared",
          width: 2560,
          height: 1600,
          refresh_hz: 60,
          bit_depth: 32,
          is_current: true,
        },
        {
          id: "mode-target",
          source_id: "display-shared",
          width: 2560,
          height: 1600,
          refresh_hz: 165,
          bit_depth: 32,
          is_current: false,
        },
      ])
    ),
    ipcSetRemoteDisplayMode: vi.fn().mockResolvedValue(
      ok({
        session_id: "lan-e2e-test-session",
        requested: {
          id: "mode-target",
          source_id: "display-shared",
          width: 2560,
          height: 1600,
          refresh_hz: 165,
          bit_depth: 32,
          is_current: false,
        },
        previous: {
          id: "mode-current",
          source_id: "display-shared",
          width: 2560,
          height: 1600,
          refresh_hz: 60,
          bit_depth: 32,
          is_current: true,
        },
        active: {
          id: "mode-target",
          source_id: "display-shared",
          width: 2560,
          height: 1600,
          refresh_hz: 165,
          bit_depth: 32,
          is_current: true,
        },
        status: "changed",
        reason: null,
        restore_required: true,
      })
    ),
    ipcRestoreRemoteDisplayMode: vi.fn().mockResolvedValue(
      ok({
        session_id: "lan-e2e-test-session",
        requested: null,
        previous: null,
        active: null,
        status: "restored",
        reason: null,
        restore_required: false,
      })
    ),
    ipcStartReceiver: vi.fn().mockResolvedValue(ok("receiver-started")),
    openRemoteDisplayWindow: vi.fn().mockResolvedValue(
      ok({
        label: "remote-display-agent-device",
        session_id: "unused",
        surface_id: "surface-1",
        role: "controller",
        renderer_attached: true,
        render_mode: "d3d11_native",
        native_surface_attached: true,
        session_window_count: 1,
      })
    ),
    ipcSessionSnapshot: vi.fn().mockResolvedValue(
      ok({
        session_id: "unused",
        role: "controller",
        state: "streaming",
        transport_kind: "quic",
        sender_active: false,
        receiver_active: true,
      })
    ),
    ipcProbeSnapshot: vi.fn().mockResolvedValue(
      ok({
        session_id: "unused",
        frames_received: 4,
        frames_decoded: 3,
        frames_dropped: 0,
        current_fps: 165,
        bitrate_mbps: 80,
        media_probe_valid: true,
        media_probe_format: "compressed_h264_test_pattern",
        media_probe_width: 2560,
        media_probe_height: 1600,
        media_probe_target_fps: 165,
        media_probe_target_bitrate_mbps: 80,
        media_probe_payload_bytes: 55555,
        last_media_sequence: 3,
        last_media_timestamp_us: 123456,
        last_media_payload_hash: "fnv1a64:abc123",
        last_error: null,
      })
    ),
    ipcMediaPipelineSnapshot: vi.fn().mockResolvedValue(
      ok({
        session_id: "unused",
        attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
        active_decoder: "nvdec",
        active_renderer: "d3d11",
        queue_depth: 1,
        dropped_frames: 0,
        stage_metrics: [
          { stage: "decode", p50_ms: 0.8, p95_ms: 1.2 },
          { stage: "render_present", p50_ms: 5.0, p95_ms: 7.0 },
        ],
        adaptation: null,
      })
    ),
    ipcStopSession: vi.fn().mockResolvedValue(ok("stopped")),
    ...overrides,
  };
}

function withCaptureSourceCommands(
  commands: LanE2EAutomationCommands,
  sources = DEFAULT_CAPTURE_SOURCES
) {
  const ipcListRemoteCaptureSources = vi.fn().mockResolvedValue(ok(sources));
  const ipcSelectRemoteCaptureSource = vi.fn().mockImplementation((_sessionId, sourceId) => {
    const selectedSource = sources.find((source) => source.id === sourceId) ?? sources[0];
    return Promise.resolve(ok({
      session_id: "lan-e2e-test-session",
      source: selectedSource,
      status: "selected",
      reason: null,
    }));
  });

  Object.assign(commands, {
    ipcListRemoteCaptureSources,
    ipcSelectRemoteCaptureSource,
  });

  return commands as LanE2EAutomationCommands & {
    ipcListRemoteCaptureSources: typeof ipcListRemoteCaptureSources;
    ipcSelectRemoteCaptureSource: typeof ipcSelectRemoteCaptureSource;
  };
}

describe("runLanE2EAutomation", () => {
  it("runs cross-device discovery without starting a session", async () => {
    const commands = createCommands();

    const result = await runLanE2EAutomation(commands, {
      scenarioId: "cross.e2e.discovery",
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
    });

    expect(result.status).toBe("completed");
    expect(result.scenarioId).toBe("cross.e2e.discovery");
    expect(result.peer?.device_id).toBe("agent-device");
    expect(result.dataPlaneVerified).toBe(false);
    expect(result.mediaVerified).toBe(false);
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
    expect(commands.ipcListRemoteCaptureSources).not.toHaveBeenCalled();
    expect(commands.ipcStartReceiver).not.toHaveBeenCalled();
    expect(commands.openRemoteDisplayWindow).not.toHaveBeenCalled();
    expect(commands.ipcStopSession).not.toHaveBeenCalled();
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "assert:completed"
    );
  });

  it("skips cross-device fault recovery when service fault injection is unavailable", async () => {
    const commands = createCommands();

    const result = await runLanE2EAutomation(commands, {
      scenarioId: "cross.fault.recovery",
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      faultPlan: { type: "network.pause_peer", durationMs: 500 },
    });

    expect(result.status).toBe("skipped");
    expect(result.failureReason).toBe("fault_injection_unsupported");
    expect(result.faultEvents).toEqual([
      expect.objectContaining({
        type: "network.pause_peer",
        status: "unsupported",
      }),
    ]);
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
    expect(commands.ipcStopSession).not.toHaveBeenCalled();
  });

  it("injects a cross-device fault before sampling when the service supports it", async () => {
    const crossE2EInjectFault = vi.fn().mockResolvedValue(ok("pause-peer injected"));
    const commands = createCommands({ crossE2EInjectFault });

    const result = await runLanE2EAutomation(commands, {
      scenarioId: "cross.fault.recovery",
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
      faultPlan: { type: "network.pause_peer", durationMs: 500 },
    });

    expect(result.status).toBe("completed");
    expect(crossE2EInjectFault).toHaveBeenCalledWith("lan-e2e-test-session", {
      type: "network.pause_peer",
      durationMs: 500,
    });
    expect(result.faultEvents).toEqual([
      expect.objectContaining({
        type: "network.pause_peer",
        status: "injected",
      }),
    ]);
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "fault:completed"
    );
  });

  it("discovers a LAN peer, starts remote display, validates frames, and stops the session", async () => {
    const commands = createCommands();

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sessionId).toBe("lan-e2e-test-session");
    expect(result.peer?.device_id).toBe("agent-device");
    expect(result.probeSnapshot?.frames_decoded).toBe(3);
    expect(result.mediaPipelineSnapshot?.active_decoder).toBe("nvdec");
    expect(result.mediaPipelineSnapshot?.queue_depth).toBe(1);
    expect(result.mediaVerified).toBe(true);
    expect(result.probeSnapshot?.media_probe_valid).toBe(true);
    expect(result.probeSnapshot?.last_media_payload_hash).toBe("fnv1a64:abc123");
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      DEFAULT_REQUESTED_PROFILE
    );
    expect(result.requestedProfile).toEqual(DEFAULT_REQUESTED_PROFILE);
    expect(commands.ipcStartReceiver).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(commands.openRemoteDisplayWindow).toHaveBeenCalledWith({
      sessionId: "lan-e2e-test-session",
      avoidCaptureSourceId: "DISPLAY1",
      requestedProfile: DEFAULT_REQUESTED_PROFILE,
    });
    expect(commands.ipcStopSession).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "assert:completed"
    );
  });

  it("uses the captured display device name when placing the receiver window", async () => {
    const dxgiDisplaySource = {
      id: "windows:display-shared:2",
      platform: "windows",
      source_kind: "display_shared",
      title: "Display 3 (D3D11 shared copy)",
      class_name: "DXGIShared:\\\\.\\DISPLAY3",
      width: 2560,
      height: 1440,
      process_id: 0,
      app_name: "Display",
    };
    const commands = createCommands({
      ipcListRemoteCaptureSources: vi.fn().mockResolvedValue(ok([dxgiDisplaySource])),
      ipcSelectRemoteCaptureSource: vi.fn().mockResolvedValue(
        ok({
          session_id: "lan-e2e-test-session",
          source: dxgiDisplaySource,
          status: "selected",
          reason: null,
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(commands.openRemoteDisplayWindow).toHaveBeenCalledWith({
      sessionId: "lan-e2e-test-session",
      avoidCaptureSourceId: "DXGIShared:\\\\.\\DISPLAY3",
      requestedProfile: DEFAULT_REQUESTED_PROFILE,
    });
  });

  it("configures adaptive media before receiver startup and reports its snapshot", async () => {
    const ipcConfigureMediaAdaptation = vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        state: "configured",
        ladder_index: 0,
        current_profile: {
          width: 2560,
          height: 1600,
          fps: 165,
          bitrate_mbps: 80,
          codec: "h264",
        },
        target_profile: {
          width: 2560,
          height: 1600,
          fps: 165,
          bitrate_mbps: 80,
          codec: "h264",
        },
        last_reason: "configured",
        last_change_ms: 1_700_000_000_000,
        observed_fps: 0,
        drop_ratio: 0,
        queue_depth: 0,
      })
    );
    const commands = createCommands({ ipcConfigureMediaAdaptation });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      adaptive: true,
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      expect.objectContaining({
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 64,
      })
    );
    expect(ipcConfigureMediaAdaptation).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({
        enabled: true,
        mode: "keyframe_ladder",
        dynamic_resolution_enabled: false,
        ceiling_profile: expect.objectContaining({
          width: 2560,
          height: 1600,
          fps: 165,
          bitrate_mbps: 80,
        }),
        floor_profile: expect.objectContaining({
          width: 1280,
          height: 800,
          fps: 60,
          bitrate_mbps: 10,
        }),
      })
    );
    expect(result.mediaAdaptationSnapshot?.state).toBe("configured");
    const events = result.stages.map((stage) => `${stage.stage}:${stage.status}`);
    expect(events.indexOf("adaptation:completed")).toBeLessThan(
      events.indexOf("receiver:started")
    );
  });

  it("preserves HEVC codec and sampling in adaptive floor profile", async () => {
    const ipcConfigureMediaAdaptation = vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        state: "configured",
        ladder_index: 0,
        current_profile: DEFAULT_REQUESTED_PROFILE,
        target_profile: DEFAULT_REQUESTED_PROFILE,
        last_reason: "configured",
        last_change_ms: 1_700_000_000_000,
        observed_fps: 0,
        drop_ratio: 0,
        queue_depth: 0,
      })
    );
    const commands = createCommands({ ipcConfigureMediaAdaptation });

    await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      adaptive: true,
      requestedProfile: {
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 120,
        codec: "hevc",
        codec_profile: "main",
        bit_depth: 8,
        chroma_subsampling: "4:2:0",
        pixel_format: "nv12",
        hdr_enabled: false,
      },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      expect.objectContaining({
        width: 2560,
        height: 1600,
        fps: 165,
        bitrate_mbps: 96,
        codec: "hevc",
        codec_profile: "main",
        bit_depth: 8,
        chroma_subsampling: "4:2:0",
        pixel_format: "nv12",
        hdr_enabled: false,
      })
    );
    expect(ipcConfigureMediaAdaptation).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({
        floor_profile: expect.objectContaining({
          width: 1280,
          height: 800,
          fps: 60,
          bitrate_mbps: 10,
          codec: "hevc",
          codec_profile: "main",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        }),
      })
    );
  });

  it("honors explicit dynamic resolution adaptive config overrides", async () => {
    const ipcConfigureMediaAdaptation = vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        state: "configured",
        ladder_index: 0,
        current_profile: DEFAULT_REQUESTED_PROFILE,
        target_profile: DEFAULT_REQUESTED_PROFILE,
        last_reason: "configured",
        last_change_ms: 1_700_000_000_000,
        observed_fps: 0,
        drop_ratio: 0,
        queue_depth: 0,
      })
    );
    const commands = createCommands({ ipcConfigureMediaAdaptation });

    await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      adaptive: true,
      adaptiveConfig: {
        enabled: true,
        dynamic_resolution_enabled: true,
      },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(ipcConfigureMediaAdaptation).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({
        enabled: true,
        dynamic_resolution_enabled: true,
      })
    );
  });

  it("keeps low bitrate adaptive session startup at the requested profile", async () => {
    const ipcConfigureMediaAdaptation = vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        state: "configured",
        ladder_index: 0,
        current_profile: DEFAULT_REQUESTED_PROFILE,
        target_profile: DEFAULT_REQUESTED_PROFILE,
        last_reason: "configured",
        last_change_ms: 1_700_000_000_000,
        observed_fps: 0,
        drop_ratio: 0,
        queue_depth: 0,
      })
    );
    const commands = createCommands({ ipcConfigureMediaAdaptation });
    const requestedProfile = {
      width: 1920,
      height: 1080,
      fps: 60,
      bitrate_mbps: 20,
      codec: "hevc",
      codec_profile: "main",
      bit_depth: 8,
      chroma_subsampling: "4:2:0",
      pixel_format: "nv12",
      hdr_enabled: false,
    };

    await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      adaptive: true,
      requestedProfile,
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      requestedProfile
    );
  });

  it("marks the display window as native once the service reports an attached render surface", async () => {
    const commands = createCommands({
      openRemoteDisplayWindow: vi.fn().mockResolvedValue(
        ok({
          label: "remote-display-agent-device",
          session_id: "unused",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        })
      ),
      ipcMediaPipelineSnapshot: vi.fn().mockResolvedValue(
        ok({
          session_id: "unused",
          attached_surfaces: [
            {
              surface_id: "surface-1",
              backend: "d3d11",
              window_handle: 1234,
            },
          ],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          stage_metrics: [],
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.displayWindow).toEqual(
      expect.objectContaining({
        renderer_attached: true,
        render_mode: "d3d11_native",
        native_surface_attached: true,
      })
    );
  });

  it("can run LAN media diagnostics without opening a render display", async () => {
    const commands = createCommands();

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      renderDisplay: false,
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minSampleDurationMs: 0,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.displayWindow).toBeUndefined();
    expect(commands.openRemoteDisplayWindow).not.toHaveBeenCalled();
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "display:skipped"
    );
  });

  it("fails remote display automation when the native surface never attaches", async () => {
    let currentTime = 0;
    const commands = createCommands({
      openRemoteDisplayWindow: vi.fn().mockResolvedValue(
        ok({
          label: "remote-display-agent-device",
          session_id: "unused",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        })
      ),
      ipcMediaPipelineSnapshot: vi.fn().mockResolvedValue(
        ok({
          session_id: "unused",
          attached_surfaces: [],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          stage_metrics: [],
          adaptation: null,
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => {
        currentTime += 200;
        return currentTime;
      },
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("display_window_failed");
    expect(result.errorMessage).toContain("native surface did not attach");
    expect(result.mediaPipelineSnapshot?.attached_surfaces).toEqual([]);
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "display:failed"
    );
  });

  it("selects the preferred remote capture source before starting the receiver", async () => {
    const commands = withCaptureSourceCommands(createCommands());

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("display-shared");
    expect(result.captureSourceSelection?.status).toBe("selected");
    expect(commands.ipcListRemoteCaptureSources).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      false,
      24
    );
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "display-shared"
    );
    const receiverCallOrder = vi.mocked(commands.ipcStartReceiver).mock.invocationCallOrder[0];
    const captureSourceCallOrder =
      commands.ipcSelectRemoteCaptureSource.mock.invocationCallOrder[0];
    expect(receiverCallOrder).toBeDefined();
    expect(captureSourceCallOrder).toBeDefined();
    expect(receiverCallOrder!).toBeGreaterThan(captureSourceCallOrder!);
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "capture_source:completed"
    );
  });

  it("caps the media profile to the receiver render pacing target", async () => {
    let activeProfile = { ...DEFAULT_REQUESTED_PROFILE };
    const ipcUpdateMediaProfile = vi.fn().mockImplementation((_sessionId, profile) => {
      activeProfile = profile;
      return Promise.resolve(ok({ status: "selected" }));
    });
    const commands = withCaptureSourceCommands(
      createCommands({
        ipcUpdateMediaProfile,
        ipcProbeSnapshot: vi.fn().mockImplementation(() =>
          Promise.resolve(
            ok({
              session_id: "unused",
              frames_received: 4,
              frames_decoded: 3,
              frames_dropped: 0,
              current_fps: activeProfile.fps,
              bitrate_mbps: activeProfile.bitrate_mbps,
              media_probe_valid: true,
              media_probe_format: "compressed_h264_test_pattern",
              media_probe_width: activeProfile.width,
              media_probe_height: activeProfile.height,
              media_probe_target_fps: activeProfile.fps,
              media_probe_target_bitrate_mbps: activeProfile.bitrate_mbps,
              media_probe_payload_bytes: 55555,
              last_media_sequence: 3,
              last_media_timestamp_us: 123456,
              last_media_payload_hash: "fnv1a64:abc123",
              last_error: null,
            })
          )
        ),
        ipcMediaPipelineSnapshot: vi.fn().mockResolvedValue(
          ok({
            session_id: "unused",
            attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
            active_decoder: "nvdec",
            active_renderer: "d3d11",
            render_pacing_target_fps: 144,
            queue_depth: 1,
            dropped_frames: 0,
            stage_metrics: [],
            adaptation: null,
          })
        ),
      })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: { ...DEFAULT_REQUESTED_PROFILE },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(ipcUpdateMediaProfile).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({ fps: 144 })
    );
    expect(result.requestedProfile?.fps).toBe(144);
  });

  it("can force a capture source kind for DXGI canary runs", async () => {
    const commands = withCaptureSourceCommands(createCommands());

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      preferredCaptureSourceKind: "display",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("display");
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "display"
    );
  });

  it("prefers an exact capture source id over the requested source kind", async () => {
    const commands = withCaptureSourceCommands(createCommands());

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      preferredCaptureSourceId: "window-codex",
      preferredCaptureSourceKind: "display",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("window-codex");
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "window-codex"
    );
  });

  it("fails instead of falling back when an exact capture source id is unavailable", async () => {
    const commands = withCaptureSourceCommands(createCommands());

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      preferredCaptureSourceId: "windows:display-shared:0",
      preferredCaptureSourceKind: "display_shared",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("capture_source_failed");
    expect(result.errorMessage).toContain("windows:display-shared:0");
    expect(commands.ipcSelectRemoteCaptureSource).not.toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "display-shared"
    );
  });

  it("sets temporary remote display mode before receiver startup and restores during cleanup", async () => {
    const commands = withCaptureSourceCommands(createCommands());

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      displayModePolicy: "temporary",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.displayModeChange?.status).toBe("changed");
    const setRemoteDisplayMode = commands.ipcSetRemoteDisplayMode;
    const restoreRemoteDisplayMode = commands.ipcRestoreRemoteDisplayMode;
    if (!setRemoteDisplayMode || !restoreRemoteDisplayMode) {
      throw new Error("Expected display mode commands to be configured");
    }
    expect(commands.ipcListRemoteDisplayModes).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(setRemoteDisplayMode).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({ id: "mode-target" }),
      true
    );
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenCalledTimes(2);
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenNthCalledWith(
      2,
      "lan-e2e-test-session",
      "display-shared"
    );
    const receiverCallOrder = vi.mocked(commands.ipcStartReceiver).mock.invocationCallOrder[0];
    const displayModeCallOrder = vi.mocked(setRemoteDisplayMode).mock
      .invocationCallOrder[0];
    const captureRefreshCallOrder =
      commands.ipcSelectRemoteCaptureSource.mock.invocationCallOrder[1];
    if (
      receiverCallOrder === undefined ||
      displayModeCallOrder === undefined ||
      captureRefreshCallOrder === undefined
    ) {
      throw new Error("Expected display mode, capture refresh, and receiver commands to be invoked");
    }
    expect(captureRefreshCallOrder).toBeGreaterThan(displayModeCallOrder);
    expect(receiverCallOrder).toBeGreaterThan(displayModeCallOrder);
    expect(receiverCallOrder).toBeGreaterThan(captureRefreshCallOrder);
    expect(restoreRemoteDisplayMode).toHaveBeenCalledWith("lan-e2e-test-session");
  });

  it("reselects the active display mode source after a display mode switch", async () => {
    const lowResolutionSource = {
      id: "display-shared-low",
      platform: "windows",
      source_kind: "display_shared",
      title: "DISPLAY0",
      class_name: "Monitor",
      width: 1706,
      height: 1066,
      process_id: 0,
      app_name: null,
    };
    const targetSource = {
      id: "display-shared-2k",
      platform: "windows",
      source_kind: "display_shared",
      title: "DISPLAY1",
      class_name: "Monitor",
      width: 2560,
      height: 1440,
      process_id: 0,
      app_name: null,
    };
    const sources = [lowResolutionSource, targetSource];
    const commands = createCommands();
    let selectedSourceId = "";
    commands.ipcListRemoteCaptureSources = vi.fn()
      .mockResolvedValueOnce(ok([lowResolutionSource]))
      .mockResolvedValue(ok(sources));
    commands.ipcSelectRemoteCaptureSource = vi.fn().mockImplementation((_sessionId, sourceId) => {
      selectedSourceId = sourceId;
      const selectedSource = sources.find((source) => source.id === sourceId) ?? lowResolutionSource;
      return Promise.resolve(ok({
        session_id: "lan-e2e-test-session",
        source: selectedSource,
        status: "selected",
        reason: null,
      }));
    });
    commands.ipcListRemoteDisplayModes = vi.fn().mockImplementation(() =>
      Promise.resolve(ok([
        {
          id: selectedSourceId === "display-shared-2k" ? "mode-2k" : "mode-low",
          source_id: selectedSourceId,
          width: selectedSourceId === "display-shared-2k" ? 2560 : 1706,
          height: selectedSourceId === "display-shared-2k" ? 1440 : 1066,
          refresh_hz: selectedSourceId === "display-shared-2k" ? 180 : 60,
          bit_depth: 32,
          is_current: true,
        },
      ]))
    );
    commands.ipcSetRemoteDisplayMode = vi.fn().mockImplementation((_sessionId, mode) =>
      Promise.resolve(ok({
        session_id: "lan-e2e-test-session",
        requested: mode,
        previous: null,
        active: { ...mode, is_current: true },
        status: "changed",
        reason: null,
        restore_required: true,
      }))
    );
    commands.ipcProbeSnapshot = vi.fn().mockResolvedValue(ok({
      session_id: "unused",
      frames_received: 4,
      frames_decoded: 3,
      frames_dropped: 0,
      current_fps: 180,
      bitrate_mbps: 100,
      media_probe_valid: true,
      media_probe_format: "compressed_h264_test_pattern",
      media_probe_width: 2560,
      media_probe_height: 1440,
      media_probe_target_fps: 180,
      media_probe_target_bitrate_mbps: 100,
      media_probe_payload_bytes: 55555,
      last_media_sequence: 3,
      last_media_timestamp_us: 123456,
      last_media_payload_hash: "fnv1a64:abc123",
      last_error: null,
    }));

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      displayModePolicy: "temporary",
      requestedProfile: {
        width: 2560,
        height: 1440,
        fps: 180,
        bitrate_mbps: 100,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("display-shared-low");
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenNthCalledWith(
      1,
      "lan-e2e-test-session",
      "display-shared-low"
    );
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenLastCalledWith(
      "lan-e2e-test-session",
      "display-shared-low"
    );
  });

  it("selects the display source that can satisfy a high-refresh requested profile", async () => {
    const sources = [
      {
        id: "display-shared-144",
        platform: "windows",
        source_kind: "display_shared",
        title: "DISPLAY0",
        class_name: "Monitor",
        width: 2560,
        height: 1440,
        process_id: 0,
        app_name: null,
      },
      {
        id: "display-shared-180",
        platform: "windows",
        source_kind: "display_shared",
        title: "DISPLAY1",
        class_name: "Monitor",
        width: 2560,
        height: 1440,
        process_id: 0,
        app_name: null,
      },
    ];
    const commands = withCaptureSourceCommands(createCommands(), sources);
    let selectedSourceId = "";
    commands.ipcSelectRemoteCaptureSource.mockImplementation((_sessionId, sourceId) => {
      selectedSourceId = sourceId;
      const selectedSource = sources.find((source) => source.id === sourceId) ?? sources[0];
      return Promise.resolve(ok({
        session_id: "lan-e2e-test-session",
        source: selectedSource,
        status: "selected",
        reason: null,
      }));
    });
    commands.ipcListRemoteDisplayModes = vi.fn().mockImplementation(() =>
      Promise.resolve(ok([
        {
          id: selectedSourceId === "display-shared-180" ? "mode-180" : "mode-144",
          source_id: selectedSourceId,
          width: 2560,
          height: 1440,
          refresh_hz: selectedSourceId === "display-shared-180" ? 180 : 144,
          bit_depth: 32,
          is_current: true,
        },
      ]))
    );
    commands.ipcSetRemoteDisplayMode = vi.fn().mockImplementation((_sessionId, mode) =>
      Promise.resolve(ok({
        session_id: "lan-e2e-test-session",
        requested: mode,
        previous: null,
        active: { ...mode, is_current: true },
        status: "changed",
        reason: null,
        restore_required: true,
      }))
    );
    commands.ipcProbeSnapshot = vi.fn().mockResolvedValue(ok({
      session_id: "unused",
      frames_received: 4,
      frames_decoded: 3,
      frames_dropped: 0,
      current_fps: 180,
      bitrate_mbps: 120,
      media_probe_valid: true,
      media_probe_format: "compressed_h264_test_pattern",
      media_probe_width: 2560,
      media_probe_height: 1440,
      media_probe_target_fps: 180,
      media_probe_target_bitrate_mbps: 120,
      media_probe_payload_bytes: 55555,
      last_media_sequence: 3,
      last_media_timestamp_us: 123456,
      last_media_payload_hash: "fnv1a64:abc123",
      last_error: null,
    }));

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      displayModePolicy: "temporary",
      requestedProfile: {
        width: 2560,
        height: 1440,
        fps: 180,
        bitrate_mbps: 120,
        codec: "hevc",
      },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("display-shared-180");
    expect(commands.ipcSetRemoteDisplayMode).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      expect.objectContaining({ id: "mode-180" }),
      true
    );
  });

  it("reuses the selected capture source when post-display-mode refresh times out", async () => {
    const commands = withCaptureSourceCommands(createCommands());
    commands.ipcListRemoteCaptureSources
      .mockResolvedValueOnce(ok(DEFAULT_CAPTURE_SOURCES))
      .mockResolvedValueOnce(err("LAN capture sources request timed out"));

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      displayModePolicy: "temporary",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.captureSource?.id).toBe("display-shared");
    expect(result.captureSourceSelection?.reason).toContain(
      "Reused pre-display-mode source after refresh failed"
    );
    expect(commands.ipcSelectRemoteCaptureSource).toHaveBeenCalledTimes(1);
    expect(commands.ipcStartReceiver).toHaveBeenCalled();
    expect(result.stages).toContainEqual(
      expect.objectContaining({
        stage: "capture_source",
        status: "skipped",
      })
    );
  });

  it("fails before receiver startup when the remote has no capture sources", async () => {
    const commands = withCaptureSourceCommands(createCommands(), []);

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("capture_source_failed");
    expect(result.errorMessage).toContain("No remote capture source available");
    expect(commands.ipcStartReceiver).not.toHaveBeenCalled();
    expect(commands.openRemoteDisplayWindow).not.toHaveBeenCalled();
    expect(commands.ipcStopSession).toHaveBeenCalledWith("lan-e2e-test-session");
  });

  it("fails when the QUIC media probe target does not match the requested profile", async () => {
    const commands = withCaptureSourceCommands(
      createCommands({
        ipcProbeSnapshot: vi.fn().mockResolvedValue(
          ok({
            session_id: "unused",
            frames_received: 4,
            frames_decoded: 3,
            frames_dropped: 0,
            current_fps: 144,
            bitrate_mbps: 64,
            media_probe_valid: true,
            media_probe_format: "compressed_test_pattern",
            media_probe_width: 1920,
            media_probe_height: 1080,
            media_probe_target_fps: 60,
            media_probe_target_bitrate_mbps: 20,
            media_probe_payload_bytes: 55555,
            last_media_sequence: 3,
            last_media_timestamp_us: 123456,
            last_media_payload_hash: "fnv1a64:abc123",
            last_error: null,
          })
        ),
      })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("media_profile_mismatch");
    expect(result.errorMessage).toContain("Runtime media profile mismatch");
    expect(result.errorMessage).toContain("2560x1600 @ 165 FPS / 80 Mbps");
  });

  it("keeps sampling transient profile mismatches until the profile stabilizes", async () => {
    let currentTime = 0;
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      currentTime += currentTime === 0 ? 10 : 60;
      const stable = currentTime >= 50;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: stable ? 24 : 4,
          frames_decoded: stable ? 24 : 3,
          frames_dropped: 0,
          current_fps: stable ? 60 : 12,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: stable ? 1920 : 1728,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: stable ? 24 : 3,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(createCommands({ ipcProbeSnapshot }));

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 500,
      minSampleDurationMs: 50,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.failureReason).toBeUndefined();
    expect(result.sampleDurationMs).toBeGreaterThanOrEqual(50);
    expect(ipcProbeSnapshot).toHaveBeenCalledTimes(2);
  });

  it("uses sample-window FPS when cumulative probe FPS includes startup delay", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    let pipelineIndex = 0;
    const decodedFrames = [10, 16, 17, 18];
    const droppedFrames = [2, 5, 5, 5];
    const sequenceGapDrops = [1, 3, 3, 3];
    const decodeErrorDrops = [1, 2, 2, 2];
    const transientDrops = [0, 1, 1, 1];
    const presentedFrames = [8, 8, 14, 15, 16];
    const renderQueueReplacements = [4, 4, 7, 7, 7];
    const renderPresentSkips = [2, 2, 4, 4, 4];
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const frameCount = decodedFrames[Math.min(probeIndex, decodedFrames.length - 1)];
      const droppedFrameCount = droppedFrames[Math.min(probeIndex, droppedFrames.length - 1)];
      const sequenceGapDropCount =
        sequenceGapDrops[Math.min(probeIndex, sequenceGapDrops.length - 1)];
      const decodeErrorDropCount =
        decodeErrorDrops[Math.min(probeIndex, decodeErrorDrops.length - 1)];
      const transientDropCount =
        transientDrops[Math.min(probeIndex, transientDrops.length - 1)];
      probeIndex += 1;
      currentTime += currentTime === 0 ? 10 : 100;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: frameCount,
          frames_decoded: frameCount,
          frames_dropped: droppedFrameCount,
          sequence_gap_drops: sequenceGapDropCount,
          decode_error_drops: decodeErrorDropCount,
          transient_drops: transientDropCount,
          current_fps: 10,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: frameCount,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockImplementation(() => {
      const renderPresentedFrames =
        presentedFrames[Math.min(pipelineIndex, presentedFrames.length - 1)];
      const renderQueueReplacementCount =
        renderQueueReplacements[Math.min(pipelineIndex, renderQueueReplacements.length - 1)];
      const renderPresentSkipCount =
        renderPresentSkips[Math.min(pipelineIndex, renderPresentSkips.length - 1)];
      pipelineIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          render_presented_frames: renderPresentedFrames,
          render_queue_replacements: renderQueueReplacementCount,
          render_present_skips: renderPresentSkipCount,
          render_lock_drops: 0,
          stage_metrics: [],
          adaptation: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 200,
      minSampleDurationMs: 100,
      minDecodedFrames: 1,
      minFps: 50,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFramesDecoded).toBe(6);
    expect(result.sampleFramesDropped).toBe(3);
    expect(result.sampleSequenceGapDrops).toBe(2);
    expect(result.sampleDecodeErrorDrops).toBe(1);
    expect(result.sampleTransientDrops).toBe(1);
    expect(result.sampleFpsElapsedMs).toBe(100);
    expect(result.sampleFpsTargetDurationMs).toBe(100);
    expect(result.sampleObservedFps).toBeGreaterThanOrEqual(50);
    expect(result.sampleObservedFpsAtTargetDuration).toBeGreaterThanOrEqual(50);
    expect(result.sampleRenderFramesPresented).toBe(6);
    expect(result.sampleObservedRenderFps).toBeGreaterThanOrEqual(50);
    expect(result.sampleObservedRenderFpsAtTargetDuration).toBeGreaterThanOrEqual(50);
    expect(result.sampleRenderQueueReplacements).toBe(3);
    expect(result.sampleRenderPresentSkips).toBe(2);
  });

  it("waits for native render presentation before starting the FPS baseline", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    let pipelineIndex = 0;
    const decodedFrames = [10, 20, 26, 26];
    const presentedFrames = [0, 0, 5, 11];
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const frameCount = decodedFrames[Math.min(probeIndex, decodedFrames.length - 1)];
      probeIndex += 1;
      currentTime += currentTime === 0 ? 10 : 100;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: frameCount,
          frames_decoded: frameCount,
          frames_dropped: 0,
          sequence_gap_drops: 0,
          decode_error_drops: 0,
          transient_drops: 0,
          current_fps: 60,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: frameCount,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockImplementation(() => {
      const renderPresentedFrames =
        presentedFrames[Math.min(pipelineIndex, presentedFrames.length - 1)];
      pipelineIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          render_presented_frames: renderPresentedFrames,
          render_queue_replacements: 0,
          render_present_skips: 0,
          render_lock_drops: 0,
          stage_metrics: [],
          adaptation: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 1_000,
      minSampleDurationMs: 100,
      minDecodedFrames: 1,
      minFps: 50,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFramesDecoded).toBe(6);
    expect(result.sampleRenderFramesPresented).toBe(6);
    expect(result.sampleFpsElapsedMs).toBe(100);
    expect(result.sampleObservedFps).toBe(60);
    expect(result.sampleObservedRenderFps).toBe(60);
  });

  it("reports target-duration FPS when the FPS sample window is within tolerance", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    let pipelineIndex = 0;
    const decodedFrames = [0, 99];
    const presentedFrames = [1, 1, 100, 100];
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const frameCount = decodedFrames[Math.min(probeIndex, decodedFrames.length - 1)];
      probeIndex += 1;
      currentTime += currentTime === 0 ? 10 : 990;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: frameCount,
          frames_decoded: frameCount,
          frames_dropped: 0,
          sequence_gap_drops: 0,
          decode_error_drops: 0,
          transient_drops: 0,
          current_fps: 10,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: frameCount,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockImplementation(() => {
      const renderPresentedFrames =
        presentedFrames[Math.min(pipelineIndex, presentedFrames.length - 1)];
      pipelineIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          render_presented_frames: renderPresentedFrames,
          render_queue_replacements: 0,
          render_lock_drops: 0,
          stage_metrics: [],
          adaptation: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 2_000,
      minSampleDurationMs: 1_000,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFpsElapsedMs).toBe(990);
    expect(result.sampleFpsTargetDurationMs).toBe(1_000);
    expect(result.sampleObservedFpsAtTargetDuration).toBe(99);
    expect(result.sampleObservedRenderFpsAtTargetDuration).toBe(99);
  });

  it("reports target-duration FPS when the FPS sample window overshoots by one poll interval", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    let pipelineIndex = 0;
    const decodedFrames = [0, 120];
    const presentedFrames = [1, 1, 121, 121];
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const frameCount = decodedFrames[Math.min(probeIndex, decodedFrames.length - 1)];
      probeIndex += 1;
      currentTime += currentTime === 0 ? 10 : 1_200;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: frameCount,
          frames_decoded: frameCount,
          frames_dropped: 0,
          sequence_gap_drops: 0,
          decode_error_drops: 0,
          transient_drops: 0,
          current_fps: 10,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: frameCount,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockImplementation(() => {
      const renderPresentedFrames =
        presentedFrames[Math.min(pipelineIndex, presentedFrames.length - 1)];
      pipelineIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          render_presented_frames: renderPresentedFrames,
          render_queue_replacements: 0,
          render_lock_drops: 0,
          stage_metrics: [],
          adaptation: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 500,
      timeoutMs: 2_000,
      minSampleDurationMs: 1_000,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFpsElapsedMs).toBe(1_200);
    expect(result.sampleFpsTargetDurationMs).toBe(1_000);
    expect(result.sampleObservedFpsAtTargetDuration).toBe(120);
    expect(result.sampleObservedRenderFpsAtTargetDuration).toBe(120);
  });

  it("omits target-duration FPS when the FPS sample window exceeds tolerance", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    let pipelineIndex = 0;
    const decodedFrames = [0, 120];
    const presentedFrames = [1, 1, 121, 121];
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const frameCount = decodedFrames[Math.min(probeIndex, decodedFrames.length - 1)];
      probeIndex += 1;
      currentTime += currentTime === 0 ? 10 : 1_200;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: frameCount,
          frames_decoded: frameCount,
          frames_dropped: 0,
          sequence_gap_drops: 0,
          decode_error_drops: 0,
          transient_drops: 0,
          current_fps: 10,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: frameCount,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockImplementation(() => {
      const renderPresentedFrames =
        presentedFrames[Math.min(pipelineIndex, presentedFrames.length - 1)];
      pipelineIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
          active_decoder: "nvdec",
          active_renderer: "d3d11",
          queue_depth: 0,
          dropped_frames: 0,
          render_presented_frames: renderPresentedFrames,
          render_queue_replacements: 0,
          render_lock_drops: 0,
          stage_metrics: [],
          adaptation: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 2_000,
      minSampleDurationMs: 1_000,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFpsElapsedMs).toBe(1_200);
    expect(result.sampleFpsTargetDurationMs).toBeUndefined();
    expect(result.sampleObservedFps).toBe(100);
    expect(result.sampleObservedFpsAtTargetDuration).toBeUndefined();
    expect(result.sampleObservedRenderFpsAtTargetDuration).toBeUndefined();
  });

  it("restarts the sample deadline after applying a render-capped profile", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    const sampleTimes = [1300, 1800, 2300, 2800];
    const decodedFrames = [0, 50, 150, 250];
    const ipcUpdateMediaProfile = vi.fn().mockResolvedValue(ok({ status: "selected" }));
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const index = Math.min(probeIndex, sampleTimes.length - 1);
      currentTime = sampleTimes[index] ?? currentTime;
      const framesDecoded = decodedFrames[index] ?? 0;
      probeIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: framesDecoded,
          frames_decoded: framesDecoded,
          frames_dropped: 0,
          current_fps: framesDecoded > 0 ? 165 : 0,
          bitrate_mbps: 100,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 2560,
          media_probe_height: 1600,
          media_probe_target_fps: 165,
          media_probe_target_bitrate_mbps: 100,
          media_probe_payload_bytes: 55555,
          last_media_sequence: framesDecoded,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockResolvedValue(
      ok({
        session_id: "unused",
        attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
        active_decoder: "nvdec",
        active_renderer: "d3d11",
        active_width: 2560,
        active_height: 1600,
        active_fps: 165,
        active_bitrate_mbps: 100,
        render_pacing_target_fps: 165,
        queue_depth: 0,
        dropped_frames: 0,
        render_presented_frames: 150,
        render_queue_replacements: 0,
        render_lock_drops: 0,
        stage_metrics: [],
        adaptation: null,
      })
    );
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot, ipcUpdateMediaProfile })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 2560,
        height: 1600,
        fps: 180,
        bitrate_mbps: 100,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 1000,
      minSampleDurationMs: 1000,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.requestedProfile?.fps).toBe(165);
    expect(result.sampleDurationMs).toBeGreaterThanOrEqual(1500);
    expect(result.sampleFpsElapsedMs).toBeGreaterThanOrEqual(1000);
    expect(ipcUpdateMediaProfile).toHaveBeenCalledWith("lan-e2e-test-session", {
      width: 2560,
      height: 1600,
      fps: 165,
      bitrate_mbps: 100,
      codec: "h264",
    });
  });

  it("can keep the requested source FPS above the local render pacing cap", async () => {
    let currentTime = 0;
    let probeIndex = 0;
    const sampleTimes = [10, 1100];
    const decodedFrames = [0, 180];
    const ipcUpdateMediaProfile = vi.fn().mockResolvedValue(ok({ status: "selected" }));
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      const index = Math.min(probeIndex, sampleTimes.length - 1);
      currentTime = sampleTimes[index] ?? currentTime;
      const framesDecoded = decodedFrames[index] ?? 0;
      probeIndex += 1;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: framesDecoded,
          frames_decoded: framesDecoded,
          frames_dropped: 0,
          current_fps: framesDecoded > 0 ? 180 : 0,
          bitrate_mbps: 120,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 180,
          media_probe_target_bitrate_mbps: 120,
          media_probe_payload_bytes: 55555,
          last_media_sequence: framesDecoded,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const ipcMediaPipelineSnapshot = vi.fn().mockResolvedValue(
      ok({
        session_id: "unused",
        attached_surfaces: [DEFAULT_ATTACHED_SURFACE],
        active_decoder: "nvdec",
        active_renderer: "d3d11",
        active_width: 2560,
        active_height: 1440,
        active_fps: 180,
        active_bitrate_mbps: 120,
        render_pacing_target_fps: 165,
        queue_depth: 0,
        dropped_frames: 0,
        render_presented_frames: 180,
        render_queue_replacements: 0,
        render_lock_drops: 0,
        stage_metrics: [],
        adaptation: null,
      })
    );
    const commands = withCaptureSourceCommands(
      createCommands({ ipcProbeSnapshot, ipcMediaPipelineSnapshot, ipcUpdateMediaProfile })
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 2560,
        height: 1440,
        fps: 180,
        bitrate_mbps: 120,
        codec: "h264",
      },
      renderProfileCap: false,
      sampleIntervalMs: 0,
      timeoutMs: 1000,
      minSampleDurationMs: 1000,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.requestedProfile?.fps).toBe(180);
    expect(result.mediaPipelineSnapshot?.render_pacing_target_fps).toBe(165);
    expect(ipcUpdateMediaProfile).not.toHaveBeenCalled();
  });

  it("skips comparison when the remote capture source downgrades the selected profile", async () => {
    const commands = withCaptureSourceCommands(
      createCommands({
        ipcProbeSnapshot: vi.fn().mockResolvedValue(
          ok({
            session_id: "unused",
            frames_received: 4,
            frames_decoded: 3,
            frames_dropped: 0,
            current_fps: 60,
            bitrate_mbps: 20,
            media_probe_valid: true,
            media_probe_format: "h264",
            media_probe_width: 1728,
            media_probe_height: 1080,
            media_probe_target_fps: 60,
            media_probe_target_bitrate_mbps: 20,
            media_probe_payload_bytes: 55555,
            last_media_sequence: 3,
            last_media_timestamp_us: 123456,
            last_media_payload_hash: "fnv1a64:abc123",
            last_error: null,
          })
        ),
      }),
      [
        {
          id: "display",
          platform: "windows",
          source_kind: "display",
          title: "DISPLAY1",
          class_name: "Monitor",
          width: 2560,
          height: 1600,
          process_id: 0,
          app_name: null,
        },
      ]
    );

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("skipped");
    expect(result.failureReason).toBe("profile_downgraded");
    expect(result.profileProbeResult?.status).toBe("degraded");
    expect(result.errorMessage).toContain("Runtime media profile downgraded");
    expect(result.errorMessage).toContain("1728x1080 @ 60 FPS / 20 Mbps");
  });

  it("keeps sampling downgraded profiles until the minimum sample duration", async () => {
    let currentTime = 0;
    const ipcProbeSnapshot = vi.fn().mockImplementation(() => {
      currentTime += currentTime === 0 ? 10 : 60;
      return Promise.resolve(
        ok({
          session_id: "unused",
          frames_received: 24,
          frames_decoded: 24,
          frames_dropped: 0,
          current_fps: 60,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_format: "h264",
          media_probe_width: 1728,
          media_probe_height: 1080,
          media_probe_target_fps: 60,
          media_probe_target_bitrate_mbps: 20,
          media_probe_payload_bytes: 55555,
          last_media_sequence: 24,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      );
    });
    const commands = withCaptureSourceCommands(createCommands({ ipcProbeSnapshot }), [
      {
        id: "display",
        platform: "windows",
        source_kind: "display",
        title: "DISPLAY1",
        class_name: "Monitor",
        width: 2560,
        height: 1600,
        process_id: 0,
        app_name: null,
      },
    ]);

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      sampleIntervalMs: 0,
      timeoutMs: 500,
      minSampleDurationMs: 50,
      minDecodedFrames: 1,
      minFps: 1,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("skipped");
    expect(result.failureReason).toBe("profile_downgraded");
    expect(result.thresholds.minSampleDurationMs).toBe(50);
    expect(result.sampleDurationMs).toBeGreaterThanOrEqual(50);
    expect(ipcProbeSnapshot).toHaveBeenCalledTimes(2);
  });

  it("retries LAN discovery during preflight until the target peer appears", async () => {
    const ipcRefreshLanDiscovery = vi
      .fn()
      .mockResolvedValueOnce(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [],
        })
      )
      .mockResolvedValueOnce(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 20,
          peers: [
            {
              device_id: "agent-device",
              device_name: "Agent PC",
              device_type: "desktop",
              ip: "192.168.1.24",
              discovery_port: 37777,
              p2p_control_addr: "192.168.1.24:37778",
              transports: [
                "quic",
                "quic_datagram",
                "quic_datagram_2k144",
                "quic_datagram_media_v2",
                "quic_datagram_media_v3",
                "media_profile_control_v1",
              ],
              protocol_version: 1,
              service_build_id: "test-build",
              media_protocol_version: 3,
              media_capabilities: [
                "quic_datagram_media_v3",
                "dxgi_capture",
                "nvenc_h264",
                "nvdec",
                "d3d11_native_render",
              ],
              age_ms: 10,
              p2p_available: true,
            },
          ],
        })
      );
    const commands = createCommands({ ipcRefreshLanDiscovery });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.peer?.device_id).toBe("agent-device");
    expect(ipcRefreshLanDiscovery).toHaveBeenCalledTimes(2);
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      H264_FALLBACK_REQUESTED_PROFILE
    );
  });

  it("falls back to the H.264 compatibility profile when the peer lacks HEVC media capabilities", async () => {
    const ipcRefreshLanDiscovery = vi.fn().mockResolvedValue(
      ok({
        enabled: true,
        running: true,
        discovery_port: 37777,
        instance_id: "controller-instance",
        last_probe_ms: 10,
        peers: [
          {
            device_id: "agent-device",
            device_name: "Agent PC",
            device_type: "desktop",
            ip: "192.168.1.24",
            discovery_port: 37777,
            p2p_control_addr: "192.168.1.24:37778",
            transports: [
              "quic",
              "quic_datagram",
              "quic_datagram_2k144",
              "quic_datagram_media_v2",
              "quic_datagram_media_v3",
              "media_profile_control_v1",
            ],
            protocol_version: 1,
            service_build_id: "test-build",
            media_protocol_version: 3,
            media_capabilities: [
              "quic_datagram_media_v3",
              "dxgi_capture",
              "nvenc_h264",
              "nvdec",
              "d3d11_native_render",
            ],
            age_ms: 20,
            p2p_available: true,
          },
        ],
      })
    );
    const commands = createCommands({ ipcRefreshLanDiscovery });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.requestedProfile).toEqual(H264_FALLBACK_REQUESTED_PROFILE);
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      H264_FALLBACK_REQUESTED_PROFILE
    );
  });

  it("accepts macOS native media capability profiles for QUIC LAN E2E", async () => {
    const commands = createCommands({
      ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [
            {
              device_id: "agent-device",
              device_name: "Agent Mac",
              device_type: "desktop",
              ip: "192.168.1.24",
              discovery_port: 37777,
              p2p_control_addr: "192.168.1.24:37778",
              transports: [
                "quic",
                "quic_datagram",
                "quic_datagram_2k144",
                "quic_datagram_media_v2",
                "quic_datagram_media_v3",
                "media_profile_control_v1",
              ],
              protocol_version: 1,
              service_build_id: "test-build",
              media_protocol_version: 3,
              media_capabilities: [
                "quic_datagram_media_v3",
                "macos_capture",
                "videotoolbox_h264",
                "videotoolbox_hevc",
                "videotoolbox",
                "media.hevc_main_420_8bit",
                "macos_native_render",
              ],
              age_ms: 10,
              p2p_available: true,
            },
          ],
        })
      ),
      ipcProbeSnapshot: vi.fn().mockResolvedValue(
        ok({
          session_id: "unused",
          frames_received: 4,
          frames_decoded: 3,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 40,
          media_probe_valid: true,
          media_probe_format: "compressed_hevc_test_pattern",
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 40,
          media_probe_payload_bytes: 55555,
          last_media_sequence: 3,
          last_media_timestamp_us: 123456,
          last_media_payload_hash: "fnv1a64:abc123",
          last_error: null,
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.requestedProfile).toEqual(MACOS_HEVC_2K144_REQUESTED_PROFILE);
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      MACOS_HEVC_2K144_REQUESTED_PROFILE
    );
  });

  it("skips paired media canaries when the LAN peer build does not match", async () => {
    const commands = createCommands();

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      expectedPeerBuildId: "newer-build",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("skipped");
    expect(result.failureReason).toBe("peer_version_mismatch");
    expect(result.errorMessage).toContain("expected newer-build");
    expect(result.errorMessage).toContain("got test-build");
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
  });

  it("treats legacy QUIC peers without datagram media capability as not ready", async () => {
    const commands = createCommands({
      ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [
            {
              device_id: "agent-device",
              device_name: "Agent PC",
              device_type: "desktop",
              ip: "192.168.1.24",
              discovery_port: 37777,
              p2p_control_addr: "192.168.1.24:37778",
              transports: ["quic"],
              protocol_version: 1,
              age_ms: 20,
              p2p_available: true,
            },
          ],
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("peer_not_ready");
    expect(result.errorMessage).toContain("quic_datagram");
    expect(result.errorMessage).toContain("Rebuild and restart");
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
  });

  it("treats QUIC datagram peers without the 2K144 media profile as not ready", async () => {
    const commands = createCommands({
      ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [
            {
              device_id: "agent-device",
              device_name: "Agent PC",
              device_type: "desktop",
              ip: "192.168.1.24",
              discovery_port: 37777,
              p2p_control_addr: "192.168.1.24:37778",
              transports: ["quic", "quic_datagram"],
              protocol_version: 1,
              age_ms: 20,
              p2p_available: true,
            },
          ],
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("peer_not_ready");
    expect(result.errorMessage).toContain("quic_datagram_2k144");
    expect(result.errorMessage).toContain("Rebuild and restart");
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
  });

  it("registers the local device before LAN session startup when the service runtime is unregistered", async () => {
    const commands = createCommands({
      ipcRuntimeSnapshot: vi.fn().mockResolvedValue(
        ok({
          device_id: null,
          is_registered: false,
          sessions: [],
        })
      ),
      getHardwareInfo: vi.fn().mockResolvedValue(
        ok({
          motherboard_serial: "MB-1234/5678",
          hostname: "Controller PC",
          os_type: "windows",
          os_version: "Windows",
          cpu_info: {
            name: "CPU",
            vendor_id: "GenuineIntel",
            cores: 8,
          },
          total_memory_mb: 16384,
          gpu_info: [],
        })
      ),
      ipcRegisterDevice: vi.fn().mockResolvedValue(ok("lan-MB12345678")),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      minDecodedFrames: 1,
      minFps: 1,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.controllerDeviceId).toBe("lan-MB12345678");
    expect(commands.ipcRegisterDevice).toHaveBeenCalledWith(
      "lan-MB12345678",
      "Controller PC"
    );
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic",
      DEFAULT_REQUESTED_PROFILE
    );
  });

  it("fails before session startup when no LAN peer is available", async () => {
    const commands = createCommands({
      ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [],
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("peer_not_found");
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
    expect(commands.openRemoteDisplayWindow).not.toHaveBeenCalled();
    expect(commands.ipcStopSession).not.toHaveBeenCalled();
  });

  it("reports discovered peers as not ready when P2P or transport support is missing", async () => {
    const commands = createCommands({
      ipcRefreshLanDiscovery: vi.fn().mockResolvedValue(
        ok({
          enabled: true,
          running: true,
          discovery_port: 37777,
          instance_id: "controller-instance",
          last_probe_ms: 10,
          peers: [
            {
              device_id: "agent-device",
              device_name: "Agent PC",
              device_type: "desktop",
              ip: "192.168.1.24",
              discovery_port: 37777,
              p2p_control_addr: "",
              transports: ["webrtc"],
              protocol_version: 1,
              age_ms: 20,
              p2p_available: true,
            },
          ],
        })
      ),
    });

    const result = await runLanE2EAutomation(commands, {
      targetDeviceId: "agent-device",
      transportKind: "quic",
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("peer_not_ready");
    expect(result.peer?.device_id).toBe("agent-device");
    expect(commands.ipcStartLanRemoteSession).not.toHaveBeenCalled();
    expect(commands.ipcStopSession).not.toHaveBeenCalled();
  });

  it("marks command failures with a structured failure reason", async () => {
    const commands = createCommands({
      ipcStartReceiver: vi.fn().mockResolvedValue(err("receiver unavailable")),
    });

    const result = await runLanE2EAutomation(commands, {
      sampleIntervalMs: 0,
      timeoutMs: 100,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("failed");
    expect(result.failureReason).toBe("receiver_start_failed");
    expect(result.errorMessage).toContain("receiver unavailable");
    expect(commands.ipcStopSession).toHaveBeenCalledWith("lan-e2e-test-session");
  });
});
