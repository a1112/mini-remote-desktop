# NVDEC HEVC Runtime Probe Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a first-stage HEVC runtime probe that reports NVDEC runtime capability separately from current decode-path wiring.

**Architecture:** Reuse the existing NVDEC runtime loading and `cuvidGetDecoderCaps` support. Add a capability request/result helper, use the support matrix for wired-support evaluation, and expose `probe_hevc_available()` as a user-facing adapter.

**Tech Stack:** Rust, handwritten Windows CUDA/NVDEC FFI, existing `cuvidGetDecoderCaps` integration, current `mrd-decode-nvdec` tests.

---

### Task 1: Add failing HEVC probe tests

**Files:**
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add tests that:
- call `probe_hevc_available()`
- assert the result is structured and mentions `hevc`
- accept either runtime unsupported or runtime supported but not wired

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec hevc_probe -- --nocapture`

Expected: FAIL because `probe_hevc_available()` does not exist yet.

**Step 3: Write minimal implementation**

Add the public probe function and the minimum internal helper surface needed for the test.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec hevc_probe -- --nocapture`

Expected: PASS.

### Task 2: Add capability request/result and runtime probe helper

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Add internal tests for capability logic that distinguish:
- H264 runtime/wired happy path
- HEVC runtime-supported but wired-unsupported path
- unsupported capability path

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec capability_probe -- --nocapture`

Expected: FAIL because capability result helpers do not exist yet.

**Step 3: Write minimal implementation**

Implement:
- capability request type
- capability result type
- runtime caps helper using `cuvidGetDecoderCaps`
- mapping into wired support via the existing support matrix

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec capability_probe -- --nocapture`

Expected: PASS.

### Task 3: Preserve H264 probe behavior and run regressions

**Files:**
- Modify: `docs/plans/2026-03-10-mrd-decode-nvdec-hevc-probe-design.md`
- Modify: `docs/plans/2026-03-10-mrd-decode-nvdec-hevc-probe-plan.md`

**Step 1: Run verification**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 2: Document final state**

Record:
- whether HEVC runtime caps were supported on this host
- whether HEVC remains not wired
- what capability information is now exposed

## Current Status

- Task 1 complete: `probe_hevc_available()` exists and is covered by a crate-level probe test.
- Task 2 complete: capability request/result flow now distinguishes runtime support from wired support.
- Task 3 verification complete:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
