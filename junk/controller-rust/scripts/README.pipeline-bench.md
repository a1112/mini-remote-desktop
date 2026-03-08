# Rust Pipeline Benchmark Scripts

All scripts run real transport path (`signaling-rs + agent-rust + controller-rust`) with recording disabled.

## Single Case

```powershell
powershell -ExecutionPolicy Bypass -File .\controller-rust\scripts\run_rust_pipeline_quic_60s.ps1 -DurationSec 60
```

## 2K vs 1080p Compare

```powershell
powershell -ExecutionPolicy Bypass -File .\controller-rust\scripts\run_rust_compare_2k_1080p.ps1
```

## 1080p Matrix (120/144)

```powershell
powershell -ExecutionPolicy Bypass -File .\controller-rust\scripts\run_rust_matrix_1080p_120_144.ps1 -DurationSec 40
```

## 1080p Matrix (30/60)

```powershell
powershell -ExecutionPolicy Bypass -File .\controller-rust\scripts\run_rust_matrix_1080p_30_60.ps1 -DurationSec 40
```

Each script prints both table and JSON rows, including `controller_log` and `agent_log` for trace-back.
