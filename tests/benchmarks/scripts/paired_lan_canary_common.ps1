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

function Select-CanaryStageMapValue {
  param($StageMap, [string[]]$Stages, $Fallback)
  if ($null -eq $StageMap) { return $Fallback }
  foreach ($stage in $Stages) {
    $value = Select-CanaryObjectPropertyValue $StageMap $stage $null
    if ($null -ne $value) { return $value }
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

function New-LocalDualProcessTauriEnvPlan {
  param(
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [Parameter(Mandatory = $true)][string]$ServiceExe,
    [switch]$NoBuild
  )

  $envPlan = [ordered]@{
    MRD_SERVICE_PREBUILT_EXE = $ServiceExe
    MRD_SERVICE_EXE = $ServiceExe
  }

  [pscustomobject]$envPlan
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
    [pscustomobject]@{ id = "4k120"; width = 3840; height = 2160; fps = 120; bitrate_mbps = 120; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k180"; width = 2560; height = 1440; fps = 180; bitrate_mbps = 100; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k180_120mbps"; width = 2560; height = 1440; fps = 180; bitrate_mbps = 120; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k180_120mbps_adaptive"; width = 2560; height = 1440; fps = 180; bitrate_mbps = 120; duration_secs = $DurationSecs; adaptive = $true },
    [pscustomobject]@{ id = "1600p165"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 80; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1600p165_120mbps"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 120; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "1600p165_120mbps_adaptive"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 120; duration_secs = $DurationSecs; adaptive = $true },
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

  $sampleDecodedFrames = [double](Select-CanaryObjectPropertyValue $Report "sampleFramesDecoded" 0)
  $sampleProbeDrops = [double](Select-CanaryObjectPropertyValue $Report "sampleFramesDropped" -1)
  $decodedFrames = if ($sampleDecodedFrames -gt 0) { $sampleDecodedFrames } else { [double](Select-CanaryValue $Probe.frames_decoded 0) }
  $probeDrops = if ($sampleProbeDrops -ge 0) { $sampleProbeDrops } else { [double](Select-CanaryValue $Probe.frames_dropped 0) }
  $totalSequencedFrames = $decodedFrames + $probeDrops
  if ($totalSequencedFrames -gt 0) {
    $probeDropRatio = $probeDrops / $totalSequencedFrames
    $maxProbeDropRatio = $script:CanaryMaxProbeDropRatio
    if ($Profile -and [bool](Select-CanaryValue $Profile.adaptive $false)) {
      $maxProbeDropRatio = 0.20
    }
    if ($probeDropRatio -gt $maxProbeDropRatio) {
      return "Visual integrity risk: drop ratio $([Math]::Round($probeDropRatio * 100, 2))% exceeds $([Math]::Round($maxProbeDropRatio * 100, 2))% ($([int64]$probeDrops) dropped / $([int64]$totalSequencedFrames) sequenced frames)."
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

  $sampleObservedRenderFps = [double](Select-CanaryValue $Report.sampleObservedRenderFps 0)
  if ($sampleObservedRenderFps -gt 0) {
    return $sampleObservedRenderFps
  }

  $decodedFrames = [double](Select-CanaryValue $Probe.frames_decoded 0)
  if ($decodedFrames -le 0) { return 0.0 }

  $renderQueueReplacements = [double](Select-CanaryValue $Pipeline.render_queue_replacements 0)
  $renderLockDrops = [double](Select-CanaryValue $Pipeline.render_lock_drops 0)
  $renderPresentSkips = [double](Select-CanaryValue $Pipeline.render_present_skips 0)
  $presentedFrames = [Math]::Max(0.0, $decodedFrames - $renderQueueReplacements - $renderLockDrops - $renderPresentSkips)
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
  $renderPresentSkips = [double](Select-CanaryValue $Pipeline.render_present_skips 0)
  ($renderQueueReplacements + $renderLockDrops + $renderPresentSkips) / $decodedFrames
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
  $renderPresentSkips = [double](Select-CanaryValue $Pipeline.render_present_skips 0)
  if ($renderPresentSkips -gt 0) { return $false }

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
    [string]$SummaryPath,
    [string]$RequestedCodec = "h264"
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
    chain = New-CanaryMediaChain -Mode "local" -Codec $RequestedCodec
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
    sequence_gap_drops = 0
    decode_error_drops = 0
    transient_drops = 0
    pipeline_dropped_frames = $localDropped
    sample_sequence_gap_drops = $null
    sample_decode_error_drops = $null
    sample_transient_drops = $null
    render_queue_replacements = 0
    render_lock_drops = 0
    render_present_skips = 0
    queue_depth = $null
    stage_p95_ms = [pscustomobject]@{
      encode = $Summary.encode_total_p95_ms
      transport = $Summary.send_write_p95_ms
      decode = $Summary.decode_total_p95_ms
      render_upload = $Summary.render_upload_p95_ms
      present = $Summary.render_present_p95_ms
    }
    raw_summary_path = $SummaryPath
    capture_source_summary = "local / dxgi / $($Summary.width)x$($Summary.height) / single-process baseline"
    active_display_mode_summary = "-"
    requested_codec = Normalize-CanaryCodec $RequestedCodec
    active_codec = Normalize-CanaryCodec $RequestedCodec
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
  $sampleProbeDropped = Select-CanaryObjectPropertyValue $Report "sampleFramesDropped" $null
  $sequenceGapDropped = [int64](Select-CanaryValue $probe.sequence_gap_drops 0)
  $decodeErrorDropped = [int64](Select-CanaryValue $probe.decode_error_drops 0)
  $transientDropped = [int64](Select-CanaryValue $probe.transient_drops 0)
  $sampleSequenceGapDropped = Select-CanaryObjectPropertyValue $Report "sampleSequenceGapDrops" $null
  $sampleDecodeErrorDropped = Select-CanaryObjectPropertyValue $Report "sampleDecodeErrorDrops" $null
  $sampleTransientDropped = Select-CanaryObjectPropertyValue $Report "sampleTransientDrops" $null
  $pipelineDropped = [int64](Select-CanaryValue $pipeline.dropped_frames 0)
  $renderQueueReplacements = [int64](Select-CanaryValue $pipeline.render_queue_replacements $pipelineDropped)
  $renderLockDrops = [int64](Select-CanaryValue $pipeline.render_lock_drops 0)
  $renderPresentSkips = [int64](Select-CanaryValue $pipeline.render_present_skips 0)
  $activeCodec = Select-CanaryValue $pipeline.active_codec $RequestedCodec
  $senderTransport = Select-CanaryObjectPropertyValue $pipeline "sender_transport" $null
  $captureSource = Select-CanaryObjectPropertyValue $Report "captureSource" $null
  $activeDisplayMode = Select-CanaryObjectPropertyValue (Select-CanaryObjectPropertyValue $Report "displayModeChange" $null) "active" $null

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
    sample_probe_dropped_frames = $sampleProbeDropped
    sequence_gap_drops = $sequenceGapDropped
    decode_error_drops = $decodeErrorDropped
    transient_drops = $transientDropped
    sample_sequence_gap_drops = $sampleSequenceGapDropped
    sample_decode_error_drops = $sampleDecodeErrorDropped
    sample_transient_drops = $sampleTransientDropped
    pipeline_dropped_frames = $pipelineDropped
    render_queue_replacements = $renderQueueReplacements
    render_lock_drops = $renderLockDrops
    render_present_skips = $renderPresentSkips
    render_presented_frames = [int64](Select-CanaryValue $pipeline.render_presented_frames 0)
    sample_render_frames_presented = [int64](Select-CanaryValue $Report.sampleRenderFramesPresented 0)
    sample_observed_render_fps = Select-CanaryValue $Report.sampleObservedRenderFps $null
    estimated_render_fps = [double](Get-CanaryEstimatedRenderFps -Probe $probe -Pipeline $pipeline -Report $Report)
    render_coalesce_ratio = $renderCoalesceRatio
    render_present_gap_p95_ms = [double](Select-CanaryStageP95Value -Pipeline $pipeline -Stage "render_present_gap" -Fallback 0)
    render_pacing_target_fps = $pipeline.render_pacing_target_fps
    queue_depth = $pipeline.queue_depth
    stage_p50_ms = Convert-MediaStageMetricsToP50Map -StageMetrics $pipeline.stage_metrics
    stage_p95_ms = Convert-MediaStageMetricsToP95Map -StageMetrics $pipeline.stage_metrics
    test_impairment = $pipeline.test_impairment
    sender_transport = $senderTransport
    adaptive = [bool](Select-CanaryValue $Profile.adaptive $false)
    adaptation = $adaptation
    adaptation_state = Select-CanaryValue $adaptation.state "-"
    adaptation_ladder_index = Select-CanaryValue $adaptation.ladder_index $null
    adaptation_last_reason = Select-CanaryValue $adaptation.last_reason ""
    display_mode = $Report.displayModeChange
    active_display_mode_summary = Get-CanaryDisplayModeSummary -DisplayMode $activeDisplayMode
    active_display_mode_source_id = Select-CanaryValue $activeDisplayMode.source_id $null
    active_display_refresh_hz = Select-CanaryValue $activeDisplayMode.refresh_hz $null
    raw_report_path = $ReportPath
    error_message = Select-CanaryValue $Report.errorMessage (Select-CanaryValue $visualIntegrityIssue $displayLimitReason)
    visual_integrity_status = if ($visualIntegrityIssue) { "risk" } elseif ($pacedRenderCoalescing) { "paced" } else { "ok" }
    visual_integrity_message = $visualIntegrityIssue
    actual_capture_source_id = Select-CanaryValue $captureSource.id $null
    actual_capture_source_kind = Select-CanaryValue $captureSource.source_kind $null
    actual_capture_source_title = Select-CanaryValue $captureSource.title $null
    actual_capture_source_class_name = Select-CanaryValue $captureSource.class_name $null
    actual_capture_source_width = Select-CanaryValue $captureSource.width $null
    actual_capture_source_height = Select-CanaryValue $captureSource.height $null
    capture_source_summary = Get-CanaryCaptureSourceSummary -Source $captureSource
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
      [bool](Select-CanaryValue $Profile.adaptive $false)
    ) {
      return "completed"
    }
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

function Get-CanaryCaptureSourceSummary {
  param($Source)

  if ($null -eq $Source) { return "-" }
  $id = Select-CanaryValue $Source.id "-"
  $kind = Select-CanaryValue $Source.source_kind "-"
  $width = [int](Select-CanaryValue $Source.width 0)
  $height = [int](Select-CanaryValue $Source.height 0)
  $title = Select-CanaryValue $Source.title "-"
  $size = if ($width -gt 0 -and $height -gt 0) { "${width}x${height}" } else { "-" }
  "$id / $kind / $size / $title"
}

function Get-CanaryDisplayModeSummary {
  param($DisplayMode)

  if ($null -eq $DisplayMode) { return "-" }
  $width = [int](Select-CanaryValue $DisplayMode.width 0)
  $height = [int](Select-CanaryValue $DisplayMode.height 0)
  $refresh = [int](Select-CanaryValue $DisplayMode.refresh_hz 0)
  if ($width -le 0 -or $height -le 0 -or $refresh -le 0) { return "-" }
  "${width}x${height}@${refresh}"
}

function Get-CanaryRowStageValue {
  param(
    $Row,
    [ValidateSet("p50", "p95")]
    [string]$Statistic,
    [string[]]$Stages,
    $Fallback = $null
  )

  $map = if ($Statistic -eq "p50") { $Row.stage_p50_ms } else { $Row.stage_p95_ms }
  Select-CanaryStageMapValue -StageMap $map -Stages $Stages -Fallback $Fallback
}

function Get-CanaryRowSendStageValue {
  param(
    $Row,
    [ValidateSet("p50", "p95")]
    [string]$Statistic,
    $Fallback = $null
  )

  $map = if ($Statistic -eq "p50") { $Row.stage_p50_ms } else { $Row.stage_p95_ms }
  Select-CanarySenderSendStageValue -StageMap $map -Fallback $Fallback
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

function Get-PairedLanCanaryGapRootCause {
  param(
    [Parameter(Mandatory = $true)]$LocalRow,
    [Parameter(Mandatory = $true)]$CrossRow,
    [Parameter(Mandatory = $true)][string]$Status
  )

  if ($Status -eq "completed") { return "none" }
  if ($CrossRow.classification -and $CrossRow.classification -ne "completed") {
    return [string]$CrossRow.classification
  }

  $crossProbeDrops = [double](Select-CanaryValue $CrossRow.sample_probe_dropped_frames (Select-CanaryValue $CrossRow.probe_dropped_frames 0))
  $crossSequenceDrops = [double](Select-CanaryValue $CrossRow.sample_sequence_gap_drops (Select-CanaryValue $CrossRow.sequence_gap_drops 0))
  $crossDecodeDrops = [double](Select-CanaryValue $CrossRow.sample_decode_error_drops (Select-CanaryValue $CrossRow.decode_error_drops 0))
  $crossTransientDrops = [double](Select-CanaryValue $CrossRow.sample_transient_drops (Select-CanaryValue $CrossRow.transient_drops 0))

  $localCaptureP95 = [double](Get-CanaryRowStageValue -Row $LocalRow -Statistic "p95" -Stages @("sender.capture", "capture") -Fallback 0)
  $crossCaptureP95 = [double](Get-CanaryRowStageValue -Row $CrossRow -Statistic "p95" -Stages @("sender.capture", "capture") -Fallback 0)
  if ($crossCaptureP95 -gt 0 -and (($crossCaptureP95 - $localCaptureP95) -ge 2.0 -or ($localCaptureP95 -gt 0 -and $crossCaptureP95 -ge ($localCaptureP95 * 2.0)))) {
    return "capture_p95_regression"
  }

  $localEncodeP95 = [double](Get-CanaryRowStageValue -Row $LocalRow -Statistic "p95" -Stages @("sender.encode", "encode") -Fallback 0)
  $crossEncodeP95 = [double](Get-CanaryRowStageValue -Row $CrossRow -Statistic "p95" -Stages @("sender.encode", "encode") -Fallback 0)
  if ($crossEncodeP95 -gt 0 -and (($crossEncodeP95 - $localEncodeP95) -ge 2.0 -or ($localEncodeP95 -gt 0 -and $crossEncodeP95 -ge ($localEncodeP95 * 2.0)))) {
    return "encode_p95_regression"
  }

  $localSendP95 = [double](Get-CanaryRowSendStageValue -Row $LocalRow -Statistic "p95" -Fallback 0)
  $crossSendP95 = [double](Get-CanaryRowSendStageValue -Row $CrossRow -Statistic "p95" -Fallback 0)
  if ($crossSendP95 -gt 0 -and (($crossSendP95 - $localSendP95) -ge 2.0 -or ($localSendP95 -gt 0 -and $crossSendP95 -ge ($localSendP95 * 2.0)))) {
    return "transport_send_p95_regression"
  }

  if (($crossProbeDrops + $crossSequenceDrops + $crossDecodeDrops + $crossTransientDrops) -gt 0) {
    return "transport_loss_or_jitter"
  }

  $localDecodeP95 = [double](Get-CanaryRowStageValue -Row $LocalRow -Statistic "p95" -Stages @("receiver.decode", "decode") -Fallback 0)
  $crossDecodeP95 = [double](Get-CanaryRowStageValue -Row $CrossRow -Statistic "p95" -Stages @("receiver.decode", "decode") -Fallback 0)
  if ($crossDecodeP95 -gt 0 -and (($crossDecodeP95 - $localDecodeP95) -ge 2.0 -or ($localDecodeP95 -gt 0 -and $crossDecodeP95 -ge ($localDecodeP95 * 2.0)))) {
    return "decode_p95_regression"
  }

  $targetFps = [double](Select-CanaryValue $CrossRow.render_pacing_target_fps (Select-CanaryValue $CrossRow.selected_profile.fps $CrossRow.fps))
  $crossPresentGapP95 = [double](Get-CanaryRowStageValue -Row $CrossRow -Statistic "p95" -Stages @("render_present_gap", "present_gap") -Fallback 0)
  if ($targetFps -gt 0 -and $crossPresentGapP95 -gt ((1000.0 / $targetFps) * $script:CanaryMaxPacedPresentGapMultiplier)) {
    return "render_pacing_jitter"
  }

  "fps_threshold_miss"
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
        root_cause = "missing_cross"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = 0.0
        fps_ratio = $null
        local_capture_source = Select-CanaryValue $local.capture_source_summary "-"
        cross_capture_source = "-"
        cross_display_mode = "-"
        reason = "Cross-device result is missing"
      }
      continue
    }

    if ($cross.classification -eq "display_refresh_limited") {
      $results += [pscustomobject]@{
        id = $local.id
        comparable = $false
        status = "display_refresh_limited"
        root_cause = "display_refresh_limited"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = [double](Select-CanaryValue $cross.fps_observed 0)
        fps_ratio = $null
        local_capture_source = Select-CanaryValue $local.capture_source_summary "-"
        cross_capture_source = Select-CanaryValue $cross.capture_source_summary "-"
        cross_display_mode = Select-CanaryValue $cross.active_display_mode_summary "-"
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
        root_cause = "profile_downgraded"
        local_fps = $localFps
        local_baseline_fps = $localBaselineFps
        cross_fps = [double](Select-CanaryValue $cross.fps_observed 0)
        fps_ratio = $null
        local_capture_source = Select-CanaryValue $local.capture_source_summary "-"
        cross_capture_source = Select-CanaryValue $cross.capture_source_summary "-"
        cross_display_mode = Select-CanaryValue $cross.active_display_mode_summary "-"
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
    $rootCause = Get-PairedLanCanaryGapRootCause -LocalRow $local -CrossRow $cross -Status $status
    $localCaptureP95 = [double](Get-CanaryRowStageValue -Row $local -Statistic "p95" -Stages @("sender.capture", "capture") -Fallback 0)
    $crossCaptureP95 = [double](Get-CanaryRowStageValue -Row $cross -Statistic "p95" -Stages @("sender.capture", "capture") -Fallback 0)
    $localEncodeP50 = [double](Get-CanaryRowStageValue -Row $local -Statistic "p50" -Stages @("sender.encode", "encode") -Fallback 0)
    $crossEncodeP50 = [double](Get-CanaryRowStageValue -Row $cross -Statistic "p50" -Stages @("sender.encode", "encode") -Fallback 0)
    $localEncodeP95 = [double](Get-CanaryRowStageValue -Row $local -Statistic "p95" -Stages @("sender.encode", "encode") -Fallback 0)
    $crossEncodeP95 = [double](Get-CanaryRowStageValue -Row $cross -Statistic "p95" -Stages @("sender.encode", "encode") -Fallback 0)
    $localSendP50 = [double](Get-CanaryRowSendStageValue -Row $local -Statistic "p50" -Fallback 0)
    $crossSendP50 = [double](Get-CanaryRowSendStageValue -Row $cross -Statistic "p50" -Fallback 0)
    $localSendP95 = [double](Get-CanaryRowSendStageValue -Row $local -Statistic "p95" -Fallback 0)
    $crossSendP95 = [double](Get-CanaryRowSendStageValue -Row $cross -Statistic "p95" -Fallback 0)
    $localDecodeP95 = [double](Get-CanaryRowStageValue -Row $local -Statistic "p95" -Stages @("receiver.decode", "decode") -Fallback 0)
    $crossDecodeP95 = [double](Get-CanaryRowStageValue -Row $cross -Statistic "p95" -Stages @("receiver.decode", "decode") -Fallback 0)
    $localPresentGapP95 = [double](Get-CanaryRowStageValue -Row $local -Statistic "p95" -Stages @("render_present_gap", "present_gap", "present") -Fallback 0)
    $crossPresentGapP95 = [double](Get-CanaryRowStageValue -Row $cross -Statistic "p95" -Stages @("render_present_gap", "present_gap", "present") -Fallback 0)

    $results += [pscustomobject]@{
      id = $local.id
      comparable = ($status -ne "profile_downgraded")
      status = $status
      root_cause = $rootCause
      local_fps = $localFps
      local_baseline_fps = $comparisonBaselineFps
      cross_fps = $crossFps
      fps_ratio = $ratio
      local_capture_source = Select-CanaryValue $local.capture_source_summary "-"
      cross_capture_source = Select-CanaryValue $cross.capture_source_summary "-"
      cross_display_mode = Select-CanaryValue $cross.active_display_mode_summary "-"
      reason = if ($status -eq "threshold_miss") { "Cross FPS below $([Math]::Round($RatioThreshold * 100)) percent of local baseline" } else { $cross.error_message }
      local_capture_p95_ms = $localCaptureP95
      cross_capture_p95_ms = $crossCaptureP95
      capture_delta_p95_ms = $crossCaptureP95 - $localCaptureP95
      local_encode_p50_ms = $localEncodeP50
      cross_encode_p50_ms = $crossEncodeP50
      local_encode_p95_ms = $localEncodeP95
      cross_encode_p95_ms = $crossEncodeP95
      encode_delta_p95_ms = $crossEncodeP95 - $localEncodeP95
      local_send_p50_ms = $localSendP50
      cross_send_p50_ms = $crossSendP50
      local_send_p95_ms = $localSendP95
      cross_send_p95_ms = $crossSendP95
      send_delta_p95_ms = $crossSendP95 - $localSendP95
      local_decode_p95_ms = $localDecodeP95
      cross_decode_p95_ms = $crossDecodeP95
      decode_delta_p95_ms = $crossDecodeP95 - $localDecodeP95
      local_render_p95_ms = $localPresentGapP95
      cross_render_p95_ms = $crossPresentGapP95
      render_delta_p95_ms = $crossPresentGapP95 - $localPresentGapP95
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
    "| Profile | Status | Class | FPS | Render FPS | Render Target | Selected | Source | Display Mode | Adaptive | Visual | Enc P50/P95 | Send P50/P95 | Present Gap P95 | Sample/Probe Drop | Drop Breakdown gap/decode/transient | Sender Drop cap/budget/impair | Render Coalesce | Render Lock Drop | Present Skip | Queue | Error |",
    "| --- | --- | --- | ---: | ---: | ---: | --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"
  )
  foreach ($row in $Report.rows) {
    $selected = "$($row.selected_profile.width)x$($row.selected_profile.height)@$($row.selected_profile.fps)/$($row.selected_profile.bitrate_mbps)Mbps"
    $error = ((Select-CanaryValue $row.error_message "") -replace "\|", "/")
    $source = ((Select-CanaryValue $row.capture_source_summary "-") -replace "\|", "/")
    $displayMode = Select-CanaryValue $row.active_display_mode_summary "-"
    $visual = Select-CanaryValue $row.visual_integrity_status "n/a"
    $adaptiveState = Select-CanaryValue $row.adaptation_state "-"
    $adaptiveReason = ((Select-CanaryValue $row.adaptation_last_reason "") -replace "\|", "/")
    $adaptive = if ($adaptiveReason) { "${adaptiveState}: $adaptiveReason" } else { $adaptiveState }
    $encodeP50 = [Math]::Round([double](Select-CanaryValue $row.stage_p50_ms.'sender.encode' 0), 2)
    $encodeP95 = [Math]::Round([double](Select-CanaryValue $row.stage_p95_ms.'sender.encode' 0), 2)
    $sendP50 = [Math]::Round([double](Select-CanarySenderSendStageValue -StageMap $row.stage_p50_ms -Fallback 0), 2)
    $sendP95 = [Math]::Round([double](Select-CanarySenderSendStageValue -StageMap $row.stage_p95_ms -Fallback 0), 2)
    $estimatedRenderFps = [Math]::Round([double](Select-CanaryValue $row.estimated_render_fps 0), 2)
    $renderTargetFps = Select-CanaryValue $row.render_pacing_target_fps "-"
    $presentGapP95 = [Math]::Round([double](Select-CanaryValue $row.render_present_gap_p95_ms 0), 2)
    $probeDrops = Select-CanaryValue $row.sample_probe_dropped_frames (Select-CanaryValue $row.probe_dropped_frames $row.dropped_frames)
    $sequenceGapDrops = Select-CanaryValue $row.sample_sequence_gap_drops (Select-CanaryValue $row.sequence_gap_drops 0)
    $decodeErrorDrops = Select-CanaryValue $row.sample_decode_error_drops (Select-CanaryValue $row.decode_error_drops 0)
    $transientDrops = Select-CanaryValue $row.sample_transient_drops (Select-CanaryValue $row.transient_drops 0)
    $dropBreakdown = "$sequenceGapDrops/$decodeErrorDrops/$transientDrops"
    $senderCapacityDrops = Select-CanaryValue $row.sender_transport.datagram_fragments_dropped_for_capacity 0
    $senderBudgetDrops = Select-CanaryValue $row.sender_transport.datagram_fragments_dropped_for_budget 0
    $senderImpairmentDrops = Select-CanaryValue $row.sender_transport.datagram_fragments_dropped_by_impairment 0
    $senderDropBreakdown = "$senderCapacityDrops/$senderBudgetDrops/$senderImpairmentDrops"
    $renderCoalesce = Select-CanaryValue $row.render_queue_replacements 0
    $renderDrops = Select-CanaryValue $row.render_lock_drops 0
    $presentSkips = Select-CanaryValue $row.render_present_skips 0
    $lines += "| $($row.id) | $($row.status) | $($row.classification) | $([Math]::Round([double](Select-CanaryValue $row.fps_observed 0), 2)) | $estimatedRenderFps | $renderTargetFps | $selected | $source | $displayMode | $adaptive | $visual | $encodeP50/$encodeP95 | $sendP50/$sendP95 | $presentGapP95 | $probeDrops | $dropBreakdown | $senderDropBreakdown | $renderCoalesce | $renderDrops | $presentSkips | $($row.queue_depth) | $error |"
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
    "| Profile | Status | Root Cause | Comparable | Local FPS | Local Baseline FPS | Cross FPS | Ratio | Cross Source | Cross Display | Enc P95 local/cross/delta | Send P95 local/cross/delta | Decode P95 local/cross/delta | Present Gap P95 local/cross/delta | Reason |",
    "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | --- |"
  )
  foreach ($row in $Rows) {
    $ratio = if ($null -eq $row.fps_ratio) { "-" } else { [Math]::Round([double]$row.fps_ratio, 3) }
    $reason = ((Select-CanaryValue $row.reason "") -replace "\|", "/")
    $crossSource = ((Select-CanaryValue $row.cross_capture_source "-") -replace "\|", "/")
    $crossDisplay = Select-CanaryValue $row.cross_display_mode "-"
    $enc = "$([Math]::Round([double](Select-CanaryValue $row.local_encode_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.cross_encode_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.encode_delta_p95_ms 0), 2))"
    $send = "$([Math]::Round([double](Select-CanaryValue $row.local_send_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.cross_send_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.send_delta_p95_ms 0), 2))"
    $decode = "$([Math]::Round([double](Select-CanaryValue $row.local_decode_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.cross_decode_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.decode_delta_p95_ms 0), 2))"
    $render = "$([Math]::Round([double](Select-CanaryValue $row.local_render_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.cross_render_p95_ms 0), 2))/$([Math]::Round([double](Select-CanaryValue $row.render_delta_p95_ms 0), 2))"
    $lines += "| $($row.id) | $($row.status) | $($row.root_cause) | $($row.comparable) | $([Math]::Round([double]$row.local_fps, 2)) | $([Math]::Round([double]$row.local_baseline_fps, 2)) | $([Math]::Round([double]$row.cross_fps, 2)) | $ratio | $crossSource | $crossDisplay | $enc | $send | $decode | $render | $reason |"
  }
  $lines -join [Environment]::NewLine | Set-Content -Path $MarkdownPath -Encoding Ascii
}
