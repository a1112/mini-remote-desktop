# Low Latency Render Strategy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a latency-first native LAN render strategy that keeps the newest frame, records stale-frame drops, exposes D3D11 swapchain pacing metadata, and makes waitable-swapchain waits visible.

**Architecture:** Keep the stable paced path as the default, but add an explicit render queue policy. `MRD_LAN_RENDER_QUEUE_POLICY=latest` or `low_latency` enables the latency-first latest-frame path, while the default `paced_fifo` preserves visual cadence. D3D11 waitable-swapchain waits move to a pre-render boundary and are exported through renderer snapshots and service pipeline metrics.

**Tech Stack:** Rust workspace, Windows D3D11/DXGI via `windows`, Tauri IPC types, React/TypeScript diagnostics UI, Cargo tests, Vitest tests, PowerShell LAN/benchmark scripts.

---

### Task 1: Service Render Queue Policy

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify: `apps/mrd-service/src/app_state.rs`
- Modify: `crates/mrd-ipc/src/lib.rs`

**Step 1: Write failing tests**

Add tests in `apps/mrd-service/src/lan_discovery.rs`:

- `render_queue_policy_env_parses_values`
- `render_queue_policy_defaults_to_paced_fifo_and_allows_latest_override`
- `latest_render_queue_policy_skips_pacing_wait`

Add a test in `apps/mrd-service/src/app_state.rs` that `take_latest_or_finish` stale drops can be propagated to `MediaPipelineSnapshot`.

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p mrd-service render_queue_policy -- --nocapture
cargo test -p mrd-service media_render_queue_can_take_latest_and_drop_stale_backlog -- --nocapture
```

Expected: compile/test failure for missing policy helpers and snapshot field.

**Step 3: Implement minimal policy**

Add:

- `MRD_LAN_RENDER_QUEUE_POLICY`
- `LanRenderQueuePolicy::{PacedFifo, Latest}`
- Parser accepting `latest`, `low_latency`, `latency`, `paced_fifo`, `paced-fifo`, and `fifo`
- `lan_render_queue_policy_for_profile(profile)`
- `render_queue_policy` field on `MediaPipelineSnapshot`

Default profiles to `paced_fifo`; enable `latest` only when explicitly requested with `MRD_LAN_RENDER_QUEUE_POLICY=latest`, `low_latency`, or `latency`.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service render_queue_policy -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs apps/mrd-service/src/app_state.rs crates/mrd-ipc/src/lib.rs
git commit -m "perf: add LAN render queue policy"
```

### Task 2: Latest-Frame Worker Drain and Stale Drops

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Test: `apps/mrd-service/src/lan_discovery.rs`
- Test: `apps/mrd-service/src/app_state.rs`

**Step 1: Write failing tests**

Add a service-side test that simulates queued frames under `latest` policy and verifies:

- worker takes the newest queued frame
- older queued frames are counted as `render_stale_frame_drops`
- `render_pacing_wait` is not recorded for latest policy when a frame is ready

**Step 2: Run test to verify failure**

Run:

```powershell
cargo test -p mrd-service latest_render_queue -- --nocapture
```

Expected: FAIL until worker uses `take_latest_or_finish`.

**Step 3: Implement worker policy**

In `run_lan_render_worker`:

- read `lan_render_queue_policy_for_profile(&render_profile)`
- bypass `pace_lan_render_frame` when policy is `Latest`
- after rendering, use `take_latest_or_finish` for latest policy
- increment `render_stale_frame_drops` by the dropped backlog count
- record `render_queue_policy` in the pipeline snapshot

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service latest_render_queue -- --nocapture
cargo test -p mrd-service render_pacing_defaults_to_interruptible_refresh_cap -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs apps/mrd-service/src/app_state.rs crates/mrd-ipc/src/lib.rs
git commit -m "perf: render latest LAN frame first"
```

### Task 3: D3D11 Waitable Pre-Render Telemetry

**Files:**
- Modify: `crates/mrd-render/src/lib.rs`
- Modify: `crates/mrd-render-d3d11/src/lib.rs`
- Modify: platform renderer snapshot constructors under `crates/mrd-render-*`
- Modify: test/harness snapshot fixtures under `apps/` and `tests/`

**Step 1: Write failing tests**

Add D3D11 unit tests for:

- waitable mode records wait count and wait duration in `RendererSnapshot`
- waitable timeout increments timeout count and uses `skipped_frame_latency_wait`
- non-waitable mode reports no waitable wait metrics

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p mrd-render-d3d11 d3d11_waitable -- --nocapture
```

