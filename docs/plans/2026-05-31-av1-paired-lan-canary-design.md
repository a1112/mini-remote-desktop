# AV1 Paired LAN Canary Design

## Goal

Add AV1 as a first-class codec choice for the paired LAN canary scripts used by the low-latency render branch.

## Scope

This design only changes benchmark and canary orchestration. The repository already has AV1 primitives in the native pipeline:

- `mrd-encode-nvenc-av1` for NVENC AV1 encode.
- `mrd-decode` and `mrd-decode-nvdec` support for `nvdec_av1` and `software_av1`.
- Rdesk custom matrix and UI paths already expose NVENC AV1 in several places.

The current gap is that the paired LAN canary scripts still validate `-Codec` as only `h264` or `hevc`, and the local release canary maps any non-HEVC codec back to H.264.

## Approach

Extend the paired LAN canary codec model to normalize `av1`, `h264`, and `hevc`.

For local release canaries:

- `h264` maps to `nvenc_h264` and `nvdec`.
- `hevc` maps to `nvenc_hevc` and `nvdec_hevc_d3d11_shared`.
- `av1` maps to `nvenc_av1` and `nvdec_av1`.

For cross-device and local dual-process canaries, pass `MRD_LAN_E2E_PROFILE_CODEC=av1` through the existing autorun path. The service-side media profile selection already understands AV1, so the canary script should not invent a parallel path.

## Reporting

Reports should show AV1 explicitly in the chain:

- Local: `dxgi/nvenc_av1/quic/nvdec_av1/d3d11_shared`
- Cross: `dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared`
- Local dual-process: `local_dual_process/dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared`

The `requested_codec` and `active_codec` fields should continue to use the normalized codec value `av1`.

## Error Handling

AV1 hardware support is capability-dependent. On unsupported GPUs or drivers, NVENC AV1 and NVDEC AV1 should classify as skipped or unsupported through the existing capability gates rather than as canary infrastructure failures.

The script layer should only reject unknown codec strings. Valid AV1 runs may still be skipped by the underlying benchmark if the host lacks AV1 encode or decode support.

## Validation

Add PowerShell unit coverage for:

- `Normalize-CanaryCodec av1`
- AV1 chain labels for local, cross, and local dual-process reports
- paired and local dual-process scripts accepting `-Codec av1`

Then run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Run AV1 local canaries after the script tests pass:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -OutputDir target/codex-low-latency-render-local-release-av1-1080p60 `
  -ProfileId 1080p60 `
  -DurationSecs 8 `
  -BitrateMbps 20 `
  -Codec av1 `
  -SkipCross `
  -NoBuild

powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -OutputDir target/codex-low-latency-render-local-release-av1-2k144 `
  -ProfileId 2k144 `
  -DurationSecs 8 `
  -BitrateMbps 40 `
  -Codec av1 `
  -SkipCross `
  -NoBuild
```
