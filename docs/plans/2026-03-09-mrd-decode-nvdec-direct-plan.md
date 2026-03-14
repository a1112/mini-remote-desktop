# Direct NVDEC Decoder Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn the Windows-only NVDEC runtime probe into a real unified `VideoDecoder` implementation that can create a decoder, consume H264 Annex-B access units, and integrate into `mrd-decode`.

**Architecture:** Keep the existing `mrd_decode::VideoDecoder` contract unchanged. Build a direct Windows-only implementation in `mrd-decode-nvdec` with three layers: DLL/symbol binding, NVDEC session ownership, and `NvdecDecoder` itself. The same decoder path must be used by crate tests and the `mrd-decode` factory.

**Tech Stack:** Rust, `windows` crate, direct CUDA driver/NVDEC FFI, H264 Annex-B access units, existing `mrd-decode` and `openh264` test helpers.

---

### Task 1: Add a real decoder construction API

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add a test that calls `NvdecDecoder::new()` and asserts it returns either:
- `Ok(_)` on a supported host, or
- a clear runtime error string mentioning the failing runtime dependency or API

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_new -- --nocapture`

Expected: FAIL because `NvdecDecoder` does not exist yet.

**Step 3: Write minimal implementation**

Implement:
- a public `NvdecDecoder` type
- direct DLL/symbol binding structs for the required CUDA/NVDEC entry points
- `NvdecDecoder::new()` with real runtime initialization and explicit error mapping

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_new -- --nocapture`

Expected: PASS.

### Task 2: Feed real H264 access units into the unified decoder

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add a test that:
- creates or skips on unsupported runtime
- constructs an `NvdecDecoder`
- uses an existing OpenH264 helper path to produce a valid H264 Annex-B access unit
- calls `push_access_unit()`
- expects a structured success result or a clear runtime limitation, never a panic

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_push_access_unit -- --nocapture`

Expected: FAIL because the parser/decode feed path is not implemented yet.

**Step 3: Write minimal implementation**

Implement:
- parser creation
- unified decoder-owned callback state
- `push_access_unit()` feeding `cuvidParseVideoData`
- explicit handling for malformed or unsupported input

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec nvdec_push_access_unit -- --nocapture`

Expected: PASS.

### Task 3: Surface decoded frames as `Rgb24`

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing test**

Add a test that:
- creates an `NvdecDecoder`
- feeds a valid H264 Annex-B access unit
- drains decoded frames
- expects at least one frame with:
  - the expected width/height
  - `Rgb24`
  - `width * height * 3` bytes

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_decoder_emits_rgb_frame -- --nocapture`

Expected: FAIL because output mapping and conversion are not implemented yet.

**Step 3: Write minimal implementation**

Implement:
- output surface mapping/unmapping
- CPU-side frame extraction
- minimal conversion into `Rgb24`
- pending frame buffering for `drain_decoded_frames()`

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec -- --nocapture`

Expected: PASS.

### Task 4: Wire the unified decoder into `mrd-decode`

**Files:**
- Modify: `crates/mrd-decode/src/lib.rs`
- Modify: `crates/mrd-decode/tests/nvdec.rs`

**Step 1: Write the failing test**

Add or tighten tests so that `create_decoder("nvdec")`:
- returns a real decoder object on supported hosts, or
- returns a clear runtime error on unsupported hosts

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode nvdec -- --nocapture`

Expected: FAIL because the factory still returns the placeholder error path.

**Step 3: Write minimal implementation**

Replace the placeholder branch with:
- real `NvdecDecoder::new()`
- conversion of backend runtime errors into `DecoderError`

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode nvdec -- --nocapture`

Expected: PASS.

### Task 5: Verify app regression and document remaining gaps

**Files:**
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-direct-design.md`
- Modify: `docs/plans/2026-03-09-mrd-decode-nvdec-direct-plan.md`

**Step 1: Run verification**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

Expected: PASS.

**Step 2: Document final status**

Record:
- whether decoder creation works on this host
- whether valid H264 AU feeding works
- whether `Rgb24` frame output works
- what remains if any callback, format, or performance constraints are still open

## Current Status

- Task 1 complete: `NvdecDecoder::new()` exists and returns a structured runtime result.
- Task 2 complete: valid H264 Annex-B access units are fed through `cuvidParseVideoData()`.
- Task 3 complete: decoded output is mapped, copied to CPU memory, and exposed as `Rgb24`.
- Task 4 complete: `mrd-decode` now returns a real NVDEC-backed `VideoDecoder`.
- Task 5 verification complete:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
