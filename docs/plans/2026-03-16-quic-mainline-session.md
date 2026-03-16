# QUIC Mainline Session Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a selectable `quic_quinn` session path to the real app flow using realtime/signaling as the control plane and QUIC datagrams as the media transport.

**Architecture:** Keep signaling for bootstrap metadata, add QUIC-specific session state beside the current WebRTC path, and route actual encoded H264 access units through the existing Quinn datagram/reassembly transport. Reuse the shared frame sink, observability, benchmark reporting, and render pipeline instead of building QUIC-only duplicates.

**Tech Stack:** Rust, Tauri, Tokio, Quinn, Rustls, existing `mrd-transport-quic-quinn`, `mrd-observability`, and app session/runtime modules.

---

### Task 1: Add QUIC transport selection to signaling state

**Files:**
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-signal-proto\src\lib.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-signal-client\src\lib.rs`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-signal-proto\src\lib.rs`

**Step 1: Write the failing test**

Add a serialization round-trip test for a session message carrying:

```rust
transport: "quic_quinn".into(),
quic_listen_addr: Some("127.0.0.1:5000".into()),
quic_server_name: Some("localhost".into()),
quic_cert_der_b64: Some("...".into()),
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-signal-proto quic -- --nocapture`

Expected: FAIL because the transport/bootstrap fields do not exist yet.

**Step 3: Write minimal implementation**

Add a transport selector plus minimal QUIC bootstrap fields to the session signaling types. Keep names explicit and JSON-friendly.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-signal-proto quic -- --nocapture`

Expected: PASS with the new fields preserved across encode/decode.

**Step 5: Commit**

```bash
git add crates/mrd-signal-proto/src/lib.rs crates/mrd-signal-client/src/lib.rs
git commit -m "feat: add quic transport bootstrap metadata"
```

### Task 2: Promote Quinn transport crate from loopback-only helper to app bootstrap API

**Files:**
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-transport-quic-quinn\src\lib.rs`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\crates\mrd-transport-quic-quinn\tests\loopback.rs`

**Step 1: Write the failing test**

Add a test for:
- building a listener/bootstrap bundle
- connecting a client with that bundle
- exchanging one datagram after bootstrap

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-transport-quic-quinn loopback_pair -- --nocapture`

Expected: FAIL because the crate only exposes `loopback()` and not reusable bootstrap helpers.

**Step 3: Write minimal implementation**

Add explicit app-facing bootstrap helpers, for example:

```rust
pub struct QuinnBootstrap {
    pub listen_addr: SocketAddr,
    pub server_name: String,
    pub cert_der: Vec<u8>,
}
```

and server/client constructors that use real bootstrap material instead of the built-in loopback shortcut.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-transport-quic-quinn -- --nocapture`

Expected: PASS, including existing fragmentation/reassembly tests.

**Step 5: Commit**

```bash
git add crates/mrd-transport-quic-quinn/src/lib.rs crates/mrd-transport-quic-quinn/tests/loopback.rs
git commit -m "feat: add quinn bootstrap endpoints"
```

### Task 3: Introduce QUIC session state in the app layer

**Files:**
- Create: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\quic_session.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\quic_session.rs`

**Step 1: Write the failing test**

Add a test that records:
- requested transport
- QUIC bootstrap material
- session snapshot fields visible after applying remote data

**Step 2: Run test to verify it fails**

Run: `cargo test -p app quic_session -- --nocapture`

Expected: FAIL because the module and snapshot shape do not exist.

**Step 3: Write minimal implementation**

Create a QUIC session coordinator parallel to `webrtc_session.rs`, with explicit transport/bootstrap fields and snapshot getters.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app quic_session -- --nocapture`

Expected: PASS with deterministic session-state snapshots.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/quic_session.rs apps/Rdesk/src-tauri/src/main.rs
git commit -m "feat: add quic session coordinator"
```

### Task 4: Add QUIC host runtime for actual media send/receive

