$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "transport_matrix_common.ps1")

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
} finally {
  Remove-Item $tmp -ErrorAction SilentlyContinue
}
