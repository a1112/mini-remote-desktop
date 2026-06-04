# GPU Zero-Copy Color Modes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add full, grayscale, monochrome, and low-chroma color modes to the GPU zero-copy NVENC path and expose the selected mode in UI and benchmark artifacts.

**Architecture:** Keep color transforms separate from bit-depth pipeline selection. `ColorMode` controls GPU pixel transforms on the shared-BGRA NVENC input path; `ColorPipeline` keeps SDR8 versus HDR/Main10 semantics separate. The first implementation targets Windows DXGI shared texture capture with NVENC H.264/HEVC.

**Tech Stack:** Rust, D3D11, NVENC, Tauri command contracts, TypeScript, PowerShell benchmark tooling.

---

### Task 1: Add Color Mode Types and Serialization

**Files:**
- Modify: `crates/mrd-pipeline-core/src/lib.rs`
- Modify: `crates/mrd-pipeline-core/src/encoder_config.rs`
- Test: `crates/mrd-pipeline-core/src/encoder_config.rs`

**Step 1: Write failing tests**

Add tests proving:

- `ColorMode::default()` is `Full`;
- serde uses snake_case values: `full`, `grayscale`, `monochrome`, `low_chroma`;
- `ColorPipeline::default()` is `Sdr8`;
- serde uses `sdr8` and `hdr_main10`.

Run:

```powershell
cargo test -p mrd-pipeline-core color_mode -- --nocapture
```

Expected: FAIL because the types do not exist.

**Step 2: Implement minimal shared types**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    #[default]
    Full,
    Grayscale,
    Monochrome,
    LowChroma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorPipeline {
    #[default]
    Sdr8,
    HdrMain10,
}
```

Export them from the crate.

**Step 3: Verify**

Run:

```powershell
cargo test -p mrd-pipeline-core color_mode -- --nocapture
```

Expected: PASS.

### Task 2: Thread Config Through Harness and Frontend Types

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Test: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Test: `apps/Rdesk/src/app/adapters/tauri/contract.test.ts`

**Step 1: Write failing tests**

Add Rust tests proving `TestConfig` defaults do not require explicit color fields and JSON can deserialize:

```json
{ "color_mode": "grayscale", "color_pipeline": "sdr8" }
```

Add a TS contract assertion that `TestConfig` accepts:

```ts
const config: TestConfig = {
  color_mode: "grayscale",
  color_pipeline: "sdr8",
};
```

Run:

```powershell
cargo test -p app test_config_color -- --nocapture
pnpm --dir apps\Rdesk type-check
```

Expected: FAIL because fields/types are missing.

**Step 2: Implement config fields**

Add optional fields:

```rust
pub color_mode: Option<ColorMode>,
pub color_pipeline: Option<ColorPipeline>,
```

Add TS fields:

```ts
color_mode?: "full" | "grayscale" | "monochrome" | "low_chroma";
color_pipeline?: "sdr8" | "hdr_main10";
```

**Step 3: Verify**

Run the same commands. Expected: PASS.

### Task 3: Report Color Mode in Metrics and Benchmarks

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/benchmark.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `tests/benchmarks/schemas/benchmark-result.schema.json`
- Modify: `tests/benchmarks/scripts/summarize_transport_results.ps1`
- Modify: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Step 1: Write failing tests**

Extend existing summary/schema tests to require:

- `color_mode`;
- `color_pipeline`;
- CSV columns for both;
- markdown rows for both.

Run:

```powershell
cargo test -p app benchmark_summary_csv_row_uses_stable_columns -- --nocapture
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\test_transport_matrix_common.ps1
```

Expected: FAIL because fields are missing.

**Step 2: Implement reporting**

Add optional string fields to metrics and summary. Populate from resolved config defaults:

- missing `color_mode` -> `full`;
- missing `color_pipeline` -> `sdr8`, except explicit HEVC Main10 scenarios can report `hdr_main10`.

**Step 3: Verify**

Run the same tests. Expected: PASS.

### Task 4: Add NVENC GPU Color Transform Pass

**Files:**
- Modify: `crates/mrd-encode-nvenc/src/lib.rs`
- Test: `crates/mrd-encode-nvenc/tests/nvenc_encoder.rs`

**Step 1: Write failing tests**

Add tests for constructor/config behavior:

- H.264 encoder reports default `ColorMode::Full`;
- HEVC encoder reports default `ColorMode::Full`;
- `with_color_mode(ColorMode::Grayscale)` preserves shared-texture capability;
- `with_color_mode(ColorMode::Monochrome)` and `LowChroma` are accepted.

Run:

```powershell
cargo test -p mrd-encode-nvenc color_mode -- --nocapture
```

Expected: FAIL because the API is missing.

**Step 2: Implement API and pass selection**

Add `color_mode` fields to Windows `NvencH264Encoder` and `NvencHevcEncoder`.

Keep existing `CopyResource` for `Full`. Add `copy_or_transform_shared_bgra_to_texture` that dispatches to:

- copy path for `Full`;
- D3D11 shader pass for `Grayscale`, `Monochrome`, `LowChroma`.

**Step 3: Implement D3D11 resources**

Add cached resources:

- shader pipeline;
- SRV for shared input texture;
- RTV for encode slot texture;
- sampler state.

Use full-screen triangle draw. Unbind SRVs after draw.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-encode-nvenc color_mode -- --nocapture
cargo test -p mrd-encode-nvenc nvenc -- --nocapture
```

