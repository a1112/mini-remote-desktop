# Low Latency Render Strategy Design

## Goal

Make native LAN rendering suitable for the latency targets agreed on 2026-05-31:

- Local perceptual pipeline p95 target: <= 3 ms, excluding display refresh/vsync pacing.
- LAN incremental/control/transport p95 target: <= 5 ms, excluding the physical scan-out interval.
- Display presentation pacing remains measured separately because a 144 Hz refresh interval is about 6.94 ms.

The immediate scope is the Windows native D3D11 receiver path used by LAN remote desktop sessions.

## Current Findings

The existing D3D11 renderer already has the right low-latency primitives:

- Flip model swapchain with `DXGI_SWAP_EFFECT_FLIP_DISCARD`.
- `Present(0, ...)` rather than vsync-blocking `Present(1, ...)`.
- Tearing support detection and `DXGI_PRESENT_ALLOW_TEARING` when supported.
- `IDXGIDevice1::SetMaximumFrameLatency(1)` by default.
- Opt-in waitable swapchain via `MRD_D3D11_RENDER_WAITABLE_OBJECT=1`.
- Snapshot metadata for swapchain present mode, waitable object, display refresh, and render thread priority.

The latest cross-device 2K144 HEVC/NVDEC/D3D11 report showed:

- `render_upload` p95: about 0.74 ms.
- `render_present` p95: about 0.74 ms.
- `render_pacing_wait` p95: about 6.69 ms.
- `render_present_gap` p95: about 8.02 ms.

That means the D3D11 upload/present work is not the current bottleneck. The remaining latency is mostly pacing and scheduling.

## Official DXGI Guidance

This design follows Microsoft DXGI guidance:

- Waitable swapchains should wait on the frame-latency waitable object before beginning work for the next frame, not after all rendering work is already complete.
- Low latency frame queues should use a maximum frame latency of 1 where supported.
- Variable refresh / tearing presentation requires a swapchain created with tearing support and `Present` with sync interval 0 plus `DXGI_PRESENT_ALLOW_TEARING`.

References:

- https://learn.microsoft.com/en-us/windows/uwp/gaming/reduce-latency-with-dxgi-1-3-swap-chains
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-getframelatencywaitableobject
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-setmaximumframelatency
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/variable-refresh-rate-displays

## Design

### Render Queue Policy

Add an explicit LAN native render queue policy:

- `paced_fifo`: existing behavior for stable frame cadence.
- `latest`: latency-first behavior that presents the newest available frame and drops stale queued frames.

Default native LAN sessions to `paced_fifo`. The latest-frame policy remains available as an explicit low-latency experiment through `MRD_LAN_RENDER_QUEUE_POLICY=latest` or `MRD_LAN_RENDER_QUEUE_POLICY=low_latency`. This keeps the stable visual path as the default while still making latency-first behavior measurable.

The `latest` policy uses the existing `MediaRenderQueueRegistry::take_latest_or_finish` path. When it drops older queued frames, increment `render_stale_frame_drops`. This makes the existing counter meaningful in the service-owned LAN path.

### Pacing Policy

Keep service render pacing available, but make it policy-aware:

- `paced_fifo`: keep the current `render_pacing_wait` behavior.
- `latest`: do not wait a full refresh interval when a fresher frame is already queued. Prefer the newest frame over cadence preservation.

When D3D11 waitable swapchain mode is enabled, skip service-side render pacing. The DXGI frame-latency waitable object becomes the pacing source, and stacking it with `render_pacing_wait` adds an extra refresh-sized delay.

This separates display cadence health from the local pipeline latency budget.

### D3D11 Waitable Policy

Move waitable-swapchain waiting from "right before Present" to a before-render boundary:

- Expose a renderer method that waits for frame-latency availability.
- In waitable mode, call it before upload/draw work.
- Record wait duration and timeout status.
- Keep timeout non-fatal. A timeout records a pacing signal and allows the caller to skip or continue according to policy.

The existing present path can still use nonblocking `Present(0, DO_NOT_WAIT | ALLOW_TEARING)` for the default nonblocking mode. Waitable mode should avoid hiding the wait inside `render_present`; it should be visible as `render_waitable_wait`.

### Telemetry

Add or wire these service-side fields:

- `render_stale_frame_drops`: already exists in IPC/UI types in places, but service LAN runtime needs to increment it when latest-frame mode drops old pending frames.
- `render_queue_policy`: active queue policy, such as `latest` or `paced_fifo`.
- `render_waitable_wait`: p50/p95 stage duration when waitable mode is active.
- `render_waitable_timeouts`: count of frame-latency wait timeouts.
- D3D11 surface metadata into service pipeline snapshots where available:
  - `swap_chain_present_mode`
  - `swap_chain_waitable_object`
  - `swap_chain_allow_tearing`
  - `display_refresh_hz`
  - `render_thread_priority`

### UI Surface

Remote diagnostics should show the native render policy and complete drop breakdown:

- Queue replacements.
- Stale frame drops.
- Lock drops.
- Present skips.
- Present gap p95.
- Waitable wait p95, if available.
- Swapchain mode / waitable / tearing / display Hz.

This keeps the UI aligned with benchmark and IPC metrics.

## Error Handling

- Unsupported tearing remains a capability value, not an error.
- Waitable object creation failure remains a renderer initialization error only when waitable mode was explicitly requested.
- Waitable wait timeout is a metric and skip reason, not a session failure.
- Unknown render queue policy env values fall back to the default policy and are not fatal.

## Validation

Unit and contract tests should cover:

- Queue policy parser defaults and overrides.
- Latest-frame service queue consumes the newest frame and increments stale drops.
- Pacing is bypassed or interrupted correctly in latest mode.
- Renderer waitable wait duration/timeout counters are exposed in snapshots or service metrics.
- IPC and TypeScript types include new optional fields.
- UI diagnostics render stale drops and swapchain mode.

Manual benchmark validation:

- 1080p60 H.264/NVDEC/D3D11 LAN smoke.
- 2K144 HEVC/NVDEC/D3D11 LAN canary.
- 2K144 waitable comparison.
- Compare `render_upload`, `render_present`, `render_pacing_wait`, `render_present_gap`, `render_stale_frame_drops`, and observed render FPS.
