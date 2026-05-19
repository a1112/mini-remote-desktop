$ErrorActionPreference = "Stop"

$script:CanaryMaxProbeDropRatio = 0.05
$script:CanaryMaxRenderDropRatio = 0.03
$script:CanaryMinPacedRenderFpsRatio = 0.88
$script:CanaryMaxPacedPresentGapMultiplier = 1.5

function Select-CanaryValue {
  param($Value, $Fallback)
  if ($null -eq $Value) { return $Fallback }
  $Value
}

function Select-CanaryObjectPropertyValue {
  param($Object, [string]$Name, $Fallback)
  if ($null -eq $Object) { return $Fallback }
  $property = $Object.PSObject.Properties[$Name]
  if ($null -eq $property) { return $Fallback }
  Select-CanaryValue $property.Value $Fallback
}

function Select-CanarySenderSendStageValue {
  param($StageMap, $Fallback)
  $reliable = Select-CanaryObjectPropertyValue $StageMap "sender.send_reliable" $null
  if ($null -ne $reliable) { return $reliable }
  Select-CanaryObjectPropertyValue $StageMap "sender.send_datagram" $Fallback
}

function Select-CanaryStageP95Value {
  param($Pipeline, [string]$Stage, $Fallback)
  if ($null -eq $Pipeline -or $null -eq $Pipeline.stage_metrics) { return $Fallback }
  foreach ($metric in @($Pipeline.stage_metrics)) {
    if ($metric.stage -eq $Stage) {
      return (Select-CanaryValue $metric.p95_ms $Fallback)
    }
  }
  $Fallback
}

function Normalize-CanaryCodec {
  param([string]$Codec)

  $normalized = (Select-CanaryValue $Codec "h264").Trim().ToLowerInvariant()
  if ($normalized -in @("hevc", "h265")) {
    return "hevc"
  }
  "h264"
}

function Resolve-PairedLanCanaryTargetDeviceId {
  param(
    $Diagnostics,
    [string]$RequestedTargetDeviceId = ""
  )

  if ($RequestedTargetDeviceId.Trim()) {
    return $RequestedTargetDeviceId.Trim()
  }

  $deviceIds = @()
  foreach ($response in @($Diagnostics.udp_responses)) {
    if (-not $response.payload) { continue }
    try {
      $payload = $response.payload | ConvertFrom-Json
    } catch {
      continue
    }
    if ($payload.type -ne "announce" -or -not $payload.device_id) {
      continue
    }
    $deviceIds += [string]$payload.device_id
  }

  $uniqueDeviceIds = @($deviceIds | Where-Object { $_ } | Select-Object -Unique)
  if ($uniqueDeviceIds.Count -eq 1) {
    return $uniqueDeviceIds[0]
  }
  ""
}

function New-CanaryMediaChain {
  param(
    [ValidateSet("local", "cross", "local-dual-process")]
    [string]$Mode = "cross",
    [string]$Codec = "h264"
  )

  $normalized = Normalize-CanaryCodec $Codec
  if ($normalized -eq "hevc") {
    $encoder = "nvenc_hevc"
    $decoder = "nvdec_hevc_d3d11_shared"
  } else {
    $encoder = "nvenc_h264"
    $decoder = "nvdec"
  }

  switch ($Mode) {
    "local-dual-process" { return "local_dual_process/dxgi/$encoder/quic_datagram_media_v3_or_v2/$decoder/d3d11_shared" }
    "cross" { return "dxgi/$encoder/quic_datagram_media_v3_or_v2/$decoder/d3d11_shared" }
    default { return "dxgi/$encoder/quic/$decoder/d3d11_shared" }
  }
}

