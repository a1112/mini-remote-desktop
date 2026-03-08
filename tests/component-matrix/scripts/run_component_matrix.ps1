param(
  [string]$RepoRoot = "."
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path $RepoRoot).Path
$cases = @(
  "tests/component-matrix/cases/capture.dxgi.json",
  "tests/component-matrix/cases/encode.openh264.json",
  "tests/component-matrix/cases/decode.h264_software.json",
  "tests/component-matrix/cases/transport_sender.webrtc_rtp.json",
  "tests/component-matrix/cases/transport_sender.quic_quinn.json",
  "tests/component-matrix/cases/transport_receiver.h264_assemble.json",
  "tests/component-matrix/cases/transport_receiver.quic_quinn.json",
  "tests/component-matrix/cases/render.d3d11.json"
)

foreach ($casePath in $cases) {
  powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tests/component-matrix/scripts/run_component_case.ps1') `
    -CasePath $casePath `
    -RepoRoot $repo
}

Write-Output "Component matrix completed."
