# NVDEC Decode Status and Sequence Diagnostics Design

**Date:** 2026-03-09

**Goal:** Extend the direct Windows NVDEC decoder diagnostics so it reports best-effort `cuvidGetDecodeStatus` results and more detailed sequence/recreate lifecycle information without changing decode success semantics.

## Context

`mrd-decode-nvdec` already has:

- a real Windows-only NVDEC decode path
- explicit stage-aware string errors
- a diagnostics snapshot with decode/display counts
- decoder recreate on supported H264 resolution changes

The remaining gap is observability depth. We still cannot answer these questions cleanly from the diagnostics snapshot alone:

- what exact sequence format the parser most recently reported
- whether the latest sequence was reused, created, recreated, or rejected
- what dimensions the recreate transitioned from and to
- what best-effort decode status the driver reported during decode and display phases

## Chosen Approach

Add more fields to the existing `NvdecDiagnostics` snapshot and populate them from the existing parser/decode/display callbacks. Load `cuvidGetDecodeStatus` dynamically as an optional entry point and record its result when available.

This pass does not change the existing decode contract and does not promote decode-status queries into hard failures.

## Alternatives Considered

### 1. Best-effort decode-status and richer lifecycle snapshot

Chosen.

Pros:

- low risk to the working decode path
- directly improves testability and field diagnosis
- keeps API stable

Cons:

- still only keeps “latest” state, not full per-access-unit history

### 2. Make `cuvidGetDecodeStatus` a hard failure gate

Rejected for this pass.

Pros:

- stricter runtime validation

Cons:

- can make the decoder brittle across drivers and callback timing
- turns an observability feature into a behavior change

### 3. Add a full decode event log

Not chosen for this phase.

Pros:

- richer postmortem visibility

Cons:

- much larger state and API surface
- unnecessary for the immediate debugging goals

## Architecture

### Cuvid Status Hook

Extend `CuvidApi` with an optional `cuvidGetDecodeStatus` function pointer loaded from `nvcuvid.dll`.

If the export is absent, diagnostics report that the query is unavailable and decode behavior remains unchanged.

### Diagnostics Snapshot Additions

Expand `NvdecDiagnostics` with three groups of fields.

#### Sequence Snapshot

- `last_sequence_coded_width`
- `last_sequence_coded_height`
- `last_sequence_display_width`
- `last_sequence_display_height`
- `last_sequence_bit_depth_minus8`
- `last_sequence_chroma_format`
- `last_sequence_decision`

#### Recreate Snapshot

- `last_recreate_from_coded_width`
- `last_recreate_from_coded_height`
- `last_recreate_to_coded_width`
- `last_recreate_to_coded_height`

#### Decode Status Snapshot

- `last_decode_status_phase`
- `last_decode_status_raw`
- `last_decode_status_description`

### Callback State Recording

`CallbackState` remains the single mutable owner for decoder lifecycle diagnostics.

It will gain helpers to:

- record the most recent parser sequence format
- record the latest sequence decision
- record recreate from/to dimensions
- record best-effort decode status for the `decode` and `display` phases

## Data Flow

1. `sequence_callback` derives `SequenceFormat` from `CUVIDEOFORMAT`.
2. The callback records the new sequence fields into diagnostics.
3. The callback evaluates whether to `create`, `reuse`, `recreate`, or reject the sequence and records that decision.
4. On recreate:
   - record the previous coded size
   - recreate the decoder
   - record the new coded size and increment recreate counters
5. `decode_callback` performs real decode and, on success, best-effort queries `cuvidGetDecodeStatus` for the current picture index and records it as phase `decode`.
6. `display_callback` best-effort queries `cuvidGetDecodeStatus` again and records it as phase `display`.

## Error Handling

### Decode Status Query

`cuvidGetDecodeStatus` is diagnostic-only.

- missing export: diagnostics note `unavailable`
- query failure: diagnostics note the query failure
- returned status code: diagnostics store the raw value and a best-effort description

This does not fail `push_access_unit()`.

### Sequence and Recreate Failures

Existing sequence/recreate errors remain hard failures. This pass only makes them easier to interpret by preserving:

- the latest parser-reported sequence
- the decision taken
- the old and new dimensions when recreate was attempted

## Testing Strategy

### Success Path

After a normal decode:

- diagnostics should contain the latest sequence fields
- diagnostics should contain a meaningful sequence decision
- diagnostics should contain a decode status phase of `decode`, `display`, or an explicit unavailable marker

### Resolution Change Path

After a `128x128 -> 256x128` transition:

- diagnostics should report `recreate` as the latest sequence decision
- diagnostics should show recreate from/to coded sizes
- decode should still continue and emit frames

### Regression

Run:

- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`
- `cargo test -p app -- --nocapture`

## Scope Limits

This phase includes:

- optional `cuvidGetDecodeStatus`
- latest sequence diagnostics
- latest recreate diagnostics

This phase does not include:

- per-access-unit history logs
- turning status queries into hard failures
- HEVC-specific diagnostics
- `cuvidReconfigureDecoder`

## Success Criteria

This work is complete when:

- diagnostics expose the latest parser sequence details
- diagnostics expose recreate from/to sizes and the latest decision
- diagnostics expose best-effort decode status phase/raw/description
- the real NVDEC decode path keeps working
- current crate and app regression tests remain green

## Current Status

- `mrd-decode-nvdec` now records the latest parser sequence dimensions, bit depth, chroma format, and sequence decision.
- recreate diagnostics now preserve the last coded-size transition through `from` and `to` fields.
- `cuvidGetDecodeStatus` is now loaded as an optional NVDEC export and queried best-effort during display-phase picture handling.
- On hosts where that export is missing or unsupported, diagnostics explicitly report that status queries are unavailable instead of failing decode.
- Verified with:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
