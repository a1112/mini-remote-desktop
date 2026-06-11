import { describe, expect, it } from "vitest";
import type { LanE2EAutomationReport } from "./lanE2eAutomationService";
import {
  mainlineE2EArtifactPayloadFromReport,
  scriptClassificationFromLanE2EReport,
  summaryFromLanE2EReport,
} from "./lanE2eTelemetryService";

describe("summaryFromLanE2EReport", () => {
  it("prefers target-duration sample FPS over elapsed sample FPS", () => {
    const report = {
      status: "completed",
      scenarioId: "lan.e2e.remote_display",
      validationMode: "quic_datagram",
      dataPlaneVerified: true,
      mediaVerified: true,
      startedAt: 0,
      finishedAt: 30_250,
      sampleDurationMs: 30_250,
      sampleObservedFps: 142.9,
      sampleObservedFpsAtTargetDuration: 144,
      sampleObservedRenderFps: 141.9,
      sampleObservedRenderFpsAtTargetDuration: 143,
      sampleFramesDecoded: 4320,
      sampleFramesDropped: 0,
      sampleRenderFramesPresented: 4320,
      sampleRenderQueueReplacements: 12,
      sampleRenderPresentSkips: 3,
      mediaPipelineSnapshot: {
        session_id: "lan-e2e-test-session",
        attached_surfaces: [],
        queue_depth: 0,
        dropped_frames: 0,
        render_presented_frames: 4_500,
        render_queue_replacements: 99,
        render_present_skips: 30,
        stage_metrics: [],
      },
      faultEvents: [],
      thresholds: {
        minSampleDurationMs: 30_000,
        minDecodedFrames: 1,
        minFps: 144,
      },
      stages: [],
    } as LanE2EAutomationReport;

    const summary = summaryFromLanE2EReport(report);
    expect(summary.capture_fps).toBe(144);
    expect(summary.render_fps).toBe(143);
    expect(summary.render_frame_count).toBe(4320);
    expect(summary.render_queue_replacements).toBe(12);
    expect(summary.render_queue_replacement_ratio).toBeCloseTo(12 / 4320);
    expect(summary.render_present_skips).toBe(3);
    expect(summary.render_present_skip_ratio).toBeCloseTo(3 / 4320);
  });
});

