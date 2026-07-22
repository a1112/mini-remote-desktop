# QUIC Mainline Session Design

**Date:** 2026-03-16

**Goal:** Add a clean, selectable QUIC session path to the main application flow while keeping the existing realtime/signaling channel as the control plane and avoiding WebRTC in verification for this milestone.

## Scope

This design targets the existing QUIC transport crate, [crates/mrd-transport-quic-quinn/src/lib.rs](G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-transport-quic-quinn\src\lib.rs), and promotes it from benchmark-only infrastructure into a real session transport option in the app.

In scope:
- Session negotiation chooses `webrtc` or `quic_quinn`.
- Realtime/signaling remains the control plane for session discovery and transport bootstrap.
- QUIC becomes a first-class media transport for the actual sender/receiver path.
- Verification focuses on QUIC component tests, QUIC benchmark harnesses, and QUIC app-path integration tests.

Out of scope:
- Replacing realtime/signaling with QUIC-native discovery.
- NAT traversal parity with WebRTC ICE.
- QUIC video/data multiplexing beyond the current H264 access-unit path.
- D3D11VA or WebRTC-specific stabilization work.

## Recommended Approach

The recommended approach is a split control-plane/data-plane design:

- Keep realtime/signaling for session bootstrap, role coordination, and transport metadata exchange.
- Add QUIC-specific signaling payloads for endpoint and certificate material.
- Introduce a QUIC host/session path parallel to the existing WebRTC path rather than mixing QUIC logic into WebRTC types.
- Reuse the existing frame sink, observability, decode path, and benchmark artifact pipeline.

This is preferred over retrofitting QUIC into `webrtc_*` types because it keeps the transport boundary explicit and reduces the chance of turning the main session layer into a transport-specific branch maze.

## Architecture

### Control Plane

The control plane remains the existing realtime/signaling path in:
- [apps/Rdesk/src-tauri/src/main.rs](G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs)
- [apps/Rdesk/src-tauri/src/realtime_runtime.rs](G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\realtime_runtime.rs)
- [crates/mrd-signal-proto](G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-signal-proto)

The new requirement is that session messages carry:
- selected transport: `webrtc` or `quic_quinn`
- QUIC listener address
- certificate or trust material sufficient for a direct QUIC connection

The control plane does not carry media.

### Data Plane

The data plane adds a QUIC-hosted session path parallel to the current WebRTC path:

- `quic_host.rs`
  - manages live QUIC sender/receiver runtime
  - owns endpoint lifecycle and decode loop
  - records per-session probe state
- `quic_session.rs`
  - stores negotiated session metadata and bootstrap materials
  - mirrors the role of the current WebRTC session coordinator, but without SDP/ICE semantics

The actual packet transport remains the `quinn` datagram + AU fragment/reassembly model already present in [crates/mrd-transport-quic-quinn/src/lib.rs](G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-transport-quic-quinn\src\lib.rs).

### Shared App Layer

These existing components stay shared:
- frame capture and encoder creation
- decode backend selection
- decoded frame sink
- render host
- observability snapshots and benchmark artifact writing

The app-level session entry points become transport-aware rather than WebRTC-only.

## Data Flow

### Session Bootstrap

1. Controller creates or requests a session with `transport=quic_quinn`.
2. Agent prepares a QUIC listener and ephemeral certificate material.
3. Agent publishes bootstrap data through realtime/signaling.
4. Controller receives bootstrap data and connects with `quinn`.
5. Both sides mark the session as established in QUIC session state.

### Media Send

1. Sender captures a frame.
2. Sender encodes H264 access units.
3. QUIC sender fragments AUs using `fragment_access_unit(...)`.
4. QUIC datagrams are written over the live connection.
5. Sender records `CaptureCopy`, `EncodeTotal`, and `SendWrite`.

### Media Receive

1. Receiver reads QUIC datagrams.
2. Reassembler reconstructs complete access units.
3. Decoder consumes H264 access units.
4. Decoded frames flow into the shared frame sink and render path.
5. Receiver records `NetworkIngress`, `DecodeTotal`, and `FrameSinkIngest`.

## Error Handling

The QUIC path should fail clearly at the transport boundary:

- bootstrap mismatch:
  - invalid or incomplete QUIC metadata in signaling should fail before session start
- connect failure:
  - endpoint, certificate, or peer-address failure should surface as explicit session errors
- datagram/reassembly failure:
  - increment probe counters and dropped-frame counters, but do not silently convert to success
- disconnect:
  - surface a clean session shutdown signal and stop sender/receiver loops deterministically

The benchmark/harness path must retain bounded teardown and artifact writing even when a QUIC run fails.

## Testing Strategy

Verification for this milestone intentionally avoids WebRTC.

Required coverage:
- QUIC transport crate tests continue to pass.
- QUIC app harness tests prove:
  - session bootstrap through signaling metadata
  - live frame delivery to frame sink
  - observability counters and stage snapshots
  - bounded shutdown without leaked processes
- benchmark/component cases:
  - QUIC sender component
  - QUIC receiver component
  - QUIC transport benchmark scenario

Regression focus:
- the main app command surface must select QUIC explicitly without regressing the existing WebRTC path
- QUIC bootstrap data must be stable and serializable through signal messages

## Merge Strategy

The implementation should land on a clean branch from `main`, with frequent commits:

1. signaling + session metadata
2. QUIC app host/session path
3. app command routing and runtime snapshots
4. QUIC benchmark/component validation

This keeps the salvage branch as historical recovery only and makes the QUIC work mergeable to `main` on its own merits.
