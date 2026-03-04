# Low-Latency Control Layer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a production-ready input control path between `controller-rust` and `agent-rust` that achieves one-way input latency `P95 < 12ms`.

**Architecture:** Add dual RTCDataChannels (`ctrl_rt`, `ctrl_rel`) for unreliable/reliable event classes, binary protocol framing with monotonic sequence and timestamp, bounded per-class queues with coalescing, and agent-side injection workers with latency telemetry (`t0..t5`) and rolling percentiles. Protocol is implemented once in a shared crate to avoid drift.

**Tech Stack:** Rust, webrtc-rs, tokio, serde/bincode-like custom framing, tracing.

---

### Task 1: Add shared control protocol crate (binary frame + event model)

**Files:**
- Create: `common-control-proto/Cargo.toml`
- Create: `common-control-proto/src/lib.rs`
- Modify: `controller-rust/Cargo.toml`
- Modify: `agent-rust/Cargo.toml`
- Modify: `controller-rust/src/input/mod.rs`

1. Write tests for encode/decode roundtrip, unknown type rejection, seq monotonic behavior.
2. Implement frame header: `ver,u8 type,u8 flags,u8 len,u16 seq,u32 ts_us,u64`.
3. Implement payload types: mouse move/button/wheel, key up/down, gamepad axis/button.
4. Run: `cargo test -p common-control-proto -- --nocapture`.
5. Commit: `feat(control): add shared binary control protocol crate and tests`.

### Task 2: End-to-end latency telemetry and SLO checks (before transport optimization)

**Files:**
- Modify: `controller-rust/src/main.rs`
- Modify: `agent-rust/src/runtime_stats.rs`
- Create: `controller-rust/scripts/verify_control_latency.ps1`

1. Record timestamps `t0..t5` and compute one-way stage durations.
2. Report rolling `P50/P95/P99` per event class every 1s.
3. Scope SLO explicitly to LAN/same-city profile and fail if any condition is broken:
   - one-way input latency `P95 < 12ms`
   - `P99 < 18ms`
   - non-droppable event loss rate `0`
4. Set validation window to 10 minutes with 1-second rolling aggregation.
5. Run: `pwsh controller-rust/scripts/verify_control_latency.ps1`.
6. Commit: `chore(obs): add control latency telemetry and slo check`.
### Task 3: Controller dual-channel transport and classification

**Files:**
- Modify: `controller-rust/src/webrtc/peer.rs`
- Modify: `controller-rust/src/main.rs`
- Modify: `controller-rust/src/input/mod.rs`

1. Add `ctrl_rt` channel (`ordered=false,max_retransmits=0`) and `ctrl_rel` (`ordered=true`).
2. Route events by reliability class (`droppable` vs `non-droppable`).
3. Add RT queue depth 8 (drop oldest), REL queue depth 128 (never drop).
4. Add 1ms coalescing for mouse move and gamepad axis on RT path.
5. Run: `cargo test -p controller-rust`.
6. Commit: `feat(controller): add dual datachannel input transport`.

### Task 4: Agent channel receive, queueing, and injection worker (Windows-first)

**Files:**
- Modify: `agent-rust/src/main.rs`
- Create: `agent-rust/src/input_injector.rs`
- Modify: `agent-rust/src/runtime_stats.rs`

1. Register `on_data_channel` and bind handlers for `ctrl_rt`/`ctrl_rel`.
2. Parse binary frames, validate seq/order per channel.
3. Queue by class; process with dedicated worker thread and OS injection adapter.
4. Implement only Windows `SendInput` for keyboard/mouse in this phase; keep gamepad as stub with explicit TODO for virtualization.
5. Run: `cargo test -p agent-rust`.
6. Commit: `feat(agent): add control receive and injection pipeline`.

### Task 5: Integration and soak validation

**Files:**
- Modify: `mini-remote-desktop/README.md`
- Create: `mini-remote-desktop/docs/control-latency-tuning.md`

1. Add config/env docs (`CTRL_RT_QUEUE`, `CTRL_REL_QUEUE`, `CTRL_COALESCE_US`).
2. Run 10-minute scenarios: mouse drag, key spam, gamepad axis/button.
3. Add weak-network regression: 5% packet loss + 20ms jitter profile, verify system remains usable (no SLO claim in this profile).
4. Capture summary table with pass/fail against `P95<12ms`, `P99<18ms`, and non-droppable loss `0` for LAN/same-city profile.
4. Commit: `docs: add control latency tuning and validation results`.