function Get-PairedLanCanaryProfiles {
  param(
    [int]$DurationSecs = 30,
    [int]$BitrateMbps = 20
  )

  @(
    [pscustomobject]@{ id = "1080p60"; width = 1920; height = 1080; fps = 60; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k60"; width = 2560; height = 1440; fps = 60; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k144"; width = 2560; height = 1440; fps = 144; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k144_adaptive"; width = 2560; height = 1440; fps = 144; bitrate_mbps = 80; duration_secs = $DurationSecs; adaptive = $true },
    [pscustomobject]@{ id = "1600p165"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 80; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1600p165_120mbps"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 120; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1080p144"; width = 1920; height = 1080; fps = 144; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1080p180"; width = 1920; height = 1080; fps = 180; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1080p249"; width = 1920; height = 1080; fps = 249; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs }
  )
}

function New-CanarySelectedProfile {
  param(
    [int]$Width,
    [int]$Height,
    [int]$Fps,
    [int]$BitrateMbps
  )

  [pscustomobject]@{
    width = $Width
    height = $Height
    fps = $Fps
    bitrate_mbps = $BitrateMbps
  }
}

function Get-CanaryVisualIntegrityIssue {
  param(
    $Probe,
    $Pipeline,
    $Report = $null,
    $Profile = $null
  )

  $decodedFrames = [double](Select-CanaryValue $Probe.frames_decoded 0)
  $probeDrops = [double](Select-CanaryValue $Probe.frames_dropped 0)
  $totalSequencedFrames = $decodedFrames + $probeDrops
  if ($totalSequencedFrames -gt 0) {
    $probeDropRatio = $probeDrops / $totalSequencedFrames
    if ($probeDropRatio -gt $script:CanaryMaxProbeDropRatio) {
      return "Visual integrity risk: drop ratio $([Math]::Round($probeDropRatio * 100, 2))% exceeds $([Math]::Round($script:CanaryMaxProbeDropRatio * 100, 2))% ($([int64]$probeDrops) dropped / $([int64]$totalSequencedFrames) sequenced frames)."
    }
  }

  $renderQueueReplacements = [double](Select-CanaryValue $Pipeline.render_queue_replacements 0)
  $renderLockDrops = [double](Select-CanaryValue $Pipeline.render_lock_drops 0)
  $renderDrops = $renderQueueReplacements + $renderLockDrops
  if ($decodedFrames -gt 0 -and $renderDrops -gt 0) {
    $renderDropRatio = $renderDrops / $decodedFrames
    if ($renderDropRatio -gt $script:CanaryMaxRenderDropRatio) {
      if (Test-CanaryPacedRenderCoalescingAcceptable -Probe $Probe -Pipeline $Pipeline -Report $Report -Profile $Profile) {
        return $null
      }
      return "Visual integrity risk: render drop/coalesce ratio $([Math]::Round($renderDropRatio * 100, 2))% exceeds $([Math]::Round($script:CanaryMaxRenderDropRatio * 100, 2))% ($([int64]$renderDrops) render drops / $([int64]$decodedFrames) decoded frames)."
    }
  }

  $null
}

function Get-CanaryEstimatedRenderFps {
  param(
    $Probe,
    $Pipeline,
    $Report = $null
  )

  $decodedFrames = [double](Select-CanaryValue $Probe.frames_decoded 0)
  if ($decodedFrames -le 0) { return 0.0 }

  $renderQueueReplacements = [double](Select-CanaryValue $Pipeline.render_queue_replacements 0)
  $renderLockDrops = [double](Select-CanaryValue $Pipeline.render_lock_drops 0)
  $presentedFrames = [Math]::Max(0.0, $decodedFrames - $renderQueueReplacements - $renderLockDrops)
  $sampleDurationMs = [double](Select-CanaryValue $Report.sampleDurationMs 0)
  if ($sampleDurationMs -gt 0) {
    return $presentedFrames / ($sampleDurationMs / 1000.0)
  }

  $observedFps = [double](Select-CanaryValue (Select-CanaryValue $Report.sampleObservedFps $Probe.current_fps) 0)
  if ($observedFps -gt 0) {
    return $observedFps * ($presentedFrames / $decodedFrames)
  }
  0.0
}

function Get-CanaryRenderCoalesceRatio {
  param(
    $Probe,
    $Pipeline
  )

  $decodedFrames = [double](Select-CanaryValue $Probe.frames_decoded 0)
  if ($decodedFrames -le 0) { return 0.0 }
  $renderQueueReplacements = [double](Select-CanaryValue $Pipeline.render_queue_replacements 0)
  $renderLockDrops = [double](Select-CanaryValue $Pipeline.render_lock_drops 0)
  ($renderQueueReplacements + $renderLockDrops) / $decodedFrames
}

function Test-CanaryPacedRenderCoalescingAcceptable {
  param(
    $Probe,
    $Pipeline,
    $Report = $null,
    $Profile = $null
  )

  if ($null -eq $Pipeline -or $null -eq $Probe) { return $false }
  $renderLockDrops = [double](Select-CanaryValue $Pipeline.render_lock_drops 0)
  if ($renderLockDrops -gt 0) { return $false }

  $targetFps = [double](Select-CanaryValue $Pipeline.render_pacing_target_fps (Select-CanaryValue $Probe.media_probe_target_fps (Select-CanaryValue $Profile.fps 0)))
  if ($targetFps -le 0) { return $false }
  $presentGapP95 = [double](Select-CanaryStageP95Value -Pipeline $Pipeline -Stage "render_present_gap" -Fallback 0)
  if ($presentGapP95 -le 0) { return $false }

  $frameBudgetMs = 1000.0 / $targetFps
  $estimatedRenderFps = [double](Get-CanaryEstimatedRenderFps -Probe $Probe -Pipeline $Pipeline -Report $Report)
  ($estimatedRenderFps -ge ($targetFps * $script:CanaryMinPacedRenderFpsRatio)) -and
    ($presentGapP95 -le ($frameBudgetMs * $script:CanaryMaxPacedPresentGapMultiplier))
}

function Convert-LocalSummaryToCanaryRow {
  param(
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$Summary,
    [string]$SummaryPath
  )

  $status = if ($Summary.run_passed) { "completed" } else { "failed" }
  $classification = if ($Summary.run_passed) { "completed" } elseif ($Summary.fps_observed -lt ($Profile.fps * 0.8)) { "threshold_miss" } else { "failed" }
  $localDropped = [int64](Select-CanaryValue $Summary.dropped_frames 0)

  [pscustomobject]@{
    id = $Profile.id
    width = [int]$Profile.width
    height = [int]$Profile.height
    fps = [int]$Profile.fps
    bitrate_mbps = [int]$Profile.bitrate_mbps
    duration_secs = [int]$Profile.duration_secs
    chain = New-CanaryMediaChain -Mode "local" -Codec "h264"
    status = $status
    classification = $classification
    fps_observed = [double](Select-CanaryValue $Summary.fps_observed 0)
    selected_profile = New-CanarySelectedProfile -Width $Summary.width -Height $Summary.height -Fps $Summary.fps_target -BitrateMbps $Profile.bitrate_mbps
    session_established = [bool]$Summary.session_established
    first_frame_seen = [bool]$Summary.first_frame_seen
    first_frame_time_ms = $Summary.first_frame_time_ms
    decoded_frames = $null
    dropped_frames = $localDropped
    probe_dropped_frames = $localDropped
    pipeline_dropped_frames = $localDropped
    render_queue_replacements = 0
    render_lock_drops = 0
    queue_depth = $null
    stage_p95_ms = [pscustomobject]@{
      encode = $Summary.encode_total_p95_ms
      transport = $Summary.send_write_p95_ms
      decode = $Summary.decode_total_p95_ms
      render_upload = $Summary.render_upload_p95_ms
      present = $Summary.render_present_p95_ms
    }
    raw_summary_path = $SummaryPath
    error_message = $null
  }
}

function Convert-CrossReportToCanaryRow {
  param(
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$Report,
    [string]$ReportPath,
    [string]$RequestedCodec = "h264"
  )

  $probe = $Report.probeSnapshot
  $pipeline = $Report.mediaPipelineSnapshot
  $adaptation = if ($Report.mediaAdaptationSnapshot) { $Report.mediaAdaptationSnapshot } elseif ($pipeline) { $pipeline.adaptation } else { $null }
  $selected = if ($probe -and $probe.media_probe_width -and $probe.media_probe_height -and $probe.media_probe_target_fps) {
    New-CanarySelectedProfile -Width $probe.media_probe_width -Height $probe.media_probe_height -Fps $probe.media_probe_target_fps -BitrateMbps (Select-CanaryValue $probe.media_probe_target_bitrate_mbps $Profile.bitrate_mbps)
  } else {
    New-CanarySelectedProfile -Width $Profile.width -Height $Profile.height -Fps $Profile.fps -BitrateMbps $Profile.bitrate_mbps
  }
  $baseClassification = Get-CrossCanaryClassification -Report $Report -Profile $Profile -SelectedProfile $selected
  $renderCoalesceRatio = [double](Get-CanaryRenderCoalesceRatio -Probe $probe -Pipeline $pipeline)
  $pacedRenderCoalescing = ($renderCoalesceRatio -gt $script:CanaryMaxRenderDropRatio) -and
    (Test-CanaryPacedRenderCoalescingAcceptable -Probe $probe -Pipeline $pipeline -Report $Report -Profile $Profile)
  $visualIntegrityIssue = if ($baseClassification -eq "completed") {
    Get-CanaryVisualIntegrityIssue -Probe $probe -Pipeline $pipeline -Report $Report -Profile $Profile
  } else {
    $null
  }
  $classification = if ($visualIntegrityIssue) { "visual_integrity_risk" } else { $baseClassification }
  $status = Get-CrossCanaryStatus -Report $Report -Classification $classification
  $displayLimitReason = Get-CanaryDisplayRefreshLimitReason -Report $Report -Profile $Profile
  $probeDropped = [int64](Select-CanaryValue $probe.frames_dropped 0)
  $pipelineDropped = [int64](Select-CanaryValue $pipeline.dropped_frames 0)
  $renderQueueReplacements = [int64](Select-CanaryValue $pipeline.render_queue_replacements $pipelineDropped)
  $renderLockDrops = [int64](Select-CanaryValue $pipeline.render_lock_drops 0)
  $activeCodec = Select-CanaryValue $pipeline.active_codec $RequestedCodec

  [pscustomobject]@{
    id = $Profile.id
    width = [int]$Profile.width
    height = [int]$Profile.height
    fps = [int]$Profile.fps
    bitrate_mbps = [int]$Profile.bitrate_mbps
    duration_secs = [int]$Profile.duration_secs
    chain = New-CanaryMediaChain -Mode "cross" -Codec $activeCodec
    status = $status
    classification = $classification
    fps_observed = [double](Select-CanaryValue (Select-CanaryValue $Report.sampleObservedFps $probe.current_fps) 0)
    selected_profile = $selected
    session_established = [bool]($Report.sessionSnapshot -and $Report.sessionSnapshot.state -ne "failed")
    first_frame_seen = [bool]($probe -and $probe.frames_decoded -gt 0)
    first_frame_time_ms = $null
    decoded_frames = [int64](Select-CanaryValue $probe.frames_decoded 0)
    dropped_frames = $probeDropped
    probe_dropped_frames = $probeDropped
    pipeline_dropped_frames = $pipelineDropped
    render_queue_replacements = $renderQueueReplacements
    render_lock_drops = $renderLockDrops
    estimated_render_fps = [double](Get-CanaryEstimatedRenderFps -Probe $probe -Pipeline $pipeline -Report $Report)
    render_coalesce_ratio = $renderCoalesceRatio
    render_present_gap_p95_ms = [double](Select-CanaryStageP95Value -Pipeline $pipeline -Stage "render_present_gap" -Fallback 0)
    render_pacing_target_fps = $pipeline.render_pacing_target_fps
    queue_depth = $pipeline.queue_depth
    stage_p50_ms = Convert-MediaStageMetricsToP50Map -StageMetrics $pipeline.stage_metrics
    stage_p95_ms = Convert-MediaStageMetricsToP95Map -StageMetrics $pipeline.stage_metrics
    test_impairment = $pipeline.test_impairment
    adaptive = [bool](Select-CanaryValue $Profile.adaptive $false)
    adaptation = $adaptation
    display_mode = $Report.displayModeChange
    raw_report_path = $ReportPath
    error_message = Select-CanaryValue $Report.errorMessage (Select-CanaryValue $visualIntegrityIssue $displayLimitReason)
    visual_integrity_status = if ($visualIntegrityIssue) { "risk" } elseif ($pacedRenderCoalescing) { "paced" } else { "ok" }
    visual_integrity_message = $visualIntegrityIssue
    requested_codec = Normalize-CanaryCodec $RequestedCodec
    active_codec = $pipeline.active_codec
    active_codec_profile = $pipeline.active_codec_profile
    active_bit_depth = $pipeline.active_bit_depth
    active_chroma_subsampling = $pipeline.active_chroma_subsampling
    active_pixel_format = $pipeline.active_pixel_format
    active_hdr_enabled = $pipeline.active_hdr_enabled
    active_width = $pipeline.active_width
    active_height = $pipeline.active_height
    active_fps = $pipeline.active_fps
    active_bitrate_mbps = $pipeline.active_bitrate_mbps
  }
}

function Get-CrossCanaryStatus {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)][string]$Classification
  )

  if ($Classification -in @("unsupported", "profile_downgraded", "peer_version_mismatch", "display_refresh_limited")) {
    return "skipped"
  }
  if ($Classification -eq "visual_integrity_risk") { return "failed" }
  if ($Report.status -eq "completed") { return "completed" }
  if ($Report.status -eq "skipped") { return "skipped" }
  return [string]$Report.status
}

