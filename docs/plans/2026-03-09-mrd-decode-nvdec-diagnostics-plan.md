# NVDEC Diagnostics and Status Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add stage-aware diagnostics and clearer CUDA/NVDEC error reporting to the direct Windows NVDEC decoder without changing the existing decode API.

**Architecture:** Keep `NvdecDecoder` as the public entry point. Store mutable diagnostics in `NvdecSession` callback state, expose them as a cloneable snapshot, and enrich error strings with stage and API names. Use best-effort CUDA error name/description lookup and optional decode-status capture.

**Tech Stack:** Rust, handwritten Windows CUDA/NVDEC FFI, existing `openh264` test helper, current `mrd-decode-nvdec` and `mrd-decode` tests.

---

### Task 1: Add failing diagnostics tests

**Files:**
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add tests that:
- decode a valid H264 access unit and assert diagnostics report decode/display activity
- feed malformed input and assert the returned error mentions the failing stage

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec diagnostics -- --nocapture`

Expected: FAIL because diagnostics accessors or stage-aware errors do not exist yet.

**Step 3: Write minimal implementation**

Implement only the public diagnostics accessor and the minimum state recording needed for those tests.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec diagnostics -- --nocapture`

Expected: PASS.

### Task 2: Record stage-aware runtime failures

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`

**Step 1: Write the failing test**

Tighten the malformed-input assertion so the message includes stage-specific wording such as `input`, `parse`, or `decode`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec malformed -- --nocapture`

Expected: FAIL because current errors are too generic.

**Step 3: Write minimal implementation**

Add helpers that capture:
- stage
- API name
- raw code
- best-effort CUDA error name/description

Use them in parser, decode, display, map, copy, and unmap paths.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec malformed -- --nocapture`

Expected: PASS.

### Task 3: Capture decode status and verify regressions

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add a best-effort assertion that successful decode leaves a diagnostics snapshot with meaningful activity fields populated.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_reports -- --nocapture`

Expected: FAIL because decode-status or richer diagnostics are not populated yet.

**Step 3: Write minimal implementation**

Load and call `cuvidGetDecodeStatus` where possible, storing the result as additional diagnostics context without changing decode success semantics.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.
