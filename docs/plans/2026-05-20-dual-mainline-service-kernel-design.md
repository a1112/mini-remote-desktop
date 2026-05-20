# Dual Mainline Service Kernel Design

## Context

The project already has a strong LAN media engine baseline, but the product surface is still uneven: WAN remote control, device trust, cross-platform control, and repeatable end-to-end verification need to become first-class architecture, not scattered feature work.

This design locks the next direction as two transport mainlines on one service-owned runtime:

- WAN remote control uses WebRTC with ICE/TURN as the product default.
- LAN and managed-network high refresh use QUIC datagram media with hardware encode/decode and native rendering.
- `mrd-service` owns session state, capability decisions, transport policy, media runtime, control channels, trust, audit, and telemetry.
- `Rdesk` stays a thin UI shell for commands, native surface lifecycle, and state display.

## Service Kernel

`mrd-service` is the only runtime owner for remote sessions.

The target service kernel is composed of these owned modules:

- `SessionRuntime`: lifecycle, peer binding, role, and high-level health.
- `MediaRuntime`: capture, scale/convert, encode, packetize, transport receive/reassemble, decode, render upload, present, and stage metrics.
- `ControlRuntime`: reliable control lane and low-latency realtime lane.
- `Capability/Profile Engine`: local and peer capability snapshots, display/source-aware profile generation, and scenario preflight.
- `Transport Policy`: chooses WebRTC, QUIC, relay, or blocked/degraded state from network type, capabilities, profile, and observed health.
- `Identity/Trust`: device identity, pairing approval, certificate fingerprint pinning, revocation, and reconnect trust state.
- `Telemetry Store`: metrics, events, logs, artifacts, and visual integrity signals bound to `run_id` and `session_id`.

Rdesk must not own high-bandwidth frame data or derive session truth from UI-local state. It should issue IPC commands, subscribe to snapshots, attach/detach native render surfaces, and present diagnostics.

## Session State

All user-visible sessions should report one normalized lifecycle:

`created -> listening|connecting -> connected -> streaming -> degraded|failed|closed`

`degraded` is not a generic warning. It must include one of the fixed reason classes:

- `network_loss`
- `encode_budget`
- `decode_budget`
- `render_budget`
- `profile_mismatch`
- `peer_version_mismatch`
- `security_blocked`

## Transport Policy

WAN mode is WebRTC-first:

- ICE/STUN/TURN for NAT traversal.
- RTP media as the default WAN media path.
- Two DataChannel lanes for control: reliable ordered and realtime lossy/coalesced.
- QUIC direct connect is not required for public internet acceptance.

LAN mode is QUIC-first:

- `quic_datagram_media_v3` for media.
- Reliable stream for configuration, profile updates, and critical control.
- High refresh profiles such as `LAN 2K144`, `LAN 2K180`, and `1600p165` are accepted only on the native media path, not WebView frame preview.

Transport policy must be service-owned. UI can request an intent such as `auto`, `lan`, `wan`, or `diagnostic`, but cannot directly force a transport that fails capability, trust, or profile preflight.

## Capability And Profile Engine

Capability snapshots must cover:

- capture: DXGI, WinRT fallback, PipeWire, platform capture status.
- encode: NVENC H.265/H.264, software fallback, bit depth and chroma support.
- decode: NVDEC H.265/H.264, software fallback.
- render: D3D11 native/shared, OpenGL interop, DX12 preview, Web fallback.
- display: source resolution, refresh rates, scaling, HDR, monitor id.
- memory path: GPU shared texture, CPU copy, NV12/P010 path.
- transport: WebRTC, TURN, QUIC datagram media version.
- control: keyboard, mouse, clipboard, file/control capabilities.
- security: identity, pairing state, certificate/fingerprint status.
- audio: capture/playback availability and codec support.

Scenario profiles are product-level targets, not raw UI presets:

- `WAN 1080p60`
- `LAN 2K144`
- `LAN 2K180`
- `1600p165`
- `quality 4K60`

Preflight returns `ready`, `degraded`, `blocked`, or `skipped`. Every non-ready result must include reason, suggested action, and a downgrade profile where applicable.

Profiles must be generated from the actual selected capture source. A requested profile mismatch with the real source is a profile decision, not a transport or decoder failure.

## Media Runtime

The canonical pipeline is:

`capture -> scale/convert -> encode -> packetize -> transport -> reassemble -> decode -> render_upload -> present`

Windows-first native baseline:

`DXGI shared -> NVENC H.265/H.264 -> QUIC datagram media -> NVDEC -> D3D11 shared/native render`

