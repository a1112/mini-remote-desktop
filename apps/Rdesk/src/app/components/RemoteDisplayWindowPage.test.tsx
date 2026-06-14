import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../test/mocks/tauri";
import {
  RemoteDisplayWindowPage,
  applyWebRtcReceiverLowLatencyHint,
  applyWebRtcVideoMotionHint,
  webCodecsMemoryPathLabelFromState,
  webPreviewDecoderLabel,
  webPreviewTransportLabel,
  browserSupportsWebCodecsWorkerRendering,
  browserWebrtcPreviewH264Profile,
  browserSupportsWebrtcVideoCodec,
  encoderForRequestedProfileCodec,
  resolveLocalWebViewPlan,
  webRtcPreviewCodecForEncoder,
  webCodecsPreviewCodecForEncoder,
  buildWebCodecsDecoderConfig,
  buildWebRtcDiagnosticsStageRows,
  WebRtcPresentationLatencyTracker,
  localThreeFrameLatencyStatus,
  shouldAutoSwitchWebRtcVideoToWebCodecs,
  summarizeWebRtcInboundVideoStats,
} from "./RemoteDisplayWindowPage";

const runtimeMock = vi.hoisted(() => ({ isTauri: true }));

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => runtimeMock.isTauri,
}));

vi.mock("../utils/tauriWindow", () => ({
  withTauriWindow: (fn: (appWindow: {
    isMaximized: () => Promise<boolean>;
    minimize: () => Promise<void>;
    close: () => Promise<void>;
    startDragging: () => Promise<void>;
    toggleMaximize: () => Promise<void>;
  }) => Promise<unknown> | unknown) =>
    fn({
      isMaximized: () => Promise.resolve(false),
      minimize: () => Promise.resolve(undefined),
      close: () => Promise.resolve(undefined),
      startDragging: () => Promise.resolve(undefined),
      toggleMaximize: () => Promise.resolve(undefined),
    }),
}));

function renderRemoteDisplay(sessionId = "p2p-quic-123", search = "?surface=surface-1") {
  render(
    <MemoryRouter initialEntries={[`/display/${sessionId}${search}`]}>
      <Routes>
        <Route path="/display/:id" element={<RemoteDisplayWindowPage />} />
      </Routes>
    </MemoryRouter>
  );
}

function mockRenderAreaRect() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 56,
    left: 0,
    top: 56,
    right: 1280,
    bottom: 776,
    width: 1280,
    height: 720,
    toJSON: () => ({}),
  } as DOMRect);
}

function mockResizeObserver() {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
  );
}

function windowsCapabilities() {
  return {
    os_type: "windows",
    cpu_brand: "Intel",
    cpu_cores: 16,
    memory_gb: 32,
    gpu_info: "NVIDIA",
    available_captures: ["dxgi", "winrt", "synthetic"],
    available_encoders: ["nvenc_h264", "nvenc_hevc", "nvenc_hevc_main10", "nvenc_av1", "openh264"],
    available_decoders: ["nvdec", "software"],
    available_renderers: ["d3d11"],
    available_memory_modes: ["cpu", "d3d11_shared"],
  };
}

function windowsOpenH264OnlyCapabilities() {
  return {
    ...windowsCapabilities(),
    available_encoders: ["openh264"],
    available_decoders: ["software"],
  };
}

function linuxCapabilities() {
  return {
    os_type: "linux",
    cpu_brand: "AMD",
    cpu_cores: 16,
    memory_gb: 32,
    gpu_info: "NVIDIA",
    available_captures: ["linux", "synthetic"],
    available_encoders: ["none", "openh264", "nvenc_h264"],
    available_decoders: ["none", "software", "linux_h264"],
    available_renderers: ["none", "linux", "webview"],
    available_memory_modes: ["cpu"],
  };
}

function macosCapabilities() {
  return {
    os_type: "macos",
    cpu_brand: "Apple",
    cpu_cores: 8,
    memory_gb: 16,
    gpu_info: "Apple GPU",
    available_captures: ["macos", "synthetic"],
    available_encoders: ["videotoolbox_hevc", "videotoolbox_h264", "openh264"],
    available_decoders: ["videotoolbox_h264", "videotoolbox_hevc", "software"],
    available_renderers: ["macos"],
    available_memory_modes: ["cpu"],
  };
}

function macosHevcWithH264DecodeOnlyCapabilities() {
  return {
    ...macosCapabilities(),
    available_decoders: ["videotoolbox_h264", "software"],
  };
}

const remoteDisplaySource = {
  id: "windows:display-shared:0",
  platform: "windows",
  source_kind: "display_shared",
  title: "Display 1 (D3D11 shared copy)",
  class_name: "WinRTMonitorShared",
  width: 2560,
  height: 1440,
  process_id: 0,
  app_name: "Display",
  bundle_identifier: null,
  preview_data_url: null,
  preview_width: null,
  preview_height: null,
};

const localDisplaySources = [
  {
    ...remoteDisplaySource,
    id: "windows:display-shared:0",
    title: "Display 1 (D3D11 shared copy)",
    width: 2560,
    height: 1440,
    preview_data_url: null,
    preview_width: null,
    preview_height: null,
  },
  {
    ...remoteDisplaySource,
    id: "windows:display-shared:1",
    title: "Display 2 (D3D11 shared copy)",
    class_name: "DXGIShared:\\\\.\\DISPLAY2",
    width: 3840,
    height: 2160,
    preview_data_url: null,
    preview_width: null,
    preview_height: null,
  },
];

const localWindowSource = {
  ...remoteDisplaySource,
  id: "windows:window:0x1234",
  source_kind: "window",
  title: "Calculator",
  class_name: "ApplicationFrameWindow",
  width: 900,
  height: 700,
  process_id: 4242,
  app_name: "Calculator",
  preview_data_url: null,
  preview_width: null,
  preview_height: null,
};

const localMixedSources = [...localDisplaySources, localWindowSource];

function lanDiscoverySnapshotWithPeerInput() {
  return {
    enabled: true,
    running: true,
    discovery_port: 49700,
    instance_id: "local-instance",
    peers: [
      {
        device_id: "target-device",
        device_name: "Target",
        device_type: "desktop",
        ip: "127.0.0.1",
        discovery_port: 49700,
        p2p_control_addr: "127.0.0.1:49701",
        transports: ["quic"],
        protocol_version: 1,
        media_protocol_version: 1,
        media_capabilities: ["control.keyboard_mouse"],
        age_ms: 10,
        p2p_available: true,
      },
    ],
  };
}

function defaultRemoteDisplayInvoke(command: string): Promise<unknown> {
  if (command === "ipc_lan_discovery_snapshot") {
    return Promise.resolve(lanDiscoverySnapshotWithPeerInput());
  }
  return Promise.resolve(null);
}

