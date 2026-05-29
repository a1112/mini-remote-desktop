# Base Capability Followups Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Repair the capability and benchmark contract gaps identified in the base capability review.

**Architecture:** Keep changes local to benchmark scripts/schema, frontend capability/classification helpers, and mrd-service capability snapshot generation. Preserve current runtime behavior except for selecting FFmpeg as a local fallback decoder and not advertising planned decode work as runnable support.

**Tech Stack:** Rust, PowerShell, TypeScript, Vitest, cargo test.

---

### Task 1: Benchmark Schema Contract

**Files:**
- Modify: `tests/benchmarks/schemas/benchmark-result.schema.json`
- Modify: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Steps:**
1. Add a failing PowerShell assertion that the synthetic summary emitted by `summarize_transport_results.ps1` has no fields missing from the JSON schema.
2. Run `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1` and confirm it fails on extra summary fields.
3. Add the missing schema properties.
4. Re-run the same PowerShell test and confirm it passes.

### Task 2: FFmpeg Classification

**Files:**
- Modify: `apps/Rdesk/src/app/services/testClassificationService.test.ts`
- Modify: `apps/Rdesk/src/app/services/testClassificationService.ts`

**Steps:**
1. Add a failing Vitest test proving `ffmpeg_h264` and `ffmpeg_hevc` are classified as software decode acceleration.
2. Run the focused Vitest test and confirm it fails.
3. Add the minimal classification cases.
4. Re-run the focused Vitest test and confirm it passes.

### Task 3: Transport Decoder Preference

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/TransportTestPage.test.tsx`
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/TransportTestPage.tsx`

**Steps:**
1. Add a failing test where local Windows transport has `ffmpeg_h264` but no `nvdec`, and verify `test_start_run` receives `decoder_type: "ffmpeg_h264"`.
2. Run the focused TransportTestPage test and confirm it fails.
3. Update local decoder candidate order.
4. Re-run the focused test and confirm it passes.

### Task 4: Capability Status Tightening

**Files:**
- Modify: `apps/mrd-service/src/capabilities.rs`

**Steps:**
1. Add a failing unit test that macOS planned VideoToolbox decode is advertised as `Unimplemented`, not `Supported`.
2. Run `cargo test -p mrd-service capabilities::tests::<test-name>`.
3. Change the planned decode capability status.
4. Re-run mrd-service capability tests.

### Task 5: Transport Benchmark Timeout

**Files:**
- Modify: `tests/benchmarks/scripts/transport_matrix_common.ps1`
- Modify: `tests/benchmarks/scripts/run_transport_matrix.ps1`
- Modify: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Steps:**
1. Add a failing PowerShell test showing a long-running command times out and returns a timeout result.
2. Update `Invoke-TransportMatrixCommand` to support `-TimeoutSeconds`, job-based execution, stdout/stderr capture, and process cleanup.
3. Add `-TimeoutSeconds` to `run_transport_matrix.ps1` and fail with a timeout-specific message.
4. Re-run benchmark script tests.

### Task 6: Verification and Commit

**Commands:**
- `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1`
- `pnpm --dir apps/Rdesk test -- src/app/services/testClassificationService.test.ts src/app/components/TestWorkbench/TransportTestPage.test.tsx`
- `cargo test -p mrd-service capabilities`
- `pnpm --dir apps/Rdesk type-check`
- `cargo fmt --check`
- `git diff --check`

Commit and push the branch after all targeted verification passes.
