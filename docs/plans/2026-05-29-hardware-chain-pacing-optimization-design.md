# Hardware Chain Scheduling and Render Pacing Optimization Design

## Goal

Make the high-performance Windows hardware path measurable and stable at 2K144:

- DXGI capture
- NVENC H.264 or HEVC encode
- WebRTC RTP transport
- NVDEC decode
- D3D11 shared-texture render

The current 2K144 baseline already reaches roughly 143 FPS with sub-2 ms encode and decode p95. The next problem is not raw codec throughput. The gap is scheduling visibility and render pacing: benchmark summaries can report a healthy FPS while render presentation is under-instrumented.

## Baseline Evidence

Measured on branch `codex/hardware-chain-pacing-optimization` at `4e3d08d`.

H.264 2K144 hardware path:

- Observed FPS: 143.4
- Encode p95: 0.36 ms
- Decode p95: 1.58 ms
- Render upload average: 0.18 ms

HEVC 2K144 hardware path:

- Observed FPS: 143.5
- Encode p95: 0.33 ms
- Decode p95: 1.58 ms
- Render upload average: 0.24 ms

Observed gaps:

- `render_present_p95_ms` is not populated with meaningful samples.
- The benchmark `fps_observed` uses the maximum of capture FPS and decoded FPS, which can hide receiver-side stalls.
- The render thread has a bounded queue, but benchmark output does not expose upload, presentation, skip, replacement, or stale-frame counts.
- Present pacing health is not visible as a frame-gap distribution.

## Recommended Approach

Use a narrow observability-first optimization:

1. Add render pacing metrics to the harness.
2. Report receiver FPS from decoded frames when a decode path is active.
3. Add render coalescing counters around the native render queue.
4. Keep the existing hardware chain selection unchanged.
5. Validate with the existing 2K144 H.264 and HEVC matrix scenarios.

This avoids risky encoder, decoder, or transport rewrites while making pacing regressions visible and enforceable.

## Data Flow

Sender side:

1. Capture frame from DXGI.
2. Encode with NVENC.
3. Send access units over WebRTC RTP.

Receiver side:

1. Decode with NVDEC.
2. Convert decoded output into render input.
3. Submit to the native D3D11 render thread.
4. Track render upload and presentation pacing metrics.

Benchmark summary:

1. Read harness metrics after run.
2. Prefer decoded FPS when decode is active.
3. Export render upload latency, present gap p95, submitted/presented/skipped frames, and coalescing/drop counters.

## Render Pacing Metrics

Add metrics that answer separate questions:

- Was the media pipeline producing decoded frames fast enough?
- Was the render thread accepting frames fast enough?
- Did the renderer present frames at a stable cadence?
- Did pacing coalesce stale frames to keep the display current?

Required metrics:

- `render_submitted_frames`
- `render_uploaded_frames`
- `render_presented_frames`
- `render_present_skipped_frames`
- `render_queue_replacements`
- `render_stale_frame_drops`
- `render_present_gap_avg_ms`
- `render_present_gap_p50_ms`
- `render_present_gap_p95_ms`

## Scheduling Behavior

The render queue should remain latest-frame biased. If the producer outruns the native render thread, the benchmark should prefer presenting the newest frame over preserving every stale frame. This keeps latency bounded and makes high-FPS overload visible through explicit coalescing counters.

The first implementation should only coalesce at the harness render boundary. Product runtime behavior can adopt the same policy later once the benchmark evidence is stable.

## Error Handling

- A stopped render thread remains a hard failure.
- Timed-out render submissions remain a hard failure.
- Queue replacement is not a failure by itself; it is a pacing signal.
- Stale-frame drops become actionable only when the drop ratio or present gap violates benchmark thresholds.

## Testing Plan

Unit tests:

- Render metrics calculate present gaps from presented frame counts.
- Summary FPS prefers decoded FPS for decode-enabled benchmarks.
- Render coalescing records queue replacement without losing the newest frame.

Benchmark checks:

- `quick.transport.webrtc.nvenc.h264_nvdec.2k144.json`
- `quick.transport.webrtc.nvenc.hevc_nvdec.2k144.json`

Acceptance criteria:

- H.264 and HEVC 2K144 hardware matrix scenarios pass.
- FPS remains at least 120.
- Decode p95 remains at most 8 ms.
- Render present gap p95 is populated.
- Render coalescing counters are exported in probe or summary output.