function Get-CrossCanaryClassification {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$SelectedProfile
  )

  if (Get-CanaryDisplayRefreshLimitReason -Report $Report -Profile $Profile) {
    return "display_refresh_limited"
  }
  if (-not (Test-CanaryProfileMatch -Expected $Profile -Actual $SelectedProfile)) {
    if (
      $Report.status -eq "completed" -and
      (Test-CanaryRenderPacedProfileCap -Profile $Profile -SelectedProfile $SelectedProfile -Pipeline $Report.mediaPipelineSnapshot)
    ) {
      return "completed"
    }
    return "profile_downgraded"
  }
  if ($Report.status -eq "completed") {
    return "completed"
  }
  if ($Report.status -eq "skipped") {
    if ($Report.failureReason) { return [string]$Report.failureReason }
    return "skipped"
  }

  switch ([string]$Report.failureReason) {
    "peer_not_ready" { return "unsupported" }
    "peer_not_found" { return "unsupported" }
    "media_profile_mismatch" { return "profile_downgraded" }
    "profile_downgraded" { return "profile_downgraded" }
    "display_mode_failed" { return "display_mode_failed" }
    "no_remote_frames" { return "threshold_miss" }
    "session_start_failed" { return "transport_loss" }
    "runtime_error" {
      if ((Select-CanaryValue $Report.errorMessage "") -match "(?i)decode|h\.264|nvdec") { return "decode_error" }
      if ((Select-CanaryValue $Report.errorMessage "") -match "(?i)transport|quic|timeout") { return "transport_loss" }
      return "runtime_error"
    }
    default {
      if ($Report.failureReason) { return [string]$Report.failureReason }
      return "failed"
    }
  }
}

