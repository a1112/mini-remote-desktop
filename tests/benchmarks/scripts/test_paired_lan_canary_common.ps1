$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "paired_lan_canary_common.ps1")

function Assert-Equal($Actual, $Expected, [string]$Message) {
  if ($Actual -ne $Expected) {
    throw "$Message. Expected '$Expected', got '$Actual'"
  }
}

function Assert-True($Condition, [string]$Message) {
  if (-not $Condition) {
    throw $Message
  }
}

$profiles = Get-PairedLanCanaryProfiles -DurationSecs 30 -BitrateMbps 20
Assert-Equal $profiles.Count 5 "Profile count"
Assert-Equal $profiles[0].id "1080p60" "First profile id"
Assert-Equal $profiles[3].fps 180 "180 FPS profile is present"
Assert-Equal $profiles[4].fps 249 "249 FPS profile is present"

$localRow = [pscustomobject]@{
  id = "1080p144"
  width = 1920
  height = 1080
  fps = 144
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 140.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 144; bitrate_mbps = 20 }
}
$crossRow = [pscustomobject]@{
  id = "1080p144"
  width = 1920
  height = 1080
  fps = 144
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 120.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 144; bitrate_mbps = 20 }
}

$comparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($crossRow) -RatioThreshold 0.8
Assert-Equal $comparison[0].status "completed" "Cross row above 80 percent passes"
Assert-Equal ([Math]::Round($comparison[0].fps_ratio, 3)) 0.857 "FPS ratio is calculated"

$slowCrossRow = $crossRow.PSObject.Copy()
$slowCrossRow.fps_observed = 100.0
$slowComparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($slowCrossRow) -RatioThreshold 0.8
Assert-Equal $slowComparison[0].status "threshold_miss" "Cross row below 80 percent is threshold_miss"

$downgradedCrossRow = $crossRow.PSObject.Copy()
$downgradedCrossRow.selected_profile = [pscustomobject]@{ width = 1728; height = 1080; fps = 144; bitrate_mbps = 20 }
$downgradeComparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($downgradedCrossRow) -RatioThreshold 0.8
Assert-Equal $downgradeComparison[0].status "profile_downgraded" "Profile mismatch is classified as downgrade"
Assert-True (-not $downgradeComparison[0].comparable) "Profile downgraded rows are not comparable"

$peerMissingReport = [pscustomobject]@{
  status = "failed"
  failureReason = "peer_not_found"
  errorMessage = "No LAN peer available"
  probeSnapshot = $null
  mediaPipelineSnapshot = $null
  sessionSnapshot = $null
}
$peerMissingRow = Convert-CrossReportToCanaryRow -Profile $profiles[0] -Report $peerMissingReport -ReportPath "raw/cross-1080p60.json"
Assert-Equal $peerMissingRow.status "skipped" "Missing LAN peer is an environment skip"
Assert-Equal $peerMissingRow.classification "unsupported" "Missing LAN peer is classified as unsupported"

Write-Host "paired LAN canary common tests passed"
