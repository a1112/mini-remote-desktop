# Window Instance Capture And Dynamic Refresh Design

## Goal

Implement HWND-level software window capture as a first-class remote desktop source, with low perceived latency, dynamic refresh-rate control, and support for multiple windows being captured at the same time.

This phase targets **window instance** capture. A selected source such as `windows:window:0x123456` binds the session to that concrete window handle. It does not implement application-level following, process-level source selection, or automatic switching between windows from the same app.

## Context

The repository already has several pieces needed for this:

- `mrd-service` owns LAN session state, capture source selection, media profile negotiation, and the LAN/QUIC media sender.
- `apps/mrd-service/src/capture_source.rs` enumerates Windows capture sources and already emits `windows:window:0x...` source ids.
- `crates/mrd-capture-winrt` supports Windows.Graphics.Capture monitor and window capture, including D3D11 shared BGRA output through `from_window_handle_shared_texture`.
- `apps/mrd-service/src/lan_discovery.rs` already selects capture sources per session and creates either DXGI shared display capture or WinRT capture depending on the selected source.
- Rdesk already has UI plumbing for capture source selection, including local and remote source listing.

The missing work is making the window path an explicit, measured, low-latency mainline path instead of a fallback behind display capture.

## Architecture

The main implementation should live in the service-owned LAN/QUIC path.

1. Rdesk lists and selects a remote capture source.
2. A selected `windows:window:0xHWND` is stored as the session's `CaptureSourceSelection`.
3. The LAN media sender resolves the selected source id when starting or reconfiguring the sender.
4. Windows window sources create a WGC/WinRT capture with shared texture output.
5. The encoder consumes the captured frame using the existing hardware path where possible.
6. Per-session pacing and diagnostics report source kind, source id, selected FPS, observed FPS, drops, capture wait, encode, send, decode, and render timing.

The UI shell should not own capture. It should select sources and show status. Capture, pacing, fallback policy, and session failure behavior belong in `mrd-service`.

## Capture Source Semantics

Window source ids are treated as concrete handles:

- `windows:window:0x123456` selects that exact HWND.
- `windows:window:123456` may be accepted only if existing parsing already permits it, but canonical ids should remain hexadecimal.
- If the HWND is invalid or closed, the session reports source loss. It must not silently fall back to display capture.
- If the window is resized, the sender updates capture dimensions and reconciles the media profile.
- If the window becomes minimized, protected, or temporarily unavailable, the sender should surface a clear status and avoid busy spinning.

The first release should not attempt to follow another window from the same process. That behavior can be added later as an application-level source kind.

## Windows Capture Backend

Windows should use WGC/WinRT for window sources:

- Display shared source: keep using `DxgiSharedTextureCapture`.
- Display non-shared source: keep using WinRT monitor capture.
- Window source: use `WinrtCapture::from_window_handle_shared_texture(hwnd)` when the encoder can consume shared BGRA.
- If shared texture capture is not compatible with the selected encoder on the current host, use the existing CPU WinRT path and mark the memory path in diagnostics.

Window capture should start with the source's native dimensions. For H.264, target dimensions must be even. If profile dimensions do not match the captured shared texture, the sender must either reconfigure to the window dimensions or use a defined GPU/CPU scaling path. It should not fail late with an opaque "requires exact selected profile dimensions" error for normal window sizes.

## Dynamic Refresh-Rate Policy

The sender should use a dynamic pacing policy rather than a fixed high-FPS loop for every selected window.

Inputs:

- Negotiated media profile FPS.
- Capture frame arrival timing.
- Whether frame content or timestamp changed.
- Encode/send queue pressure.
- Session priority and number of active window captures.
- Window availability state.

Recommended initial tiers:

- Active tier: negotiated FPS, capped by refresh/profile and resource budget.
- Warm tier: 30 FPS for recently active windows.
- Idle tier: 10-15 FPS for stable windows.
- Suspended tier: 1-2 FPS or status-only when minimized, closed, protected, or repeatedly timing out.

The policy should be conservative. It can start with simple hysteresis:

- Enter active tier immediately after input activity, new frames, or visible motion.
- Stay active for a short hold window to avoid oscillation.
- Step down only after a quiet interval.
- Step back up immediately on changed frames or control/input activity.

For low perceived latency, frame pacing should keep the existing high-resolution timer and precise sleep guard for high-FPS profiles. Queues should remain shallow: prefer dropping stale captured frames over letting old frames build latency.

## Multi-Window Capture

Multiple sessions may select different `windows:window:*` sources at the same time. The service should handle them as independent capture pipelines with shared resource policy.

Rules:

- Each session owns its capture instance, encoder instance, and sender metrics.
- Source selections are keyed by session id.
- Two sessions may select the same HWND, but this should be visible in diagnostics.
- Resource pressure should reduce FPS tiers rather than blocking input or allowing unbounded queues.
- Foreground or user-focused sessions should get higher dynamic FPS before background sessions.

The first implementation can use a simple per-session cap plus active-session count. More advanced process/GPU budgeting can come later.

## Error Handling

Failures should be explicit and source-specific:

- Invalid source id: reject selection.
- HWND not found: `WINDOW_CAPTURE_SOURCE_NOT_FOUND`.
- WGC item creation fails: `WINDOW_CAPTURE_UNAVAILABLE`.
- Protected content or permission denial: `WINDOW_CAPTURE_DENIED`.
- Window closes after stream start: mark session source lost and stop sender for that source.
- Window dimensions become zero: enter suspended tier and retry briefly before reporting source loss.

Do not silently switch to full-screen capture. That would violate user intent and can leak unrelated desktop contents.

## Diagnostics

Diagnostics should identify the selected source and dynamic FPS state:

- `capture_source_id`
- `capture_source_kind`
- `capture_source_title`
- `capture_memory_path`
- `target_fps`
- `dynamic_fps_tier`
- `observed_capture_fps`
- `capture_wait_p50/p95`
- `encode_p50/p95`
- `send_queue_depth`
- `dropped_stale_frames`
- `source_lost_reason`

Benchmark reports should distinguish display shared capture from WGC window capture.

## Testing

Use TDD for implementation.

Rust service tests should cover:

- Parsing and accepting `windows:window:0x...` source ids.
- Rejecting invalid or empty window source ids.
- LAN sender capture factory selects WinRT shared window capture for window sources.
- Window source selection is stored per session and supports multiple sessions.
- Dynamic FPS policy enters active, warm, idle, and suspended tiers.
- Dynamic FPS policy applies a lower cap when multiple window captures are active.
- Source-lost errors do not fall back to display capture.

Rdesk tests should cover:

- Window sources are visible in the capture source selector.
- Selecting a window source calls `selectRemoteCaptureSource(sessionId, source.id)`.
- Capture source status clearly shows `窗口`/`window` sources.
- Local diagnostic config carries `source_id` and `source_kind` for window sources.

Manual or ignored Windows tests should cover:

- Capture one normal desktop application window.
- Capture two different application windows at the same time.
- Resize a captured window while streaming.
- Close a captured window while streaming and verify no display fallback occurs.
- Compare 60 FPS active window latency against display capture baselines.

## Acceptance

The work is complete when:

- A remote session can select a `windows:window:0x...` source and stream only that software window.
- Window capture uses WGC/WinRT and reports whether it is shared-texture or CPU backed.
- The stream adapts FPS down for idle windows and back up for active/moving windows.
- Two or more window captures can run concurrently without unbounded queues or obvious latency buildup.
- Closing the selected window reports source loss and does not fall back to full-screen capture.
- Focused tests pass, and Windows manual canaries document source ids, FPS behavior, and latency metrics.