function Get-CanaryDisplayRefreshLimitReason {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)]$Profile
  )

  $active = $Report.displayModeChange.active
  if (-not $active -or -not $active.refresh_hz) {
    return $null
  }

  $activeRefreshHz = [int]$active.refresh_hz
  $requestedFps = [int]$Profile.fps
  if ($requestedFps -le 0 -or $activeRefreshHz -le 0 -or $activeRefreshHz -ge $requestedFps) {
    return $null
  }

  $activeWidth = [int](Select-CanaryValue $active.width 0)
  $activeHeight = [int](Select-CanaryValue $active.height 0)
  $requestedWidth = [int](Select-CanaryValue $Profile.width 0)
  $requestedHeight = [int](Select-CanaryValue $Profile.height 0)
  if ($activeWidth -gt 0 -and $activeHeight -gt 0 -and ($activeWidth -ne $requestedWidth -or $activeHeight -ne $requestedHeight)) {
    return $null
  }

  "Active display mode $($activeWidth)x$($activeHeight)@$($activeRefreshHz)Hz is below requested $($requestedFps) FPS"
}

function Convert-MediaStageMetricsToP95Map {
  param($StageMetrics)

  $map = [ordered]@{}
  if ($StageMetrics) {
    foreach ($metric in $StageMetrics) {
      $map[$metric.stage] = $metric.p95_ms
    }
  }
  [pscustomobject]$map
}

