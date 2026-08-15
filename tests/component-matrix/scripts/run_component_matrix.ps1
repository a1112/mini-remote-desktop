param(
  [string]$RepoRoot = "."
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path $RepoRoot).Path
$cases = @(
  "tests/component-matrix/cases/capture.dxgi.json",
  "tests/component-matrix/cases/encode.openh264.json",
  "tests/component-matrix/cases/encode.openh264.speed.json",
  "tests/component-matrix/cases/encode.nvenc.json",
  "tests/component-matrix/cases/encode.nvenc.ll_p1.json",
  "tests/component-matrix/cases/encode.nvenc.hq_p5.json",
  "tests/component-matrix/cases/decode.h264_software.json",
  "tests/component-matrix/cases/decode.ffmpeg_h264.json",
  "tests/component-matrix/cases/decode.nvenc_720p.json",
  "tests/component-matrix/cases/transport_sender.webrtc_rtp.json",
  "tests/component-matrix/cases/transport_sender.quic_quinn.json",
  "tests/component-matrix/cases/transport_receiver.h264_assemble.json",
  "tests/component-matrix/cases/transport_receiver.quic_quinn.json",
  "tests/component-matrix/cases/render.d3d11.json"
)

$failures = @()
foreach ($casePath in $cases) {
  powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tests/component-matrix/scripts/run_component_case.ps1') `
    -CasePath $casePath `
    -RepoRoot $repo
  if ($LASTEXITCODE -ne 0) {
    $failures += "${casePath} (exit=$LASTEXITCODE)"
  }
}

if ($failures.Count -gt 0) {
  Write-Error "Component matrix failed: $($failures -join '; ')"
  exit 2
}

Write-Output "Component matrix completed."