Web fallback is diagnostic only. It may show low-rate previews, but it is not a high-performance remote display path.

Adaptation should use profile ladder switches at keyframe boundaries before depending on encoder hot reconfiguration. Each switch must emit telemetry for old profile, new profile, reason, transition time, and stall duration.

## Control Runtime

The control plane is split into two lanes:

- `ctrl_rel`: reliable ordered lane for keyboard critical events, clipboard, file control, authorization, audit, profile updates, and device/session commands.
- `ctrl_rt`: low-latency lane for mouse movement, wheel, cursor hints, and prediction-style events. It may drop, merge, or replace stale events.

Windows is the product baseline for complete input and clipboard. Linux and macOS should be marked `partial` or `preview` until equivalent control coverage is implemented and tested.

## Identity And Audit

Transport encryption is not enough to answer "who am I connected to". Device identity and trust must be explicit.

Required model:

- stable device id.
- first-pair approval.
- certificate or public-key fingerprint pinning.
- revoke and re-pair.
- session consent.
- reconnect trust state.

Audit events cover:

- pairing requested, approved, revoked.
- session connect, stream start, degrade, recover, disconnect.
- input control granted or denied.
- clipboard and file control actions.
- profile downgrade and transport fallback.
- abnormal disconnect and service crash.

## Telemetry And Test Model

All tests and sessions should write the same telemetry model:

- metrics: FPS, bitrate, drop ratio, queue depth, p50/p95 stage times, profile transition counters.
- events: transport selection, profile evaluation, adaptation decision, identity/audit event, visual integrity warning.
- logs: source-tagged structured logs.
- artifacts: JSON/MD reports, screenshots, trace files, raw captures when enabled.

The four required verification modes share one report schema:

- single-process local baseline.
- local dual-process LAN simulation.
- LAN cross-device Windows peer.
- WAN WebRTC/TURN.

Visual integrity is part of acceptance. Severe distortion, tearing, long non-refresh windows, and repeated burst pacing must be classified and visible in the report.

## IPC Direction

The service-facing IPC surface should converge on these groups:

- capability: `GetCapabilitySnapshot`, `GetPeerCapabilitySnapshot`, `EvaluateScenarioProfile`.
- session: `StartSession`, `StopSession`, `SubscribeSessionSnapshot`.
- transport/media: `SetTransportPolicy`, `UpdateMediaProfile`, `ConfigureMediaAdaptation`, `AttachRenderSurface`, `DetachRenderSurface`.
- trust: `PairDevice`, `ApprovePairing`, `RevokeDevice`, `GetDeviceIdentitySnapshot`, `GetAuditEvents`.
- telemetry: `GetTelemetryBundle`.
- control: `GetControlChannelSnapshot` and later explicit control injection commands.

Old peers that do not expose protocol version, media capability, or trust fields should be skipped or fail-fast with `peer_version_mismatch`. They must not enter high-refresh matrix runs.

## First Implementation Slice

The first slice establishes the contract boundary:

- IPC DTOs for scenario evaluation, transport policy snapshot, control channel snapshot, device identity snapshot, and telemetry bundle.
- `mrd-service` handlers for local scenario evaluation, peer capability snapshot, transport policy, pairing approval/revocation, device identity, control channel snapshot, and telemetry bundle.
- Capability evaluator based on the existing service capability snapshot and scenario profiles.
- Smoke and contract tests proving the new IPC path serializes and crosses the service handler.

This is intentionally a structural slice. It does not claim to complete WAN WebRTC, persistent trust storage, full telemetry persistence, or runtime transport switching.

## Acceptance Roadmap

P0 acceptance:

- `EvaluateScenarioProfile` explains `ready/degraded/blocked/skipped` before session start.
- LAN `2K144@80Mbps HEVC` passes local dual-process with decoded/render FPS >= 115.
- LAN cross-device reaches at least 80% of the matching local profile where selected profiles match.
- No high-refresh acceptance depends on WebView frame data.
- Telemetry pages can open metrics, events, logs, and artifacts for the same `run_id`.

P1 acceptance:

- WebRTC/TURN WAN mode connects without QUIC direct reachability.
- Device pairing, fingerprint pinning, revoke, and re-pair survive service restart.
- Control lanes separate reliable keyboard/clipboard from coalesced mouse movement.

P2 acceptance:

- Browser gateway and RDP compatibility have separate product acceptance and do not weaken LAN native acceptance.
- Linux/macOS capabilities are truthful and do not report parity before control/media paths pass their own matrices.
