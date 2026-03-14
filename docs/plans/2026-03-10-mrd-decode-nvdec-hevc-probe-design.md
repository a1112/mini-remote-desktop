# NVDEC HEVC Runtime Probe Design

**Date:** 2026-03-10

**Goal:** Add a first-stage HEVC runtime probe to `mrd-decode-nvdec` that distinguishes runtime capability from wired decode support, without implementing real HEVC decode.

## Context

`mrd-decode-nvdec` now has:

- direct Windows NVDEC runtime loading
- H264 decode path
- support-matrix checks for codec, bit depth, and chroma format
- diagnostics for runtime, sequence, recreate, and reconfigure decisions

What is still missing is a clear capability probe for HEVC. Right now the crate can say “HEVC not wired yet” during sequence evaluation, but it cannot independently answer whether the local GPU/driver runtime would support HEVC decode if the decode path were implemented.

## Chosen Approach

Add a small capability-probe layer on top of `cuvidGetDecoderCaps`:

- keep runtime probing and decode-path support separate
- introduce a reusable capability request/result type
- expose `probe_hevc_available()` as the first public HEVC probe

This phase only checks `HEVC + 8-bit + 4:2:0`. It does not expose HEVC as a working decoder backend.

## Alternatives Considered

### 1. Add a reusable capability probe plus `probe_hevc_available()`

Chosen.

Pros:

- clean separation between runtime capability and wired decode support
- naturally extends to HEVC Main10 later
- keeps public scope small

Cons:

- introduces a second probe result model beside the existing runtime summary

### 2. Add only `probe_hevc_available()` with hardcoded checks

Rejected.

Pros:

- smaller short-term diff

Cons:

- duplicates logic for future HEVC Main10 or other probe variants
- weaker testability

### 3. Expand `probe_runtime()` into a huge capability string

Rejected.

Pros:

- no new types

Cons:

- poor structure for tests and future features
- mixes runtime presence with feature wiring

## Architecture

### Capability Request

Add an internal `NvdecCapabilityRequest` that captures:

- codec
- bit depth minus 8
- chroma format

It is intentionally similar to the existing support-matrix request, but answers a different question.

### Capability Probe Result

Add a public `NvdecCapabilityProbe` with:

- `codec`
- `bit_depth_minus8`
- `chroma_format`
- `runtime_supported`
- `runtime_reason`
- `wired_supported`
- `wired_reason`

This result cleanly distinguishes:

- runtime can do it
- current Rust implementation can do it

### Probe Flow

Add an internal helper that:

1. ensures the NVDEC runtime is present
2. calls `cuvidGetDecoderCaps` for the requested codec/bit-depth/chroma
3. evaluates current wired support via the existing support-matrix logic
4. returns a structured capability result

Public helpers then adapt that into user-facing `Result<(), String>`:

- `probe_h264_available()`
- `probe_hevc_available()`

## Data Flow

1. Public probe helper creates a capability request.
2. Capability helper runs runtime/library checks.
3. If `cuvidGetDecoderCaps` is available, it probes runtime support for that request.
4. The existing support matrix determines whether the decode path is actually wired.
5. The final result reports both dimensions.

Example outcomes:

- runtime unsupported: `runtime_supported = false`, `wired_supported = false`
- runtime supported but not wired: `runtime_supported = true`, `wired_supported = false`
- H264 wired path: `runtime_supported = true`, `wired_supported = true`

## Error Handling

### Runtime Unavailable

If DLLs, exports, or CUDA device setup fail, the probe returns a structured result with:

- `runtime_supported = false`
- runtime reason containing the failing runtime/API

### Runtime Capability Unsupported

If `cuvidGetDecoderCaps` reports unsupported:

- `runtime_supported = false`
- reason explicitly naming codec/bit depth/chroma

### Wired Support Missing

If runtime supports HEVC but the implementation does not:

- `runtime_supported = true`
- `wired_supported = false`
- reason explicitly says HEVC decode path is not wired yet

## Testing Strategy

### Unit and Probe Tests

Add tests that:

- verify the capability helper distinguishes H264 supported vs HEVC not wired
- ensure `probe_hevc_available()` returns a structured result and mentions `hevc`

### Regression

Run:

- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

## Scope Limits

This phase includes:

- a first-stage HEVC runtime probe
- structured capability reporting
- explicit distinction between runtime support and wired support

This phase does not include:

- real HEVC decode
- Main10 decode support
- exposing HEVC as a public decoder backend

## Success Criteria

This work is complete when:

- `probe_hevc_available()` exists
- the probe can distinguish runtime unsupported from runtime supported but not wired
- current H264 probe behavior remains intact
- regression suites remain green

## Current Status

- `mrd-decode-nvdec` now exposes `probe_hevc_available()` and a structured capability-probe layer behind it.
- the probe distinguishes runtime capability from wired decode support using `cuvidGetDecoderCaps` plus the existing support matrix.
- on this host, HEVC probing now returns a structured HEVC-specific result instead of falling into generic NVDEC errors.
- verified with:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
