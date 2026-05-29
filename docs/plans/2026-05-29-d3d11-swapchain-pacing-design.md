# D3D11 Swapchain Pacing Design

## Goal

Make D3D11 presentation pacing explicit and comparable without changing the default low-latency path blindly.

The current 2K144 hardware chain already reaches about 143 FPS with sub-2 ms NVDEC p95. The remaining instability is at the presentation boundary: the renderer uses nonblocking `Present(0, DO_NOT_WAIT)` and records occasional skipped presents. This keeps the media pipeline responsive, but it does not expose enough swapchain policy or display-refresh context to explain present p95.

## Current Behavior

- `IDXGIDevice1::SetMaximumFrameLatency(1)` is configured by default.
- Swapchain tearing is enabled when supported.
- Present defaults to `Present(0, DXGI_PRESENT_DO_NOT_WAIT | DXGI_PRESENT_ALLOW_TEARING)`.
- `DXGI_ERROR_WAS_STILL_DRAWING` is counted as a skipped present.
- The benchmark harness now decouples render submission from the main media loop and records latest-frame replacements.
- The swapchain waitable object is intentionally off today.

## Design

Keep the current nonblocking present path as the default. Add an opt-in waitable swapchain pacing mode for benchmark comparison:

- `MRD_D3D11_RENDER_WAITABLE_OBJECT=1`
  - Adds `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT`.
  - Casts the swapchain to `IDXGISwapChain2`.
  - Configures `SetMaximumFrameLatency(1)` on the swapchain when available.
  - Waits for the frame-latency waitable object before presenting, with a short timeout.
  - If the wait times out, skip/record rather than blocking the media pipeline.

Add render diagnostics to `RendererSnapshot` and benchmark output:

- `swap_chain_waitable_object`
- `swap_chain_present_mode`
- `display_refresh_hz`
- `render_thread_priority`

Add harness-only thread-priority opt-in:

- `MRD_RENDER_THREAD_PRIORITY=above_normal`
- Default remains normal priority.

## Error Handling

- Failure to create a waitable object when the mode is requested is a hard renderer initialization error.
- Wait timeout is a pacing signal, not a fatal error.
- Unsupported thread priority values are ignored and reported as `normal`.

## Testing

Unit tests:

- D3D11 swapchain desc keeps waitable off by default.
- D3D11 swapchain desc enables waitable when requested.
- Present mode reports `nonblocking` by default and `waitable` when requested.
- Benchmark summary exports swapchain/display/thread pacing diagnostics.

Manual benchmark comparison:

- `quick.transport.webrtc.nvenc.h264_nvdec.2k144.json`
- `quick.transport.webrtc.nvenc.hevc_nvdec.2k144.json`

Compare default mode against:

- `MRD_D3D11_RENDER_WAITABLE_OBJECT=1`
- `MRD_RENDER_THREAD_PRIORITY=above_normal`