Expected: PASS on Windows with NVENC support.

### Task 5: Connect Harness Encoder Construction

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write failing tests**

Add tests proving NVENC H.264/HEVC creation receives `ColorMode` from `TestConfig`.

Run:

```powershell
cargo test -p app nvenc_color_mode -- --nocapture
```

Expected: FAIL because config is not used.

**Step 2: Implement connection**

Resolve:

```rust
let color_mode = config.color_mode.unwrap_or_default();
let color_pipeline = config.color_pipeline.unwrap_or_default();
```

Pass `color_mode` into NVENC constructors. Preserve existing `NvencHevcMain10` behavior and map it to `ColorPipeline::HdrMain10` for reporting.

**Step 3: Verify**

Run the same test. Expected: PASS.

### Task 6: Add Benchmark Scenarios

**Files:**
- Create: `tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.grayscale.json`
- Create: `tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.monochrome.json`
- Create: `tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.low_chroma.json`
- Modify: `tests/benchmarks/scripts/test_transport_matrix_common.ps1`

**Step 1: Write failing scenario coverage test**

Extend scenario coverage to require all three color scenarios.

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\test_transport_matrix_common.ps1
```

Expected: FAIL because scenarios are missing.

**Step 2: Add scenarios**

Clone the existing 4K120 HEVC/NVDEC waitable scenario and set:

```json
"color_mode": "grayscale"
```

Repeat for `monochrome` and `low_chroma`.

**Step 3: Verify**

Run the same script. Expected: PASS.

### Task 7: Full Verification and Real Benchmarks

**Files:**
- No production files expected.

**Step 1: Run static and unit gates**

Run:

```powershell
cargo fmt --check
cargo test -p mrd-pipeline-core color_mode -- --nocapture
cargo test -p mrd-encode-nvenc color_mode -- --nocapture
cargo test -p app color_mode -- --nocapture
pnpm --dir apps\Rdesk type-check
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\test_transport_matrix_common.ps1
```

Expected: PASS.

**Step 2: Run real benchmark matrix**

Run full and color-mode scenarios:

```powershell
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\run_transport_matrix.ps1 -ScenarioPath tests\benchmarks\scenarios\quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.json
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\run_transport_matrix.ps1 -ScenarioPath tests\benchmarks\scenarios\quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.grayscale.json
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\run_transport_matrix.ps1 -ScenarioPath tests\benchmarks\scenarios\quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.monochrome.json
powershell -ExecutionPolicy Bypass -File tests\benchmarks\scripts\run_transport_matrix.ps1 -ScenarioPath tests\benchmarks\scenarios\quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.low_chroma.json
```

Expected: all PASS; summaries include `zero_copy_enabled: true`, `color_mode`, FPS, encode p95, bitrate, and total bitstream bytes.

**Step 3: Summarize result**

Compare:

- FPS observed;
- encode p95;
- render present p95;
- bitrate kbps;
- total bitstream bytes.

Report whether each color mode preserves zero-copy and whether it reduces data volume.
