# NVDEC Decode Status and Sequence Diagnostics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add best-effort `cuvidGetDecodeStatus` reporting and richer sequence/recreate lifecycle diagnostics to the direct Windows NVDEC decoder without changing decode behavior.

**Architecture:** Keep `NvdecDecoder` and `NvdecSession` public behavior unchanged. Extend the diagnostics snapshot, load `cuvidGetDecodeStatus` as an optional NVDEC export, and update sequence/decode/display callbacks to capture the latest sequence decision, recreate transition, and decode-status observation.

**Tech Stack:** Rust, handwritten Windows CUDA/NVDEC FFI, current `mrd-decode-nvdec` diagnostics surface, existing `openh264`-based decode tests.

---

### Task 1: Add failing diagnostics tests for sequence and decode-status fields

**Files:**
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add tests that:
- decode a valid H264 access unit and assert diagnostics include latest sequence fields and decode-status phase information
- run the existing resolution-change path and assert diagnostics include recreate decision plus from/to coded sizes

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec status_sequence -- --nocapture`

Expected: FAIL because the new diagnostics fields are not populated yet.

**Step 3: Write minimal implementation**

Add only the new diagnostics fields and enough callback wiring to satisfy the tests.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec status_sequence -- --nocapture`

Expected: PASS.

### Task 2: Load and record `cuvidGetDecodeStatus`

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Tighten the success-path diagnostics assertion so `last_decode_status_phase` and `last_decode_status_description` must be filled with either a phase result or an explicit unavailable marker.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec decode_status -- --nocapture`

Expected: FAIL because no decode-status query is recorded yet.

**Step 3: Write minimal implementation**

Implement:
- optional `cuvidGetDecodeStatus` loading
- best-effort decode-status query helper
- status recording from `decode` and `display` phases

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec decode_status -- --nocapture`

Expected: PASS.

### Task 3: Record detailed sequence and recreate diagnostics

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Tighten the resolution-change diagnostics test so it requires:
- latest sequence dimensions
- latest sequence decision
- recreate from/to coded dimensions

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec recreate_on_resolution_change -- --nocapture`

Expected: FAIL because current diagnostics do not preserve all those details.

**Step 3: Write minimal implementation**

Implement helpers that record:
- latest sequence fields
- latest sequence decision
- recreate from/to coded sizes

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec recreate_on_resolution_change -- --nocapture`

Expected: PASS.

### Task 4: Run regression verification and update docs

**Files:**
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-status-sequence-design.md`
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-status-sequence-plan.md`

**Step 1: Run verification**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 2: Document final state**

Record:
- whether `cuvidGetDecodeStatus` was available on this host
- what sequence and recreate fields are now exposed
- any remaining unsupported or best-effort-only diagnostics

## Current Status

- Task 1 complete: crate tests now assert latest sequence fields, sequence decisions, decode-status fields, and recreate from/to coded sizes.
- Task 2 complete: `cuvidGetDecodeStatus` is loaded as an optional export and recorded best-effort in diagnostics.
- Task 3 complete: sequence and recreate diagnostics now expose the latest parser format plus recreate transition details.
- Task 4 verification complete:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
