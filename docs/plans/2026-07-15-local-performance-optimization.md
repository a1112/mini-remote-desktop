# Local Performance Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Run, diagnose, optimize, and fully re-verify every performance test supported by the current Windows workstation.

**Architecture:** Add a thin PowerShell orchestrator around the existing component, transport, integration, and local-canary runners, preserving their native artifacts and verdicts. Use measured failures to drive test-first changes in the owning Rust crates, then compare three-run medians and finish with a complete serial rerun.

**Tech Stack:** PowerShell 7/Windows PowerShell, Rust/Cargo release tests, JSON artifacts, existing component matrix and transport benchmark harnesses.

---

### Task 1: Local suite contract and environment manifest

**Files:**
- Create: `tests/benchmarks/scripts/local_performance_suite_common.ps1`
- Create: `tests/benchmarks/scripts/test_local_performance_suite.ps1`

1. Write failing PowerShell tests for deterministic scenario discovery, exclusion
   of peer/TURN-only tests, environment manifest shape, verdict precedence,
   artifact existence, and stable unsupported classification.
2. Run the test and confirm RED because the helper does not exist.
3. Implement pure discovery, classification, manifest, and aggregation helpers.
4. Run the test and confirm PASS.

### Task 2: Full local suite runner

**Files:**
- Create: `tests/benchmarks/scripts/run_local_performance_suite.ps1`
- Modify: `tests/benchmarks/scripts/test_local_performance_suite.ps1`

1. Add failing tests using fake child runners to prove serial execution, release
   mode, continue-after-failure, timeout classification, output linking, and
   aggregate exit codes.
2. Confirm RED.
3. Implement component, integration, transport-scenario, and local-canary phases.
4. Confirm GREEN without running hardware tests from the contract suite.

### Task 3: Complete baseline collection

**Files:**
- Artifacts only under `artifacts/local-performance/<run-id>/` and existing native artifact roots.

1. Run the component matrix.
2. Run automated E2E matrix and pipeline tests in release mode.
3. Run every JSON scenario under `tests/benchmarks/scenarios` serially.
4. Run all local paired-canary profiles with cross-device work disabled.
5. Aggregate failures and lowest threshold margins.

### Task 4: Capture bottleneck optimization when evidence requires it

**Files:**
- Modify: `crates/mrd-capture-dxgi/src/lib.rs`
- Modify: `crates/mrd-capture-dxgi/tests/perf_capture.rs`
- Test: relevant non-performance tests in `crates/mrd-capture-dxgi/tests/`

1. Add a deterministic regression test for the measured scheduling, copy, or
   allocation bottleneck.
2. Confirm RED.
3. Implement the smallest production fix while retaining frame validity and
   shared-texture semantics.
4. Run crate tests and three affected component/transport attempts.

### Task 5: Software encoder bottleneck optimization when evidence requires it

**Files:**
- Modify: `crates/mrd-encode-openh264/src/lib.rs`
- Modify: `crates/mrd-encode-openh264/tests/perf_encode.rs`

1. Add a deterministic regression test for the measured buffer/configuration
   overhead.
2. Confirm RED.
3. Reuse buffers or adjust the production speed configuration without weakening
   codec validity or quality contracts.
4. Run crate tests and three affected component/transport attempts.

### Task 6: Hardware encoder/decoder bottleneck optimization when evidence requires it

**Files:**
- Modify: `crates/mrd-encode-nvenc/src/lib.rs`
- Modify: `crates/mrd-encode-nvenc/tests/perf_encode.rs`
- Modify only if implicated: `crates/mrd-decode-nvdec/src/lib.rs`

1. Add a deterministic configuration/queue regression test matching the failing
   scenario.
2. Confirm RED.
3. Adjust buffer reuse, preset, queue depth, or synchronization only where the
   evidence identifies it.
4. Run functional tests and three affected hardware scenarios.

### Task 7: Transport/render optimization when evidence requires it

**Files:**
- Modify only the measured owner under `crates/mrd-transport-webrtc/`, `crates/mrd-transport-quic-quinn/`, or `crates/mrd-render-d3d11/`.
- Add a focused test beside the changed implementation.

1. Reproduce the exact queue, batching, pacing, or tail-latency failure in a
   deterministic test.
2. Confirm RED, implement the minimal fix, then confirm GREEN.
3. Run three affected end-to-end scenarios and compare medians.

### Task 8: Full regression and report

**Files:**
- Generated artifacts only.

1. Re-run the complete local suite serially.
2. Run all affected crate tests and harness contract tests.
3. Run `cargo fmt --all -- --check` and `git diff --check`.
4. Produce the final JSON/Markdown report with PASS, unsupported, remaining
   infrastructure limitations, and before/after performance deltas.
