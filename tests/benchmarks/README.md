# Benchmark Harness

This directory contains the new mainline benchmark harness for transport comparisons.

Current status:
- `webrtc` baseline is runnable now.
- `quic_quinn` smoke scenarios can use the same artifact schema.
- all benchmark artifacts are written under `artifacts/benchmarks/`.

Quick run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.json
```

`run_transport_matrix.ps1` runs the benchmark test in Cargo release mode by
default. Add `-Debug` only when investigating harness behavior and not comparing
performance numbers.

Quick NVENC runs:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.nvenc.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.nvenc.json
```

H.264 vs HEVC WebRTC/NVDEC comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.json
```

Compare the generated `summary.json` files for observed FPS, bitrate, keyframes,
encode/send/decode/render p95 latency, render queue replacement/drop counters,
swapchain present mode, and NVDEC capability fields. These two scenarios use the
same capture, transport, renderer, resolution, FPS, and duration; only the
codec-specific encoder/decoder pair changes.

D3D11 waitable-object render pacing comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.waitable.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.2k144.waitable.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.av1_nvdec.2k144.waitable.json
```

These scenarios set `MRD_D3D11_RENDER_WAITABLE_OBJECT=1` and
`MRD_RENDER_THREAD_PRIORITY=above_normal` through the scenario file rather than
requiring shell-local environment variables. D3D11 scenarios at 120fps or above
also default to this waitable/above-normal render pacing policy in
`run_transport_matrix.ps1` unless the scenario explicitly overrides it.

4K120 waitable-object hardware comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.4k120.waitable.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.av1_nvdec.4k120.waitable.json
```

These 4K120 scenarios use 120Mbps, D3D11 shared render, NVDEC, waitable
swapchain pacing, and `transport.4k120.json` thresholds. Compare
`swap_chain_max_frame_latency`, `swap_chain_allow_tearing`,
`swap_chain_waitable_object`, `swap_chain_present_mode`, `display_refresh_hz`,
stage p95 values, and queue/drop counters before treating a 4K120 run as valid.

2K H.264 decode comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.openh264.h264_software.2k.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.openh264.ffmpeg_h264.2k.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.openh264.nvdec.2k.json
```

Use these scenarios when comparing decode latency under the same synthetic QUIC
sender and OpenH264 encoder. A local run on 2026-05-28 produced:

| Decoder | decode p95 | observed FPS | Notes |
| --- | ---: | ---: | --- |
| `h264_software` | `17.161ms` | `10.45` | Optimized I420 output; run failed only on encode p95 threshold. |
| `ffmpeg_h264` | `5.467ms` | `11.00` | Optional CLI fallback; prefer `measured_throughput_fps` in FFmpeg compare artifacts. |
| `nvdec` | `2.376ms` | `11.25` | Fastest decode path; still encode-bound by OpenH264. |

The 2K FPS in the OpenH264 comparison is encode-bound. For a hardware 2K smoke
comparison, run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.nvenc.nvdec.2k.json
```

For a 2K144 end-to-end hardware path, use
`quick.transport.webrtc.nvenc.h264_nvdec.2k144.json`. A local run on 2026-05-28
with DXGI, NVENC H.264, NVDEC, D3D11 shared textures, WebRTC, and 80Mbps observed
`136.67fps`, with encode p95 `0.314ms` and decode p95 `1.396ms`.

Paired LAN canary comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -TargetDeviceId lan-PEER-ID
```

This runs the fixed `dxgi / nvenc_h264 / quic_datagram / nvdec / d3d11_shared`
local baseline and the LAN QUIC media v2 cross-device autorun for
`1080p60`, `2K60`, `2K144`, `1600p165`, `1600p165_120mbps`,
`1080p144`, `1080p180`, and `1080p249`.
Reports are written to `target/codex-matrix-compare/`:

- `local-canary-report.json` and `.md`
- `cross-device-canary-report.json` and `.md`
- `matrix-comparison-report.json` and `.md`

Use `-SkipLocal` or `-SkipCross` when collecting only one side, and `-NoBuild`
when the app and service binaries have already been built. Local benchmark
canaries use Cargo release mode by default; add `-DebugLocalBenchmark` only for
local harness debugging.

Steady baseline:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/steady.transport.60s.json
```

Steady QUIC baseline:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/steady.transport.60s.quic.json
```

Generated files:
- `manifest.json`
- `summary.json`
- `summary.csv`
- `sessions/<session_id>.probe.json`
- `logs/*.log`
- `reports/markdown-report.md`
