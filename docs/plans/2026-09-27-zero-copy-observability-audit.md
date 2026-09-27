# Local zero-copy observation audit

Date: 2026-09-27. Scope: Rdesk benchmark/harness, shared-frame application adapters and benchmark report compatibility.

## Evidence and semantics

`zero_copy_requested` records the requested configuration. `zero_copy_enabled` remains an optional boolean for compatibility, but is now derived from observed memory at capture, encoder input, decoder output and renderer input/completion. A request, backend name, initialized renderer, skipped run or empty run cannot establish success.

Each stage records CPU, shared and unknown frame counts in `memory_path_evidence`. Any observed CPU pixel buffer makes the result false. True requires observed shared frames at all four stages and no CPU/unknown observations. Missing or ambiguous evidence stays null. The derived path is `cpu`, `mixed`, `d3d11-shared` or `unknown`.

Capture evidence checks the actual buffer or nonzero shared handle. Encoder evidence is recorded only after nonempty output and requires a compatible shared-input contract for a shared frame. Decode evidence checks actual CPU bytes or nonzero shared-plane handles. Render evidence checks the input and a successful upload; a shared input additionally requires the concrete D3D11 implementation with no CPU transfer and an increase in its presented-frame counter during this upload. Opaque/OpenGL renderers, no target and skipped presentation remain unknown, even if an earlier frame was presented.

The explicit scope is `cpu_pixel_transfers`: these observations cover pixel-buffer CPU boundaries. They do not measure GPU-internal texture copies, compressed packet copies or allocator activity. CPU evidence remains false even if another stage is unobserved. Shared handles alone never establish strict physical zero-copy.

## Compatibility and integration

- JSON fields are optional/defaulted; older reports deserialize with unknown evidence.
- CSV preserves existing column positions and appends requested/scope/path/evidence fields. Evidence JSON is quoted correctly.
- JSON Schema and PowerShell aggregation support the same optional fields. Markdown reports explain the scope and retain unknown values.
- Shared NV12/P010 GPU leases propagate through harness NVDEC conversion, harness render conversion, Rdesk render host, test orchestrator and legacy render host. Owner-lifetime tests ensure dropping the decoded frame cannot recycle storage while its render frame is alive.
- Existing renderer-agnostic CPU NV12/P010-to-BGRA adapter behavior is retained. Direct D3D11 plane upload is available in the renderer crate, but these compatibility adapters do not claim to use it.

## Verification record

The initial policy regressions reproduced a software encoder qualifying as zero copy, and skipped/unstarted runs claiming shared success. Source-extracted Rust tests using the real core/render rlibs also reproduced opaque-renderer and no-presentation false positives and the two application-layer lease drops. Each was observed failing before its corresponding fix; the latest extracted render/lease group passes four tests. These extracted tests are narrow checks, not full application build validation.

`tests/benchmarks/scripts/test_transport_matrix_common.ps1` passed after regression-first changes covering schema, CSV evidence, Markdown fields and copy scope. `git diff --check` passed for the modified observation/application adapters.

The first full application test command found two missing P010 match arms in Rdesk render_host; both were repaired. Its unrelated integration-test compilation was stopped after that failure to release the shared Cargo lock. The focused application command then completed successfully:

- `cargo test -p app --bin app zero_copy_ -- --nocapture`: 23 passed, 0 failed, 0 ignored.
- The same compiled application test executable, `test_harness::tests --nocapture --test-threads=1`: 64 passed, 0 failed, 1 ignored (the explicitly manual hardware performance test).
- The same executable, `benchmark::tests --nocapture --test-threads=1`: 20 passed, 0 failed. The environment-driven benchmark hook returns early in this unit suite; actual hardware execution is recorded separately below.
- `cargo test -p rdesk-legacy-harness --lib shared_frame_conversion_retains_gpu_lease_until_render_drop -- --nocapture`: 1 passed, 0 failed, covering both shared formats.
- All three actual summary CSV files were parsed by PowerShell `Import-Csv`; their nested evidence JSON parsed successfully and matched the JSON summary counters.
- `cargo check -p app --bin app` after AV1 lifecycle and poisoned-CUDA-slot follow-ups completed successfully (exit 0). This check preceded the later initial-resource-map correction.
- After the encoder owner confirmed final initial-resource-map source stability and all seven strict encoder probes passed, `cargo check -p app --bin app` was rerun: exit 0, 3.26 seconds, checking both NVENC crates and the application. This establishes final application compilation, not a repeat of the three pipeline runs.