Expected: compile/test failure for missing snapshot metrics.

**Step 3: Implement minimal telemetry**

Add optional snapshot fields:

- `waitable_wait_count`
- `waitable_wait_total_ms`
- `waitable_timeout_count`
- `last_waitable_wait_ms`

Move D3D11 waitable waiting to the beginning of `upload_frame` before upload/draw work. Do not wait again inside `present_swap_chain`.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-render-d3d11 d3d11_waitable -- --nocapture
cargo test -p mrd-render-d3d11
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add crates/mrd-render/src/lib.rs crates/mrd-render-d3d11/src/lib.rs crates/mrd-render-*/src/lib.rs apps tests
git commit -m "perf: expose D3D11 waitable render waits"
```

### Task 4: Service IPC and UI Diagnostics

**Files:**
- Modify: `apps/mrd-service/src/app_state.rs`
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Test: `crates/mrd-ipc/tests/contracts.rs`
- Test: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Step 1: Write failing tests**

Add tests that pipeline snapshots expose:

- `render_queue_policy`
- `render_stale_frame_drops`
- `swap_chain_present_mode`
- `swap_chain_waitable_object`
- `swap_chain_allow_tearing`
- `display_refresh_hz`
- `render_thread_priority`
- waitable wait stage p95

Add UI test that diagnostics displays stale drops and swapchain policy.

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p mrd-ipc media_pipeline_snapshot -- --nocapture
cd apps/Rdesk; pnpm test -- RemoteDisplayWindowPage.test.tsx
```

Expected: FAIL until fields are wired.

**Step 3: Implement IPC/UI wiring**

Add optional fields to Rust IPC and TypeScript types. In service LAN render completion, diff renderer snapshots and record:

- swapchain metadata into the pipeline snapshot
- `render_waitable_wait` stage duration from wait count/total deltas
- `render_waitable_timeouts` counter

Update diagnostics rows to include native render policy and stale drops.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-ipc media_pipeline_snapshot -- --nocapture
cd apps/Rdesk; pnpm test -- RemoteDisplayWindowPage.test.tsx
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/app_state.rs apps/mrd-service/src/lan_discovery.rs crates/mrd-ipc/src/lib.rs crates/mrd-ipc/tests/contracts.rs apps/Rdesk/src/app/adapters/tauri/types.ts apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx
git commit -m "feat: surface native render pacing diagnostics"
```

### Task 5: Verification and Benchmark Comparison

**Files:**
- No source edits unless tests expose a bug.

**Step 1: Run focused Rust tests**

Run:

```powershell
cargo test -p mrd-render-d3d11
cargo test -p mrd-service render_queue_policy -- --nocapture
cargo test -p mrd-service latest_render_queue -- --nocapture
cargo test -p mrd-ipc media_pipeline_snapshot -- --nocapture
```

Expected: PASS.

**Step 2: Run frontend type/test checks**

Run:

```powershell
cd apps/Rdesk
pnpm type-check
pnpm test -- RemoteDisplayWindowPage.test.tsx
```

Expected: PASS.

**Step 3: Run benchmark script checks**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: PASS.

**Step 4: Run native LAN performance comparison**

Run a 1080p60 smoke and a 2K144 HEVC/NVDEC/D3D11 canary. Compare:

- `render_upload`
- `render_present`
- `render_pacing_wait`
- `render_present_gap`
- `render_queue_replacements`
- `render_stale_frame_drops`
- `render_waitable_wait`
- observed render FPS

Expected: default paced FIFO keeps visual integrity stable. Explicit latest policy reduces service-side render pacing wait and keeps decode/main pipeline independent, but can increase stale-frame drops under overloaded or same-host fixtures. Present gap remains bounded by display refresh and should be reported separately from the 3 ms local pipeline target.

**Step 5: Commit final verification notes if artifacts or docs change**

Commit only source/docs changes. Do not commit generated benchmark artifacts unless explicitly requested.
