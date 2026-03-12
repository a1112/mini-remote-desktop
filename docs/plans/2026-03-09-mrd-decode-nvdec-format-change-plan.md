# NVDEC Format Change and Decoder Recreate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Teach the direct Windows NVDEC decoder to survive supported H264 resolution changes by destroying and recreating the decoder handle while preserving the outer decoder object and decode API.

**Architecture:** Keep `NvdecDecoder` and the parser lifetime unchanged. Introduce internal format/config snapshots, compare them during sequence callbacks, and recreate only the decoder handle when a supported format change is detected. Expose recreate activity through diagnostics rather than through new public control APIs.

**Tech Stack:** Rust, handwritten CUDA/NVDEC FFI, Windows dynamic loading, existing `openh264`-based test helpers, current `mrd-decode-nvdec` diagnostics surface.

---

### Task 1: Add failing tests for recreate decision logic

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Add focused unit tests for internal comparison helpers that assert:
- unchanged H264 8-bit 4:2:0 dimensions do not require recreate
- coded width or height changes do require recreate
- display width or height changes do require recreate
- unsupported bit depth or chroma changes are rejected

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec recreate_decision -- --nocapture`

Expected: FAIL because format/config helpers do not exist yet.

**Step 3: Write minimal implementation**

Implement:
- `SequenceFormat`
- `DecoderConfig`
- helper(s) for compatibility and recreate decisions

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec recreate_decision -- --nocapture`

Expected: PASS.

### Task 2: Recreate decoder handle on supported sequence changes

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Add or tighten a test that expects sequence change handling to update lifecycle state rather than reusing stale dimensions.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec sequence_change -- --nocapture`

Expected: FAIL because the current callback only creates the decoder once.

**Step 3: Write minimal implementation**

Implement:
- decoder destroy helper
- decoder create helper
- sequence callback logic that:
  - creates on first sequence
  - reuses on compatible config
  - destroys and recreates on supported resolution changes
  - rejects unsupported format changes with explicit errors

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec sequence_change -- --nocapture`

Expected: PASS.

### Task 3: Prove decode continues after a real resolution change

**Files:**
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add a decode-path test that:
- creates one decoder
- feeds a valid `128x128` H264 access unit
- feeds a second valid H264 access unit at a different supported resolution such as `256x128`
- drains frames and asserts later output reflects the new resolution
- asserts diagnostics report at least one recreate event

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_recreates_on_resolution_change -- --nocapture`

Expected: FAIL because the current implementation assumes fixed sequence dimensions.

**Step 3: Write minimal implementation**

Extend diagnostics and sequence state so the decoder:
- records recreate count and last recreate reason
- updates active dimensions after a recreate
- continues to emit `Rgb24` frames after the change

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_recreates_on_resolution_change -- --nocapture`

Expected: PASS.

### Task 4: Run regression verification and update docs

**Files:**
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-format-change-design.md`
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-format-change-plan.md`

**Step 1: Run verification**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 2: Document final state**

Record:
- whether recreate works on this host
- what dimensions were validated
- what unsupported sequence changes still return explicit errors

## Current Status

- Task 1 complete: internal recreate decision helpers are covered by unit tests.
- Task 2 complete: sequence callback now destroys and recreates the decoder on supported resolution changes.
- Task 3 complete: one decoder instance now survives a `128x128` to `256x128` H264 resolution change and continues emitting `Rgb24` frames.
- Task 4 verification complete:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
