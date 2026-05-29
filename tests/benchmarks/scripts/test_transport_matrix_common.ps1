$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "transport_matrix_common.ps1")
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..\..")).Path

function Assert-ArrayEqual([object[]]$Actual, [object[]]$Expected, [string]$Message) {
  if ($Actual.Count -ne $Expected.Count) {
    throw "$Message. Expected $($Expected.Count) item(s), got $($Actual.Count)"
  }
  for ($i = 0; $i -lt $Expected.Count; $i++) {
    if ($Actual[$i] -ne $Expected[$i]) {
      throw "$Message. Item $i expected '$($Expected[$i])', got '$($Actual[$i])'"
    }
  }
}

function Assert-Throws([scriptblock]$Action, [string]$Pattern, [string]$Message) {
  try {
    & $Action
  } catch {
    if ($_.Exception.Message -match $Pattern) {
      return
    }
    throw "$Message. Threw unexpected message: $($_.Exception.Message)"
  }
  throw "$Message. Expected an exception matching '$Pattern'"
}

$hevcArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_hevc"
Assert-ArrayEqual $hevcArgs @("--features", "production-software-codecs") "HEVC software matrix enables production software codecs"

$main10Args = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_hevc_main10"
Assert-ArrayEqual $main10Args @("--features", "production-software-codecs") "HEVC Main10 software matrix enables production software codecs"

$av1Args = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_av1"
Assert-ArrayEqual $av1Args @("--features", "production-software-codecs") "AV1 software matrix enables production software codecs"

$vvcArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_vvc"
Assert-ArrayEqual $vvcArgs @("--features", "production-vvc-software-codec") "VVC software matrix enables VVC software codec"

$noneArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "nvdec"
Assert-ArrayEqual $noneArgs @() "Hardware decode matrix does not enable software codec features"

$bitrateBps = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_bps = 12000000 })
if ($bitrateBps -ne "12000000") {
  throw "bitrate_bps scenario field should pass through unchanged"
}

$bitrateMbps = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_mbps = 12 })
if ($bitrateMbps -ne "12000000") {
  throw "bitrate_mbps scenario field should convert to bps"
}

$noBitrate = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ profile = "default" })
if ($null -ne $noBitrate) {
  throw "scenario without bitrate should leave MRD_BENCH_BITRATE_BPS unset"
}

Assert-Throws { Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_bps = 0 }) } "greater than zero" "Zero bitrate_bps must be rejected"

$scenarioSpecs = @(
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.h264_software.2k.json"
    profile = "transport-quic-openh264-h264-software-2k"
    encode = "openh264"
    decode = "h264_software"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.ffmpeg_h264.2k.json"
    profile = "transport-quic-openh264-ffmpeg-h264-2k"
    encode = "openh264"
    decode = "ffmpeg_h264"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.nvdec.2k.json"
    profile = "transport-quic-openh264-nvdec-2k"
    encode = "openh264"
    decode = "nvdec"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.nvenc.nvdec.2k.json"
    profile = "transport-quic-nvenc-nvdec-2k"
    encode = "nvenc"
    decode = "nvdec"
  }
)

foreach ($spec in $scenarioSpecs) {
  $scenarioPath = Join-Path $repoRoot $spec.path
  if (-not (Test-Path $scenarioPath)) {
    throw "Expected formal 2K transport scenario at $($spec.path)"
  }
  $scenario = Get-Content $scenarioPath -Raw | ConvertFrom-Json
  if ($scenario.profile -ne $spec.profile) { throw "$($spec.path) profile mismatch" }
  if ($scenario.transport -ne "quic_quinn") { throw "$($spec.path) should use quic_quinn" }
  if ($scenario.encode_backend -ne $spec.encode) { throw "$($spec.path) encode backend mismatch" }
  if ($scenario.decode_backend -ne $spec.decode) { throw "$($spec.path) decode backend mismatch" }
  if ($scenario.width -ne 2560 -or $scenario.height -ne 1440) { throw "$($spec.path) should be 2560x1440" }
  if ($scenario.fps -ne 30) { throw "$($spec.path) should target 30fps" }
}

$processTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-process-{0}" -f ([guid]::NewGuid()))
New-Item -ItemType Directory -Force -Path $processTmp | Out-Null
try {
  $stdout = Join-Path $processTmp "stdout.log"
  $stderr = Join-Path $processTmp "stderr.log"
  $exitCode = Invoke-TransportMatrixCommand `
    -FilePath "powershell" `
    -ArgumentList @("-NoProfile", "-Command", "Write-Output 'stdout-ok'; [Console]::Error.WriteLine('stderr-ok'); exit 7") `
    -WorkingDirectory $processTmp `
    -StdoutPath $stdout `
    -StderrPath $stderr

  if ($exitCode -ne 7) { throw "Invoke-TransportMatrixCommand should return the native exit code" }
  if ((Get-Content $stdout -Raw) -notmatch "stdout-ok") { throw "Invoke-TransportMatrixCommand should capture stdout" }
  if ((Get-Content $stderr -Raw) -notmatch "stderr-ok") { throw "Invoke-TransportMatrixCommand should capture stderr" }
} finally {
  Remove-Item $processTmp -Recurse -Force -ErrorAction SilentlyContinue
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-summary-{0}.json" -f ([guid]::NewGuid()))
try {
  @{ run_passed = $false; scenario = "quick.transport"; profile = "hevc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-Throws { Assert-TransportMatrixSummaryPassed -SummaryPath $tmp } "failed thresholds" "Failed matrix summary must throw"

  @{ run_passed = $true; scenario = "quick.transport"; profile = "hevc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-TransportMatrixSummaryPassed -SummaryPath $tmp

  @{ run_passed = $false; run_skipped = $true; scenario = "quick.transport"; profile = "vvc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-TransportMatrixSummaryPassed -SummaryPath $tmp
} finally {
  Remove-Item $tmp -ErrorAction SilentlyContinue
}
