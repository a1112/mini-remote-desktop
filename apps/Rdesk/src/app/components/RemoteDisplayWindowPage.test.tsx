import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../test/mocks/tauri";
import {
  RemoteDisplayWindowPage,
  applyWebRtcReceiverLowLatencyHint,
  applyWebRtcVideoMotionHint,
  browserWebrtcPreviewH264Profile,
  buildWebRtcDiagnosticsStageRows,
  WebRtcPresentationLatencyTracker,
  summarizeWebRtcInboundVideoStats,
} from "./RemoteDisplayWindowPage";

vi.mock("../utils/runtime", () => ({
  isTauriRuntime: () => true,
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

function renderRemoteDisplay(sessionId = "p2p-quic-123") {
  render(
    <MemoryRouter initialEntries={[`/display/${sessionId}?surface=surface-1`]}>
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
  preview_data_url: "data:image/png;base64,BBBB",
  preview_width: 240,
  preview_height: 135,
};

describe("RemoteDisplayWindowPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    getMockInvoke().mockReset();
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

  it("shows a green LAN diagnostics popover with HEVC and chroma metadata", async () => {
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
          stage_metrics: [
            { stage: "sender.capture", p95_ms: 1.7, samples: 20 },
            { stage: "sender.encode", p95_ms: 2.4, samples: 20 },
            { stage: "sender.send_datagram", p95_ms: 3.1, samples: 20 },
            { stage: "receiver.decode", p95_ms: 1.2, samples: 20 },
            { stage: "receiver.present", p95_ms: 4.6, samples: 20 },
          ],
        });
      }
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    const diagnosticsChip = await screen.findByRole("button", { name: /连接诊断/ });
    fireEvent.mouseEnter(diagnosticsChip);

    expect(await screen.findByText("远程诊断")).toBeInTheDocument();
    expect(screen.getByText("连接质量")).toBeInTheDocument();
    expect(screen.getByText("性能曲线")).toBeInTheDocument();
    expect(screen.getByText("阶段延迟 P95")).toBeInTheDocument();
    expect(screen.getByText("sender.encode")).toBeInTheDocument();
    expect(screen.getByText("H.265 Main")).toBeInTheDocument();
    expect(screen.getByText("8-bit")).toBeInTheDocument();
    expect(screen.getByText("4:2:0")).toBeInTheDocument();
    expect(screen.getByText("NVDEC HEVC / D3D11")).toBeInTheDocument();
    expect(screen.getByText("DXGINative")).toBeInTheDocument();

    fireEvent.click(diagnosticsChip);
    fireEvent.mouseLeave(diagnosticsChip.parentElement ?? diagnosticsChip);
    expect(screen.getByText("远程诊断")).toBeInTheDocument();

    fireEvent.pointerDown(document.body);
    await waitFor(() => {
      expect(screen.queryByText("远程诊断")).not.toBeInTheDocument();
    });
  });

  it("exposes 180 and 249 FPS high-refresh profile options", async () => {
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    expect(await screen.findByText("180 FPS")).toBeInTheDocument();
    expect(screen.getByText("249 FPS")).toBeInTheDocument();
  });

  it("allows switching back to Metal after selecting Web View on macOS", async () => {
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
          available_decoders: ["videotoolbox", "software"],
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
          renderer_attached: false,
          render_mode: "web",
          native_surface_attached: false,
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    const webButton = await screen.findByRole("button", { name: "Web View" });
    fireEvent.click(webButton);

    const metalButton = await screen.findByRole("button", { name: "Metal native" });
    await waitFor(() => expect(metalButton).toBeEnabled());
    fireEvent.click(metalButton);

    await waitFor(() => {
      expect(screen.getByText("render: Metal native")).toBeInTheDocument();
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
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
            visual_preview: false,
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
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
            visual_preview: false,
            zero_copy: false,
          }),
        })
      );
    });
  });

  it("shows WebCodecs ultra-low-latency as an explicit prototype path", async () => {
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "WebCodecs Ultra" }));

    expect(
      await screen.findByText("WebCodecs 超低延迟路径", {
        selector: ".text-sm",
      })
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(/WebCodecs 超低延迟路径需要/)
    ).not.toHaveLength(0);
    expect(screen.getByRole("button", { name: "开始测试" })).toBeDisabled();
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "WebCodecs Ultra" }));

    expect(screen.getByRole("button", { name: "开始测试" })).toBeEnabled();
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    const startButton = await screen.findByRole("button", {
      name: "Start local pipeline test",
    });

    await waitFor(() => {
      expect(startButton).toBeDisabled();
      expect(
        screen.getByText(/网页 144 FPS 本机采集需要硬件 H\.264 编码器/)
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
        return Promise.resolve(null);
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
            visual_preview: false,
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

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
          visual_preview: false,
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
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
            visual_preview: false,
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.change(screen.getByLabelText("ENC"), { target: { value: "nvenc_av1" } });
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
        return Promise.resolve(null);
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

    fireEvent.click(await screen.findByRole("button", { name: "测试配置" }));
    fireEvent.change(screen.getByLabelText("ENC"), { target: { value: "nvenc_hevc_main10" } });
    fireEvent.change(screen.getByLabelText("NET"), { target: { value: "quic" } });
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
      if (command === "present_remote_preview_frame_on_native_surface") {
        return Promise.resolve(true);
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
      return Promise.resolve(null);
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
      return Promise.resolve(null);
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
            preview_data_url: "data:image/png;base64,AAAA",
            preview_width: 240,
            preview_height: 135,
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "刷新捕获源" }));
    fireEvent.change(await screen.findByLabelText("远端捕获源下拉"), {
      target: { value: "windows:window:0x1234" },
    });

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_list_remote_capture_sources", {
        sessionId: "p2p-quic-123",
        includePreviews: false,
        limit: 24,
      });
      expect(mockInvoke).toHaveBeenCalledWith("ipc_list_remote_capture_sources", {
        sessionId: "p2p-quic-123",
        includePreviews: true,
        limit: 1,
      });
      expect(mockInvoke).toHaveBeenCalledWith("ipc_select_remote_capture_source", {
        sessionId: "p2p-quic-123",
        sourceId: "windows:window:0x1234",
      });
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
      return Promise.resolve(null);
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

  it("renders the latest decoded remote desktop preview frame", async () => {
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
          latest_frame_data_url: "data:image/png;base64,REMOTE",
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
      return Promise.resolve(null);
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

    const frame = await screen.findByAltText("Remote desktop frame");
    expect(frame).toHaveAttribute("src", "data:image/png;base64,REMOTE");
    expect(frame).toHaveStyle({ aspectRatio: "1280 / 720" });
  });

  it("does not cover an attached native remote surface with the low-frequency preview frame", async () => {
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
          latest_frame_data_url: "data:image/png;base64,REMOTE",
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    await screen.findByText(/remote rx 3/);
    expect(screen.queryByAltText("Remote desktop frame")).toBeNull();
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("present_remote_preview_frame_on_native_surface", {
        dataUrl: "data:image/png;base64,REMOTE",
      });
    });
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "Start remote receiver" }));

    await screen.findByText("远端未发现可捕获的全屏/窗口源，无法启动接收");
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByRole("button", { name: "配置" }));
    await screen.findByLabelText("PICK");
    await waitFor(() => expect(screen.getByLabelText("远端捕获源下拉")).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("PICK"), { target: { value: "modal" } });
    fireEvent.click(screen.getByRole("button", { name: "打开捕获源弹窗" }));

    expect(await screen.findByText("远端捕获源选择")).toBeInTheDocument();
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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

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
      return Promise.resolve(null);
    });

    renderRemoteDisplay("local-display-test-1");

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
      return Promise.resolve(null);
    });

    renderRemoteDisplay();

    fireEvent.click(await screen.findByTitle("Close"));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("close_remote_display_window", {
        label: "render-p2p-quic-123-1",
      });
    });
  });
});
