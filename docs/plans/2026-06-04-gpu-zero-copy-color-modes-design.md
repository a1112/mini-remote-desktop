# GPU Zero-Copy Color Modes Design

## Goal

Implement a complete color-mode feature for the hardware remote-desktop media path without CPU readback. The first production path is the Windows DXGI shared-texture capture -> NVENC H.264/HEVC encoder path.

## Scope

The feature has two separate dimensions:

- `ColorMode`: a color transform applied before encode.
- `ColorPipeline`: the bit-depth and codec pipeline used to carry the frame.

This keeps low-color modes separate from HDR/Main10. A grayscale frame can be carried by an 8-bit SDR H.264/HEVC stream, while `HdrMain10` is a pipeline choice that controls HEVC Main10 configuration and render pixel format.

## Model

```rust
enum ColorMode {
    Full,
    Grayscale,
    Monochrome,
    LowChroma,
}

enum ColorPipeline {
    Sdr8,
    HdrMain10,
}
```

`Full` is the default and preserves current behavior. `Grayscale`, `Monochrome`, and `LowChroma` are GPU transforms. `HdrMain10` remains a pipeline capability and should not be treated as a low-color transform.

## Data Flow

1. UI/test/benchmark configuration supplies `color_mode` and optionally `color_pipeline`.
2. The Rust harness maps those fields into encoder construction.
3. The NVENC shared-BGRA path keeps the existing fast `CopyResource` for `Full`.
4. Non-full modes use a D3D11 full-screen triangle pass from the shared BGRA texture SRV into the NVENC input texture RTV.
5. NVENC still receives a GPU texture registered as `NVencBufferFormat::ARGB`.
6. Metrics and benchmark artifacts record the active color mode and pipeline.

## GPU Implementation

The first implementation should live in `crates/mrd-encode-nvenc`. It can mirror the D3D11 shader utilities already used by `crates/mrd-render-d3d11`:

- compile a small vertex shader and pixel shader with `D3DCompile`;
- create and cache one SRV per opened shared input texture;
- create one RTV per shared encode slot texture;
- draw a full-screen triangle into the registered NVENC input texture;
- unbind SRVs after draw to avoid D3D11 hazards.

Pixel behavior:

- `Grayscale`: compute luma from source RGB and output `float4(luma, luma, luma, alpha)`.
- `Monochrome`: compute luma and threshold to black or white. Dithering can be added later, but the first version should be deterministic.
- `LowChroma`: compute luma and mix source color toward luma with a fixed chroma factor.

## Reporting

The active mode must be visible in:

- Rust `TestConfig`;
- frontend `TestConfig`;
- `HarnessMetrics`;
- benchmark `BenchmarkSummary`;
- `tests/benchmarks/schemas/benchmark-result.schema.json`;
- CSV and markdown summaries from `summarize_transport_results.ps1`.

## Compatibility

The first version intentionally does not enable codec-level monochrome bitstream flags. It encodes ordinary H.264/HEVC pictures whose pixels have already been transformed on the GPU. This preserves compatibility with existing NVDEC, FFmpeg, and renderer paths.

## Validation

Completion requires:

- unit/contract tests proving config serialization and summary reporting;
- tests proving `Full` remains the default;
- a Windows NVENC/NVDEC benchmark for `Full`, `Grayscale`, `Monochrome`, and `LowChroma`;
- benchmark artifacts proving zero-copy remains enabled;
- comparison of FPS, encode p95, observed bitrate, and total bitstream bytes.

## Deferred Work

- codec-level monochrome bitstream support;
- HDR/Main10 plus non-full color transform combinations;
- adaptive mode switching based on bandwidth;
- visual quality metrics for monochrome dithering.
