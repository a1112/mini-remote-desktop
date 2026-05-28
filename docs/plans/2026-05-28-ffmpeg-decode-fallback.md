# FFmpeg Decode Fallback Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate optional FFmpeg as a runtime decode fallback for cross-device LAN remote sessions without replacing hardware decode or OpenH264.

**Architecture:** `mrd-decode` owns a small persistent FFmpeg CLI decoder for H.264 and HEVC. `mrd-service` only selects these backends through the existing LAN receiver candidate chain. FFmpeg remains optional: missing tools make the FFmpeg backend unavailable while existing NVDEC, platform, and OpenH264 paths continue to work.

**Tech Stack:** Rust, `mrd-ffmpeg` tool probing, `std::process::Command`, `mrd-pipeline-core::VideoDecoder`, LAN receiver fallback selection.

---

### Task 1: Add FFmpeg Decoder Descriptor Tests

**Files:**
- Modify: `crates/mrd-decode/tests/software_codecs.rs`

**Steps:**
- Add a failing test that expects `ffmpeg_h264` and `ffmpeg_hevc` decoder descriptors.
- Run `cargo test -p mrd-decode ffmpeg_descriptors`.
- Implement descriptors in `crates/mrd-decode/src/lib.rs`.
- Re-run the test.

### Task 2: Add FFmpeg Decoder Construction Tests

**Files:**
- Create: `crates/mrd-decode/tests/ffmpeg_decode.rs`
- Modify: `crates/mrd-decode/src/lib.rs`
- Modify: `crates/mrd-decode/Cargo.toml`

**Steps:**
- Add a failing test for deterministic FFmpeg decoder construction failure with a missing configured executable.
- Add `mrd-ffmpeg` as a dependency.
- Implement a `FfmpegCliDecoder` that probes the optional tool settings, starts a persistent FFmpeg process after the first SPS-bearing access unit, reads raw NV12 frames from stdout, and returns `DecodedFrame::CpuNv12`.
- Re-run targeted `mrd-decode` tests.

### Task 3: Add LAN Receiver Candidate Tests

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`

**Steps:**
- Add failing tests that expect FFmpeg after hardware decode and before legacy software decode for H.264, and after hardware decode for HEVC.
- Add explicit `MRD_LAN_RECEIVER_DECODER=ffmpeg` preference handling.
- Re-run `cargo test -p mrd-service windows_receiver_decoder_defaults_to_hardware_then_ffmpeg_fallback`.

### Task 4: Verify

**Commands:**
- `cargo test -p mrd-decode`
- `cargo test -p mrd-service ffmpeg`
- `cargo test --release -p mrd-decode perf_ffmpeg_decode_compare_reports_results -- --ignored --nocapture`

**Expected:** Existing decode tests pass, LAN candidate tests pass, and the ignored FFmpeg performance comparison still runs when FFmpeg is installed.
