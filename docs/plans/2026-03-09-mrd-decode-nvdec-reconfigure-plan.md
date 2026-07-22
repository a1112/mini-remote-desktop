# NVDEC Reconfigure-First Decoder Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Teach the direct Windows NVDEC decoder to try `cuvidReconfigureDecoder` first on supported sequence changes and fall back to destroy/recreate when reconfigure is unavailable or fails.

**Architecture:** Keep the current parser and outer decoder object unchanged. Add optional `cuvidReconfigureDecoder` loading, a reconfigure-eligibility helper, and a lifecycle helper that updates diagnostics and falls back to the existing recreate path when necessary.

**Tech Stack:** Rust, handwritten Windows CUDA/NVDEC FFI, current `mrd-decode-nvdec` diagnostics and recreate logic, existing `openh264` test helpers.

---

### Task 1: Add failing tests for reconfigure diagnostics

**Files:**
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Tighten the resolution-change test so it asserts:
- `last_reconfigure_attempted == true`
- `last_reconfigure_result` is populated
- `reconfigure_fallback_used` is populated consistently with the observed path

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec reconfigure -- --nocapture`

Expected: FAIL because reconfigure diagnostics do not exist yet.

**Step 3: Write minimal implementation**

Add only the diagnostics fields and enough lifecycle wiring to satisfy the new assertions.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec reconfigure -- --nocapture`

Expected: PASS.

### Task 2: Add reconfigure-eligibility helpers and failing unit tests

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Add focused tests for internal helpers that assert:
- supported size/display changes are reconfigure-eligible
- bit-depth or chroma changes are not

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec reconfigure_decision -- --nocapture`

Expected: FAIL because those helpers do not exist yet.

**Step 3: Write minimal implementation**

Implement:
- reconfigure eligibility helper(s)
- any internal enum or plan structure needed for the decision

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec reconfigure_decision -- --nocapture`

Expected: PASS.

### Task 3: Load and use `cuvidReconfigureDecoder` with recreate fallback

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Run the tightened resolution-change test and confirm it still fails because the decoder only recreates today.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_recreates_on_resolution_change -- --nocapture`

Expected: FAIL on the new reconfigure assertions.

**Step 3: Write minimal implementation**

Implement:
- optional `cuvidReconfigureDecoder` export loading
- `CUVIDRECONFIGUREDECODERINFO`
- a helper that tries reconfigure first and falls back to recreate
- diagnostics updates for attempted, success, unavailable, failed, and fallback cases

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_recreates_on_resolution_change -- --nocapture`

Expected: PASS.

### Task 4: Run regression verification and update docs

**Files:**
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-reconfigure-design.md`
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-reconfigure-plan.md`

**Step 1: Run verification**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 2: Document final state**

Record:
- whether `cuvidReconfigureDecoder` was available on this host
- whether the host used reconfigure success or recreate fallback
- what unsupported changes still bypass reconfigure

## Current Status

- Task 1 complete: the resolution-change test now asserts reconfigure attempt and fallback diagnostics.
- Task 2 and 3 complete: the decoder now attempts `cuvidReconfigureDecoder` first and falls back to recreate when needed.
- Task 4 verification complete:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
