# Agent Rust Support Matrix

This document summarizes the current supported capabilities in `mini-remote-desktop/agent-rust` based on source code and runtime config.

## Scope

- Project: `mini-remote-desktop/agent-rust`
- Key files reviewed:
- `src/main.rs`
- `src/lib.rs`
- `src/capture_policy.rs`
- `src/capture_runtime.rs`
- `src/encoder_policy.rs`
- `src/encoder_runtime.rs`
- `src/input_injector.rs`
- `src/net_adapt.rs`
- `src/quic_tx.rs`
- `src/webtransport_tx.rs`
- `config.json`

## Current Support List

| Area | Status | Details |
|---|---|---|
| Signaling | Supported | WebSocket signaling (`ws_url`), register handshake, offer/answer, ICE candidate exchange. |
| Protocol negotiation | Supported | Negotiates `webrtc` / `quic` / `webtransport` per session. |
| Media codec | Supported (video) | Advertised codec is `h264`. |
| Capture backends | Supported | `dxgi`, `wgc`, `powershell`, `dummy`, with fallback policy. |
| Encoder backends | Supported | `nvenc`, `qsv`, `amf`, `openh264`, with fallback policy and strict mode. |
| WebRTC media path | Supported | RTP H264 send loop with manual packetizer strategy and IDR bootstrap gating. |
| QUIC media path | Supported | Access Unit send over QUIC uni-stream with frame envelope and queueing. |
| WebTransport media path | Supported | WebTransport uni-stream send with self-signed cert + hash advertisement. |
| RTCP feedback loop | Supported | Handles PLI/FIR/NACK/REMB and updates adaptive targets. |
| Adaptive quality control | Supported | Tiered FPS/bitrate adaptation, recovery logic, RTT-based network type detection hooks. |
| Dynamic capture update | Supported | `control/updateCapture` applies patch and restarts peer sessions. |
| Input injection | Partial | Mouse move/button/wheel + keyboard via Windows SendInput. |
| Gamepad control | Not fully supported | Gamepad event branch exists but is stubbed (warning only). |
| Multi-client sessions | Supported | Session map with per-controller connection, max-client guard (`AGENT_MAX_CLIENTS`). |
| Stats/observability | Supported | Runtime stats panel, RTCP counters, control latency panel logs. |
| Platform runtime | Windows-focused | Capability advertises `platforms: ["windows"]`; Windows capture/input paths are primary. |

## Configuration Surface (High-Level)

Main configurable dimensions in `CaptureConfig`:

- Capture and render pacing: `fps`, `min_fps`, `max_fps`, `idle_repeat_fps`, `frame_pacing_enable`, `queue_depth`, `queue_strategy`
- Capture backend and fallback: `backend`, `allow_fallback`, `strict_gpu_direct`
- Encoder and rate control: `encoder`, `allow_encoder_fallback`, `encoder_preset`, `encoder_tune`, `rc_mode`, `bitrate_kbps`, `max_bitrate_kbps`, `gop`, `bframes`
- RTP path options: `rtp_use_manual_packetizer`, `rtp_mtu`, `rtp_au_align`, `force_idr_on_pli`, `idr_interval_sec`
- Adaptation and profile: `adapt_enable`, `adapt_mode`, `performance_profile`, `profile_template`, `network_adapt_enable`
- Multi-tier limits: `tier_limit_enable` + L1-L5 FPS/bitrate ladder settings

## Known Gaps

| Gap | Impact |
|---|---|
| No audio pipeline | Remote desktop experience is video-only; missing voice/system-audio use cases. |
| Gamepad not implemented | Controller/gamepad scenarios cannot be executed end-to-end. |
| Clipboard/file transfer not in agent-rust data path | Productivity workflows remain incomplete for desktop control. |
| Cross-platform execution is limited | Linux/macOS capture/input parity is not complete in this Rust agent. |
| WebRTC codec diversity not exposed | H264-only path limits compatibility/performance tuning options. |

## Recommended Expansion Directions

### Priority 1: Complete control-plane ergonomics

- Implement gamepad injection path (axis/button mapping and deadzone handling).
- Add clipboard sync as reliable control channel messages.
- Add file transfer control/data protocol (chunking, resume, integrity check).

Why first:

- These features directly increase user-perceived completeness of remote control.
- They reuse existing reliable/realtime control channel architecture.

### Priority 2: Add audio streaming and A/V synchronization

- Add optional audio capture and transport channel.
- Define sync strategy between video timestamps and audio playout timestamps.
- Expose config toggles for audio bitrate/latency profile.

Why second:

- Audio is a major capability gap for practical remote desktop usage.
- Existing transport/metrics code can be extended for audio telemetry.

### Priority 3: Transport hardening and resilience

- Unify QUIC/WebTransport sender behavior and backpressure policy.
- Add better reconnect and session-resume behavior for transient network loss.
- Add end-to-end transport compatibility tests across protocol modes.

Why third:

- Current core works, but robustness under unstable networks is where production quality is decided.

### Priority 4: Cross-platform roadmap

- Introduce Linux/macOS capture backend abstractions with capability advert updates.
- Separate platform-specific input injectors behind trait-based interfaces.

Why fourth:

- Enables wider deployment while preserving current Windows-optimized path.

## Practical Next-Step Plan

- Step 1: Implement gamepad injection (smallest high-impact gap).
- Step 2: Add clipboard sync over reliable channel.
- Step 3: Add audio pipeline prototype behind feature flag.
- Step 4: Add protocol-mode integration tests (webrtc/quic/webtransport matrix).

## Validation Checklist for Future Extensions

- Unit tests for config normalization and fallback behavior.
- Integration tests for offer negotiation by selected transport.
- Runtime soak tests with adaptive mode on/off and tier limit on/off.
- Latency and dropped-frame metrics baselines captured per protocol mode.
