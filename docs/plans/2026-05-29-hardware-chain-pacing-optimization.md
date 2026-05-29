# Hardware Chain Pacing Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Windows hardware capture/encode/transport/decode/render path report real receiver-side FPS and render pacing health for 2K144 benchmark runs.

**Architecture:** Keep the existing DXGI/NVENC/WebRTC/NVDEC/D3D11 path intact and add observability at the harness render boundary. Render submission returns renderer snapshots so the harness can derive upload, present, skip, replacement, stale-drop, and present-gap metrics without changing codec or transport selection.

**Tech Stack:** Rust, Tauri `app` crate, D3D11 renderer abstraction, existing benchmark JSON/probe output, PowerShell benchmark matrix scripts.

---

### Task 1: Commit the Approved Design and Plan

**Files:**
- Create: `docs/plans/2026-05-29-hardware-chain-pacing-optimization-design.md`
- Create: `docs/plans/2026-05-29-hardware-chain-pacing-optimization.md`

**Step 1: Validate documentation diff**

Run: `git diff --check`

Expected: PASS with no whitespace errors.

**Step 2: Commit documentation**

Run:

```bash
git add docs/plans/2026-05-29-hardware-chain-pacing-optimization-design.md docs/plans/2026-05-29-hardware-chain-pacing-optimization.md
git commit -m "docs: add hardware pacing optimization plan"
git push
```

Expected: branch `codex/hardware-chain-pacing-optimization` pushes cleanly.

### Task 2: Add Render Pacing Metrics to HarnessMetrics

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write failing metrics tests**

Add tests near the existing `update_metrics_reports_*` tests:

```rust
#[test]
fn update_metrics_reports_render_present_gap_distribution() {
    let mut state = TestHarnessState::default();
    let started = Instant::now();
    state.start_time = Some(started);
    state.captured_frames = 12;
    state.encoded_units = 12;
    state.decoded_frames = 12;
    state.render_submitted_frames = 12;
    state.render_uploaded_frames = 11;
    state.render_presented_frames = 10;
    state.render_present_skipped_frames = 1;
    state.render_queue_replacements = 2;
    state.render_stale_frame_drops = 2;
    state.render_present_gaps.push(Duration::from_millis(6));
    state.render_present_gaps.push(Duration::from_millis(7));
    state.render_present_gaps.push(Duration::from_millis(9));

    TestHarness::update_metrics(&state);
    let metrics = state.metrics_snapshot();

    assert_eq!(metrics.render_submitted_frames, 12);
    assert_eq!(metrics.render_uploaded_frames, 11);
    assert_eq!(metrics.render_presented_frames, 10);
    assert_eq!(metrics.render_present_skipped_frames, 1);
    assert_eq!(metrics.render_queue_replacements, 2);
    assert_eq!(metrics.render_stale_frame_drops, 2);
    assert_eq!(metrics.render_present_gap_p50_ms, 7.0);
    assert_eq!(metrics.render_present_gap_p95_ms, 9.0);
}
```

Expected initial failure: new fields do not exist.

**Step 2: Add minimal metrics fields**

Add these fields to `HarnessMetrics` and its `Default` implementation:

```rust
pub render_latency_p50_ms: f64,
pub render_latency_p95_ms: f64,
pub render_submitted_frames: u64,
pub render_uploaded_frames: u64,
pub render_presented_frames: u64,
pub render_present_skipped_frames: u64,
pub render_queue_replacements: u64,
pub render_stale_frame_drops: u64,
pub render_present_gap_avg_ms: f64,
pub render_present_gap_p50_ms: f64,
pub render_present_gap_p95_ms: f64,
```

Keep existing `render_latency_avg_ms` and `present_latency_avg_ms` for compatibility. Set `present_latency_avg_ms` from `render_present_gap_avg_ms`.

**Step 3: Store state counters and gap samples**

Add matching state fields to `TestHarnessState`, plus a bounded `VecDeque<Duration>` for present gaps. Trim it alongside the existing latency buffers.

**Step 4: Update metrics calculation**

In `TestHarness::update_metrics`, calculate:

- render upload average, p50, and p95 from `state.render_latencies`
- present gap average, p50, and p95 from `state.render_present_gaps`
- render counters by copying state fields into `HarnessMetrics`

Run: `cargo test -p app update_metrics_reports_render_present_gap_distribution`

Expected: PASS.

### Task 3: Return Renderer Snapshots From Render Submission

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write a pure helper test for snapshot deltas**

Add a small helper that consumes the previous renderer snapshot, current snapshot, and render duration, then updates render counters/gaps. Test it with synthetic snapshots so no D3D11 device is required.

Example expected behavior:

```rust
#[test]
fn record_render_completion_tracks_present_gap_only_on_new_present() {
    let mut state = TestHarnessState::default();
    let previous = RendererSnapshot {
        uploaded_frame_count: 7,
        presented_frame_count: 3,
        present_skipped_count: 1,
        ..RendererSnapshot::default()
    };
    let current = RendererSnapshot {
        uploaded_frame_count: 8,
        presented_frame_count: 4,
        present_skipped_count: 1,
        ..RendererSnapshot::default()
    };

    state.last_render_present_at = Some(Instant::now() - Duration::from_millis(7));
    TestHarness::record_render_completion(
        &mut state,
        Some(&previous),
        &current,
        Duration::from_micros(300),
        Instant::now(),
    );

    assert_eq!(state.render_uploaded_frames, 8);
    assert_eq!(state.render_presented_frames, 4);
    assert_eq!(state.render_present_gaps.len(), 1);
}
```