No hardware success is inferred from source-level checks or early-returning test hooks.

## Actual local pipeline runs

The application executable ran three serial H.264 harness cases with the existing benchmark hook explicitly enabled. `prepare_local_nvidia_runtime.ps1` supplied aliases of installed driver DLLs through this test process's PATH and its children; no machine/user PATH or driver installation was changed. Requested dimensions were 1280×720, target 30 FPS, 4 Mbps, three seconds of sampling after first-present detection, loopback transport and D3D11 rendering. An independent OpenH264 decode of each dumped first access unit verified actual **1280×720** dimensions. The harness includes startup and its first-present wait; the counts below are not three seconds multiplied by target FPS.

| Capture → NVENC → decoder → D3D11 | Presented frames | Requested shared path | Observed result | Run result |
| --- | ---: | --- | --- | --- |
| DXGI → NVENC → software H.264 → D3D11 | 120 | false | CPU at all four stages; zero_copy_enabled=false | passed, not skipped |
| Synthetic → NVENC → NVDEC CPU → D3D11 | 121 | false | CPU at all four stages; zero_copy_enabled=false | passed, not skipped |
| DXGI shared → NVENC → optional NVDEC shared → D3D11 | 120 | true | Shared capture 121 / encoder 120; CPU decode 120 / render 120; mixed, zero_copy_enabled=false | passed through CPU fallback, not skipped |

The optional shared-decoder case attempted shared CUDA/D3D11 copying 120 times, all failed at `register`, API `cuGraphicsD3D11RegisterResource:Y`, with `CUDA_ERROR_INVALID_DEVICE`. The decoder still produced CPU frames and the renderer presented them; this is working fallback, **not** a successful shared decode/render pipeline. Shared successes serialize as null because the existing counter-to-summary conversion omits zero counters. The separate strict NVDEC probe provides the explicit shared-unavailable result.

These are short local harness correctness/evidence checks. They are not a complete service/UI session test, and different capture content and startup costs prevent treating their FPS numbers as a controlled performance-improvement comparison.

The three pipeline runs also **precede the later NVENC initial-resource-map correction** for H.264/HEVC/AV1. Initial registration previously mapped the resource before the first CPU upload; the follow-up addresses that first-frame ownership boundary. The recorded runs and their hashes are preserved as executed and do not establish hardware coverage of this later correction. Strict encoder-probe results and subsequent application compilation for that correction are reported separately; these three pipelines were not rerun.

Artifacts:

- `artifacts/benchmarks/2026-09-27/local-zero-copy-observed/{dxgi-nvenc-software,synthetic-nvenc-nvdec-cpu,dxgi-nvenc-nvdec-shared}/summary.json` and sibling CSV/Markdown/manifest files.
- `artifacts/local-zero-copy/2026-09-27/observed-pipeline-results.json`: concise outcomes and independent decoded dimensions.
- `observed-pipeline-test-executable.json`, `observed-pipeline-built-dependencies.json`, `observed-pipeline-build-source-checkpoint.json` in the same directory record the executable SHA256, the actual linked core/codec/render rlib hashes, the build command and source checkpoint. This is a dirty workspace, not a clean Git revision.
- The build includes the verified H.264 NVENC/capture lifecycle changes. Independent AV1 lifecycle and poisoned-CUDA-slot follow-ups happened after this application build; they do not change the measured H.264 normal/registration-failure path, but are not claimed as covered by these pipeline runs.
