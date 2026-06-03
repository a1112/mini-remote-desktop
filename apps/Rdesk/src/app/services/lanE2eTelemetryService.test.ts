import { describe, expect, it } from "vitest";
import type { LanE2EAutomationReport } from "./lanE2eAutomationService";
import { summaryFromLanE2EReport } from "./lanE2eTelemetryService";

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
