# NVDEC Reconfigure-First Decoder Lifecycle Design

**Date:** 2026-03-09

**Goal:** Extend the direct Windows NVDEC decoder to try `cuvidReconfigureDecoder` first for supported H264 resolution and display-area changes, while preserving the existing destroy/recreate path as a stable fallback.

## Context

`mrd-decode-nvdec` already supports:

- direct Windows NVDEC decoding
- diagnostics for decode, sequence, and recreate activity
- destroy/recreate lifecycle handling for supported H264 resolution changes

That current lifecycle is correct but not optimal. NVIDIA provides `cuvidReconfigureDecoder` for in-place decoder updates on supported sequence changes. The right next step is to prefer that lighter path without sacrificing the already-working fallback.

## Chosen Approach

Add a reconfigure-first lifecycle path:

- try `cuvidReconfigureDecoder` when the stream remains H264 / 8-bit / 4:2:0 and only supported dimensions or display-area parameters changed
- if reconfigure succeeds, keep the current decoder handle
- if reconfigure is unavailable or fails, fall back to the existing destroy/recreate path

This keeps the system robust because it never depends solely on reconfigure support.

## Alternatives Considered

### 1. Reconfigure first, fallback to recreate

Chosen.

Pros:

- preserves the working fallback path
- allows incremental optimization without regressions
- diagnostics can clearly distinguish optimized and fallback cases

Cons:

- more lifecycle branches to test

### 2. Reconfigure only, no fallback

Rejected.

Pros:

- simpler lifecycle model on paper

Cons:

- drops the already-proven recovery path
- too risky across driver and hardware variations

### 3. Keep recreate only

Rejected for this phase.

Pros:

- no new FFI surface

Cons:

- leaves an available optimization and lifecycle API unused

## Architecture

### Optional Reconfigure API

Extend `CuvidApi` with an optional `cuvidReconfigureDecoder` export loaded from `nvcuvid.dll`.

If the symbol is absent, diagnostics record that reconfigure was unavailable and the decoder immediately falls back to recreate.

### Reconfigure Plan

Add an internal helper that decides whether a sequence change is eligible for reconfigure-first behavior.

This phase only treats these as eligible:

- H264 remains unchanged
- chroma remains 4:2:0
- bit depth remains 8-bit
- decoder already exists
- change is limited to coded size, display size, or related target/display rectangles

### Lifecycle Helper

Add a helper such as `apply_reconfigure_or_recreate(...)` that:

1. records diagnostics for the attempted transition
2. tries `cuvidReconfigureDecoder` when eligible
3. updates active config on success
4. falls back to destroy/recreate on unavailability or failure

## Data Flow

1. `sequence_callback` receives a new `SequenceFormat`.
2. The callback records the latest sequence snapshot.
3. If the current and next formats differ in a supported way:
   - record a reconfigure attempt
   - try `cuvidReconfigureDecoder`
4. On reconfigure success:
   - update `DecoderConfig`
   - record `reconfigure success`
   - continue decode with the same decoder handle
5. On reconfigure failure or unavailability:
   - record diagnostics
   - fall back to existing destroy/recreate
6. If the change is unsupported:
   - keep the current explicit unsupported error path

## Diagnostics

`NvdecDiagnostics` should expand with:

- `last_reconfigure_attempted`
- `last_reconfigure_result`
- `last_reconfigure_from_coded_width`
- `last_reconfigure_from_coded_height`
- `last_reconfigure_to_coded_width`
- `last_reconfigure_to_coded_height`
- `reconfigure_fallback_used`

These fields must make it obvious whether the last sequence change:

- reused the decoder
- reconfigured in place
- attempted reconfigure then fell back
- failed entirely

## Error Handling

### Reconfigure Unavailable

If `cuvidReconfigureDecoder` is not exported:

- diagnostics record `unavailable`
- lifecycle falls back to recreate
- decode should continue if recreate succeeds

### Reconfigure Failure

If `cuvidReconfigureDecoder` returns an error:

- diagnostics record the raw result and a readable message
- lifecycle falls back to recreate
- this is not treated as terminal if recreate succeeds

### Unsupported Format Changes

If the sequence change alters unsupported properties such as bit depth or chroma format:

- do not attempt reconfigure
- do not silently coerce into recreate
- return the existing explicit unsupported error

## Testing Strategy

### Decision Logic

Add tests for internal helpers that verify:

- supported dimension changes are marked reconfigure-eligible
- unsupported chroma or bit-depth changes are not

### Decode Path

Keep the existing `128x128 -> 256x128` resolution-change test and tighten it to assert:

- `last_reconfigure_attempted == true`
- `last_reconfigure_result` is populated
- if the host cannot reconfigure, `reconfigure_fallback_used == true`
- frames still appear at the new resolution

### Regression

Run:

- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

## Scope Limits

This phase includes:

- optional `cuvidReconfigureDecoder`
- reconfigure-first for supported H264 8-bit 4:2:0 dimension/display changes
- recreate fallback

This phase does not include:

- HEVC reconfigure support
- bit depth or chroma transitions
- removing the recreate path

## Success Criteria

This work is complete when:

- sequence changes attempt `cuvidReconfigureDecoder` first when eligible
- fallback to recreate remains stable
- diagnostics clearly distinguish reconfigure success, unavailability, failure, and fallback
- crate and app regression suites remain green

## Current Status

- `mrd-decode-nvdec` now loads `cuvidReconfigureDecoder` as an optional export and attempts it first on supported H264 resolution/display changes.
- diagnostics now expose reconfigure attempt, result, from/to coded sizes, and whether recreate fallback was used.
- on this host, the validated `128x128 -> 256x128` transition attempted reconfigure and then fell back to the existing recreate path while continuing to decode correctly.
- verified with:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
