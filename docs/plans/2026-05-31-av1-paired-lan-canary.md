# AV1 Paired LAN Canary Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add AV1 to paired LAN canary and local dual-process canary codec selection.

**Architecture:** Keep the existing service and harness AV1 implementation unchanged. Update the PowerShell canary orchestration layer so `-Codec av1` selects `nvenc_av1`/`nvdec_av1`, passes AV1 through LAN autorun, and reports AV1 chain labels consistently.

**Tech Stack:** PowerShell benchmark scripts, Rust app benchmark harness invoked by Cargo, existing NVENC AV1 and NVDEC AV1 crates.

---

### Task 1: Add AV1 Script Tests

**Files:**
- Modify: `tests/benchmarks/scripts/test_paired_lan_canary_common.ps1`

**Step 1: Write failing tests**

Add assertions for:

```powershell
Assert-Equal (Normalize-CanaryCodec "av1") "av1" "AV1 codec normalizes to av1"
Assert-Equal (New-CanaryMediaChain -Mode "local" -Codec "av1") "dxgi/nvenc_av1/quic/nvdec_av1/d3d11_shared" "AV1 local chain uses NVENC/NVDEC AV1"
Assert-Equal (New-CanaryMediaChain -Mode "cross" -Codec "av1") "dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared" "AV1 cross chain uses NVENC/NVDEC AV1"
Assert-Equal (New-CanaryMediaChain -Mode "local-dual-process" -Codec "av1") "local_dual_process/dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared" "AV1 local dual chain uses NVENC/NVDEC AV1"
```

Also assert both canary scripts contain `ValidateSet("h264", "hevc", "av1")`.

**Step 2: Verify red**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: FAIL because `Normalize-CanaryCodec "av1"` returns `h264` and scripts reject `av1`.

**Step 3: Commit tests after green**

Do not commit until implementation is green in Task 2.

### Task 2: Implement AV1 Codec Mapping

**Files:**
- Modify: `tests/benchmarks/scripts/paired_lan_canary_common.ps1`
- Modify: `tests/benchmarks/scripts/run_paired_lan_canary.ps1`
- Modify: `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1`

**Step 1: Update normalization**

Change `Normalize-CanaryCodec` to return:

- `hevc` for `hevc` and `h265`
- `av1` for `av1`
- `h264` otherwise

**Step 2: Add backend label helper**

Add a helper in `paired_lan_canary_common.ps1`:

```powershell
function Get-CanaryCodecBackends {
  param([string]$Codec = "h264")

  switch (Normalize-CanaryCodec $Codec) {
    "hevc" { return [pscustomobject]@{ encoder = "nvenc_hevc"; decoder = "nvdec_hevc_d3d11_shared"; local_decoder = "nvdec_hevc_d3d11_shared" } }
    "av1" { return [pscustomobject]@{ encoder = "nvenc_av1"; decoder = "nvdec_av1"; local_decoder = "nvdec_av1" } }
    default { return [pscustomobject]@{ encoder = "nvenc_h264"; decoder = "nvdec"; local_decoder = "nvdec" } }
  }
}
```

Use it in `New-CanaryMediaChain`.

**Step 3: Update script ValidateSet**

Allow `av1` in both:

- `tests/benchmarks/scripts/run_paired_lan_canary.ps1`
- `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1`

**Step 4: Update local release canary backend selection**

In `Invoke-LocalCanaryProfile`, replace the HEVC-only conditional with `Get-CanaryCodecBackends`. Pass:

- `-EncodeBackend $backends.encoder`
- `-DecodeBackend $backends.local_decoder`
- `MRD_BENCH_ENCODE_BACKEND=$backends.encoder`
- `MRD_BENCH_DECODE_BACKEND=$backends.local_decoder`

**Step 5: Update local dual report labels**

In `run_local_dual_process_lan_canary.ps1`, use `Get-CanaryCodecBackends` for row and report chain labels.

**Step 6: Verify green**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: PASS.

**Step 7: Commit**

```powershell
git add tests/benchmarks/scripts/paired_lan_canary_common.ps1 tests/benchmarks/scripts/run_paired_lan_canary.ps1 tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
git commit -m "feat: add AV1 paired LAN canary codec"
```

### Task 3: AV1 Capability and Benchmark Verification

**Files:**
- No source edits unless verification exposes a script bug.

**Step 1: Run focused script tests**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: PASS.

**Step 2: Run AV1 1080p60 local canary**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -OutputDir target/codex-low-latency-render-local-release-av1-1080p60 `
  -ProfileId 1080p60 `
  -DurationSecs 8 `
  -BitrateMbps 20 `
  -Codec av1 `
  -SkipCross `
  -NoBuild
```

Expected: completed if the GPU supports NVENC/NVDEC AV1; otherwise an unsupported/capability skip or clear benchmark failure reason from the harness.

**Step 3: Run AV1 2K144 local canary**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -OutputDir target/codex-low-latency-render-local-release-av1-2k144 `
  -ProfileId 2k144 `
  -DurationSecs 8 `
  -BitrateMbps 40 `
  -Codec av1 `
  -SkipCross `
  -NoBuild
```

Expected: completed if the GPU supports NVENC/NVDEC AV1; otherwise an unsupported/capability skip or clear benchmark failure reason from the harness.

**Step 4: Summarize results**

Report:

- whether AV1 is runnable on this machine
- encode/decode/render p95 if completed
- exact output report paths
- any unsupported hardware reason if skipped
