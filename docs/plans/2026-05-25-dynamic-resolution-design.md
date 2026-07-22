# Dynamic Resolution Design

## Goal

Add explicit dynamic resolution control for LAN remote display and application-window sessions. The feature lowers sampling and encoded video resolution when enabled, without changing the remote source's logical size and without cropping the source image.

## Requirements

- Dynamic resolution is opt-in through configuration. Existing adaptive media users keep fixed resolution unless they explicitly enable dynamic resolution.
- Resolution changes lower the encoded frame size only. The receiver continues to render into the existing display surface or browser canvas.
- The full source image must remain visible. Lower-resolution frames are produced by proportional scaling, not by center crop or source-region crop.
- Single-window and multi-window sessions remain independent. Each session keeps its own selected source, ladder, and media profile negotiation.
- Dynamic window FPS remains responsible for idle/background frame-rate reduction. Dynamic resolution handles sustained encode/decode/render/transport pressure.

## Current State

`AdaptiveMediaConfig` already drives a keyframe ladder with bitrate, FPS, and resolution entries. `media_adaptation` applies a selected ladder rung by requesting a LAN media profile update; the sender then recreates capture and encoder when the selected profile changes.

CPU frame preparation in `prepare_frame_for_h264` scales the full frame to the selected profile dimensions. Windows shared texture paths are different: `DxgiSharedTextureCapture` and WinRT shared window capture copy a centered source region when target dimensions are smaller than the source. That behavior is correct for region capture, but it violates dynamic resolution's "no crop" requirement.

## Design

Add a `dynamic_resolution_enabled` boolean to `AdaptiveMediaConfig`, defaulting to `false` for backward-compatible protocol behavior.

When `dynamic_resolution_enabled` is `false`, the effective adaptive ladder keeps the current negotiated width and height for every rung while still allowing bitrate and FPS changes. This keeps the existing adaptive controller useful for low-latency stability without triggering resolution reconfiguration.

When `dynamic_resolution_enabled` is `true`, the effective ladder may include lower width and height rungs. The ladder generation continues to preserve the selected capture source aspect ratio and clamps dimensions to even H.264-compatible values.

For Windows shared capture, low-resolution dynamic rungs must not use the existing shared texture crop behavior. The first implementation will select a scaling-safe capture path whenever the selected profile dimensions are smaller than the selected source dimensions. Full-size profiles can continue to use shared texture capture for low latency. A later optimization can add D3D11 GPU scaling to keep low-resolution rungs on a zero-copy path.

## Data Flow

1. UI or automation sends `ConfigureMediaAdaptation` with `dynamic_resolution_enabled`.
2. `media_adaptation::effective_ladder` builds a session ladder from the current selected source and current profile.
3. If dynamic resolution is disabled, ladder entries inherit the current selected width and height.
4. If dynamic resolution is enabled, lower rungs can reduce width and height while preserving source aspect ratio.
5. The adaptation task requests profile updates for downshift/upshift decisions.
6. The sender recreates capture and encoder when the active profile changes.
7. Receiver rendering continues to stretch decoded frames into the existing render surface.

## Testing

- IPC contract round-trip proves the new config field defaults and serializes correctly.
- Media adaptation unit tests prove disabled dynamic resolution keeps width and height fixed while still changing bitrate/FPS.
- Media adaptation unit tests prove enabled dynamic resolution includes lower source-aspect rungs.
- LAN discovery tests prove Windows shared capture backend avoids crop-prone shared paths for reduced-resolution profiles and keeps shared paths for full-size profiles.
- Service and IPC package tests verify the migration does not break session or protocol behavior.