function Convert-MediaStageMetricsToP50Map {
  param($StageMetrics)

  $map = [ordered]@{}
  if ($StageMetrics) {
    foreach ($metric in $StageMetrics) {
      $map[$metric.stage] = $metric.p50_ms
    }
  }
  [pscustomobject]$map
}

function Test-CanaryProfileMatch {
  param(
    [Parameter(Mandatory = $true)]$Expected,
    [Parameter(Mandatory = $true)]$Actual
  )

  ([int]$Expected.width -eq [int]$Actual.width) -and
  ([int]$Expected.height -eq [int]$Actual.height) -and
  ([int]$Expected.fps -eq [int]$Actual.fps) -and
  ([int]$Expected.bitrate_mbps -eq [int]$Actual.bitrate_mbps)
}

function Test-CanaryRenderPacedProfileCap {
  param(
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$SelectedProfile,
    $Pipeline = $null
  )

  $targetFps = [int](Select-CanaryValue $Pipeline.render_pacing_target_fps 0)
  if ($targetFps -le 0) { return $false }

  ([int]$SelectedProfile.width -eq [int]$Profile.width) -and
  ([int]$SelectedProfile.height -eq [int]$Profile.height) -and
  ([int]$SelectedProfile.bitrate_mbps -eq [int]$Profile.bitrate_mbps) -and
  ([int]$SelectedProfile.fps -eq $targetFps) -and
  ($targetFps -lt [int]$Profile.fps)
}

