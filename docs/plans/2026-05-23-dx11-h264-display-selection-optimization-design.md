# DX11 H.264 Display Selection Optimization Design

## Goal

Complete the local simulated cross-device optimization for the Windows DX11 + H.264 path, with explicit multi-monitor selection for a 2K144 display and a 4K120 display.

The primary performance baseline is the service-owned local dual-process LAN/QUIC path. The Rdesk WebRTC/WebCodecs local preview remains a diagnostic path and must not define high-refresh acceptance.

## Context

The repository already has most of the remote-display primitives:

- `mrd-service` owns LAN media sender state, selected capture source state, media profile negotiation, and display mode control.
- Windows remote LAN capture can already create `DxgiSharedTextureCapture::new_for_device_name()` from a selected `windows:display-shared:N` source.
- Rdesk remote windows already list and select remote capture sources.
- The local browser preview paths still call `DxgiSharedTextureCapture::new_primary()` and cannot target a non-primary display.
- The local test page hides capture source selection for `local-display-test` sessions.

## Architecture

### B. Primary Baseline: Local Dual-Process LAN/QUIC

The local simulated cross-device baseline should reuse the same service-owned contracts as true LAN peers:

1. Enumerate selectable capture sources through the existing capture-source IPC/LAN model.
2. Select `windows:display-shared:N` before starting or reconfiguring the sender.
3. Run H.264 profiles over the LAN media runtime, preferably QUIC datagram media and native D3D11/NVDEC rendering where available.
4. Record the actual selected source id, source resolution, active media profile, FPS, bitrate, and stage metrics in diagnostics or reports.

This keeps the baseline close to real cross-device behavior. The UI may request a profile such as `2560x1440@144/H.264` or `3840x2160@120/H.264`, but the service decides whether the selected source and runtime can satisfy it.

### A. Diagnostic Preview: Local WebRTC/WebCodecs

The browser preview path gets the same display selection, but only for diagnostics:

1. Add optional `source_id` to WebRTC and WebCodecs preview start requests.
2. If `source_id` is present and points to a Windows display source, create the matching DXGI shared capture.
3. If absent, keep the current primary-display behavior for backward compatibility.
4. Rdesk passes the selected local capture source id when starting WebRTC or WebCodecs preview.

The browser path can be used to inspect first-frame latency, browser decode behavior, and rough capture/encode behavior. It is not the acceptance path for DX11 + H.264 performance.

## UI Behavior

For local display tests, Rdesk should expose a capture-source selector in the test configuration panel:

- Display shared sources appear first.
- Each display option shows source kind, title, resolution, and preview when available.
- Choosing the 2K144 or 4K120 display updates local test configuration and restarts the diagnostic preview when needed.
- Remote sessions keep their existing remote capture-source selector.

The local selector should use the existing `CaptureSource` shape rather than inventing a parallel monitor DTO.

## Service Behavior

The service should preserve one source of truth:

- `CaptureSourceSelection` remains keyed by session id.
- LAN sender setup reads the selected source id and builds the capture backend from it.
- Windows display shared sources use DXGI shared capture.
- Windows display copy/window fallbacks keep using existing WinRT/scrap paths.
- Browser preview start requests may resolve a source id directly without mutating LAN session selection, because they are diagnostic preview sessions.

## Optimization Target

The first optimization pass should tune and verify H.264 for two explicit profiles:

- `2560x1440 @ 144 FPS`, H.264, NVENC, DXGI shared, low latency, QUIC/native baseline.
- `3840x2160 @ 120 FPS`, H.264, NVENC, DXGI shared, low latency, QUIC/native baseline.

The implementation should prefer measurement over speculative tuning:

- Capture p50/p95.
- Encode p50/p95.
- Transport send queue depth and drops.
- Decode/render p50/p95 where native receiver is active.
- Actual displayed FPS.

## Error Handling

Source selection failures should be explicit:

- Empty or unknown source id: reject before sender start.
- Display source not backed by DXGI shared output: fall back only when the user chose a fallback source or the UI explicitly allows fallback.
- Profile larger than the selected display: clamp or downgrade through the existing media profile negotiation path, not by stretching.
- Browser preview unsupported: report diagnostic-only failure without failing the service-owned LAN baseline.

## Testing

Use TDD for implementation.

Rust tests should cover:

- Preview request `source_id` serde.
- Windows display source id parsing into a DXGI shared target helper.
- Browser preview capture factory selecting a requested display instead of primary.
- LAN sender capture configuration changing when selected source changes.

Rdesk tests should cover:

- Local display tests show capture-source selection.
- Selecting `windows:display-shared:1` passes `sourceId` into WebRTC preview start.
- WebCodecs worker start receives `sourceId`.
- Local test run config includes `source_id` for the selected display.
- Remote capture-source behavior remains unchanged.

Verification commands should include:

- `cargo test -p mrd-service browser_webrtc_preview browser_webcodecs_preview lan_discovery -- --nocapture`
- `cargo test -p mrd-capture-dxgi`
- `pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/adapters/tauri/commands.webBridge.test.ts`
- `pnpm --dir apps/Rdesk type-check`

## Acceptance

The work is complete when:

- The local dual-process LAN/QUIC baseline can target either attached display by source id.
- The selected display is visible in diagnostics/report output.
- The WebRTC/WebCodecs local preview can target the same display id for diagnosis.
- 2K144 and 4K120 H.264 runs do not silently fall back to the primary monitor.
- Browser-preview limitations do not block the service-owned native baseline.
