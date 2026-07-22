# Transport Receiver Component Matrix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move H264 RTP access-unit assembly into `mrd-transport-webrtc` and add a transport receiver component-matrix test with latency and throughput reporting.

**Architecture:** Reuse the proven H264 Annex-B reassembly logic currently living in `src-tauri` by moving it into `mrd-transport-webrtc` as a transport-level ingress component. Keep the receiver boundary strictly at `RTP payload + marker -> EncodedAccessUnit`, then wire `src-tauri` and the component matrix to consume the shared crate implementation.

**Tech Stack:** Rust, Cargo integration tests, PowerShell component-matrix scripts, existing `mrd-observability` result model.

---

### Task 1: Add the failing receiver tests in `mrd-transport-webrtc`

**Files:**
- Modify: `G:/Project/mini-remote-desktop/crates/mrd-transport-webrtc/src/lib.rs`
- Create: `G:/Project/mini-remote-desktop/crates/mrd-transport-webrtc/tests/perf_transport_receiver.rs`

**Step 1: Write the failing test**

Add a crate test that expects a new ingress type to reconstruct an `EncodedAccessUnit` from FU-A and single-NAL payloads, plus an ignored perf test that expects a receiver case to emit a `ComponentResult`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-transport-webrtc transport_receiver -- --nocapture`

Expected: FAIL because the ingress type and/or receiver perf test support does not exist yet.

**Step 3: Write minimal implementation**

Add the ingress type and just enough public API for tests to compile.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-transport-webrtc transport_receiver -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/mrd-transport-webrtc/src/lib.rs crates/mrd-transport-webrtc/tests/perf_transport_receiver.rs
git commit -m "feat: add transport receiver component tests"
```

### Task 2: Move the assembler from `src-tauri` into `mrd-transport-webrtc`

**Files:**
- Modify: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/webrtc_media.rs`
- Modify: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Modify: `G:/Project/mini-remote-desktop/crates/mrd-transport-webrtc/src/lib.rs`

**Step 1: Write the failing test**

Update `src-tauri` tests/imports to expect the assembler to come from `mrd-transport-webrtc`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app webrtc_media -- --nocapture`

Expected: FAIL because `src-tauri` still owns the local implementation.

**Step 3: Write minimal implementation**

Move the logic into `mrd-transport-webrtc`, keep equivalent behavior, and replace `src-tauri` with a thin re-export or direct crate usage.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app webrtc_media -- --nocapture`

Expected: PASS with the same behavioral coverage as before.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/webrtc_media.rs apps/Rdesk/src-tauri/src/webrtc_host.rs crates/mrd-transport-webrtc/src/lib.rs
git commit -m "refactor: share h264 transport receiver assembly"
```

### Task 3: Add the receiver component-matrix case and thresholds

**Files:**
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/cases/transport_receiver.h264_assemble.json`
- Create: `G:/Project/mini-remote-desktop/tests/component-matrix/thresholds/transport_receiver.json`
- Modify: `G:/Project/mini-remote-desktop/tests/component-matrix/scripts/run_component_matrix.ps1`
- Modify: `G:/Project/mini-remote-desktop/tests/component-matrix/scripts/summarize_component_results.ps1`
- Modify: `G:/Project/mini-remote-desktop/tests/component-matrix/README.md`

**Step 1: Write the failing test**

Add the new case entry before support exists in the receiver perf test and matrix run.

**Step 2: Run test to verify it fails**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/transport_receiver.h264_assemble.json -RepoRoot .`

Expected: FAIL because the receiver perf test/result path is not implemented yet.

**Step 3: Write minimal implementation**

Wire the case into the component matrix and include receiver-specific result fields using existing summary plumbing.

**Step 4: Run test to verify it passes**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/transport_receiver.h264_assemble.json -RepoRoot .`

Expected: PASS and create a result directory under `artifacts/component-matrix/.../transport/...`.

**Step 5: Commit**

```bash
git add tests/component-matrix
git commit -m "test: add transport receiver component matrix case"
```

### Task 4: Full verification

**Files:**
- Verify only

**Step 1: Run focused crate tests**

Run: `cargo test -p mrd-transport-webrtc -- --nocapture`

Expected: PASS, including sender tests and receiver unit tests.

**Step 2: Run app regression tests**

Run: `cargo test -p app webrtc_host -- --nocapture`

Expected: PASS, proving `src-tauri` still reconstructs and decodes video correctly.

**Step 3: Run ignored receiver perf test**

Run: `cargo test -p mrd-transport-webrtc perf_webrtc_transport_receiver_reports_latency_distribution -- --ignored --nocapture`

Expected: PASS and emit `result.json` when env vars are set by the matrix script.

**Step 4: Run the full component matrix**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_matrix.ps1 -RepoRoot .`

Expected: PASS, with capture/encode/decode/transport sender/transport receiver/render all producing summaries.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add transport receiver component matrix"
```