describe("RemoteDisplayWindowPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    getMockInvoke().mockReset();
    runtimeMock.isTauri = true;
    mockRenderAreaRect();
    mockResizeObserver();
  });

  it("summarizes WebRTC inbound video stats for stutter diagnosis", () => {
    const firstReport = new Map<string, Record<string, unknown>>([
      [
        "inbound-video",
        {
          type: "inbound-rtp",
          kind: "video",
          timestamp: 1_000,
          framesDecoded: 1_000,
          framesDropped: 2,
          packetsLost: 0,
          jitter: 0.002,
          jitterBufferDelay: 0.5,
          jitterBufferEmittedCount: 1_000,
          totalDecodeTime: 0.3,
          totalProcessingDelay: 0.8,
          totalInterFrameDelay: 8.2,
          freezeCount: 0,
        },
      ],
    ]) as unknown as RTCStatsReport;
    const first = summarizeWebRtcInboundVideoStats(firstReport, null, 1_000);

    const secondReport = new Map<string, Record<string, unknown>>([
      [
        "inbound-video",
        {
          type: "inbound-rtp",
          kind: "video",
          timestamp: 2_000,
          framesDecoded: 1_120,
          framesDropped: 3,
          packetsLost: 1,
          jitter: 0.003,
          jitterBufferDelay: 0.68,
          jitterBufferEmittedCount: 1_120,
          totalDecodeTime: 0.42,
          totalProcessingDelay: 1.16,
          totalInterFrameDelay: 9.2,
          freezeCount: 1,
        },
      ],
    ]) as unknown as RTCStatsReport;
    const second = summarizeWebRtcInboundVideoStats(secondReport, first.counters, 2_000);

    expect(second.stats?.decodedFps).toBeCloseTo(120);
    expect(second.stats?.decodeAvgMs).toBeCloseTo(1);
    expect(second.stats?.jitterBufferDelayAvgMs).toBeCloseTo(1.5);
    expect(second.stats?.processingDelayAvgMs).toBeCloseTo(3);
    expect(second.stats?.interFrameDelayAvgMs).toBeCloseTo(8.33, 1);
    expect(second.stats?.framesDropped).toBe(3);

    const rows = buildWebRtcDiagnosticsStageRows(second.stats);
    expect(rows.map((row) => row.label)).toEqual([
      "webrtc.decode_avg",
      "webrtc.jitter_buffer_avg",
      "webrtc.processing_avg",
      "webrtc.render_interval_avg",
    ]);
  });

  it("selects H.264 High for NVENC browser preview independent of matrix decoder field", () => {
    expect(browserWebrtcPreviewH264Profile("nvenc_h264", "none")).toBe("high");
    expect(browserWebrtcPreviewH264Profile("nvenc_h264", "software")).toBe("high");
    expect(browserWebrtcPreviewH264Profile("openh264", "none")).toBe("baseline");
  });

  it("selects browser WebRTC codecs for hardware H.264, HEVC Main, and AV1", () => {
    expect(webRtcPreviewCodecForEncoder("nvenc_h264")).toBe("h264");
    expect(webRtcPreviewCodecForEncoder("openh264")).toBe("h264");
    expect(webRtcPreviewCodecForEncoder("nvenc_hevc")).toBe("hevc");
    expect(webRtcPreviewCodecForEncoder("nvenc_hevc_main10")).toBeNull();
    expect(webRtcPreviewCodecForEncoder("nvenc_av1")).toBe("av1");
  });

  it("does not auto-select unavailable AV1 encoders from an explicit profile codec", () => {
    expect(
      encoderForRequestedProfileCodec(
        "av1",
        "macos",
        ["videotoolbox_h264", "videotoolbox_hevc", "openh264"],
        null,
        null
      )
    ).toBeNull();
    expect(
      encoderForRequestedProfileCodec("av1", "macos", undefined, null, null)
    ).toBeNull();
  });

  it("does not select AV1 for macOS browser-preview local profiles", () => {
    const plan = resolveLocalWebViewPlan({
      capabilities: {
        os_type: "macos",
        cpu_brand: "Apple",
        cpu_cores: 8,
        memory_gb: 16,
        gpu_info: "Apple GPU",
        available_captures: ["macos"],
        available_encoders: ["videotoolbox_av1", "openh264"],
        available_decoders: ["software"],
        available_renderers: ["macos", "webview"],
        available_memory_modes: ["cpu"],
      },
      hostOs: "macos",
      webPreviewEngine: "webrtc",
      hevcWebRtcSupported: false,
      capture: "macos",
      encoder: "videotoolbox_av1",
      decoder: "none",
      transport: "webrtc",
      fps: "60",
      bitrate: "20",
      capHighFpsBitrate: false,
    });

    expect(plan.profile).toBeNull();
    expect(plan.reason).toContain("硬件浏览器预览编码器");
  });

  it("detects browser WebRTC HEVC receive capability beside H.264", () => {
    vi.stubGlobal("RTCRtpReceiver", {
      getCapabilities: () => ({
        codecs: [{ mimeType: "video/H265" }],
      }),
    });

    expect(browserSupportsWebrtcVideoCodec("hevc")).toBe(true);
    expect(browserSupportsWebrtcVideoCodec("h264")).toBe(false);
    expect(browserSupportsWebrtcVideoCodec("av1")).toBe(false);
  });

  it("selects HEVC for WebCodecs preview when the HEVC encoder is active", () => {
    expect(webCodecsPreviewCodecForEncoder("nvenc_h264")).toBe("h264");
    expect(webCodecsPreviewCodecForEncoder("nvenc_hevc")).toBe("hevc");
    expect(webCodecsPreviewCodecForEncoder("nvenc_hevc_main10")).toBe("hevc_main10");
  });

  it("builds HEVC WebCodecs decoder config without AVC metadata", () => {
    const config = buildWebCodecsDecoderConfig({
      type: "mrd.webcodecs.ready.v1",
      session_id: "s1",
      codec: "hev1.1.6.L156.B0",
      codec_format: "annexb",
      width: 2560,
      height: 1440,
      fps: 120,
      bitrate_mbps: 40,
    });

    expect(config).toMatchObject({ hevc: { format: "annexb" } });
    expect("avc" in config).toBe(false);
  });

  it("builds HEVC Main10 WebCodecs decoder config without AVC metadata", () => {
    const config = buildWebCodecsDecoderConfig({
      type: "mrd.webcodecs.ready.v1",
      session_id: "s1",
      codec: "hev1.2.4.L156.B0",
      codec_format: "annexb",
      width: 2560,
      height: 1440,
      fps: 120,
      bitrate_mbps: 40,
    });

    expect(config).toMatchObject({ hevc: { format: "annexb" } });
    expect("avc" in config).toBe(false);
  });

  it("marks browser WebRTC video tracks as motion content", () => {
    const track = { contentHint: "" } as MediaStreamTrack;

    applyWebRtcVideoMotionHint(track);

    expect(track.contentHint).toBe("motion");
  });

  it("applies low-latency playout hints to browser WebRTC receivers when supported", () => {
    const receiver = {
      jitterBufferTarget: 0.2,
      playoutDelayHint: 0.2,
    };

    applyWebRtcReceiverLowLatencyHint(receiver as unknown as RTCRtpReceiver);

    expect(receiver.jitterBufferTarget).toBeCloseTo(0.02);
    expect(receiver.playoutDelayHint).toBeCloseTo(0.02);
  });

  it("estimates capture-to-present latency from browser WebRTC frame timing metadata", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    tracker.addMetadata(
      JSON.stringify({
        type: "mrd.frame_timing.v1",
        sequence: 1,
        capture_unix_us: 1_000_010_000,
      })
    );
    tracker.addMetadata(
      JSON.stringify({
        type: "mrd.frame_timing.v1",
        sequence: 2,
        capture_unix_us: 1_000_030_000,
      })
    );

    const first = tracker.observeFrame(50, { presentedFrames: 1, presentationTime: 50 });
    const second = tracker.observeFrame(82, { presentedFrames: 2, presentationTime: 82 });

    expect(first?.latestMs).toBeCloseTo(40);
    expect(second?.latestMs).toBeCloseTo(52);
    expect(second?.p50Ms).toBeCloseTo(40);
    expect(second?.p95Ms).toBeCloseTo(52);
    expect(second?.samples).toBe(2);
  });

  it("catches up queued frame timing metadata on the first video callback", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    for (let sequence = 1; sequence <= 120; sequence += 1) {
      tracker.addMetadata(
        JSON.stringify({
          type: "mrd.frame_timing.v1",
          sequence,
          capture_unix_us: 1_000_000_000 + sequence * 8_333,
        })
      );
    }

    const stats = tracker.observeFrame(1_000, {
      presentedFrames: 120,
      presentationTime: 1_000,
    });

    expect(stats?.latestMs).toBeGreaterThanOrEqual(0);
    expect(stats?.latestMs).toBeLessThan(20);
  });

  it("orders unordered browser frame timing metadata by sequence before fallback matching", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    tracker.addMetadata(
      JSON.stringify({
        type: "mrd.frame_timing.v1",
        sequence: 2,
        capture_unix_us: 1_000_020_000,
      })
    );
    tracker.addMetadata(
      JSON.stringify({
        type: "mrd.frame_timing.v1",
        sequence: 1,
        capture_unix_us: 1_000_010_000,
      })
    );

    const first = tracker.observeFrame(40, { presentedFrames: 1, presentationTime: 40 });
    const second = tracker.observeFrame(50, { presentedFrames: 2, presentationTime: 50 });

    expect(first?.latestMs).toBeCloseTo(30);
    expect(second?.latestMs).toBeCloseTo(30);
  });

  it("drops stale startup frame timing metadata before estimating presentation latency", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    for (let sequence = 1; sequence <= 120; sequence += 1) {
      tracker.addMetadata(
        JSON.stringify({
          type: "mrd.frame_timing.v1",
          sequence,
          capture_unix_us: 1_000_000_000 + sequence * 8_333,
        })
      );
    }

    const stats = tracker.observeFrame(1_000, {
      presentedFrames: 1,
      presentationTime: 1_000,
    });

    expect(stats?.latestMs).toBeLessThan(220);
  });

  it("prefers native WebRTC video frame captureTime when the browser exposes it", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    const stats = tracker.observeFrame(120, {
      presentedFrames: 1,
      presentationTime: 120,
      captureTime: 91,
    });

    expect(stats?.latestMs).toBeCloseTo(29);
    expect(stats?.p95Ms).toBeCloseTo(29);
  });

  it("marks RTP timestamp matched frame timing as a precise E2E source", () => {
    const tracker = new WebRtcPresentationLatencyTracker({ timeOriginMs: 1_000_000 });

    tracker.addMetadata(
      JSON.stringify({
        type: "mrd.frame_timing.v1",
        sequence: 1,
        capture_unix_us: 1_000_040_000,
        rtp_timestamp: 1234,
      })
    );

    const stats = tracker.observeFrame(90, {
      presentedFrames: 1,
      presentationTime: 90,
      rtpTimestamp: 1234,
    });

    expect(stats?.latestMs).toBeCloseTo(50);
    expect(stats?.source).toBe("rtp_frame_timing_channel");
  });

  it("does not auto-switch explicit WebRTC video tests to WebCodecs", () => {
    expect(
      shouldAutoSwitchWebRtcVideoToWebCodecs({
        targetFps: 120,
        actualFps: 57,
        latencyP95Ms: 196,
        metadataAgeMs: 109,
        jitterBufferMs: 39,
        webCodecsAvailable: true,
        allowAutoSwitch: false,
        alreadyAttempted: false,
      })
    ).toEqual({ shouldSwitch: false, reason: null });
  });

  it("auto-switches production WebRTC video backlog to the separate WebCodecs web path", () => {
    expect(
      shouldAutoSwitchWebRtcVideoToWebCodecs({
        targetFps: 120,
        actualFps: 57,
        latencyP95Ms: 196,
        metadataAgeMs: 109,
        jitterBufferMs: 39,
        webCodecsAvailable: true,
        allowAutoSwitch: true,
        alreadyAttempted: false,
      })
    ).toEqual({
      shouldSwitch: true,
      reason:
        "WebRTC video backlog: p95 196.0 ms, metadata age 109.0 ms, fps 57.0/120. Switching to WebCodecs web path.",
    });
  });

  it("keeps WebRTC video when the WebCodecs web path is unavailable or already attempted", () => {
    expect(
      shouldAutoSwitchWebRtcVideoToWebCodecs({
        targetFps: 120,
        actualFps: 57,
        latencyP95Ms: 196,
        metadataAgeMs: 109,
        jitterBufferMs: 39,
        webCodecsAvailable: false,
        allowAutoSwitch: true,
        alreadyAttempted: false,
      }).shouldSwitch
    ).toBe(false);

    expect(
      shouldAutoSwitchWebRtcVideoToWebCodecs({
        targetFps: 120,
        actualFps: 57,
        latencyP95Ms: 196,
        metadataAgeMs: 109,
        jitterBufferMs: 39,
        webCodecsAvailable: true,
        allowAutoSwitch: true,
        alreadyAttempted: true,
      }).shouldSwitch
    ).toBe(false);
  });

  it("shows a green LAN diagnostics popover with HEVC and chroma metadata", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 900,
          frames_decoded: 896,
          frames_dropped: 2,
          current_fps: 144,
          bitrate_mbps: 78.4,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          attached_surfaces: [{ surface_id: "surface-1", backend: "d3d11", window_handle: 20 }],
          active_decoder: "nvdec_hevc_d3d11_shared",
          active_renderer: "d3d11",
          active_codec: "hevc",
          active_codec_profile: "main",
          active_bit_depth: 8,
          active_chroma_subsampling: "4:2:0",
          active_pixel_format: "d3d11_shared_nv12",
          active_hdr_enabled: false,
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          codec_fallback_reason: null,
          queue_depth: 0,
          dropped_frames: 2,
          render_queue_replacements: 1,
          render_stale_frame_drops: 4,
          render_lock_drops: 2,
          render_present_skips: 3,
          render_queue_policy: "latest",
          swap_chain_max_frame_latency: 1,
          swap_chain_allow_tearing: true,
          swap_chain_waitable_object: true,
          swap_chain_present_mode: "waitable",
          display_refresh_hz: 144,
          render_thread_priority: "highest",
          render_waitable_timeouts: 1,
          stage_metrics: [
            { stage: "sender.capture", p95_ms: 1.7, samples: 20 },
            { stage: "sender.encode", p95_ms: 2.4, samples: 20 },
            { stage: "sender.send_datagram", p95_ms: 3.1, samples: 20 },
            { stage: "receiver.decode", p95_ms: 1.2, samples: 20 },
            { stage: "render_lock_wait", p95_ms: 0.3, samples: 20 },
            { stage: "render_waitable_wait", p95_ms: 0.8, samples: 20 },
            { stage: "receiver.present", p95_ms: 4.6, samples: 20 },
          ],
        });
      }
      if (command === "get_system_resource_snapshot") {
        const isDisplay = args?.target === "display";
        return Promise.resolve({
          target_name: isDisplay ? "Rdesk display" : "mrd-service",
          target_pid: isDisplay ? 111 : 222,
          target_found: true,
          cpu_metrics_available: true,
          cpu_metrics_scope: "process",
          cpu_usage_percent: isDisplay ? 6.5 : 12.5,
          memory_used_mb: isDisplay ? 384 : 256,
          memory_total_mb: 32768,
          memory_usage_percent: isDisplay ? 1.2 : 0.8,
          memory_metrics_scope: "process",
          gpu_usage_percent: isDisplay ? 18 : 22,
          gpu_memory_used_mb: isDisplay ? 512 : 1024,
          gpu_memory_total_mb: 8192,
          gpu_metrics_available: true,
          gpu_metrics_scope: isDisplay ? "system" : "process",
          gpu_usage_metrics_scope: "system",
          gpu_memory_metrics_scope: isDisplay ? "system" : "process",
          network_rx_bps: isDisplay ? 2_000_000 : 3_000_000,
          network_tx_bps: isDisplay ? 1_000_000 : 4_000_000,
          network_metrics_available: true,
          network_metrics_scope: "system",
          sampled_at_ms: Date.now(),
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    const diagnosticsChip = await screen.findByRole("button", { name: /连接诊断/ });
    fireEvent.mouseEnter(diagnosticsChip);

    expect(await screen.findByText("远程诊断")).toBeInTheDocument();
    const diagnosticsPopover = screen.getByTestId("remote-diagnostics-popover");
    expect(diagnosticsPopover).toHaveClass("fixed");
    expect(diagnosticsPopover.className).toContain("z-[1000]");
    expect(screen.getByText("连接质量")).toBeInTheDocument();
    expect(screen.getByText("性能曲线")).toBeInTheDocument();
    expect(screen.getByText("资源占用曲线")).toBeInTheDocument();
    expect(screen.getByText("mrd-service CPU / 内存")).toBeInTheDocument();
    expect(screen.getByText("接收显示 CPU / 内存")).toBeInTheDocument();
    expect(screen.getByText("13% / 256 MB")).toBeInTheDocument();
    expect(screen.getByText("6.5% / 384 MB")).toBeInTheDocument();
    expect(screen.getByText("阶段延迟 P95")).toBeInTheDocument();
    expect(screen.getByText("sender.encode")).toBeInTheDocument();
    expect(screen.getByText("H.265 Main")).toBeInTheDocument();
    expect(screen.getByText("8-bit")).toBeInTheDocument();
    expect(screen.getByText("4:2:0")).toBeInTheDocument();
    expect(screen.getByText("NVDEC HEVC / D3D11")).toBeInTheDocument();
    expect(screen.getByText("DXGINative")).toBeInTheDocument();
    expect(screen.getByText("渲染丢帧细分")).toBeInTheDocument();
    expect(screen.getByText("队列 1 / 过期 4 / 锁 2 / Present 3")).toBeInTheDocument();
    expect(screen.getByText("渲染策略")).toBeInTheDocument();
    expect(screen.getByText("latest / waitable / tearing / 144 Hz")).toBeInTheDocument();
    expect(screen.getByText("渲染锁等待 p95")).toBeInTheDocument();
    expect(screen.getAllByText("0.30 ms").length).toBeGreaterThan(0);
    expect(screen.getByText("Waitable 等待 p95")).toBeInTheDocument();
    expect(screen.getByText("0.80 ms / timeout 1")).toBeInTheDocument();

    fireEvent.click(diagnosticsChip);
    fireEvent.mouseLeave(diagnosticsChip.parentElement ?? diagnosticsChip);
    expect(screen.getByText("远程诊断")).toBeInTheDocument();

    fireEvent.pointerDown(document.body);
    await waitFor(() => {
      expect(screen.queryByText("远程诊断")).not.toBeInTheDocument();
    });
  });

  it("captures pointer input on the focused remote render area", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerMove(renderArea, { clientX: 640, clientY: 416 });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 640, clientY: 416 });
    fireEvent.pointerUp(renderArea, { button: 0, clientX: 640, clientY: 416 });
    fireEvent.wheel(renderArea, { deltaY: 120 });
    fireEvent.wheel(renderArea, { deltaX: 120 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_wheel", delta: -120 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_horizontal_wheel", delta: -120 },
        },
      ]);
    });
  });

  it("enables remote control from service-owned keyboard mouse capability snapshots", async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const fetchMock = vi.fn().mockImplementation(async (_url: string, init?: RequestInit) => {
      const body = JSON.parse(String(init?.body ?? "{}")) as {
        request?: { type?: string; session_id?: string };
      };
      const request = body.request;
      if (request?.type === "CapabilitySnapshot") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "CapabilitySnapshot",
              snapshot: {
                schema_version: 1,
                platform: "windows",
                service_version: "0.1.0",
                capabilities: [
                  {
                    id: "control.keyboard_mouse",
                    domain: "control",
                    label: "Keyboard and mouse control",
                    status: "available",
                    platform: "windows",
                  },
                ],
                constraints: [],
                profiles: [],
                updated_at_ms: 1,
              },
            },
          }),
        };
      }
      if (request?.type === "SessionRuntimeSnapshot") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "SessionRuntimeSnapshot",
              snapshot: {
                session_id: request.session_id,
                role: "controller",
                state: "streaming",
                transport_kind: "quic",
                last_error: null,
                sender_active: false,
                receiver_active: true,
                peer_device_id: "target-device",
              },
            },
          }),
        };
      }
      if (request?.type === "LanDiscoverySnapshot") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "LanDiscoverySnapshot",
              snapshot: lanDiscoverySnapshotWithPeerInput(),
            },
          }),
        };
      }
      if (request?.type === "ProbeSnapshot") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "ProbeSnapshot",
              snapshot: {
                session_id: request.session_id,
                frames_received: 2,
                frames_decoded: 2,
                frames_dropped: 0,
                current_fps: 60,
                bitrate_mbps: 20,
                media_probe_valid: true,
                media_probe_width: 1920,
                media_probe_height: 1080,
                media_probe_target_fps: 60,
                media_probe_target_bitrate_mbps: 20,
                latest_frame_width: 1920,
                latest_frame_height: 1080,
                last_error: null,
              },
            },
          }),
        };
      }
      if (request?.type === "MediaPipelineSnapshot") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "MediaPipelineSnapshot",
              snapshot: {
                session_id: request.session_id,
                active_width: 1920,
                active_height: 1080,
                active_fps: 60,
                active_bitrate_mbps: 20,
                queue_depth: 0,
                dropped_frames: 0,
                stage_metrics: [],
              },
            },
          }),
        };
      }
      if (request?.type === "SendControlInput") {
        return {
          ok: true,
          json: async () => ({
            response: {
              type: "ControlInputAccepted",
              session_id: request.session_id,
              lane: "realtime",
              event_count: 1,
            },
          }),
        };
      }
      return {
        ok: true,
        json: async () => ({ response: { type: "Ack" } }),
      };
    });
    vi.stubGlobal("fetch", fetchMock);

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    fireEvent.pointerMove(renderArea, { clientX: 640, clientY: 416 });

    await waitFor(() => {
      const sentRequests = fetchMock.mock.calls
        .map(([, init]) => JSON.parse(String((init as RequestInit | undefined)?.body ?? "{}")))
        .map((body) => body.request)
        .filter((request) => request?.type === "SendControlInput");
      expect(sentRequests).toEqual([
        {
          type: "SendControlInput",
          session_id: "p2p-quic-123",
          event: { kind: "mouse_move", x: 960, y: 540 },
        },
      ]);
    });
    expect(getMockInvoke()).not.toHaveBeenCalledWith("test_get_capabilities", expect.anything());
  });

  it("captures extended pointer buttons on the focused remote render area", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { button: 3, clientX: 640, clientY: 416 });
    fireEvent.pointerUp(renderArea, { button: 3, clientX: 640, clientY: 416 });
    fireEvent.pointerDown(renderArea, { button: 4, clientX: 640, clientY: 416 });
    fireEvent.pointerUp(renderArea, { button: 4, clientX: 640, clientY: 416 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "x1", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "x1", pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "x2", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "x2", pressed: false },
        },
      ]);
    });
  });

  it("keeps remote input disabled until the receiver session is streaming", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          current_fps: null,
          bitrate_mbps: null,
          latest_frame_width: null,
          latest_frame_height: null,
          latest_frame_pixel_format: null,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");

    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "-1"));
    fireEvent.pointerMove(renderArea, { clientX: 640, clientY: 416 });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 640, clientY: 416 });
    fireEvent.keyDown(renderArea, { key: "a", code: "KeyA" });

    expect(
      mockInvoke.mock.calls.some(([command]) => command === "ipc_send_control_input")
    ).toBe(false);
  });

  it("moves the remote cursor before sending a direct pointer button press", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 640, clientY: 416 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
      ]);
    });
  });

  it("moves the remote cursor before sending a wheel event", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    const wheelEvent = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      deltaY: 120,
    });
    Object.defineProperty(wheelEvent, "clientX", { value: 640 });
    Object.defineProperty(wheelEvent, "clientY", { value: 416 });
    fireEvent(renderArea, wheelEvent);

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_wheel", delta: -120 },
        },
      ]);
    });
  });

  it("releases active remote pointer input when pointer capture is cancelled", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 640, clientY: 416 });
    fireEvent.pointerCancel(renderArea, { pointerId: 1 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("releases active remote pointer input when pointer capture is lost", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 640, clientY: 416 });
    fireEvent(renderArea, new Event("lostpointercapture", { bubbles: true }));

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("releases active remote pointer input when the browser context menu opens", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 2560,
          active_height: 1440,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { button: 2, clientX: 640, clientY: 416 });

    const contextMenu = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
    });
    fireEvent(renderArea, contextMenu);

    expect(contextMenu.defaultPrevented).toBe(true);
    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 1280, y: 720 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "right", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("ignores pointer input in a letterboxed remote render area", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1000, height: 600 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 1600,
          media_probe_height: 900,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 1600,
          latest_frame_height: 900,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 1600,
          active_height: 900,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    vi.spyOn(renderArea, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 1000,
      bottom: 600,
      width: 1000,
      height: 600,
      toJSON: () => ({}),
    } as DOMRect);
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerMove(renderArea, { clientX: 500, clientY: 10 });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 500, clientY: 10 });
    fireEvent.pointerMove(renderArea, { clientX: 500, clientY: 300 });
    fireEvent.pointerDown(renderArea, { button: 0, clientX: 500, clientY: 300 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 800, y: 450 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
      ]);
    });
  });

  it("releases remote pointer input when pointer up lands in a letterbox gutter", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1000, height: 600 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 4,
          frames_decoded: 4,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 1600,
          media_probe_height: 900,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          latest_frame_width: 1600,
          latest_frame_height: 900,
          latest_frame_pixel_format: "d3d11_shared_nv12",
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_media_pipeline_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          active_width: 1600,
          active_height: 900,
          active_fps: 144,
          active_bitrate_mbps: 80,
          stage_metrics: [],
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    vi.spyOn(renderArea, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 1000,
      bottom: 600,
      width: 1000,
      height: 600,
      toJSON: () => ({}),
    } as DOMRect);
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.pointerDown(renderArea, { pointerId: 7, button: 0, clientX: 500, clientY: 300 });
    fireEvent.pointerUp(renderArea, { pointerId: 7, button: 0, clientX: 500, clientY: 10 });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_move", x: 800, y: 450 },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "mouse_button", button: "left", pressed: false },
        },
      ]);
    });
  });

  it("captures keyboard input and releases tracked input on blur", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "a", code: "KeyA" });
    fireEvent.keyUp(renderArea, { key: "a", code: "KeyA" });
    fireEvent.keyDown(renderArea, { key: "Shift", code: "ShiftLeft" });
    fireEvent.blur(renderArea);

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x41 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x41 }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x10 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("clears stale control input errors after a later input succeeds", async () => {
    const mockInvoke = getMockInvoke();
    let controlInputAttempts = 0;
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        controlInputAttempts += 1;
        if (controlInputAttempts === 1) {
          return Promise.reject(new Error("input injection failed"));
        }
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "a", code: "KeyA" });
    expect(await screen.findByText("input injection failed")).toBeInTheDocument();

    fireEvent.keyUp(renderArea, { key: "a", code: "KeyA" });

    await waitFor(() => {
      expect(screen.queryByText("input injection failed")).not.toBeInTheDocument();
    });
  });

  it("captures function key input for remote control shortcuts", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "F5", code: "F5" });
    fireEvent.keyUp(renderArea, { key: "F5", code: "F5" });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x74 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x74 }, pressed: false },
        },
      ]);
    });
  });

  it("captures punctuation key input for remote typing", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "-", code: "Minus" });
    fireEvent.keyUp(renderArea, { key: "-", code: "Minus" });
    fireEvent.keyDown(renderArea, { key: "/", code: "Slash" });
    fireEvent.keyUp(renderArea, { key: "/", code: "Slash" });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbd }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbd }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbf }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbf }, pressed: false },
        },
      ]);
    });
  });

  it("captures numpad operator key input for remote typing", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "*", code: "NumpadMultiply" });
    fireEvent.keyUp(renderArea, { key: "*", code: "NumpadMultiply" });
    fireEvent.keyDown(renderArea, { key: "+", code: "NumpadAdd" });
    fireEvent.keyUp(renderArea, { key: "+", code: "NumpadAdd" });
    fireEvent.keyDown(renderArea, { key: "=", code: "NumpadEqual" });
    fireEvent.keyUp(renderArea, { key: "=", code: "NumpadEqual" });
    fireEvent.keyDown(renderArea, { key: "-", code: "NumpadSubtract" });
    fireEvent.keyUp(renderArea, { key: "-", code: "NumpadSubtract" });
    fireEvent.keyDown(renderArea, { key: ".", code: "NumpadDecimal" });
    fireEvent.keyUp(renderArea, { key: ".", code: "NumpadDecimal" });
    fireEvent.keyDown(renderArea, { key: "/", code: "NumpadDivide" });
    fireEvent.keyUp(renderArea, { key: "/", code: "NumpadDivide" });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6a }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6a }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6b }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6b }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbb }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0xbb }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6d }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6d }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6e }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6e }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6f }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x6f }, pressed: false },
        },
      ]);
    });
  });

  it("captures system key input for remote control", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "PrintScreen", code: "PrintScreen" });
    fireEvent.keyUp(renderArea, { key: "PrintScreen", code: "PrintScreen" });
    fireEvent.keyDown(renderArea, { key: "NumLock", code: "NumLock" });
    fireEvent.keyUp(renderArea, { key: "NumLock", code: "NumLock" });
    fireEvent.keyDown(renderArea, { key: "ScrollLock", code: "ScrollLock" });
    fireEvent.keyUp(renderArea, { key: "ScrollLock", code: "ScrollLock" });
    fireEvent.keyDown(renderArea, { key: "ContextMenu", code: "ContextMenu" });
    fireEvent.keyUp(renderArea, { key: "ContextMenu", code: "ContextMenu" });

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x2c }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x2c }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x90 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x90 }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x91 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x91 }, pressed: false },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x5d }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x5d }, pressed: false },
        },
      ]);
    });
  });

  it("enables remote control from the peer input capability when local injection is unavailable", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: [],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_lan_discovery_snapshot") {
        return Promise.resolve({
          enabled: true,
          running: true,
          discovery_port: 49700,
          instance_id: "local-instance",
          peers: [
            {
              device_id: "target-device",
              device_name: "Target",
              device_type: "desktop",
              ip: "127.0.0.1",
              discovery_port: 49700,
              p2p_control_addr: "127.0.0.1:49701",
              transports: ["quic"],
              protocol_version: 1,
              media_protocol_version: 1,
              media_capabilities: ["control.keyboard_mouse"],
              age_ms: 10,
              p2p_available: true,
            },
          ],
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    fireEvent.pointerMove(renderArea, { clientX: 640, clientY: 416 });

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_send_control_input", {
        sessionId: "p2p-quic-123",
        event: { kind: "mouse_move", x: 1280, y: 720 },
      });
    });
    expect(mockInvoke.mock.calls.some(([command]) => command === "ipc_list_sessions")).toBe(false);
  });

  it("keeps remote control disabled until the peer advertises keyboard mouse capability", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_lan_discovery_snapshot") {
        return Promise.resolve({
          enabled: true,
          running: true,
          discovery_port: 49700,
          instance_id: "local-instance",
          peers: [
            {
              device_id: "target-device",
              device_name: "Target",
              device_type: "desktop",
              ip: "127.0.0.1",
              discovery_port: 49700,
              p2p_control_addr: "127.0.0.1:49701",
              transports: ["quic"],
              protocol_version: 1,
              media_protocol_version: 1,
              media_capabilities: ["codec.h264"],
              age_ms: 10,
              p2p_available: true,
            },
          ],
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "realtime",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() =>
      expect(mockInvoke.mock.calls.some(([command]) => command === "ipc_lan_discovery_snapshot"))
        .toBe(true)
    );

    expect(renderArea).toHaveAttribute("tabindex", "-1");
    fireEvent.pointerMove(renderArea, { clientX: 640, clientY: 416 });
    expect(mockInvoke.mock.calls.some(([command]) => command === "ipc_send_control_input")).toBe(
      false
    );
  });

  it("releases active remote keyboard input when the window loses focus", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "Shift", code: "ShiftLeft" });
    window.dispatchEvent(new Event("blur"));

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x10 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("releases active remote keyboard input when the page is hidden", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "Shift", code: "ShiftLeft" });
    window.dispatchEvent(new Event("pagehide"));

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x10 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });
  });

  it("releases active remote keyboard input when document visibility becomes hidden", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "reliable",
          event_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "Shift", code: "ShiftLeft" });

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));

    await waitFor(() => {
      const inputCalls = mockInvoke.mock.calls.filter(([command]) => command === "ipc_send_control_input");
      expect(inputCalls.map(([, args]) => args)).toEqual([
        {
          sessionId: "p2p-quic-123",
          event: { kind: "key", key: { kind: "virtual_key", code: 0x10 }, pressed: true },
        },
        {
          sessionId: "p2p-quic-123",
          event: { kind: "release_all" },
        },
      ]);
    });

    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
  });

  it("exposes 4K and high-refresh profile options", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    expect(await screen.findByText("4K")).toBeInTheDocument();
    expect(await screen.findByText("180 FPS")).toBeInTheDocument();
    expect(screen.getByText("249 FPS")).toBeInTheDocument();
  });

  it("allows switching back to Metal after selecting browser WebRTC video on macOS", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          os_type: "macos",
          cpu_brand: "Apple",
          cpu_cores: 8,
          memory_gb: 16,
          gpu_info: "Apple GPU",
          available_captures: ["macos", "synthetic"],
          available_encoders: ["videotoolbox_h264", "openh264"],
          available_decoders: ["videotoolbox_h264", "software"],
          available_renderers: ["macos"],
          available_memory_modes: ["cpu"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "macos" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    const webButton = await screen.findByRole("button", { name: "WebRTC video" });
    fireEvent.click(webButton);

    const metalButton = await screen.findByRole("button", { name: "Metal native" });
    await waitFor(() => expect(metalButton).toBeEnabled());
    fireEvent.click(metalButton);

    await waitFor(() => {
      expect(screen.getByText("render: Metal native")).toBeInTheDocument();
    });
  });

  it("keeps remote display native rendering enabled when capability fallback omits d3d11", async () => {
    const mockInvoke = getMockInvoke();
    let resolveCapabilities: (value: ReturnType<typeof windowsCapabilities>) => void = () => {};
    const capabilitiesPromise = new Promise<ReturnType<typeof windowsCapabilities>>((resolve) => {
      resolveCapabilities = resolve;
    });
    const configureCalls: Record<string, unknown>[] = [];

    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return capabilitiesPromise;
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        configureCalls.push(args ?? {});
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("p2p-quic-123");

    await waitFor(() => {
      expect(configureCalls.some((call) => call.enabled === true)).toBe(true);
    });

    await act(async () => {
      resolveCapabilities({
        ...windowsCapabilities(),
        available_renderers: ["webview"],
      });
    });

    await waitFor(() => {
      expect(screen.getByText("render: D3D11 native")).toBeInTheDocument();
    });
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 50));
    });

    expect(configureCalls.some((call) => call.enabled === false)).toBe(false);
  });

  it("passes the current remote frame size to native surface input configuration", async () => {
    const mockInvoke = getMockInvoke();
    const configureCalls: Record<string, unknown>[] = [];
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        configureCalls.push(args ?? {});
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 70,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("p2p-quic-123");

    await waitFor(() => {
      expect(
        configureCalls.some((call) => {
          const controlFrameSize = call.controlFrameSize as
            | { width?: number; height?: number }
            | undefined;
          return (
            call.enabled === true &&
            controlFrameSize?.width === 2560 &&
            controlFrameSize.height === 1440
          );
        })
      ).toBe(true);
    });
  });

  it("probes the D3D11 native surface before starting a local pipeline test", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-1",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "configure_remote_display_native_surface",
        expect.objectContaining({ enabled: true })
      );
    });
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "present_test_harness_frame_on_native_surface",
        undefined
      );
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            renderer_type: "d3d11",
            render_display: true,
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("classifies local E2E p95 latency against a three-frame budget", () => {
    expect(localThreeFrameLatencyStatus(120, 24)).toEqual({
      budgetMs: 25,
      withinBudget: true,
      label: "3帧内",
    });
    expect(localThreeFrameLatencyStatus(60, 55)).toEqual({
      budgetMs: 50,
      withinBudget: false,
      label: "超过3帧",
    });
    expect(localThreeFrameLatencyStatus(null, 20)).toEqual({
      budgetMs: null,
      withinBudget: null,
      label: "等待样本",
    });
  });

  it("fails the local pipeline test when the run state disappears after start", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-missing");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 0,
          frame_count: 0,
          total_latency_p95_ms: 0,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return defaultRemoteDisplayInvoke(command);
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(screen.getAllByText("测试运行状态丢失: run-missing").length).toBeGreaterThan(0);
      expect(screen.getByRole("button", { name: "Start local pipeline test" })).toBeEnabled();
    });
  });

  it("auto-selects a Web View compatible local pipeline before starting a local test", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-web");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 60,
          frame_count: 12,
          total_latency_p95_ms: 16,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-web",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });

    await waitFor(() => expect(startButton).toBeEnabled());
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "dxgi",
            encoder_type: "nvenc_h264",
            decoder_type: "none",
            transport_kind: "webrtc",
            fps: 144,
            render_display: false,
            visual_preview: true,
            zero_copy: false,
          }),
        })
      );
    });
  });

  it("applies the explicit 2K144 WebRTC low-latency browser profile", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-web-2k144-lowlat");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 144,
          frame_count: 12,
          total_latency_p95_ms: 46,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-web-2k144-lowlat",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "WebRTC 2K144" }));
    fireEvent.click(await screen.findByRole("button", { name: "开始测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "dxgi",
            encoder_type: "nvenc_h264",
            decoder_type: "none",
            transport_kind: "webrtc",
            resolution: [2560, 1440],
            fps: 144,
            bitrate: 20_000_000,
            render_display: false,
            visual_preview: true,
            zero_copy: false,
          }),
        })
      );
    });
  });

  it("disables WebCodecs WebGL2 when browser decoder APIs are missing", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    const webCodecsButton = await screen.findByRole("button", { name: "WebCodecs WebGL2" });

    expect(webCodecsButton).toBeDisabled();
    expect(webCodecsButton).toHaveAttribute(
      "title",
      expect.stringContaining("缺少 VideoDecoder")
    );
  });

  it("shows browser render path constraints instead of unsupported matrix mappings", async () => {
    vi.stubGlobal("VideoDecoder", class {});
    vi.stubGlobal("EncodedVideoChunk", class {});
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));

    expect(await screen.findByText("Browser video decode")).toBeInTheDocument();
    expect(screen.getByText("WebRTC RTP")).toBeInTheDocument();

    expect(screen.getByRole("button", { name: "ENC NVENC H.264" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "ENC NVENC HEVC Main" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "ENC NVENC AV1" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "FPS 144 FPS" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "FPS 165 FPS" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "FPS 180 FPS" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "FPS 249 FPS" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "WebCodecs WebGL2" }));

    expect(screen.getByText("Browser WebCodecs")).toBeInTheDocument();
    expect(screen.getByText("WebSocket AU")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "ENC NVENC HEVC Main" })).toBeEnabled();
    });
  });

  it("uses the selected 60S duration for local display tests", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-60s");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 13,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-60s", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "DURATION 60S" }));
    fireEvent.click(screen.getByRole("button", { name: "开始测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            duration_ms: 60_000,
          }),
        })
      );
    });
  });

  it("uses a stop action inside the test config modal while the local run is active", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-modal-stop");
      }
      if (command === "test_stop_run") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 13,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-modal-stop", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "Start local pipeline test" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Stop local pipeline test" })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "测试配置" }));

    const dialog = await screen.findByRole("dialog", { name: "测试配置" });
    expect(within(dialog).getByText("当前测试运行中；停止后修改才会影响下一次启动")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "FPS 144 FPS" })).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("button", { name: "停止测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-modal-stop" });
    });
  });

  it("shows the completed test summary on the main display surface", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-completed");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: false,
          capture_fps: 120.4,
          frame_count: 3612,
          total_latency_p95_ms: 12.8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-completed",
          status: "completed",
          summary: {
            total_duration_ms: 30_000,
            capture_fps: 120.4,
            total_latency_p95: 12.8,
            encode_latency_p95: 2.2,
            transport_latency_p95: 1.4,
            decode_latency_p95: 0.9,
            dropped_frames: 1,
            frame_count: 3612,
          },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "Start local pipeline test" }));

    expect(await screen.findByText("完整测试报告")).toBeInTheDocument();
    expect(screen.getByText("run-completed")).toBeInTheDocument();
    expect(screen.getByText("120.4 FPS")).toBeInTheDocument();
    expect(screen.getAllByText("12.8 ms").length).toBeGreaterThan(0);
    expect(screen.getByText("1 dropped")).toBeInTheDocument();
    expect(screen.getByText("运行配置")).toBeInTheDocument();
    expect(screen.getByText("阶段 P95")).toBeInTheDocument();
  });

  it("hides desktop window controls in the browser display path", async () => {
    runtimeMock.isTauri = false;
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    expect(await screen.findByRole("button", { name: "测试配置" })).toBeInTheDocument();
    expect(screen.queryByTitle("Minimize")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Maximize")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Close")).not.toBeInTheDocument();
  });

  it("allows WebCodecs ultra-low-latency start when the browser decoder APIs exist", async () => {
    vi.stubGlobal("VideoDecoder", class {});
    vi.stubGlobal("EncodedVideoChunk", class {});
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "WebCodecs WebGL2" }));

    expect(screen.getByRole("button", { name: "开始测试" })).toBeEnabled();
  });

  it("detects when WebCodecs can render through a worker offscreen canvas", () => {
    vi.stubGlobal("VideoDecoder", class {});
    vi.stubGlobal("EncodedVideoChunk", class {});
    vi.stubGlobal("Worker", class {});
    vi.stubGlobal("OffscreenCanvas", class {});

    expect(
      browserSupportsWebCodecsWorkerRendering({
        transferControlToOffscreen: () => ({}) as OffscreenCanvas,
      })
    ).toBe(true);
    expect(browserSupportsWebCodecsWorkerRendering(null)).toBe(false);
    expect(browserSupportsWebCodecsWorkerRendering({})).toBe(false);
  });

  it("labels the active WebCodecs worker renderer backend", () => {
    expect(webCodecsMemoryPathLabelFromState("webcodecs-worker:webgl2")).toBe(
      "WebGL2 OffscreenCanvas"
    );
    expect(webCodecsMemoryPathLabelFromState("webcodecs-worker:2d")).toBe(
      "OffscreenCanvas 2D"
    );
    expect(webCodecsMemoryPathLabelFromState("webcodecs-worker:connecting")).toBe(
      "OffscreenCanvas"
    );
  });

  it("lets test config override the initial URL profile after it has been applied", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-url-override");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 144,
          frame_count: 12,
          total_latency_p95_ms: 12,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-url-override", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay(
      "local-display-test-1",
      "?surface=surface-1&width=2560&height=1440&fps=120&bitrateMbps=20"
    );

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "FPS 144 FPS" }));
    fireEvent.click(screen.getByRole("button", { name: "开始测试" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            fps: 144,
            resolution: [2560, 1440],
            bitrate: 20_000_000,
          }),
        })
      );
    });
  });

  it("uses browser-preview labels instead of matrix-only decoder and transport labels", () => {
    expect(webPreviewDecoderLabel("webcodecs", "No decode")).toBe("Browser WebCodecs");
    expect(webPreviewTransportLabel("webcodecs", "WebRTC")).toBe("WebSocket AU bridge");
    expect(webPreviewDecoderLabel("webrtc", "No decode")).toBe("Browser video decode");
    expect(webPreviewTransportLabel("webrtc", "WebRTC")).toBe("WebRTC RTP");
  });

  it("blocks high-FPS browser rendering when only the OpenH264 diagnostic fallback is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsOpenH264OnlyCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });

    await waitFor(() => {
      expect(startButton).toBeDisabled();
      expect(
        screen.getByText(/网页 144 FPS 本机采集需要硬件 H\.264 编码器或 HEVC Main 编码器/)
      ).toBeInTheDocument();
    });
  });

  it("uses actual Linux capture for Linux local WebRTC video tests", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(linuxCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: "web",
          attached: false,
          visible: false,
          parent_hwnd: null,
          hwnd: null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-linux-web");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 30,
          frame_count: 12,
          total_latency_p95_ms: 20,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-linux-web", status: "running", summary: null });
      }
      return Promise.resolve(args ?? null);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });
    await waitFor(() => expect(startButton).toBeEnabled());
    fireEvent.click(startButton);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "nvenc_h264",
            decoder_type: "none",
            transport_kind: "webrtc",
            fps: 144,
            render_display: false,
            visual_preview: true,
            zero_copy: false,
          }),
        })
      );
    });
  });

  it("starts Linux native rendering with an embedded native surface", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(linuxCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "linux" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: args?.enabled ? "0xA" : null,
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-linux-native");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 30,
          frame_count: 12,
          total_latency_p95_ms: 20,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-linux-native", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    const linuxNativeButton = await screen.findByRole("button", { name: "Linux native" });
    await waitFor(() => expect(linuxNativeButton).toBeEnabled());
    fireEvent.click(linuxNativeButton);
    fireEvent.click(await screen.findByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      const startCall = mockInvoke.mock.calls.find(([command]) => command === "test_start_run");
      expect(startCall).toBeTruthy();
      const config = (startCall?.[1] as { config?: Record<string, unknown> } | undefined)?.config;
      expect(config).toEqual(
        expect.objectContaining({
          capture_type: "linux",
          encoder_type: "none",
          decoder_type: "none",
          transport_kind: "loopback",
          renderer_type: "linux",
          render_display: true,
          renderer_target_hwnd: "0x14",
          visual_preview: true,
          zero_copy: false,
        })
      );
      expect(mockInvoke).toHaveBeenCalledWith(
        "present_test_harness_frame_on_native_surface",
        undefined
      );
    });
  });

  it("uses the Linux platform path for the low latency local profile", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(linuxCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "linux" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: args?.enabled ? "0xA" : null,
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-linux-low-latency");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 30,
          frame_count: 12,
          total_latency_p95_ms: 20,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-linux-low-latency",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "Low latency" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "linux",
            encoder_type: "none",
            decoder_type: "none",
            renderer_type: "linux",
            render_display: true,
            renderer_target_hwnd: "0x14",
            transport_kind: "loopback",
            visual_preview: true,
          }),
        })
      );
    });
  });

  it("uses software decode for macOS HEVC local pipeline when only H.264 VideoToolbox decode is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(macosHevcWithH264DecodeOnlyCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "macos_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "macos" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-macos-hevc-software");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 60,
          frame_count: 12,
          total_latency_p95_ms: 12,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-macos-hevc-software",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            capture_type: "macos",
            encoder_type: "videotoolbox_hevc",
            decoder_type: "software",
            renderer_type: "macos",
            render_display: true,
            renderer_target_hwnd: "0x14",
          }),
        })
      );
    });
  });

  it("starts the local native pipeline with AV1 zero-copy when selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-1", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(screen.getByRole("button", { name: "ENC NVENC AV1" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            encoder_type: "nvenc_av1",
            decoder_type: "nvdec",
            renderer_type: "d3d11",
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("starts the local native pipeline with HEVC Main10 zero-copy when selected", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-1", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(screen.getByRole("button", { name: "ENC NVENC HEVC Main10" }));
    fireEvent.click(screen.getByRole("button", { name: "NET QUIC" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            encoder_type: "nvenc_hevc_main10",
            decoder_type: "nvdec",
            transport_kind: "quic",
            renderer_type: "d3d11",
            renderer_target_hwnd: "0x14",
            zero_copy: true,
          }),
        })
      );
    });
  });

  it("does not start the local test harness for LAN remote sessions", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          current_fps: null,
          bitrate_mbps: null,
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([remoteDisplaySource]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: remoteDisplaySource,
          status: "selected",
          reason: null,
        });
      }
      if (command === "ipc_start_receiver") {
        return Promise.resolve("p2p-quic-123");
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "Start remote receiver" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_start_receiver", {
        sessionId: "p2p-quic-123",
      });
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "test_start_run",
      expect.anything()
    );
  });

  it("applies remote media profile changes through IPC negotiation", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({
          requested: {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "hevc",
            codec_profile: "main",
            bit_depth: 8,
            chroma_subsampling: "4:2:0",
            pixel_format: "nv12",
            hdr_enabled: false,
          },
          selected: {
            width: 1920,
            height: 1080,
            fps: 144,
            bitrate_mbps: 20,
            codec: "hevc",
            codec_profile: "main",
            bit_depth: 8,
            chroma_subsampling: "4:2:0",
            pixel_format: "nv12",
            hdr_enabled: false,
          },
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 20,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_update_media_profile", {
        sessionId: "p2p-quic-123",
        requestedProfile: {
          width: 1920,
          height: 1080,
          fps: 144,
          bitrate_mbps: 20,
          codec: "hevc",
          codec_profile: "main",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        },
      });
    });
  });

  it("keeps remote macOS H.264 profile updates from the window URL", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(macosCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-mac-1",
          session_id: "p2p-quic-mac",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "macos_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-mac-1",
          backend: "macos",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({
          requested: args?.requestedProfile,
          selected: args?.requestedProfile,
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-mac",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 80,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 80,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay(
      "p2p-quic-mac",
      "?surface=surface-1&profileWidth=2560&profileHeight=1440&profileFps=144&profileBitrateMbps=80&profileCodec=h264"
    );

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_update_media_profile", {
        sessionId: "p2p-quic-mac",
        requestedProfile: expect.objectContaining({
          width: 2560,
          height: 1440,
          fps: 144,
          bitrate_mbps: 80,
          codec: "h264",
          codec_profile: "high",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        }),
      });
    });
  });

  it("keeps remote color profile updates from the window URL", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({
          requested: args?.requestedProfile,
          selected: args?.requestedProfile,
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 40,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 40,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay(
      "p2p-quic-123",
      "?surface=surface-1&profileWidth=2560&profileHeight=1440&profileFps=144&profileBitrateMbps=40&profileCodec=hevc&profileColorMode=monochrome&profileColorPipeline=hdr_main10"
    );

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_update_media_profile", {
        sessionId: "p2p-quic-123",
        requestedProfile: expect.objectContaining({
          width: 2560,
          height: 1440,
          fps: 144,
          bitrate_mbps: 40,
          codec: "hevc",
          color_mode: "monochrome",
          color_pipeline: "hdr_main10",
        }),
      });
    });
  });

  it("keeps remote AV1 profile updates from the window URL", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        return Promise.resolve({
          requested: args?.requestedProfile,
          selected: args?.requestedProfile,
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 40,
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 40,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay(
      "p2p-quic-123",
      "?surface=surface-1&profileWidth=2560&profileHeight=1440&profileFps=144&profileBitrateMbps=40&profileCodec=av1"
    );

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_update_media_profile", {
        sessionId: "p2p-quic-123",
        requestedProfile: expect.objectContaining({
          width: 2560,
          height: 1440,
          fps: 144,
          bitrate_mbps: 40,
          codec: "av1",
          codec_profile: "main",
          bit_depth: 8,
          pixel_format: "nv12",
        }),
      });
    });
  });

  it("applies remote dynamic resolution adaptation from the settings panel", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_update_media_profile") {
        const selected = {
          width: 1920,
          height: 1080,
          fps: 144,
          bitrate_mbps: 20,
          codec: "hevc",
          codec_profile: "main",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        };
        return Promise.resolve({
          requested: selected,
          selected,
          status: "accepted",
          reason: null,
        });
      }
      if (command === "ipc_configure_media_adaptation") {
        const currentProfile = {
          width: 1920,
          height: 1080,
          fps: 144,
          bitrate_mbps: 20,
          codec: "hevc",
          codec_profile: "main",
          bit_depth: 8,
          chroma_subsampling: "4:2:0",
          pixel_format: "nv12",
          hdr_enabled: false,
        };
        return Promise.resolve({
          enabled: true,
          state: "steady",
          ladder_index: 0,
          current_profile: currentProfile,
          target_profile: currentProfile,
          last_reason: null,
          last_change_ms: 0,
          observed_fps: 144,
          drop_ratio: 0,
          queue_depth: 0,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 1,
          frames_decoded: 1,
          frames_dropped: 0,
          current_fps: 144,
          bitrate_mbps: 20,
          media_probe_valid: true,
          media_probe_width: 1920,
          media_probe_height: 1080,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 20,
          last_error: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByLabelText("启用远端自适应媒体"));
    fireEvent.click(await screen.findByLabelText("启用远端动态分辨率"));
    fireEvent.click(await screen.findByRole("button", { name: "应用远端" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_configure_media_adaptation", {
        sessionId: "p2p-quic-123",
        config: expect.objectContaining({
          enabled: true,
          dynamic_resolution_enabled: true,
        }),
      });
    });
  });

  it("lists remote window capture sources and selects one", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([
          {
            id: "windows:window:0x1234",
            platform: "windows",
            source_kind: "window",
            title: "Target App",
            class_name: "ApplicationFrameWindow",
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: "Target App",
            bundle_identifier: null,
            preview_data_url: null,
            preview_width: null,
            preview_height: null,
          },
        ]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: {
            id: "windows:window:0x1234",
            platform: "windows",
            source_kind: "window",
            title: "Target App",
            class_name: "ApplicationFrameWindow",
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: "Target App",
            bundle_identifier: null,
            preview_data_url: null,
            preview_width: null,
            preview_height: null,
          },
          status: "selected",
          reason: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "刷新捕获源" }));
    expect((await screen.findByLabelText("被捕获设备") as HTMLSelectElement).value).toBe("remote");
    fireEvent.change(await screen.findByLabelText("捕获源下拉"), {
      target: { value: "windows:window:0x1234" },
    });

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_list_remote_capture_sources", {
        sessionId: "p2p-quic-123",
        includePreviews: false,
        limit: 24,
      });
      expect(mockInvoke).not.toHaveBeenCalledWith(
        "ipc_list_remote_capture_sources",
        expect.objectContaining({
          includePreviews: true,
        })
      );
      expect(mockInvoke).toHaveBeenCalledWith("ipc_select_remote_capture_source", {
        sessionId: "p2p-quic-123",
        sourceId: "windows:window:0x1234",
      });
    });
  });

  it("uses unified capture source controls for local capture sources", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "ipc_list_local_capture_sources") {
        return Promise.resolve(localMixedSources);
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    await waitFor(() => expect(screen.getAllByText("捕获源").length).toBeGreaterThan(0));
    expect(screen.queryByText("本机捕获源")).not.toBeInTheDocument();
    expect((screen.getByLabelText("被捕获设备") as HTMLSelectElement).value).toBe("local");
    fireEvent.click(screen.getByRole("button", { name: "刷新捕获源" }));
    await waitFor(() => expect(screen.getByLabelText("捕获源下拉")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("捕获源下拉"), {
      target: { value: "windows:display-shared:1" },
    });
    await waitFor(() => {
      expect(screen.getByText(/当前选择: 全屏 shared \/ Display 2/)).toBeInTheDocument();
    });

    fireEvent.click(await screen.findByRole("button", { name: "选择捕获源" }));
    const picker = await screen.findByRole("dialog", { name: "捕获源选择" });
    expect(within(picker).getByText("捕获源选择")).toBeInTheDocument();

    fireEvent.click(within(picker).getByRole("button", { name: "采集方式 DXGI" }));
    fireEvent.click(within(picker).getByRole("button", { name: "刷新" }));

    await waitFor(() => {
      expect(within(picker).getAllByText(/Display 2/).length).toBeGreaterThan(0);
    });
    expect(within(picker).getAllByText("Calculator").length).toBeGreaterThan(0);
    expect(within(picker).getAllByText("原生链路").length).toBeGreaterThan(0);

    fireEvent.click(within(picker).getByRole("button", { name: "选择 Display 2 (D3D11 shared copy)" }));

    await waitFor(() => {
      expect(screen.getAllByText(/Display 2/).length).toBeGreaterThan(0);
    });
    expect(screen.getAllByText(/3840x2160/).length).toBeGreaterThan(0);
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_list_local_capture_sources", {
        includePreviews: false,
        limit: 24,
      });
      expect(mockInvoke).not.toHaveBeenCalledWith(
        "ipc_list_local_capture_sources",
        expect.objectContaining({
          includePreviews: true,
        })
      );
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "ipc_select_remote_capture_source",
      expect.anything()
    );
  });

  it("filters window capture sources to DXGI and hides them for WinRT copy mode", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "ipc_list_local_capture_sources") {
        return Promise.resolve(localMixedSources);
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "选择捕获源" }));
    const picker = await screen.findByRole("dialog", { name: "捕获源选择" });
    fireEvent.click(within(picker).getByRole("button", { name: "采集方式 DXGI" }));
    fireEvent.click(within(picker).getByRole("button", { name: "刷新" }));

    await waitFor(() => {
      expect(within(picker).getAllByText("Calculator").length).toBeGreaterThan(0);
    });

    fireEvent.click(within(picker).getByRole("button", { name: "采集方式 WinRT" }));

    await waitFor(() => {
      expect(within(picker).queryAllByText("Calculator")).toHaveLength(0);
    });
    expect(within(picker).getByText(/Display 1/)).toBeInTheDocument();
  });

  it("passes the selected local display source into native local test config", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_list_local_capture_sources") {
        return Promise.resolve(localMixedSources);
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-local-source");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({ run_id: "run-local-source", status: "running", summary: null });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "选择捕获源" }));
    const picker = await screen.findByRole("dialog", { name: "捕获源选择" });
    fireEvent.click(within(picker).getByRole("button", { name: "采集方式 DXGI" }));
    fireEvent.click(within(picker).getByRole("button", { name: "刷新" }));
    fireEvent.click(await within(picker).findByRole("button", { name: "选择 Display 2 (D3D11 shared copy)" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            input_source: "screen",
            source_id: "windows:display-shared:1",
            source_kind: "display_shared",
            display_id: "windows:display-shared:1",
          }),
        })
      );
    });
  });

  it("expands local matrix capture dimension over refreshed capture sources", async () => {
    const mockInvoke = getMockInvoke();
    let runIndex = 0;
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_list_local_capture_sources") {
        return Promise.resolve(localDisplaySources);
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        runIndex += 1;
        return Promise.resolve(`run-local-source-${runIndex}`);
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: args?.runId,
          status: "completed",
          summary: {
            capture_fps: 120,
            total_latency_p95: 8,
            dropped_frames: 0,
            frame_count: 120,
          },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新捕获源" }));
    await waitFor(() => expect(screen.getByLabelText("捕获源下拉")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "单次测试" }));
    fireEvent.click(screen.getByRole("button", { name: "MATRIX 捕获源" }));
    fireEvent.click(screen.getByRole("button", { name: "MATRIX FPS" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            source_id: "windows:display-shared:0",
            source_kind: "display_shared",
          }),
        })
      );
      expect(mockInvoke).toHaveBeenCalledWith(
        "test_start_run",
        expect.objectContaining({
          config: expect.objectContaining({
            source_id: "windows:display-shared:1",
            source_kind: "display_shared",
          }),
        })
      );
    });
  });

  it("passes the selected local window source as a window input without display_id", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_list_local_capture_sources") {
        return Promise.resolve(localMixedSources);
      }
      if (command === "present_test_harness_frame_on_native_surface") {
        return Promise.resolve(true);
      }
      if (command === "test_harness_stop") {
        return defaultRemoteDisplayInvoke(command);
      }
      if (command === "test_start_run") {
        return Promise.resolve("run-local-window-source");
      }
      if (command === "test_harness_get_metrics") {
        return Promise.resolve({
          is_running: true,
          capture_fps: 120,
          frame_count: 12,
          total_latency_p95_ms: 8,
          error_message: null,
        });
      }
      if (command === "test_get_run") {
        return Promise.resolve({
          run_id: "run-local-window-source",
          status: "running",
          summary: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "选择捕获源" }));
    const picker = await screen.findByRole("dialog", { name: "捕获源选择" });
    fireEvent.click(within(picker).getByRole("button", { name: "采集方式 DXGI" }));
    fireEvent.click(within(picker).getByRole("button", { name: "刷新" }));
    fireEvent.click(await within(picker).findByRole("button", { name: "选择 Calculator" }));
    fireEvent.click(screen.getByRole("button", { name: "Start local pipeline test" }));

    await waitFor(() => {
      const startCall = mockInvoke.mock.calls.find(([command]) => command === "test_start_run");
      expect(startCall?.[1]).toEqual(
        expect.objectContaining({
          scenarioId: "custom",
          config: expect.objectContaining({
            input_source: "window",
            source_id: "windows:window:0x1234",
            source_kind: "window",
            window_hwnd: "0x1234",
            window_title: "Calculator",
          }),
        })
      );
      expect((startCall?.[1] as { config?: Record<string, unknown> }).config).not.toHaveProperty(
        "display_id"
      );
    });
  });

  it("auto-selects the best fullscreen shared capture source for LAN remote sessions", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          current_fps: null,
          bitrate_mbps: null,
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([
          {
            id: "windows:window:0x1234",
            platform: "windows",
            source_kind: "window",
            title: "Target App",
            class_name: "ApplicationFrameWindow",
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: "Target App",
            bundle_identifier: null,
            preview_data_url: null,
            preview_width: null,
            preview_height: null,
          },
          remoteDisplaySource,
        ]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: remoteDisplaySource,
          status: "selected",
          reason: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_list_remote_capture_sources", {
        sessionId: "p2p-quic-123",
        includePreviews: false,
        limit: 24,
      });
      expect(mockInvoke).toHaveBeenCalledWith("ipc_select_remote_capture_source", {
        sessionId: "p2p-quic-123",
        sourceId: "windows:display-shared:0",
      });
    });
  });

  it("does not render decoded remote desktop data URLs as preview frames", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "web",
          attached: false,
          visible: false,
          parent_hwnd: "0xA",
          hwnd: null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 3,
          frames_decoded: 3,
          frames_dropped: 0,
          current_fps: 30,
          bitrate_mbps: 4,
          media_probe_valid: true,
          media_probe_format: "h264_desktop_frame",
          media_probe_width: 1280,
          media_probe_height: 720,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 64,
          media_probe_payload_bytes: 2048,
          last_media_sequence: 3,
          last_media_timestamp_us: 3000,
          last_media_payload_hash: "fnv1a64:abc123",
          latest_frame_data_url: "legacy-frame-payload",
          latest_frame_width: 1280,
          latest_frame_height: 720,
          latest_frame_pixel_format: "rgb24",
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([remoteDisplaySource]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: remoteDisplaySource,
          status: "selected",
          reason: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        "configure_remote_display_native_surface",
        expect.objectContaining({
          enabled: true,
          visible: true,
        })
      );
    });

    await screen.findByText(/remote rx 3/);
    expect(screen.queryByAltText("Remote desktop frame")).toBeNull();
    expect(mockInvoke.mock.calls.some(([command]) => String(command).includes("preview_frame"))).toBe(
      false
    );
  });

  it("does not present decoded data URLs onto an attached native remote surface", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 3,
          frames_decoded: 3,
          frames_dropped: 0,
          current_fps: 30,
          bitrate_mbps: 4,
          media_probe_valid: true,
          media_probe_format: "h264_desktop_frame",
          media_probe_width: 1280,
          media_probe_height: 720,
          media_probe_target_fps: 144,
          media_probe_target_bitrate_mbps: 64,
          media_probe_payload_bytes: 2048,
          last_media_sequence: 3,
          last_media_timestamp_us: 3000,
          last_media_payload_hash: "fnv1a64:abc123",
          latest_frame_data_url: "legacy-frame-payload",
          latest_frame_width: 1280,
          latest_frame_height: 720,
          latest_frame_pixel_format: "rgb24",
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([remoteDisplaySource]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: remoteDisplaySource,
          status: "selected",
          reason: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    await screen.findByText(/remote rx 3/);
    expect(screen.queryByAltText("Remote desktop frame")).toBeNull();
    expect(mockInvoke.mock.calls.some(([command]) => String(command).includes("preview_frame"))).toBe(
      false
    );
  });

  it("blocks remote receiver start when no remote capture source is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([]);
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "Start remote receiver" }));

    expect((await screen.findAllByText("远端未发现可捕获的全屏/窗口源，无法启动接收")).length).toBeGreaterThan(0);
    expect(mockInvoke).not.toHaveBeenCalledWith("ipc_start_receiver", expect.anything());
  });

  it("offers dropdown and modal remote capture source picker modes", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 0, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "connected",
          transport_kind: "quic",
          sender_active: false,
          receiver_active: false,
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          frames_received: 0,
          frames_decoded: 0,
          frames_dropped: 0,
          current_fps: null,
          bitrate_mbps: null,
          last_error: null,
        });
      }
      if (command === "ipc_list_remote_capture_sources") {
        return Promise.resolve([remoteDisplaySource]);
      }
      if (command === "ipc_select_remote_capture_source") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          source: remoteDisplaySource,
          status: "selected",
          reason: null,
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    await screen.findByLabelText("PICK");
    await waitFor(() => expect(screen.getByLabelText("捕获源下拉")).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("PICK"), { target: { value: "modal" } });
    fireEvent.click(screen.getByRole("button", { name: "选择捕获源" }));

    expect(await screen.findByText("捕获源选择")).toBeInTheDocument();
  });

  it("shows DX12 as unavailable until a D3D12 native renderer is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    expect(screen.queryByRole("button", { name: "DX12 native" })).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    const dx12Button = await screen.findByRole("button", { name: "DX12 native" });
    expect(dx12Button).toBeDisabled();
    expect(dx12Button).toHaveAttribute("title", expect.stringContaining("D3D12"));
  });

  it("keeps DX12 native disabled when only the independent D3D12 probe is available", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_renderers: ["d3d11", "d3d12"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          session_id: "local-display-test-1",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-local-display-test-1",
          backend: args?.enabled ? "d3d11" : "web",
          attached: Boolean(args?.enabled),
          visible: Boolean(args?.visible),
          parent_hwnd: "0xA",
          hwnd: args?.enabled ? "0x14" : null,
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay("local-display-test-1");

    expect(screen.queryByRole("button", { name: "DX12 native" })).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    const dx12Button = await screen.findByRole("button", { name: "DX12 native" });
    expect(dx12Button).toBeDisabled();
    expect(dx12Button).toHaveAttribute("title", expect.stringContaining("渲染测试"));
  });

  it("routes the title bar close button through the remote display cleanup command", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve(windowsCapabilities());
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: false,
          render_mode: "d3d11_native",
          native_surface_attached: false,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByTitle("Close"));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("close_remote_display_window", {
        label: "render-p2p-quic-123-1",
      });
    });
  });

  it("releases remote input before closing the remote display window", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "test_get_capabilities") {
        return Promise.resolve({
          ...windowsCapabilities(),
          available_controls: ["keyboard_mouse"],
        });
      }
      if (command === "current_remote_display_window_context") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          session_id: "p2p-quic-123",
          surface_id: "surface-1",
          role: "controller",
          renderer_attached: true,
          render_mode: "d3d11_native",
          native_surface_attached: true,
          session_window_count: 1,
        });
      }
      if (command === "configure_remote_display_native_surface") {
        return Promise.resolve({
          label: "render-p2p-quic-123-1",
          backend: "d3d11",
          attached: true,
          visible: true,
          parent_hwnd: "0xA",
          hwnd: "0x14",
          rect: { x: 0, y: 56, width: 1280, height: 720 },
        });
      }
      if (command === "ipc_session_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          role: "controller",
          state: "streaming",
          transport_kind: "quic",
          last_error: null,
          sender_active: false,
          receiver_active: true,
          peer_device_id: "target-device",
        });
      }
      if (command === "ipc_probe_snapshot") {
        return Promise.resolve({
          session_id: "p2p-quic-123",
          media_probe_valid: true,
          media_probe_width: 2560,
          media_probe_height: 1440,
          latest_frame_width: 2560,
          latest_frame_height: 1440,
          latest_frame_data_url: null,
          last_error: null,
        });
      }
      if (command === "ipc_send_control_input") {
        return Promise.resolve({
          session_id: args?.sessionId,
          lane: "cleanup",
          event_count: 1,
        });
      }
      if (command === "close_remote_display_window") {
        return defaultRemoteDisplayInvoke(command);
      }
      return defaultRemoteDisplayInvoke(command);
    });

    renderRemoteDisplay();
    const renderArea = await screen.findByTestId("remote-render-area");
    await waitFor(() => expect(renderArea).toHaveAttribute("tabindex", "0"));

    act(() => {
      renderArea.focus();
    });
    fireEvent.keyDown(renderArea, { key: "Shift", code: "ShiftLeft" });
    fireEvent.click(await screen.findByTitle("Close"));

    await waitFor(() => {
      const inputCallIndex = mockInvoke.mock.calls.findIndex(
        ([command, args]) =>
          command === "ipc_send_control_input" &&
          (args as { event?: { kind?: string } })?.event?.kind === "release_all"
      );
      const closeCallIndex = mockInvoke.mock.calls.findIndex(
        ([command]) => command === "close_remote_display_window"
      );

      expect(inputCallIndex).toBeGreaterThanOrEqual(0);
      expect(closeCallIndex).toBeGreaterThan(inputCallIndex);
    });
  });
});
