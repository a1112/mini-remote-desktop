$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "pipeline_compare_common.ps1")

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

$hevcArgs = Get-PipelineCompareCargoFeatureArgs -Codec "software-hevc"
Assert-ArrayEqual $hevcArgs @("--features", "production-software-codecs") "HEVC software compare enables production software codecs"

$main10Args = Get-PipelineCompareCargoFeatureArgs -Codec "software-hevc-main10"
Assert-ArrayEqual $main10Args @("--features", "production-software-codecs") "HEVC Main10 software compare enables production software codecs"

$av1Args = Get-PipelineCompareCargoFeatureArgs -Codec "software-av1"
Assert-ArrayEqual $av1Args @("--features", "production-software-codecs") "AV1 software compare enables production software codecs"

$vvcArgs = Get-PipelineCompareCargoFeatureArgs -Codec "software-h266"
Assert-ArrayEqual $vvcArgs @("--features", "production-vvc-software-codec") "H.266 software compare enables VVC software codec"

$h264Args = Get-PipelineCompareCargoFeatureArgs -Codec "h264"
Assert-ArrayEqual $h264Args @() "Hardware H.264 compare does not enable software codec features"
