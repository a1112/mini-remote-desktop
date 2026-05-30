# Software VVC Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `software_vvc` a real feature-gated local harness and benchmark codec path instead of an ambiguous legacy alias.

**Architecture:** Add a dedicated `mrd-encode-vvenc` crate that implements `VideoEncoder` with a no-system-library default build and a `software-vvenc` feature for the real VVenC backend. Wire Rdesk's harness and benchmark parser to `EncoderType::SoftwareVvc`, and keep service/UI capabilities disabled unless the feature/probe says the VVenC/VVdeC path is usable.

**Tech Stack:** Rust workspace crates, `vvenc`/`vvenc-sys` optional dependency, existing `mrd-decode` VVdeC path, Rdesk Tauri benchmark harness, Vitest capability matrix tests.

---

### Task 1: Harness Contract Tests

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`
- Modify: `apps/mrd-service/src/capabilities.rs`
- Modify: `apps/Rdesk/src/app/services/capabilityMatrix.test.ts`

**Steps:**
1. Add tests proving `software_vvc` and `software_h266` parse to a distinct software VVC encoder type instead of falling through to NVENC H.264.
2. Add tests proving the default non-feature build reports a clear not-compiled reason.
3. Add capability tests proving service/UI only mark VVC software paths usable when an explicit VVC feature/probe is available.

### Task 2: VVenC Encoder Crate

**Files:**
- Create: `crates/mrd-encode-vvenc/Cargo.toml`
- Create: `crates/mrd-encode-vvenc/src/lib.rs`
- Modify: `Cargo.toml`

**Steps:**
1. Add the crate as a workspace member with `vvenc` as an optional dependency.
2. Export `VvencSoftwareEncoder::new` and `new_with_bitrate`.
3. In default builds, return a clear `PipelineError` saying VVenC is not compiled.
4. With `software-vvenc`, convert CPU BGRA/RGBA/RGB24/NV12 frames to 8-bit 4:2:0 planes for VVenC and emit `VideoCodec::Vvc` Annex-B access units.

### Task 3: Rdesk Integration

**Files:**
- Modify: `apps/Rdesk/src-tauri/Cargo.toml`
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`

**Steps:**
1. Add `mrd-encode-vvenc` dependency.
2. Add `EncoderType::SoftwareVvc`.
3. Select `VideoCodec::Vvc` for that encoder.
4. Route `DecoderType::Software` to `mrd_decode::create_decoder("software_vvc")`.
5. Keep H.264/HEVC/AV1 incompatible decoders rejected with explicit errors.

### Task 4: Capabilities and UI

**Files:**
- Modify: `apps/mrd-service/Cargo.toml`
- Modify: `apps/mrd-service/src/capabilities.rs`
- Modify: `apps/Rdesk/src/app/services/capabilityMatrix.ts`
- Modify: `apps/Rdesk/src/app/services/capabilityMatrix.test.ts`

**Steps:**
1. Make local service VVC software capability status feature/probe-driven.
2. Keep peer-advertised VVC software capabilities non-runnable unless the local capability policy says they are implemented.
3. Let the UI inherit structured service capability status; legacy fallback remains disabled by default.

### Task 5: Verification

**Commands:**
- `pnpm --dir apps/Rdesk test -- src/app/services/capabilityMatrix.test.ts`
- `pnpm --dir apps/Rdesk type-check`
- `cargo test -p mrd-encode-vvenc`
- `cargo test -p app benchmark_h266_encoder_backend_is_capability_gated -- --nocapture`
- `cargo test -p app software_vvc -- --nocapture`
- `cargo test -p mrd-service capabilities -- --nocapture`
- `cargo fmt --check`
- `git diff --check`

**Feature verification:** If the host lacks `libvvenc >= 1.13.0`, `cargo test -p mrd-encode-vvenc --features software-vvenc` is expected to fail at dependency probing. Record that as an environment prerequisite rather than marking the default build broken.