function Test-CanaryCrossRowRenderPacedProfileCap {
  param(
    [Parameter(Mandatory = $true)]$LocalRow,
    [Parameter(Mandatory = $true)]$CrossRow
  )

  if (-not $CrossRow.selected_profile) { return $false }
  Test-CanaryRenderPacedProfileCap `
    -Profile $LocalRow `
    -SelectedProfile $CrossRow.selected_profile `
    -Pipeline ([pscustomobject]@{ render_pacing_target_fps = $CrossRow.render_pacing_target_fps })
}

function Get-CanaryComparisonBaselineFps {
  param(
    [Parameter(Mandatory = $true)]$LocalRow
  )

  $observed = [double](Select-CanaryValue $LocalRow.fps_observed 0)
  $requested = [double](Select-CanaryValue $LocalRow.fps 0)
  $selected = if ($LocalRow.selected_profile) {
    [double](Select-CanaryValue $LocalRow.selected_profile.fps $requested)
  } else {
    $requested
  }
  $cap = if ($requested -gt 0 -and $selected -gt 0) {
    [Math]::Min($requested, $selected)
  } elseif ($selected -gt 0) {
    $selected
  } else {
    $requested
  }
  if ($cap -gt 0) {
    return [Math]::Min($observed, $cap)
  }
  $observed
}

function Compare-PairedLanCanaryRows {
  param(
    [Parameter(Mandatory = $true)]$LocalRows,
    [Parameter(Mandatory = $true)]$CrossRows,
    [double]$RatioThreshold = 0.8
  )

  $results = @()
  foreach ($local in $LocalRows) {
    $cross = @($CrossRows | Where-Object { $_.id -eq $local.id } | Select-Object -First 1)[0]
    $localFps = [double](Select-CanaryValue $local.fps_observed 0)
    $localBaselineFps = [double](Get-CanaryComparisonBaselineFps -LocalRow $local)
    if ($local.classification -eq "display_refresh_limited") {
      $results += [pscustomobject]@{
        id = $local.id
        comparable = $false
        status = "display_refresh_limited"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = 0.0
        fps_ratio = $null
        reason = $local.error_message
      }
      continue
    }
    if (-not $cross) {
      $results += [pscustomobject]@{
        id = $local.id
        comparable = $false
        status = "missing_cross"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = 0.0
        fps_ratio = $null
        reason = "Cross-device result is missing"
      }
      continue
    }

    if ($cross.classification -eq "display_refresh_limited") {
      $results += [pscustomobject]@{
        id = $local.id
        comparable = $false
        status = "display_refresh_limited"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = [double](Select-CanaryValue $cross.fps_observed 0)
        fps_ratio = $null
        reason = $cross.error_message
      }
      continue
    }

    $renderPacedProfileCap = Test-CanaryCrossRowRenderPacedProfileCap -LocalRow $local -CrossRow $cross
    $profilesMatch =
      (Test-CanaryProfileMatch -Expected $local.selected_profile -Actual $cross.selected_profile) -and
      (Test-CanaryProfileMatch -Expected $local -Actual $cross.selected_profile)
    $profilesMatch = $profilesMatch -or $renderPacedProfileCap
    if (-not $profilesMatch) {
      $results += [pscustomobject]@{
        id = $local.id
        comparable = $false
        status = "profile_downgraded"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = [double](Select-CanaryValue $cross.fps_observed 0)
        fps_ratio = $null
        reason = "Selected local/cross profiles differ"
      }
      continue
    }

    $crossFps = [double](Select-CanaryValue $cross.fps_observed 0)
    $comparisonBaselineFps = if ($renderPacedProfileCap) {
      [Math]::Min($localBaselineFps, [double](Select-CanaryValue $cross.selected_profile.fps $localBaselineFps))
    } else {
      $localBaselineFps
    }
    $ratio = if ($comparisonBaselineFps -gt 0) { $crossFps / $comparisonBaselineFps } else { 0.0 }
    $status = if ($local.status -ne "completed") {
      "local_failed"
    } elseif ($cross.status -ne "completed") {
      $cross.classification
    } elseif ($ratio -ge $RatioThreshold) {
      "completed"
    } else {
      "threshold_miss"
    }

    $results += [pscustomobject]@{
      id = $local.id
      comparable = ($status -ne "profile_downgraded")
      status = $status
      local_fps = $localFps
      local_baseline_fps = $comparisonBaselineFps
      cross_fps = $crossFps
      fps_ratio = $ratio
      reason = if ($status -eq "threshold_miss") { "Cross FPS below $([Math]::Round($RatioThreshold * 100)) percent of local baseline" } else { $cross.error_message }
      local_decode_p95_ms = $local.stage_p95_ms.decode
      cross_decode_p95_ms = $cross.stage_p95_ms.decode
      local_render_p95_ms = $local.stage_p95_ms.present
      cross_render_p95_ms = $cross.stage_p95_ms.render_present
    }
  }
  $results
}

function New-PairedLanCanaryReport {
  param(
    [Parameter(Mandatory = $true)][string]$Mode,
    [Parameter(Mandatory = $true)]$Rows,
    [string]$GitCommit,
    [string]$GeneratedAt = (Get-Date).ToString("o"),
    [string]$Codec = "h264"
  )

  [pscustomobject]@{
    schema_version = 1
    mode = $Mode
    generated_at = $GeneratedAt
    git_commit = $GitCommit
    chain = New-CanaryMediaChain -Mode $Mode -Codec $Codec
    codec = Normalize-CanaryCodec $Codec
    completed = @($Rows | Where-Object { $_.status -eq "completed" }).Count
    skipped = @($Rows | Where-Object { $_.status -eq "skipped" }).Count
    failed = @($Rows | Where-Object { $_.status -ne "completed" -and $_.status -ne "skipped" }).Count
    rows = @($Rows)
  }
}

function Write-CanaryJsonAndMarkdown {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)][string]$JsonPath,
    [Parameter(Mandatory = $true)][string]$MarkdownPath,
    [string]$Title = "Paired LAN Canary Report"
  )

  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $JsonPath), (Split-Path -Parent $MarkdownPath) | Out-Null
  $Report | ConvertTo-Json -Depth 16 | Set-Content -Path $JsonPath -Encoding Ascii

  $lines = @(
    "# $Title",
    "",
    "- Mode: $($Report.mode)",
    "- Commit: $($Report.git_commit)",
    "- Chain: $($Report.chain)",
    "- Completed: $($Report.completed)",
    "- Skipped: $($Report.skipped)",
    "- Failed: $($Report.failed)",
    "",
    "| Profile | Status | Class | FPS | Render FPS | Render Target | Selected | Visual | Enc P50/P95 | Send P50/P95 | Present Gap P95 | Probe Drop | Render Coalesce | Render Drop | Queue | Error |",
    "| --- | --- | --- | ---: | ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"
  )
  foreach ($row in $Report.rows) {
    $selected = "$($row.selected_profile.width)x$($row.selected_profile.height)@$($row.selected_profile.fps)/$($row.selected_profile.bitrate_mbps)Mbps"
    $error = ((Select-CanaryValue $row.error_message "") -replace "\|", "/")
    $visual = Select-CanaryValue $row.visual_integrity_status "n/a"
    $encodeP50 = [Math]::Round([double](Select-CanaryValue $row.stage_p50_ms.'sender.encode' 0), 2)
    $encodeP95 = [Math]::Round([double](Select-CanaryValue $row.stage_p95_ms.'sender.encode' 0), 2)
    $sendP50 = [Math]::Round([double](Select-CanarySenderSendStageValue -StageMap $row.stage_p50_ms -Fallback 0), 2)
    $sendP95 = [Math]::Round([double](Select-CanarySenderSendStageValue -StageMap $row.stage_p95_ms -Fallback 0), 2)
    $estimatedRenderFps = [Math]::Round([double](Select-CanaryValue $row.estimated_render_fps 0), 2)
    $renderTargetFps = Select-CanaryValue $row.render_pacing_target_fps "-"
    $presentGapP95 = [Math]::Round([double](Select-CanaryValue $row.render_present_gap_p95_ms 0), 2)
    $probeDrops = Select-CanaryValue $row.probe_dropped_frames $row.dropped_frames
    $renderCoalesce = Select-CanaryValue $row.render_queue_replacements 0
    $renderDrops = Select-CanaryValue $row.render_lock_drops 0
    $lines += "| $($row.id) | $($row.status) | $($row.classification) | $([Math]::Round([double](Select-CanaryValue $row.fps_observed 0), 2)) | $estimatedRenderFps | $renderTargetFps | $selected | $visual | $encodeP50/$encodeP95 | $sendP50/$sendP95 | $presentGapP95 | $probeDrops | $renderCoalesce | $renderDrops | $($row.queue_depth) | $error |"
  }
  if ($Report.codec_request) {
    $lines += ""
    $lines += "## Codec Request"
    $lines += ""
    $lines += "- Codec: $($Report.codec_request.codec)"
    $lines += "- CodecProfile: $(if ($Report.codec_request.codec_profile) { $Report.codec_request.codec_profile } else { '-' })"
    $lines += "- BitDepth: $(if ($Report.codec_request.bit_depth) { $Report.codec_request.bit_depth } else { '-' })"
    $lines += "- ChromaSubsampling: $(if ($Report.codec_request.chroma_subsampling) { $Report.codec_request.chroma_subsampling } else { '-' })"
    $lines += "- PixelFormat: $(if ($Report.codec_request.pixel_format) { $Report.codec_request.pixel_format } else { '-' })"
    $lines += "- HdrEnabled: $($Report.codec_request.hdr_enabled)"
  }
  $lines -join [Environment]::NewLine | Set-Content -Path $MarkdownPath -Encoding Ascii
}

function Write-PairedLanComparisonMarkdown {
  param(
    [Parameter(Mandatory = $true)]$Rows,
    [Parameter(Mandatory = $true)][string]$MarkdownPath,
    [string]$GitCommit
  )

  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $MarkdownPath) | Out-Null
  $completed = @($Rows | Where-Object { $_.status -eq "completed" }).Count
  $skipped = @($Rows | Where-Object { -not $_.comparable }).Count
  $failed = @($Rows | Where-Object { $_.comparable -and $_.status -ne "completed" }).Count
  $lines = @(
    "# Matrix Comparison Report",
    "",
    "- Commit: $GitCommit",
    "- Completed: $completed",
    "- Skipped: $skipped",
    "- Failed: $failed",
    "- Rule: cross FPS must be at least 80 percent of local baseline FPS when selected profiles match.",
    "- Local baseline FPS caps local observed FPS to the selected/requested profile FPS.",
    "",
    "| Profile | Status | Comparable | Local FPS | Local Baseline FPS | Cross FPS | Ratio | Local decode p95 | Cross decode p95 | Local present p95 | Cross present p95 | Reason |",
    "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"
  )
  foreach ($row in $Rows) {
    $ratio = if ($null -eq $row.fps_ratio) { "-" } else { [Math]::Round([double]$row.fps_ratio, 3) }
    $reason = ((Select-CanaryValue $row.reason "") -replace "\|", "/")
    $lines += "| $($row.id) | $($row.status) | $($row.comparable) | $([Math]::Round([double]$row.local_fps, 2)) | $([Math]::Round([double]$row.local_baseline_fps, 2)) | $([Math]::Round([double]$row.cross_fps, 2)) | $ratio | $($row.local_decode_p95_ms) | $($row.cross_decode_p95_ms) | $($row.local_render_p95_ms) | $($row.cross_render_p95_ms) | $reason |"
  }
  $lines -join [Environment]::NewLine | Set-Content -Path $MarkdownPath -Encoding Ascii
}
