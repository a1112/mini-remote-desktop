# Session Agent Native Render Design

## Goal

Move receiver-side desktop-bound decode and presentation into the authenticated interactive-session agent while keeping session policy, transport, and authorization ownership in `mrd-service`.

## Chosen approach

Use a hybrid Windows render adapter. It prefers NVDEC output exposed as D3D11 shared textures and presents those textures through `mrd-render-d3d11`. If NVDEC cannot initialize, it falls back to the in-process software H.264 decoder and converts its decoded frames into a D3D11-supported CPU format. Render capability is advertised only when the production adapter can initialize a viable decoder and renderer path.

The alternatives were rejected as product defaults: a hardware-only adapter excludes non-NVIDIA systems, while a software-only adapter unnecessarily makes CPU copies and latency the normal path.

## Ownership and data flow

1. Rdesk creates the native child window and sends its stable `surface_id` and HWND to `mrd-service`.
2. `mrd-service` retains session, transport, consent, grant, and route ownership. A signed `StartRender` command binds the exact session, resource, surface identity, and HWND.
3. The authenticated session agent creates one render worker for that immutable resource. The worker owns its decoder and D3D11 renderer and attaches only to the authorized HWND.
4. The service forwards bounded encoded access units over agent IPC. No raw frame enters Rdesk or crosses the service-agent boundary.
5. The worker decodes and presents frames. NVDEC shared textures remain on the GPU; the fallback converts software I420 output to a renderer-supported CPU frame.
6. `StopRender`, session revocation, desktop change, disconnect, renderer failure, or invalid HWND tears down the exact worker and clears its queue.

## Concurrency and backpressure

Decoder and D3D11 operations remain on one dedicated worker thread per render resource so device/context ownership is stable. The IPC/runtime thread submits to a bounded channel with latest-frame semantics for disposable interframes. A keyframe is never silently replaced by an interframe. Queue admission and replacement counters are exposed for process-boundary timing and loss diagnostics.

## Failure and capability semantics

- Invalid, zero, stale, or mismatched HWND/surface bindings fail before resource activation.
- Decoder selection tries `nvdec_d3d11_shared` first and `h264_software` second.
- Render is advertised only if the adapter factory proves that at least one decoder path and the D3D11 renderer can initialize.
- Start succeeds only after the worker has attached the HWND and initialized its pipeline.
- Decode, presentation, or worker failure makes subsequent submissions fail closed and causes exact resource cleanup; it never falls back to service-local rendering after agent ownership was established.
- Capability reporting does not claim Capture until the production capture adapter is assembled alongside the render adapter.

## Testing and evidence

- Unit tests use injected decoder/renderer factories to prove hardware preference, software fallback, frame conversion, exact HWND attachment, bounded queue behavior, and teardown.
- Runtime tests prove capability truthfulness and that render commands reach only the exact authorized resource.
- The Task 25 dual-process integration test proves encoded units reach the agent adapter, revocation clears work, and Rdesk carries no raw frames.
- The local dual-process LAN canary records boundary enqueue, decode, and present timings and compares the agent path with the retained local baseline.

