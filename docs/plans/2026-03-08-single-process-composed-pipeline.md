# Single-Process Composed Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add single-process composed pipeline tests that exercise host orchestration, in-memory signaling, media send/receive, decode, frame sink updates, and probe visibility without external services.

**Architecture:** Build a lightweight test-only harness around the existing `WebrtcHost` runtime instead of inventing a new stack. The harness will hold paired hosts, shared frame sink state, and in-memory offer/answer exchange, then expose assertions for remote-frame delivery, probe-stage visibility, and non-stalling behavior over a fixed duration.

**Tech Stack:** Rust, Tokio async tests, existing `WebrtcHost`, `ProbeRegistry`, `DecodedFrameSink`, OpenH264 sender path.

---

### Task 1: Add the first failing composed-pipeline test

**Files:**
- Modify: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Create: `G:/Project/mini-remote-desktop/docs/plans/2026-03-08-single-process-composed-pipeline.md`

**Step 1: Write the failing test**

Add `single_process_pipeline_exposes_probe_stages` in the existing `webrtc_host` test module. The test should call a not-yet-existing `HostedPairHarness` helper that sets up sender/receiver hosts and runs the in-memory loop.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app single_process_pipeline_exposes_probe_stages -- --nocapture`

Expected: FAIL because `HostedPairHarness` or its methods do not exist yet.

**Step 3: Write minimal implementation**

Add a test-only harness with paired hosts, fake capture, in-memory offer/answer exchange, and helpers to wait for decoded frames and read probe snapshots.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app single_process_pipeline_exposes_probe_stages -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/webrtc_host.rs docs/plans/2026-03-08-single-process-composed-pipeline.md
git commit -m "test: add single-process probe harness"
```

### Task 2: Add remote-frame delivery and no-stall tests

**Files:**
- Modify: `G:/Project/mini-remote-desktop/apps/Rdesk/src-tauri/src/webrtc_host.rs`

**Step 1: Write the failing test**

Add:
- `single_process_pipeline_delivers_remote_frames`
- `single_process_pipeline_runs_for_fixed_duration_without_stalling`

Both tests should use the harness and assert behavior that is not yet fully exposed.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app single_process_pipeline_ -- --nocapture`

Expected: FAIL because the harness does not yet expose all required snapshots/timing helpers.

**Step 3: Write minimal implementation**

Expose only the helpers needed:
- latest sink snapshot
- controller/agent snapshots
- latest frame count sampling over a fixed interval

**Step 4: Run test to verify it passes**

Run: `cargo test -p app single_process_pipeline_ -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add apps/Rdesk/src-tauri/src/webrtc_host.rs
git commit -m "test: add single-process pipeline behavior coverage"
```

### Task 3: Full verification

**Files:**
- Verify only

**Step 1: Run focused host tests**

Run: `cargo test -p app webrtc_host -- --nocapture`

Expected: PASS, including existing sender loopback tests and the new composed-pipeline tests.

**Step 2: Run broader app tests**

Run: `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 3: Run component matrix smoke**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_matrix.ps1 -RepoRoot .`

Expected: PASS, proving component-level regression coverage still works after host-test changes.

**Step 4: Commit**

```bash
git add -A
git commit -m "test: add single-process composed pipeline coverage"
```
