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

Steady baseline:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/steady.transport.60s.json
```

Generated files:
- `manifest.json`
- `summary.json`
- `summary.csv`
- `sessions/<session_id>.probe.json`
- `logs/*.log`
- `reports/markdown-report.md`
