# D3D11 Swapchain Pacing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add opt-in D3D11 waitable-object pacing, render-thread priority reporting, display refresh diagnostics, and benchmark exports.

**Architecture:** Keep nonblocking D3D11 present as the default. Add a waitable-object path behind environment flags and report the selected pacing policy through `RendererSnapshot`, harness metrics, benchmark summaries, CSV, and reports.

**Tech Stack:** Rust workspace, Windows D3D11/DXGI via `windows` crate, PowerShell benchmark scripts, Cargo tests.

---

### Task 1: Add Snapshot Fields

**Files:**
- Modify: `crates/mrd-render/src/lib.rs`
- Modify: all `RendererSnapshot` constructors in render crates and harness tests

**Step 1: Write the failing test**

Add assertions in D3D11 renderer tests that `RendererSnapshot` has `swap_chain_waitable_object`, `swap_chain_present_mode`, `display_refresh_hz`, and `render_thread_priority`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-render-d3d11 d3d11_renderer_reports_low_latency_frame_latency_target -- --nocapture`

Expected: compile failure for missing fields.

**Step 3: Implement minimal fields**

Add optional fields to `RendererSnapshot`, update all constructors with `None`, and set D3D11 defaults.

**Step 4: Verify**

Run: `cargo test -p mrd-render-d3d11 d3d11_renderer_reports_low_latency_frame_latency_target -- --nocapture`

### Task 2: Add Waitable Swapchain Policy

**Files:**
- Modify: `crates/mrd-render-d3d11/src/lib.rs`
- Test: `crates/mrd-render-d3d11/src/lib.rs`

**Step 1: Write failing tests**

Add tests for:

- waitable flag is off by default.
- `MRD_D3D11_RENDER_WAITABLE_OBJECT=1` adds `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT`.
- present mode reports `waitable` when the env var is enabled.

**Step 2: Run tests**

Run: `cargo test -p mrd-render-d3d11 d3d11_swap_chain_desc -- --nocapture`

Expected: fail for missing waitable policy.

**Step 3: Implement minimal policy**

Add env parser, swapchain flag, `IDXGISwapChain2` storage, waitable handle storage, wait-before-present, and snapshot reporting.

**Step 4: Verify**

Run: `cargo test -p mrd-render-d3d11`

### Task 3: Add Render Thread Priority Diagnostics

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Test: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write failing tests**

Add a pure helper test for parsing `MRD_RENDER_THREAD_PRIORITY=above_normal`.

**Step 2: Run test**

Run: `cargo test -p app render_thread_priority -- --nocapture`

Expected: fail for missing helper.

**Step 3: Implement helper and Windows thread priority call**

Set priority at render-thread start only when requested. Report the configured label into `RendererSnapshot` via harness completion.

**Step 4: Verify**

Run: `cargo test -p app render_thread_priority -- --nocapture`

### Task 4: Export Diagnostics Through Benchmark Summary

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`
- Modify: `tests/benchmarks/scripts/summarize_transport_results.ps1`
- Test: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Step 1: Write failing tests**

Add benchmark summary assertions for swapchain waitable, present mode, display refresh, and render thread priority.

**Step 2: Run tests**

Run: `cargo test -p app harness_probe_exports_render_upload_and_present_gap_p95 -- --nocapture`

Expected: fail for missing fields.

**Step 3: Implement exports**

Carry fields from renderer snapshot to harness metrics, probe counters/stages where appropriate, summary JSON/CSV, and report.

**Step 4: Verify**

Run:

```powershell
cargo test -p app
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1
```

### Task 5: Benchmark Compare

**Files:**
- No source edits.

**Step 1: Run default benchmark**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.json
```

**Step 2: Run waitable benchmark**

Run:

```powershell
$env:MRD_D3D11_RENDER_WAITABLE_OBJECT='1'
$env:MRD_RENDER_THREAD_PRIORITY='above_normal'
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.json
Remove-Item Env:\MRD_D3D11_RENDER_WAITABLE_OBJECT
Remove-Item Env:\MRD_RENDER_THREAD_PRIORITY
```

**Step 3: Compare**

Compare FPS, decode p95, render present p95, skipped presents, queue replacements, and stale drops.

