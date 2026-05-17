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
  height: 1440,
  fps: 144,
  bitrate_mbps: 64,
  codec: "h264",
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
    height: 1440,
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
    height: 1440,
    process_id: 0,
    app_name: null,
  },
];

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
          height: 1440,
          refresh_hz: 60,
          bit_depth: 32,
          is_current: true,
        },
        {
          id: "mode-target",
          source_id: "display-shared",
          width: 2560,
          height: 1440,
          refresh_hz: 144,
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
          height: 1440,
          refresh_hz: 144,
          bit_depth: 32,
          is_current: false,
        },
        previous: {
          id: "mode-current",
          source_id: "display-shared",
          width: 2560,
          height: 1440,
          refresh_hz: 60,
          bit_depth: 32,
          is_current: true,
        },
        active: {
          id: "mode-target",
          source_id: "display-shared",
          width: 2560,
          height: 1440,
          refresh_hz: 144,
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
        current_fps: 144,
        bitrate_mbps: 64,
        media_probe_valid: true,
        media_probe_format: "compressed_2k144_test_pattern",
        media_probe_width: 2560,
        media_probe_height: 1440,
        media_probe_target_fps: 144,
        media_probe_target_bitrate_mbps: 64,
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
        attached_surfaces: [],
        active_decoder: "nvdec",
        active_renderer: "d3d11",
        queue_depth: 1,
        dropped_frames: 0,
        stage_metrics: [
          { stage: "decode", p50_ms: 0.8, p95_ms: 1.2 },
          { stage: "render_present", p50_ms: 5.0, p95_ms: 7.0 },
        ],
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
    });
    expect(commands.ipcStopSession).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "assert:completed"
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
    expect(result.errorMessage).toContain("2560x1440 @ 144 FPS / 64 Mbps");
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
    const decodedFrames = [10, 16, 17, 18];
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
      timeoutMs: 200,
      minSampleDurationMs: 100,
      minDecodedFrames: 1,
      minFps: 50,
      now: () => currentTime,
      createSessionId: () => "lan-e2e-test-session",
    });

    expect(result.status).toBe("completed");
    expect(result.sampleFramesDecoded).toBe(6);
    expect(result.sampleObservedFps).toBeGreaterThanOrEqual(50);
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
      DEFAULT_REQUESTED_PROFILE
    );
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
