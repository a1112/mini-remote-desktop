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
Assert-Equal $profiles.Count 9 "Profile count"
Assert-Equal $profiles[0].id "1080p60" "First profile id"
Assert-Equal $profiles[2].id "2k144" "2K144 profile is present"
Assert-Equal $profiles[3].id "2k144_adaptive" "Adaptive 2K144 profile is present"
Assert-Equal $profiles[3].bitrate_mbps 80 "Adaptive 2K144 profile uses the 80 Mbps ceiling"
Assert-True $profiles[3].adaptive "Adaptive 2K144 profile enables adaptive autorun"
Assert-Equal $profiles[4].id "1600p165" "Native 1600p165 profile is present"
Assert-Equal $profiles[4].bitrate_mbps 80 "Native 1600p165 profile uses the higher default bitrate"
Assert-Equal $profiles[5].id "1600p165_120mbps" "Native 1600p165 high-bitrate profile is present"
Assert-Equal $profiles[5].bitrate_mbps 120 "Native 1600p165 high-bitrate profile reaches 120 Mbps"
Assert-Equal $profiles[7].fps 180 "180 FPS profile is present"
Assert-Equal $profiles[8].fps 249 "249 FPS profile is present"

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

$local60Row = [pscustomobject]@{
  id = "1080p60"
  width = 1920
  height = 1080
  fps = 60
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 144.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 60; bitrate_mbps = 20 }
}
$cross60Row = [pscustomobject]@{
  id = "1080p60"
  width = 1920
  height = 1080
  fps = 60
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 56.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 60; bitrate_mbps = 20 }
}
$cappedComparison = Compare-PairedLanCanaryRows -LocalRows @($local60Row) -CrossRows @($cross60Row) -RatioThreshold 0.8
Assert-Equal $cappedComparison[0].status "completed" "Comparison caps local baseline to requested FPS"
Assert-Equal ([Math]::Round($cappedComparison[0].fps_ratio, 3)) 0.933 "Capped FPS ratio uses requested FPS"

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

$displayLimitedReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 144.0
  displayModeChange = [pscustomobject]@{
    active = [pscustomobject]@{ width = 1920; height = 1080; refresh_hz = 144 }
  }
  probeSnapshot = [pscustomobject]@{
    current_fps = 144.0
    frames_decoded = 1440
    frames_dropped = 0
    media_probe_width = 1920
    media_probe_height = 1080
    media_probe_target_fps = 249
    media_probe_target_bitrate_mbps = 20
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @()
    test_impairment = $null
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$displayLimitedRow = Convert-CrossReportToCanaryRow -Profile $profiles[8] -Report $displayLimitedReport -ReportPath "raw/cross-1080p249.json"
Assert-Equal $displayLimitedRow.status "skipped" "Display refresh capped profiles are environment skips"
Assert-Equal $displayLimitedRow.classification "display_refresh_limited" "Display refresh cap is classified explicitly"
Assert-True ($displayLimitedRow.error_message -match "144Hz") "Display refresh cap carries an actionable reason"

$displayLimitedComparison = Compare-PairedLanCanaryRows -LocalRows @($displayLimitedRow) -CrossRows @($crossRow) -RatioThreshold 0.8
Assert-Equal $displayLimitedComparison[0].status "display_refresh_limited" "Display-limited local rows are not compared"
Assert-True (-not $displayLimitedComparison[0].comparable) "Display-limited local rows are non-comparable"

$sampleFpsReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 57.0
  probeSnapshot = [pscustomobject]@{
    current_fps = 44.0
    frames_decoded = 484
    frames_dropped = 0
    media_probe_width = 1920
    media_probe_height = 1080
    media_probe_target_fps = 60
    media_probe_target_bitrate_mbps = 20
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @()
    test_impairment = [pscustomobject]@{
      loss_pct = 1.0
      base_delay_ms = 2
      jitter_ms = 3
      mtu_bytes = 1200
      seed = 42
      datagrams_sent = 100
      datagrams_dropped = 1
      datagrams_delayed = 80
      datagrams_fragmented_by_mtu = 10
    }
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$sampleFpsRow = Convert-CrossReportToCanaryRow -Profile $profiles[0] -Report $sampleFpsReport -ReportPath "raw/cross-1080p60.json"
Assert-Equal $sampleFpsRow.fps_observed 57.0 "Cross report prefers sample-window FPS over cumulative probe FPS"
Assert-Equal $sampleFpsRow.test_impairment.datagrams_dropped 1 "Cross row carries media impairment counters"

$renderDropReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 136.0
  probeSnapshot = [pscustomobject]@{
    current_fps = 136.0
    frames_decoded = 8282
    frames_dropped = 37
    media_probe_width = 2560
    media_probe_height = 1440
    media_probe_target_fps = 144
    media_probe_target_bitrate_mbps = 80
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 457
    render_queue_replacements = 455
    render_lock_drops = 2
    queue_depth = 0
    stage_metrics = @()
    test_impairment = $null
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$renderDropRow = Convert-CrossReportToCanaryRow -Profile $profiles[2] -Report $renderDropReport -ReportPath "raw/cross-2k144.json"
Assert-Equal $renderDropRow.dropped_frames 37 "Cross row dropped_frames tracks probe/transport drops"
Assert-Equal $renderDropRow.probe_dropped_frames 37 "Cross row exposes probe drops separately"
Assert-Equal $renderDropRow.pipeline_dropped_frames 457 "Cross row preserves legacy pipeline dropped frames"
Assert-Equal $renderDropRow.render_queue_replacements 455 "Cross row exposes render queue replacements"
Assert-Equal $renderDropRow.render_lock_drops 2 "Cross row exposes render lock drops"

Write-Host "paired LAN canary common tests passed"
