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
            transports: ["quic"],
            protocol_version: 1,
            age_ms: 20,
            p2p_available: true,
          },
        ],
      })
    ),
    ipcStartLanRemoteSession: vi.fn().mockResolvedValue(ok("session-started")),
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
        current_fps: 24,
        bitrate_mbps: 8,
        last_error: null,
      })
    ),
    ipcStopSession: vi.fn().mockResolvedValue(ok("stopped")),
    ...overrides,
  };
}

describe("runLanE2EAutomation", () => {
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
    expect(commands.ipcStartLanRemoteSession).toHaveBeenCalledWith(
      "lan-e2e-test-session",
      "agent-device",
      "quic"
    );
    expect(commands.ipcStartReceiver).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(commands.openRemoteDisplayWindow).toHaveBeenCalledWith({
      sessionId: "lan-e2e-test-session",
    });
    expect(commands.ipcStopSession).toHaveBeenCalledWith("lan-e2e-test-session");
    expect(result.stages.map((stage) => `${stage.stage}:${stage.status}`)).toContain(
      "assert:completed"
    );
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
      "quic"
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