describe("mainlineE2EArtifactPayloadFromReport", () => {
  it("builds the canonical artifact payload for LAN E2E reports", () => {
    const report = {
      status: "completed",
      scenarioId: "cross.e2e.remote_display_smoke",
      sessionId: "lan-e2e-agent-device-123",
      controllerDeviceId: "controller-device",
      peer: {
        device_id: "agent-device",
        device_name: "Agent",
        device_type: "windows",
        ip: "192.168.1.20",
        discovery_port: 21116,
        p2p_control_addr: "192.168.1.20:21117",
        transports: ["quic"],
        protocol_version: 1,
        service_build_id: "test-build",
        media_protocol_version: 3,
        media_capabilities: ["dxgi_capture", "nvenc_h264", "nvdec", "d3d11_native_render"],
        age_ms: 10,
        p2p_available: true,
      },
      requestedProfile: {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        codec: "h264",
      },
      mediaAdaptationSnapshot: {
        enabled: true,
        session_id: "lan-e2e-agent-device-123",
        state: "stable",
        ladder_index: 0,
        current_profile: {
          width: 1280,
          height: 720,
          fps: 60,
          bitrate_mbps: 8,
          codec: "h264",
        },
        target_profile: {
          width: 1920,
          height: 1080,
          fps: 60,
          bitrate_mbps: 20,
          codec: "h264",
        },
        last_reason: "network",
        last_change_ms: 1,
        observed_fps: 58,
        drop_ratio: 0.01,
        queue_depth: 1,
      },
      sessionSnapshot: {
        session_id: "lan-e2e-agent-device-123",
        state: "streaming",
        role: "controller",
        transport_kind: "quic",
        receiver_active: true,
        sender_active: true,
      },
      probeSnapshot: {
        session_id: "lan-e2e-agent-device-123",
        frames_received: 121,
        current_fps: 60,
        frames_decoded: 120,
        frames_dropped: 1,
        media_probe_valid: true,
      },
      mediaPipelineSnapshot: {
        session_id: "lan-e2e-agent-device-123",
        attached_surfaces: [],
        queue_depth: 1,
        dropped_frames: 1,
        render_presented_frames: 120,
        render_queue_replacements: 2,
        render_present_skips: 3,
        stage_metrics: [],
      },
      validationMode: "quic_datagram",
      dataPlaneVerified: true,
      mediaVerified: true,
      startedAt: Date.UTC(2026, 5, 11, 1, 2, 3),
      finishedAt: Date.UTC(2026, 5, 11, 1, 2, 33),
      sampleDurationMs: 30_000,
      sampleFramesDecoded: 118,
      sampleFramesDropped: 1,
      sampleRenderFramesPresented: 117,
      sampleObservedFps: 59,
      sampleObservedRenderFps: 58.5,
      firstFrameAt: Date.UTC(2026, 5, 11, 1, 2, 8),
      firstFrameTimeMs: 5_000,
      maxZeroFrameWindowAfterFirstFrameMs: 1_200,
      sampleRenderQueueReplacements: 2,
      sampleRenderPresentSkips: 3,
      thresholds: {
        minSampleDurationMs: 30_000,
        minDecodedFrames: 20,
        minFps: 2,
      },
      faultEvents: [],
      stages: [
        {
          stage: "preflight",
          status: "completed",
          timestamp: Date.UTC(2026, 5, 11, 1, 2, 4),
        },
      ],
    } as LanE2EAutomationReport;

    const payload = mainlineE2EArtifactPayloadFromReport(
      report,
      {
        capture_type: "dxgi",
        encoder_type: "nvenc_h264",
        decoder_type: "nvdec",
        renderer_type: "d3d11",
        transport_kind: "quic",
      },
      {
        environment: {
          os_type: "windows",
          cpu_brand: "CPU",
          cpu_cores: 8,
          memory_gb: 32,
          gpu_info: "GPU",
          available_encoders: ["nvenc_h264"],
          available_decoders: ["nvdec"],
          available_renderers: ["d3d11"],
        },
        gitCommit: "abc123",
        runIdPrefix: "e2e-lan",
      }
    );

    expect(payload.kind).toBe("mainline_e2e_artifacts_v1");
    expect(payload.run_id).toBe("e2e-lan-lan-e2e-agent-device-123");
    expect(payload.artifact_date).toBe("2026-06-11");
    expect(payload.git_commit).toBe("abc123");
    expect(payload.script_classification).toBe("completed");
    expect(payload.controller.device_id).toBe("controller-device");
    expect(payload.agent?.service_build_id).toBe("test-build");
    expect(payload.selected_profile).toEqual(
      expect.objectContaining({ width: 1280, height: 720, bitrate_mbps: 8 })
    );
    expect(payload.first_frame_time_ms).toBe(5_000);
    expect(payload.max_zero_frame_window_after_first_frame_ms).toBe(1_200);
    expect(payload.summary.first_frame_latency_ms).toBe(5_000);
    expect(payload.classification.run_scope).toBe("cross_device");
    expect(payload.metric_series[0]).toEqual(
      expect.objectContaining({
        frames_decoded: 118,
        render_frames_presented: 117,
        queue_depth: 1,
        first_frame_time_ms: 5_000,
        max_zero_frame_window_after_first_frame_ms: 1_200,
      })
    );
    expect(payload.metrics_csv).toContain("timestamp,sample_duration_ms,frames_decoded");
    expect(payload.metrics_csv).toContain(
      "first_frame_time_ms,max_zero_frame_window_after_first_frame_ms"
    );
    expect(payload.metrics_csv).toContain(",5000,1200");
    expect(payload.artifacts).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ path: "summary.json", required: true, status: "generated" }),
        expect.objectContaining({ path: "timeline.json", required: true, status: "generated" }),
        expect.objectContaining({ path: "metrics.csv", required: true, status: "generated" }),
        expect.objectContaining({ path: "controller.log", required: true, status: "generated" }),
        expect.objectContaining({ path: "agent.log", required: true, status: "generated" }),
        expect.objectContaining({ path: "first-frame.png", required: false, status: "missing" }),
        expect.objectContaining({ path: "last-frame.png", required: false, status: "missing" }),
      ])
    );
    expect(payload.report).toBe(report);
  });
});

describe("scriptClassificationFromLanE2EReport", () => {
  it("matches paired LAN canary failure classes", () => {
    expect(
      scriptClassificationFromLanE2EReport({
        status: "failed",
        failureReason: "no_remote_frames",
      })
    ).toBe("threshold_miss");
    expect(
      scriptClassificationFromLanE2EReport({
        status: "failed",
        failureReason: "runtime_error",
        errorMessage: "NVDEC decode failed",
      })
    ).toBe("decode_error");
    expect(
      scriptClassificationFromLanE2EReport({
        status: "failed",
        failureReason: "session_start_failed",
      })
    ).toBe("transport_loss");
    expect(
      scriptClassificationFromLanE2EReport({
        status: "skipped",
        failureReason: "peer_not_found",
      })
    ).toBe("unsupported");
  });
});