Expected initial failure: helper and state fields do not exist.

**Step 2: Change render job completion type**

Replace the render job completion channel payload:

```rust
struct RenderCompletion {
    snapshot: RendererSnapshot,
}

struct RenderJob {
    input: RenderInput,
    completion: mpsc::SyncSender<Result<RenderCompletion, String>>,
}
```

`complete_render_job` should upload the render input, then return `renderer.snapshot()` in `RenderCompletion`.

**Step 3: Change PipelineRenderer::submit_frame**

Return `Result<RenderCompletion>` instead of `Result<()>`. Keep stopped-thread and timeout handling as hard failures.

**Step 4: Wire completion into the process loop**

Around the existing render submission:

- increment `render_submitted_frames`
- capture the previous renderer snapshot
- call `submit_frame`
- call `record_render_completion` with previous/current snapshots and elapsed upload duration

Run:

```bash
cargo test -p app record_render_completion_tracks_present_gap_only_on_new_present
cargo test -p app update_metrics_reports_render_present_gap_distribution
```

Expected: PASS.

### Task 4: Export Receiver FPS and Render Pacing in Benchmark Output

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`
- Modify: `tests/benchmarks/schemas/benchmark-result.schema.json`

**Step 1: Write failing summary tests**

Add focused tests near benchmark summary tests:

```rust
#[test]
fn benchmark_summary_prefers_decoded_fps_when_decode_backend_is_active() {
    let metrics = HarnessMetrics {
        capture_fps: 144.0,
        decoded_fps: 118.0,
        ..HarnessMetrics::default()
    };
    let config = BenchmarkConfig {
        decode_backend: DecoderType::Nvdec,
        ..BenchmarkConfig::default()
    };

    let summary = BenchmarkSummary::from_harness_metrics_for_test(&config, &metrics);

    assert_eq!(summary.fps_observed, 118.0);
}

#[test]
fn benchmark_probe_exports_render_present_gap_stats() {
    let metrics = HarnessMetrics {
        render_latency_avg_ms: 0.20,
        render_latency_p50_ms: 0.18,
        render_latency_p95_ms: 0.35,
        render_present_gap_avg_ms: 6.94,
        render_present_gap_p50_ms: 6.90,
        render_present_gap_p95_ms: 7.40,
        ..HarnessMetrics::default()
    };

    let probe = probe_from_metrics_for_test(&metrics);
    let present = probe.stage("render_present_gap").expect("present gap stage");

    assert_eq!(present.p95_ms, 7.40);
}
```

Expected initial failure: helper functions or fields do not exist.

**Step 2: Extract summary FPS helper**

Add a helper used by `run_harness_benchmark`:

```rust
fn observed_fps_for_summary(config: &BenchmarkConfig, metrics: &HarnessMetrics) -> f64 {
    if config.decode_backend != DecoderType::None && metrics.decoded_fps > 0.0 {
        metrics.decoded_fps
    } else {
        metrics.capture_fps
    }
}
```

Use this helper for `BenchmarkSummary.fps_observed` and probe-level observed FPS.

**Step 3: Use p95 render metrics**

Set:

- `render_upload_p95_ms` from `metrics.render_latency_p95_ms`
- `render_present_p95_ms` from `metrics.render_present_gap_p95_ms`

In probe output, emit:

- `render_upload` stats from upload avg/p50/p95
- `render_present_gap` stats from present gap avg/p50/p95
- keep any existing `render_present` compatibility stage only if required by existing scripts

**Step 4: Extend benchmark JSON schema**

If new top-level fields are added, update `tests/benchmarks/schemas/benchmark-result.schema.json`. If only existing `render_upload_p95_ms` and `render_present_p95_ms` become populated, keep schema unchanged.

Run:

```bash
cargo test -p app benchmark_summary_prefers_decoded_fps_when_decode_backend_is_active
cargo test -p app benchmark_probe_exports_render_present_gap_stats
```

Expected: PASS.

### Task 5: Validate Existing Matrix Dispatch and Hardware Runs

**Files:**
- Modify only if tests reveal a real contract gap:
  - `tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.json`
  - `tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.2k144.json`
  - `tests/benchmarks/scripts/*.ps1`

**Step 1: Run formatting and focused tests**

Run:

```bash
cargo fmt --check
cargo test -p app update_metrics_reports_render_present_gap_distribution
cargo test -p app record_render_completion_tracks_present_gap_only_on_new_present
cargo test -p app benchmark_summary_prefers_decoded_fps_when_decode_backend_is_active
cargo test -p app benchmark_probe_exports_render_present_gap_stats
cargo test -p app matrix_dispatch_maps_explicit_encoder_decoder_pairs
```

Expected: all PASS.

**Step 2: Run 2K144 hardware benchmarks**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'E:\codex-target\mini-remote-desktop-hardware-pacing'
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.json
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.2k144.json
```

Expected:

- H.264 and HEVC scenarios PASS.
- `fps_observed` reflects decoded FPS when NVDEC is active.
- `render_present_p95_ms` is non-null.
- Probe stages include `render_present_gap`.
- `decode_p95_ms` remains at or below 8 ms.
- observed FPS remains at or above 120.

**Step 3: Commit and push implementation**

Run:

```bash
git add apps/Rdesk/src-tauri/src/test_harness.rs apps/Rdesk/src-tauri/src/benchmark.rs tests/benchmarks/schemas/benchmark-result.schema.json
git commit -m "perf: expose render pacing metrics"
git push
```

Expected: branch `codex/hardware-chain-pacing-optimization` pushes cleanly.
