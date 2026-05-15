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

Quick NVENC runs:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.nvenc.json

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.quic.nvenc.json
```

Paired LAN canary comparison:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -TargetDeviceId lan-PEER-ID
```

This runs the fixed `dxgi / nvenc_h264 / quic_datagram / nvdec / d3d11_shared`
local baseline and the LAN QUIC media v2 cross-device autorun for
`1080p60`, `2K60`, `1080p144`, `1080p180`, and `1080p249`.
Reports are written to `target/codex-matrix-compare/`:

- `local-canary-report.json` and `.md`
- `cross-device-canary-report.json` and `.md`
- `matrix-comparison-report.json` and `.md`

Use `-SkipLocal` or `-SkipCross` when collecting only one side, and `-NoBuild`
when the app and service binaries have already been built.

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
