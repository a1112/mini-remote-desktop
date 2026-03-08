# QUIC Matrix Rollout Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a first-class QUIC transport path based on `quinn` and integrate it into the existing component matrix, single-process composed pipeline tests, and benchmark harness.

**Architecture:** Introduce a dedicated `mrd-transport-quic-quinn` crate that owns QUIC sender/receiver behavior behind the same pipeline-facing boundaries already used for WebRTC transport. Validate QUIC in the same three layers as WebRTC: component matrix, single-process composed pipeline, and benchmark artifacts.

**Tech Stack:** Rust, `quinn`, Tokio, existing `mrd-observability`, component-matrix PowerShell scripts, app-side composed pipeline tests.

---

### Task 1: Add the QUIC transport crate skeleton

**Files:**
- Create: `G:/Project/mini-remote-desktop/crates/mrd-transport-quic-quinn/Cargo.toml`
- Create: `G:/Project/mini-remote-desktop/crates/mrd-transport-quic-quinn/src/lib.rs`
- Modify: `G:/Project/mini-remote-desktop/Cargo.toml`

**Step 1: Write the failing test**

Create a minimal test expecting a sender/receiver pair to initialize and expose transport metadata.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-transport-quic-quinn -- --nocapture`

Expected: FAIL because the crate or API does not exist yet.

**Step 3: Write minimal implementation**

Add the crate, dependencies, and minimal sender/receiver setup types.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-transport-quic-quinn -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml crates/mrd-transport-quic-quinn
git commit -m "feat: add quinn transport crate skeleton"
```

### Task 2: Add QUIC component-matrix coverage

**Files:**
- Create: `G:/Project/mini-remote-desktop/crates/mrd-transport-quic-quinn/tests/perf_transport_sender.rs`
- Create: `G:/Project/mini-remote-desktop/crates/mrd-transport-quic-quinn/tests/perf_transport_receiver.rs`
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/cases/transport_sender.quic_quinn.json`
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/cases/transport_receiver.quic_quinn.json`
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/thresholds/transport_sender_quic.json`
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/thresholds/transport_receiver_quic.json`
- Modify: `G:/Project/mini-remote-desktop/tests/component-matrix/scripts/run_component_matrix.ps1`
- Modify: `G:/Project/mini-remote-desktop/tests/component-matrix/README.md`

**Step 1: Write the failing test**

Add sender/receiver perf tests and case files that expect QUIC result output.

**Step 2: Run test to verify it fails**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/transport_sender.quic_quinn.json -RepoRoot .`

Expected: FAIL because QUIC sender perf support does not exist yet.

**Step 3: Write minimal implementation**

Add sender and receiver perf tests that emit `ComponentResult` through the existing observability model.

**Step 4: Run test to verify it passes**

Run the sender and receiver cases individually and then via the full matrix script.

**Step 5: Commit**

```bash
git add crates/mrd-transport-quic-quinn tests/component-matrix
git commit -m "test: add quinn transport component matrix"
```

### Task 3: Add single-process composed QUIC coverage

**Files:**
- Modify: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Or create: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/transport_quic_harness.rs`

**Step 1: Write the failing test**

Add a new single-process composed test that expects a QUIC sender/receiver path to deliver frames and emit probe stages.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app quic_single_process_pipeline -- --nocapture`

Expected: FAIL because the QUIC composed harness does not exist yet.

**Step 3: Write minimal implementation**

Add a test-only QUIC harness that mirrors the WebRTC composed harness:
- sender side
- receiver side
- frame delivery
- probe snapshot access

**Step 4: Run test to verify it passes**

Run: `cargo test -p app quic_single_process_pipeline -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src
git commit -m "test: add quic composed pipeline coverage"
```

### Task 4: Add QUIC benchmark scenarios

**Files:**
- Modify: `G:/Project/mini-remote-desktop/tests/benchmarks/scenarios/quick.transport.json`
- Modify: `G:/Project/mini-remote-desktop/tests/benchmarks/scenarios/steady.transport.60s.json`
- Modify: `G:/Project/mini-remote-desktop/tests/benchmarks/scenarios/stress.transport.180s.json`
- Modify: `G:/Project/mini-remote-desktop/tests/benchmarks/scripts/run_transport_matrix.ps1`
- Modify: `G:/Project/mini-remote-desktop/tests/benchmarks/README.md`

**Step 1: Write the failing test**

Add QUIC scenario entries and run the benchmark harness expecting QUIC support.

**Step 2: Run test to verify it fails**

Run: `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.json -RepoRoot .`

Expected: FAIL because QUIC transport selection and result collection do not exist yet.

**Step 3: Write minimal implementation**

Add QUIC scenario routing, result metadata, and artifact collection using the same schema already used for WebRTC.

**Step 4: Run test to verify it passes**

Run the quick scenario first, then the steady scenario.

**Step 5: Commit**

```bash
git add tests/benchmarks
git commit -m "feat: add quic benchmark scenarios"
```

### Task 5: Full verification

**Files:**
- Verify only

**Step 1: Run QUIC crate tests**

Run: `cargo test -p mrd-transport-quic-quinn -- --nocapture`

Expected: PASS.

**Step 2: Run host tests**

Run: `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 3: Run full component matrix**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_matrix.ps1 -RepoRoot .`

Expected: PASS with both WebRTC and QUIC transport entries.

**Step 4: Run benchmark smoke**

Run: `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.json -RepoRoot .`

Expected: PASS and emit QUIC artifacts beside WebRTC artifacts.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: integrate quinn transport into validation matrix"
```