**Files:**
- Create: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\quic_host.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\frame_sink.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\quic_host.rs`

**Step 1: Write the failing test**

Add an app integration test that:
- creates agent/controller QUIC hosts
- bootstraps using signaling-style metadata
- starts sender on agent
- waits for a frame in the shared sink on controller

**Step 2: Run test to verify it fails**

Run: `cargo test -p app quic_single_process_pipeline_delivers_remote_frames -- --nocapture`

Expected: FAIL because the current QUIC harness is test-only and not wired into app host/session state.

**Step 3: Write minimal implementation**

Move the reusable pieces out of `quic_transport_harness.rs` into a real `quic_host.rs` runtime that:
- starts listener/connect
- runs sender loop
- runs receiver loop
- reuses `mrd_decode::create_decoder(...)`
- writes frames into `DecodedFrameSink`

**Step 4: Run test to verify it passes**

Run: `cargo test -p app quic_single_process_pipeline_delivers_remote_frames -- --nocapture`

Expected: PASS with at least one decoded frame and QUIC probe stages populated.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/quic_host.rs apps/Rdesk/src-tauri/src/quic_transport_harness.rs apps/Rdesk/src-tauri/src/main.rs
git commit -m "feat: add quic app session runtime"
```

### Task 5: Route app commands through transport-aware session selection

**Files:**
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\session_runtime.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\render_host.rs`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write the failing test**

Add a transport-aware app test that requests a QUIC session and verifies:
- QUIC host state updates
- session runtime snapshot exposes transport=`quic_quinn`
- no WebRTC-only command path is required for frame delivery

**Step 2: Run test to verify it fails**

Run: `cargo test -p app session_runtime_quic -- --nocapture`

Expected: FAIL because `main.rs` still routes real sessions through WebRTC-only structures.

**Step 3: Write minimal implementation**

Add transport-aware routing in `main.rs`, keeping the command surface stable where possible and branching internally on selected transport.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app session_runtime_quic -- --nocapture`

Expected: PASS with QUIC snapshots and frame sink integration.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src-tauri/src/session_runtime.rs apps/Rdesk/src-tauri/src/render_host.rs
git commit -m "feat: route app sessions through quic transport"
```

### Task 6: Reuse the app QUIC path in benchmark/component validation

**Files:**
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\quic_transport_harness.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\benchmark.rs`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\tests\benchmarks\scripts\run_transport_matrix.ps1`
- Modify: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\tests\benchmarks\scenarios\quick.transport.quic.json`
- Test: `G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write the failing test**

Add or tighten a benchmark-path test asserting the QUIC transport run writes artifacts from the app-backed QUIC path rather than a disconnected special case.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app benchmark_run_writes_requested_artifacts -- --nocapture`

Expected: FAIL or require updates because the benchmark harness still assumes the old QUIC-only helper path.

**Step 3: Write minimal implementation**

Repoint benchmark QUIC runs to the shared QUIC runtime path while preserving current artifact shape.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app benchmark_run_writes_requested_artifacts -- --nocapture`

Expected: PASS with `transport=quic_quinn` artifacts written.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/quic_transport_harness.rs apps/Rdesk/src-tauri/src/benchmark.rs tests/benchmarks/scripts/run_transport_matrix.ps1 tests/benchmarks/scenarios/quick.transport.quic.json
git commit -m "test: route quic benchmark through app runtime"
```

### Task 7: Final verification for merge readiness

**Files:**
- Verify only

**Step 1: Run targeted Rust verification**

Run:

```bash
cargo test -p mrd-transport-quic-quinn -- --nocapture
cargo test -p app quic -- --nocapture
cargo test -p app benchmark_run_writes_requested_artifacts -- --nocapture
```

Expected: all targeted QUIC and benchmark tests pass.

**Step 2: Run script-level verification**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/transport_sender.quic_quinn.json -RepoRoot G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/transport_receiver.quic_quinn.json -RepoRoot G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.json -RepoRoot G:\Project\mini-remote-desktop\recovered-repo\.worktrees\quic-mainline
```

Expected: QUIC sender, receiver, and benchmark artifacts complete without WebRTC participation.

**Step 3: Commit any final harness fixes**

```bash
git add -A
git commit -m "test: verify quic mainline session path"
```
