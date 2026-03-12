# NVDEC Diagnostics and Status Design

**Date:** 2026-03-09

**Goal:** Add explicit NVDEC runtime diagnostics so successful decodes expose parser/decode/display activity and failures identify the exact CUDA/NVDEC stage that failed.

## Context

`mrd-decode-nvdec` now has a real Windows-only NVDEC decoder path. The remaining gap is observability: most failures still collapse into a short string with a numeric code, and successful runs do not expose enough state to confirm whether parser, decode, and display callbacks all executed.

This design adds a small diagnostics surface without changing the public decode contract used by `mrd-decode`.

## Chosen Approach

Add a lightweight diagnostics snapshot API on `NvdecDecoder` backed by callback-owned state inside `NvdecSession`.

The decoder continues to return `Result<(), String>` for `push_access_unit()`, but that string will now include the failing stage and API. A caller that needs deeper inspection can fetch a diagnostics snapshot after construction or decode attempts.

## Alternatives Considered

### 1. Structured diagnostics snapshot plus clearer string errors

Chosen.

Pros:

- keeps the existing public contract stable
- improves both testability and field debugging
- avoids pushing FFI-specific types into `mrd-decode`

Cons:

- duplicates some information between `Result` strings and diagnostics state

### 2. Replace all string errors with a new public error enum

Not chosen for this pass.

Pros:

- stronger typing
- cleaner matching for downstream callers

Cons:

- expands API churn across crates
- slows down this observability pass

### 3. Add logging only

Rejected.

Pros:

- minimal code changes

Cons:

- logs are not test-friendly
- logs do not help callers make programmatic assertions

## Architecture

### Public Snapshot

Add `NvdecDiagnostics` as a cloneable snapshot type returned by `NvdecDecoder::diagnostics()`.

It records:

- `last_stage`
- `last_api`
- `last_code`
- `last_error_name`
- `last_error_description`
- `last_picture_index`
- `decode_calls`
- `display_calls`

### Internal Error Recording

Callback and session code will report failures through one helper that captures:

- logical stage such as `parse`, `decode`, `display`, `map`, `copy`, `unmap`
- concrete API name
- raw CUDA/NVDEC status code
- best-effort CUDA driver error name and description

### Decode Status

Where available, the implementation will call `cuvidGetDecodeStatus` after decode/display activity and store a best-effort summary in diagnostics. This is additive observability, not a new hard failure gate.

## Data Flow

1. `NvdecDecoder::new()` initializes an empty diagnostics snapshot.
2. `push_access_unit()` resets per-access-unit activity counters and the last failure fields.
3. Parser and callback code update diagnostics as each stage runs.
4. On failure, `push_access_unit()` returns a string that includes the stage and API.
5. Tests and callers can inspect `decoder.diagnostics()` after success or failure.

## Error Handling

Environment, input, and runtime errors remain string-based at the boundary, but each message must now include the stage and API name when applicable.

Examples:

- `nvdec parse failed at cuvidParseVideoData: ...`
- `nvdec decode failed at cuvidDecodePicture: ...`
- `nvdec copy failed at cuMemcpyDtoH_v2: ...`

Input validation errors such as non-Annex-B input should still be explicit and not pretend to be CUDA failures.

## Testing Strategy

Add two focused crate-level tests first:

- success path: after decoding a valid H264 access unit, diagnostics report decode/display activity
- failure path: malformed input returns an error string containing stage information

Then rerun existing `mrd-decode` and `app` tests to confirm the observability additions do not change behavior.

## Success Criteria

This work is complete when:

- `NvdecDecoder` exposes a diagnostics snapshot
- successful decodes report activity counters above zero
- decode failures identify a stage and API
- current `mrd-decode` and `app` regression suites stay green
