# mrd-decode-nvdec Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a first-phase `mrd-decode-nvdec` backend that fits the existing `VideoDecoder` interface and can enter the matrix as a hardware H264 decode candidate on Windows/NVIDIA hosts.

**Architecture:** Phase 1 keeps the current host-facing decode contract unchanged: `push_access_unit(&[u8])` and `drain_decoded_frames()` still return CPU `Rgb24` frames. `mrd-decode-nvdec` now owns a Windows-only direct NVDEC runtime probe based on `LoadLibraryA` and `GetProcAddress`, instead of the broken `cuda-rs`/`nvcodec` dependency chain. `mrd-decode` remains the backend registry and factory layer; a later phase will add the real bitstream parser, decode session, and CPU frame extraction on top of this runtime entry point.

**Tech Stack:** Rust, `windows` crate, `nvcuvid.dll` / `nvcuda.dll` runtime probing, `mrd-decode`, component matrix.

---

### Task 1: Scaffold the crate and workspace wiring

**Files:**
- Create: `crates/mrd-decode-nvdec/Cargo.toml`
- Create: `crates/mrd-decode-nvdec/src/lib.rs`
- Create: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`
- Modify: `Cargo.toml`

**Step 1: Write the failing probe test**

Add a crate test that expects a probe function such as `probe_h264_available()` to exist and return either `Ok(())` or a structured unsupported error instead of panicking.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_probe -- --nocapture`

Expected: FAIL because the crate does not exist yet.

**Step 3: Write minimal implementation**

Create the crate with:
- a `probe_h264_available()` function
- a lightweight `NvdecDecoder` skeleton type
- Windows-only dependency wiring for `nvcodec`

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec nvdec_probe -- --nocapture`

Expected: PASS.

### Task 2: Register `nvdec` in `mrd-decode`

**Files:**
- Modify: `crates/mrd-decode/src/lib.rs`
- Modify: `crates/mrd-decode/Cargo.toml`
- Create: `crates/mrd-decode/tests/nvdec.rs`

**Step 1: Write the failing registration test**

Add tests that:
- `available_decoder_descriptors()` contains `nvdec`
- `create_decoder("nvdec")` returns a decoder or a clear runtime-unavailable error

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode nvdec -- --nocapture`

Expected: FAIL because the backend is not registered yet.

**Step 3: Write minimal implementation**

Expose the new descriptor and factory branch in `mrd-decode`, mapping probe failures to `DecoderError` messages instead of panics.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode nvdec -- --nocapture`

Expected: PASS.

### Task 3: Replace the broken dependency chain with a Windows-native runtime probe

**Files:**
- Modify: `crates/mrd-decode-nvdec/src/lib.rs`
- Modify: `crates/mrd-decode-nvdec/tests/nvdec_probe.rs`

**Step 1: Write the failing runtime probe test**

Add a test that:
- calls a `probe_runtime()` helper
- verifies the backend id and human-readable summary are populated
- on Windows, verifies `nvcuvid.dll` is explicitly checked

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode-nvdec nvdec_runtime_probe_reports_library_state -- --nocapture`

Expected: FAIL because the runtime probe helper does not exist yet.

**Step 3: Write minimal implementation**

Implement the smallest Windows-only bridge needed to:
- load `nvcuda.dll`
- load `nvcuvid.dll`
- verify core exports such as `cuInit`, `cuDeviceGetCount`, `cuvidGetDecoderCaps`, `cuvidCreateDecoder`, and `cuvidDestroyDecoder`
- return a structured probe summary without panicking

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-decode-nvdec -- --nocapture`

Expected: PASS with either a runtime-backed success probe or an explicit Windows-only unsupported message.

### Task 4: Stop and report before GPU-native phase

**Files:**
- Modify: `docs/plans/2026-03-08-mrd-decode-nvdec-plan.md`

**Step 1: Verify current phase**

Run:
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`

**Step 2: Document what is complete**

Record:
- probe status
- backend registration status
- whether CPU `Rgb24` output path is working
- what remains for Phase 2 direct NVDEC session creation, bitstream parsing, and frame extraction

## Current Status

- `mrd-decode-nvdec` no longer depends on `cuda-rs`, `nvcodec`, `npp`, or `ffmpeg-sys-next`.
- The crate now performs a Windows-only direct runtime probe through `LoadLibraryA` and `GetProcAddress`.
- Verified commands:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
- Current limitation: the factory can now verify NVDEC runtime entry points, but the actual H264 decode session and `Rgb24` frame output path are still not implemented.
