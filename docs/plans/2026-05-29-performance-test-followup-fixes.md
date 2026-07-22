# Performance Test Follow-up Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the benchmark/reporting issues surfaced by the 2026-05-29 performance run and make the recommended high-performance paths explicit.

**Architecture:** Keep benchmark orchestration in PowerShell scripts, artifact schema and synthetic runs in Rust, and user-facing capability/default policy in the Tauri UI layer. Changes should be incremental and covered by helper/unit tests before production changes.

**Tech Stack:** PowerShell scripts, Rust cargo tests, Vitest/TypeScript, Tauri benchmark artifacts.

---

### Task 1: Component Matrix Execution Reliability

**Files:**
- Modify: `tests/component-matrix/scripts/run_component_case.ps1`
- Create: `tests/component-matrix/scripts/component_matrix_common.ps1`
- Create: `tests/component-matrix/scripts/test_component_matrix_common.ps1`

**Steps:**
1. Add failing PowerShell helper tests for timeout exit codes, process-tree cleanup command selection, and summary generation when `result.json` exists.
2. Move reusable process execution into `component_matrix_common.ps1`.
3. Update `run_component_case.ps1` to use the helper, accept `-TimeoutSeconds`, and always run summary if the cargo test produced `result.json`.
4. Run `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/test_component_matrix_common.ps1`.
5. Run one component case with a short successful timeout.

### Task 2: Threshold and Skip Semantics

**Files:**
- Modify: `tests/benchmarks/thresholds/*.json`
- Modify: `tests/benchmarks/scripts/transport_matrix_common.ps1`
- Modify: `tests/benchmarks/scripts/summarize_transport_results.ps1`
- Modify: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Steps:**
1. Add tests that skipped runs are reported as skipped, not as threshold failures.
2. Add performance thresholds for 2K144 render present p95 and render queue/drop counters.
3. Keep smoke thresholds loose for diagnostic OpenH264 scenarios.
4. Verify PowerShell tests pass.

### Task 3: FFmpeg Benchmark Measurement Fields

**Files:**
- Modify: `crates/mrd-decode/tests/perf_ffmpeg_compare.rs`

**Steps:**
1. Add expected report fields for `warmup_frames`, `measured_frames`, and steady-state throughput.
2. Update the benchmark to separate startup/warmup from measured decode throughput.
3. Run the FFmpeg compare test at a small resolution.

### Task 4: Capability Defaults and Diagnostic Labels

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx`
- Test: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`
- Modify as needed: `apps/Rdesk/src/app/services/capabilityMatrix.ts`

**Steps:**
1. Add tests proving unsupported AV1 is disabled/skipped with a capability reason.
2. Add tests proving 2K/high-FPS defaults prefer NVDEC or FFmpeg over software, and OpenH264 high-resolution paths are diagnostic.
3. Update UI policy and labels with minimal surface change.
4. Run targeted Vitest.

### Task 5: Render Pacing Configuration

**Files:**
- Modify: `tests/benchmarks/scenarios/*.json`
- Modify: `tests/benchmarks/scripts/run_transport_matrix.ps1`
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`
- Test: existing benchmark/script tests

**Steps:**
1. Add scenario fields for D3D11 waitable object and render thread priority.
2. Have the script map those fields to `MRD_D3D11_RENDER_WAITABLE_OBJECT` and `MRD_RENDER_THREAD_PRIORITY`.
3. Include queue replacement/drop rates or stricter checks in summaries where render fields are present.
4. Run targeted benchmark summary tests.

### Task 6: Final Verification

**Commands:**
- `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/test_component_matrix_common.ps1`
- `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1`
- `cargo test -p mrd-decode --test perf_ffmpeg_compare perf_ffmpeg_decode_compare_reports_results -- --ignored --nocapture`
- `pnpm --dir apps/Rdesk test -- src/app/components/TestWorkbench/MatrixTestPage.test.tsx`
- `cargo fmt --check`
- `git diff --check`
