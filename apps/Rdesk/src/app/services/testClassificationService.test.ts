import { describe, expect, it } from "vitest";
import type { EnvironmentSnapshot, TestConfig, TestRun } from "../adapters/tauri/types";
import {
  classificationForRun,
  deriveTestClassification,
  groupRowsByClassification,
  performanceRowFromRun,
} from "./testClassificationService";

const env: EnvironmentSnapshot = {
  os_type: "windows",
  cpu_brand: "Intel",
  cpu_cores: 16,
  memory_gb: 32,
  gpu_info: "NVIDIA RTX",
  available_captures: ["dxgi"],
  available_encoders: ["nvenc_h264", "openh264"],
  available_decoders: ["nvdec", "software"],
  available_renderers: ["d3d11", "webview"],
  available_memory_modes: ["cpu", "d3d11_shared"],
};

describe("testClassificationService", () => {
  it("classifies local DXGI/NVENC/NVDEC zero-copy as hardware native", () => {
    const config: TestConfig = {
      capture_type: "dxgi",
      encoder_type: "nvenc_h264",
      decoder_type: "nvdec",
      renderer_type: "d3d11",
      render_display: true,
      zero_copy: true,
      transport_kind: "loopback",
    };

    expect(deriveTestClassification(config, env)).toMatchObject({
      run_scope: "local",
      memory_path: "zero_copy_d3d11_shared",
      encode_accel: "hardware",
      decode_accel: "hardware",
      transport_path: "loopback",
      render_path: "native_d3d11",
    });
  });

  it("classifies browser WebRTC decode as WebRTC MediaStream", () => {
    const config: TestConfig = {
      capture_type: "dxgi",
      encoder_type: "nvenc_h264",
      decoder_type: "none",
      render_display: false,
      zero_copy: false,
      transport_kind: "webrtc",
      resolution: [1920, 1080],
      fps: 144,
      bitrate: 8_000_000,
    };

    expect(deriveTestClassification(config, env)).toMatchObject({
      memory_path: "webrtc_media_stream",
      encode_accel: "hardware",
      decode_accel: "browser",
      transport_path: "webrtc",
      render_path: "browser_video",
    });
  });

  it("classifies software OpenH264 and groups comparison rows", () => {
    const run: TestRun = {
      run_id: "run-1",
      scenario_id: "matrix",
      run_mode: "matrix",
      status: "completed",
      started_at: 1000,
      config_snapshot: {
        capture_type: "synthetic",
        encoder_type: "openh264",
        decoder_type: "software",
        transport_kind: "loopback",
        resolution: [1280, 720],
        fps: 60,
      },
      environment_snapshot: env,
      summary: {
        total_duration_ms: 5000,
        capture_fps: 58,
        total_latency_p95: 18,
        dropped_frames: 2,
        frame_count: 300,
      },
    };

    expect(classificationForRun(run)).toMatchObject({
      encode_accel: "software",
      decode_accel: "software",
      memory_path: "cpu_copy",
    });

    const row = performanceRowFromRun(run);
    expect(row.dropRatePct).toBeCloseTo(0.662, 2);
    expect(groupRowsByClassification([row])[0]).toMatchObject({
      count: 1,
      fpsAvg: 58,
      latencyP95Ms: 18,
    });
  });

  it("classifies FFmpeg decode backends as software acceleration", () => {
    for (const decoder of ["ffmpeg_h264", "ffmpeg_hevc"] as const) {
      expect(
        deriveTestClassification(
          {
            capture_type: "dxgi",
            encoder_type: "nvenc_h264",
            decoder_type: decoder,
            renderer_type: "d3d11",
            render_display: true,
            transport_kind: "loopback",
          },
          {
            ...env,
            available_decoders: [decoder],
          }
        )
      ).toMatchObject({
        decode_accel: "software",
      });
    }
  });

  it("classifies cross-device peer metadata", () => {
    const classification = deriveTestClassification(
      {
        encoder_type: "nvenc_h264",
        decoder_type: "nvdec",
        renderer_type: "d3d11",
        render_display: true,
        transport_kind: "quic",
      },
      env,
      {
        runScope: "cross_device",
        peer: {
          device_id: "peer-1",
          device_name: "Receiver",
          device_type: "windows",
          ip: "192.168.1.10",
          discovery_port: 3999,
          p2p_control_addr: "192.168.1.10:3999",
          transports: ["quic_datagram_media_v3"],
          protocol_version: 1,
          media_protocol_version: 3,
          age_ms: 10,
          p2p_available: true,
        },
      }
    );

    expect(classification.run_scope).toBe("cross_device");
    expect(classification.peer_device?.device_name).toBe("Receiver");
    expect(classification.transport_path).toBe("quic");
  });
});
