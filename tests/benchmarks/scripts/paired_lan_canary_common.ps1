$ErrorActionPreference = "Stop"

function Select-CanaryValue {
  param($Value, $Fallback)
  if ($null -eq $Value) { return $Fallback }
  $Value
}

function Get-PairedLanCanaryProfiles {
  param(
    [int]$DurationSecs = 30,
    [int]$BitrateMbps = 20
  )

  @(
    [pscustomobject]@{ id = "1080p60"; width = 1920; height = 1080; fps = 60; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
    [pscustomobject]@{ id = "2k60"; width = 2560; height = 1440; fps = 60; bitrate_mbps = $BitrateMbps; duration_secs = $DurationSecs },
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

function Convert-LocalSummaryToCanaryRow {
  param(
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$Summary,
    [string]$SummaryPath
  )

  $status = if ($Summary.run_passed) { "completed" } else { "failed" }
  $classification = if ($Summary.run_passed) { "completed" } elseif ($Summary.fps_observed -lt ($Profile.fps * 0.8)) { "threshold_miss" } else { "failed" }

  [pscustomobject]@{
    id = $Profile.id
    width = [int]$Profile.width
    height = [int]$Profile.height
    fps = [int]$Profile.fps
    bitrate_mbps = [int]$Profile.bitrate_mbps
    duration_secs = [int]$Profile.duration_secs
    chain = "dxgi/nvenc_h264/quic/nvdec/d3d11_shared"
    status = $status
    classification = $classification
    fps_observed = [double](Select-CanaryValue $Summary.fps_observed 0)
    selected_profile = New-CanarySelectedProfile -Width $Summary.width -Height $Summary.height -Fps $Summary.fps_target -BitrateMbps $Profile.bitrate_mbps
    session_established = [bool]$Summary.session_established
    first_frame_seen = [bool]$Summary.first_frame_seen
    first_frame_time_ms = $Summary.first_frame_time_ms
    decoded_frames = $null
    dropped_frames = [int64](Select-CanaryValue $Summary.dropped_frames 0)
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
    [string]$ReportPath
  )

  $probe = $Report.probeSnapshot
  $pipeline = $Report.mediaPipelineSnapshot
  $selected = if ($probe -and $probe.media_probe_width -and $probe.media_probe_height -and $probe.media_probe_target_fps) {
    New-CanarySelectedProfile -Width $probe.media_probe_width -Height $probe.media_probe_height -Fps $probe.media_probe_target_fps -BitrateMbps (Select-CanaryValue $probe.media_probe_target_bitrate_mbps $Profile.bitrate_mbps)
  } else {
    New-CanarySelectedProfile -Width $Profile.width -Height $Profile.height -Fps $Profile.fps -BitrateMbps $Profile.bitrate_mbps
  }
  $classification = Get-CrossCanaryClassification -Report $Report -Profile $Profile -SelectedProfile $selected
  $status = Get-CrossCanaryStatus -Report $Report -Classification $classification

  [pscustomobject]@{
    id = $Profile.id
    width = [int]$Profile.width
    height = [int]$Profile.height
    fps = [int]$Profile.fps
    bitrate_mbps = [int]$Profile.bitrate_mbps
    duration_secs = [int]$Profile.duration_secs
    chain = "dxgi/nvenc_h264/quic_datagram_media_v3_or_v2/nvdec/d3d11_shared"
    status = $status
    classification = $classification
    fps_observed = [double](Select-CanaryValue (Select-CanaryValue $Report.sampleObservedFps $probe.current_fps) 0)
    selected_profile = $selected
    session_established = [bool]($Report.sessionSnapshot -and $Report.sessionSnapshot.state -ne "failed")
    first_frame_seen = [bool]($probe -and $probe.frames_decoded -gt 0)
    first_frame_time_ms = $null
    decoded_frames = [int64](Select-CanaryValue $probe.frames_decoded 0)
    dropped_frames = [int64](Select-CanaryValue (Select-CanaryValue $pipeline.dropped_frames $probe.frames_dropped) 0)
    queue_depth = $pipeline.queue_depth
    stage_p95_ms = Convert-MediaStageMetricsToP95Map -StageMetrics $pipeline.stage_metrics
    raw_report_path = $ReportPath
    error_message = $Report.errorMessage
  }
}

function Get-CrossCanaryStatus {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)][string]$Classification
  )

  if ($Report.status -eq "completed") { return "completed" }
  if ($Report.status -eq "skipped") { return "skipped" }
  if ($Classification -in @("unsupported", "profile_downgraded", "peer_version_mismatch")) {
    return "skipped"
  }
  return [string]$Report.status
}

function Get-CrossCanaryClassification {
  param(
    [Parameter(Mandatory = $true)]$Report,
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)]$SelectedProfile
  )

  if (-not (Test-CanaryProfileMatch -Expected $Profile -Actual $SelectedProfile)) {
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

    $profilesMatch =
      (Test-CanaryProfileMatch -Expected $local.selected_profile -Actual $cross.selected_profile) -and
      (Test-CanaryProfileMatch -Expected $local -Actual $cross.selected_profile)
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
    $ratio = if ($localBaselineFps -gt 0) { $crossFps / $localBaselineFps } else { 0.0 }
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
      local_baseline_fps = $localBaselineFps
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
    [string]$GeneratedAt = (Get-Date).ToString("o")
  )

  [pscustomobject]@{
    schema_version = 1
    mode = $Mode
    generated_at = $GeneratedAt
    git_commit = $GitCommit
    chain = if ($Mode -eq "cross") { "dxgi/nvenc_h264/quic_datagram_media_v3_or_v2/nvdec/d3d11_shared" } else { "dxgi/nvenc_h264/quic_datagram/nvdec/d3d11_shared" }
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
    "| Profile | Status | Class | FPS | Selected | Dropped | Queue | Error |",
    "| --- | --- | --- | ---: | --- | ---: | ---: | --- |"
  )
  foreach ($row in $Report.rows) {
    $selected = "$($row.selected_profile.width)x$($row.selected_profile.height)@$($row.selected_profile.fps)/$($row.selected_profile.bitrate_mbps)Mbps"
    $error = ((Select-CanaryValue $row.error_message "") -replace "\|", "/")
    $lines += "| $($row.id) | $($row.status) | $($row.classification) | $([Math]::Round([double](Select-CanaryValue $row.fps_observed 0), 2)) | $selected | $($row.dropped_frames) | $($row.queue_depth) | $error |"
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
