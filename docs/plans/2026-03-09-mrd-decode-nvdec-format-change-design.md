# NVDEC Format Change and Decoder Recreate Design

**Date:** 2026-03-09

**Goal:** Extend the direct Windows NVDEC decoder so H264 8-bit NV12 streams can survive resolution changes by destroying and recreating the decoder in place while keeping the outer `NvdecDecoder` API unchanged.

## Context

`mrd-decode-nvdec` already has a real Windows-only NVDEC path:

- `NvdecDecoder::new()` creates CUDA context and parser
- `push_access_unit()` feeds H264 Annex-B access units into the parser
- display callbacks map NVDEC surfaces and emit CPU `Rgb24`
- diagnostics now expose decode and display activity plus stage-aware runtime failures

The remaining gap is stream lifecycle robustness. The current implementation assumes a fixed sequence format after the first parser callback. If a valid H264 stream changes resolution, the existing decoder handle remains pinned to stale dimensions. That is fragile and will eventually fail against real-world streams.

## Chosen Approach

Support resolution changes by recreating the NVDEC decoder handle when the parser reports a new H264 8-bit NV12 sequence format that is incompatible with the currently active decoder configuration.

This design does not introduce `cuvidReconfigureDecoder` yet. Instead it uses explicit destroy-and-create lifecycle management because it is easier to reason about with the current handwritten FFI and current test coverage.

## Alternatives Considered

### 1. Destroy and recreate the decoder on incompatible sequence changes

Chosen.

Pros:

- simplest lifecycle boundary with the current code
- avoids partial-state bugs from in-place reconfiguration
- easier to test with the existing parser callback flow
- keeps the FFI surface small

Cons:

- more expensive than true reconfigure
- may briefly drop state across a format change

### 2. Prefer `cuvidReconfigureDecoder`, then fall back to full recreate

Not chosen for this phase.

Pros:

- closer to the ideal NVDEC runtime path
- potentially lower overhead on sequence changes

Cons:

- requires more FFI surface and structure definitions
- harder to debug before recreate semantics are proven

### 3. Detect the change and return a clear unsupported error

Rejected for this phase.

Pros:

- minimal implementation effort

Cons:

- does not make the decoder robust enough for real streams
- keeps a common production case unsupported

## Architecture

### SequenceFormat

Add an internal `SequenceFormat` snapshot derived from `CUVIDEOFORMAT`.

It stores the fields needed to decide whether the active decoder is still compatible:

- `coded_width`
- `coded_height`
- `display_width`
- `display_height`
- `chroma_format`
- `bit_depth_minus8`
- `min_decode_surfaces`

### DecoderConfig

Add an internal `DecoderConfig` snapshot that represents the actual settings used to create the current decoder:

- coded size
- target/display size
- chroma format
- bit depth
- decode surface count

This becomes the stable “current decoder contract” used during future sequence callbacks.

### Decoder Lifecycle Helpers

Add helper functions inside `NvdecSession` or callback-owned state to:

- derive `SequenceFormat` from `CUVIDEOFORMAT`
- compare a new `SequenceFormat` against the current `DecoderConfig`
- destroy an existing decoder if present
- create a new decoder from a `SequenceFormat`

The parser handle remains alive across format changes. Only the decoder handle is recreated.

## Data Flow

1. The parser receives a sequence callback.
2. The callback converts `CUVIDEOFORMAT` into a `SequenceFormat`.
3. The callback checks whether the sequence is supported:
   - codec remains H264
   - chroma remains 4:2:0
   - bit depth remains 8-bit
4. If there is no active decoder, create one from the new format.
5. If there is an active decoder and the configuration is unchanged, keep it.
6. If the configuration changed in a supported way, destroy the old decoder and create a new one.
7. Update callback/session state with the new sequence dimensions and active `DecoderConfig`.
8. Continue normal decode and display callbacks using the new handle.

## Error Handling

### Supported and Recoverable Changes

Resolution changes within H264 / 8-bit / NV12 trigger a `recreate` event:

- destroy old decoder
- create new decoder
- update diagnostics with recreate reason

If recreate succeeds, decode continues on the same outer decoder instance.

### Unsupported Sequence Changes

If the parser reports an incompatible sequence change such as:

- non-H264 codec
- non-420 chroma
- bit depth above 8-bit

the callback records an explicit error such as:

- `nvdec sequence change unsupported: bit depth change`
- `nvdec sequence change unsupported: chroma format change`

This phase does not attempt partial support for those cases.

### Recreate Failure

If decoder recreate fails:

- the current access unit fails clearly
- diagnostics record `sequence` or `recreate` as the failing stage
- no stale decoder handle remains registered as active

The session remains internally consistent and may recover on a later valid sequence.

## Diagnostics

The diagnostics snapshot should grow to expose sequence lifecycle information such as:

- recreate count
- last recreate reason
- active coded size
- active target size

That makes it possible to test and debug sequence changes without exposing raw NVDEC handles.

## Testing Strategy

### Internal State Tests

Add narrow tests for the configuration comparison logic:

- same dimensions does not recreate
- coded size change recreates
- display size change recreates
- bit depth or chroma change is rejected

### Decode Path Tests

Keep the existing `128x128` decode path green, then add a two-stage test that feeds:

1. a valid `128x128` H264 access unit
2. a valid second H264 stream at a different supported resolution such as `256x128`

The test should assert:

- decode succeeds across the change
- diagnostics report at least one recreate event
- post-change frames have the new dimensions

### Regression

Run:

- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

## Scope Limits

This phase includes:

- H264
- 8-bit
- NV12 output surfaces
- decoder recreate on supported resolution changes

This phase does not include:

- HEVC
- `cuvidReconfigureDecoder`
- bit depth changes
- chroma format changes
- optimized zero-copy output

## Success Criteria

This work is complete when:

- sequence callbacks can detect compatible resolution changes
- the decoder is destroyed and recreated automatically when needed
- diagnostics expose recreate activity
- decoded frames continue after the change with correct output dimensions
- current `mrd-decode` and `app` tests remain green

## Current Status

- `mrd-decode-nvdec` now tracks active decoder configuration and compares it against new parser sequence formats.
- Supported H264 8-bit 4:2:0 resolution changes trigger decoder destroy-and-recreate inside the sequence callback.
- Diagnostics now expose recreate count, recreate reason, and active coded/display dimensions.
- Verified with:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
- Validated decode transition on this host:
  - `128x128` H264 Annex-B access unit
  - `256x128` H264 Annex-B access unit
- Still unsupported in this phase:
  - chroma format changes
  - bit depth changes
  - HEVC
  - `cuvidReconfigureDecoder`
