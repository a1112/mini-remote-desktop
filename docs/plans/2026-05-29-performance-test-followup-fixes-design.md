# Performance Test Follow-up Fixes Design

## Goal

Use the 2026-05-29 main-branch performance run to fix benchmark reliability, capability reporting, and actionable performance defaults in one branch.

## Scope

This branch addresses three groups of issues:

1. Test and reporting correctness:
   - Make component-matrix case execution resilient to hung child processes.
   - Ensure a completed `result.json` still produces `summary.csv` and markdown reports.
   - Split loose smoke thresholds from stricter performance thresholds.
   - Treat unsupported AV1 hardware paths as skipped capability outcomes, not performance failures.

2. Performance path clarity:
   - Keep OpenH264 2K/high-refresh runs available for diagnostics, but avoid presenting them as viable performance defaults.
   - Make FFmpeg fallback measurements separate startup/warmup cost from steady-state throughput.
   - Prefer NVDEC, then FFmpeg NV12, then software decode for high-resolution H.264 fallback policy where the local capability set supports it.

3. Render pacing visibility:
   - Expose D3D11 waitable-object pacing as an explicit benchmark/UI option instead of an environment-only switch.
   - Keep reporting `render_queue_replacements` and `render_stale_frame_drops`, and add rates or threshold checks where summaries compare runs.

## Architecture

The benchmark scripts remain the entry points for reproducible command-line runs. PowerShell helpers carry timeout, process cleanup, threshold, and summary behavior; Rust benchmark/test code emits richer artifacts but should not own CI orchestration policy.

The app/UI keeps capability selection conservative. Unsupported codecs are shown as unavailable or skipped with concrete reasons. High-throughput defaults favor hardware paths; diagnostic software paths remain selectable but are labelled and thresholded separately.

## Testing Strategy

Use script-level tests for benchmark helpers, Rust tests for benchmark summary serialization and FFmpeg measurement fields, and Vitest tests for UI/capability behavior. For performance-sensitive changes, verify with targeted benchmark commands after unit tests pass.
