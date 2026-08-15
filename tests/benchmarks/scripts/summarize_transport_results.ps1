param(
  [Parameter(Mandatory = $true)]
  [string]$RunDir,
  [string]$ThresholdPath
)

$summaryPath = Join-Path $RunDir 'summary.json'
$manifestPath = Join-Path $RunDir 'manifest.json'
$reportPath = Join-Path $RunDir 'reports/markdown-report.md'
$csvPath = Join-Path $RunDir 'summary.csv'
$hostStdout = Join-Path $RunDir 'logs/host.stdout.log'
$hostStderr = Join-Path $RunDir 'logs/host.stderr.log'
$signalingStdout = Join-Path $RunDir 'logs/signaling.stdout.log'
$signalingStderr = Join-Path $RunDir 'logs/signaling.stderr.log'

$summary = Get-Content $summaryPath -Raw | ConvertFrom-Json
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json

$warningPattern = '(?i)\bwarning\b|Warning:'
$errorPattern = '(?i)\berror:\b|panic|(?-i:(^|\s)FAILED(\s|$))'
$restartPattern = '(?i)\brestart\b|recreated'

$warningCount = 0
$errorCount = 0
$restartCount = 0
foreach ($path in @($hostStdout, $hostStderr, $signalingStdout, $signalingStderr)) {
  if (Test-Path $path) {
    $content = Get-Content $path -Raw
    if ($null -eq $content) { $content = "" }
    if ($path -eq $hostStderr) {
      $runtimeStart = [regex]::Match($content, '(?m)^\s+Running (?:unittests|tests\\)')
      if ($runtimeStart.Success) {
        $content = $content.Substring($runtimeStart.Index + $runtimeStart.Length)
      }
    }
    $warningCount += ([regex]::Matches($content, $warningPattern)).Count
    $errorCount += ([regex]::Matches($content, $errorPattern)).Count
    $restartCount += ([regex]::Matches($content, $restartPattern)).Count
  }
}

$summary.warning_count = $warningCount
$summary.error_count = $errorCount
$summary.restart_count = $restartCount
if (-not ($summary.PSObject.Properties.Name -contains 'failure_reason')) {
  $summary | Add-Member -NotePropertyName failure_reason -NotePropertyValue $null
}
if (-not ($summary.PSObject.Properties.Name -contains 'run_skipped')) {
  $summary | Add-Member -NotePropertyName run_skipped -NotePropertyValue $false
}

function Test-HasProperty($Object, [string]$Name) {
  return @($Object.PSObject.Properties.Name) -contains $Name
}

function New-RateValue($Count, $DurationSecs) {
  if ($null -eq $Count -or $null -eq $DurationSecs -or [double]$DurationSecs -le 0) {
    return $null
  }
  return [Math]::Round(([double]$Count / [double]$DurationSecs), 4)
}

function Test-FiniteNumber($Value) {
  if ($null -eq $Value) { return $false }
  try {
    $number = [double]$Value
    return -not ([double]::IsNaN($number) -or [double]::IsInfinity($number))
  } catch {
    return $false
  }
}

$summary | Add-Member -Force -NotePropertyName render_queue_replacement_rate -NotePropertyValue (New-RateValue $summary.render_queue_replacements $summary.duration_secs)
$summary | Add-Member -Force -NotePropertyName render_stale_frame_drop_rate -NotePropertyValue (New-RateValue $summary.render_stale_frame_drops $summary.duration_secs)
$summary | Add-Member -Force -NotePropertyName render_present_skipped_rate -NotePropertyValue (New-RateValue $summary.render_present_skipped_frames $summary.duration_secs)
$displayRefreshLimited = (
  (Test-FiniteNumber $summary.display_refresh_hz) -and
  (Test-FiniteNumber $summary.fps_target) -and
  ([double]$summary.display_refresh_hz -gt 0) -and
  ([double]$summary.display_refresh_hz -lt [double]$summary.fps_target)
)
$summary | Add-Member -Force -NotePropertyName render_present_validation_limited_by_display -NotePropertyValue $displayRefreshLimited

if ((-not $summary.run_skipped) -and $ThresholdPath -and (Test-Path $ThresholdPath)) {
  $thresholds = Get-Content $ThresholdPath -Raw | ConvertFrom-Json
  $requiredEvidence = [ordered]@{
    first_frame_time_ms = $summary.first_frame_time_ms
    fps_observed = $summary.fps_observed
    encode_total_p95_ms = $summary.encode_total_p95_ms
    send_write_p95_ms = $summary.send_write_p95_ms
    decode_total_p95_ms = $summary.decode_total_p95_ms
  }
  $missingEvidence = @($requiredEvidence.GetEnumerator() | Where-Object { -not (Test-FiniteNumber $_.Value) } | ForEach-Object { $_.Key })
  $requiredEvidenceValid = $missingEvidence.Count -eq 0
  # A local window cannot present at a rate higher than its attached display.
  # Keep the media-pipeline checks strict, but do not treat physical refresh
  # limits as a renderer regression in a high-refresh transport benchmark.
  $hasRenderPresentThreshold = (Test-HasProperty $thresholds "max_render_present_p95_ms") -and (-not $displayRefreshLimited)
  $hasRenderExecuteThreshold = Test-HasProperty $thresholds "max_render_execute_p95_ms"
  $hasRenderPrepareWaitThreshold = Test-HasProperty $thresholds "max_render_prepare_wait_p95_ms"
  $hasRenderSharedResourceThreshold = Test-HasProperty $thresholds "max_render_shared_resource_p95_ms"
  $hasRenderDrawPresentThreshold = Test-HasProperty $thresholds "max_render_draw_present_p95_ms"
  $hasRenderQueueThreshold = Test-HasProperty $thresholds "max_render_queue_replacements"
  $hasRenderStaleThreshold = Test-HasProperty $thresholds "max_render_stale_frame_drops"
  $hasRenderSkippedThreshold = (Test-HasProperty $thresholds "max_render_present_skipped_frames") -and (-not $displayRefreshLimited)
  $hasRenderSkippedRateThreshold = (Test-HasProperty $thresholds "max_render_present_skipped_rate") -and (-not $displayRefreshLimited)
  $summary.run_passed = (
    $requiredEvidenceValid -and
    $summary.run_passed -and
    $summary.session_established -and
    $summary.first_frame_seen -and
    ($summary.first_frame_time_ms -le $thresholds.max_first_frame_time_ms) -and
    ($summary.fps_observed -ge $thresholds.min_fps_observed) -and
    (($null -eq $summary.encode_total_p95_ms) -or ($summary.encode_total_p95_ms -le $thresholds.max_encode_total_p95_ms)) -and
    (($null -eq $summary.send_write_p95_ms) -or ($summary.send_write_p95_ms -le $thresholds.max_send_write_p95_ms)) -and
    (($null -eq $summary.decode_total_p95_ms) -or ($summary.decode_total_p95_ms -le $thresholds.max_decode_total_p95_ms)) -and
    ((-not $hasRenderExecuteThreshold) -or ($null -eq $summary.render_execute_p95_ms) -or ($summary.render_execute_p95_ms -le $thresholds.max_render_execute_p95_ms)) -and
    ((-not $hasRenderPrepareWaitThreshold) -or ($null -eq $summary.render_prepare_wait_p95_ms) -or ($summary.render_prepare_wait_p95_ms -le $thresholds.max_render_prepare_wait_p95_ms)) -and
    ((-not $hasRenderSharedResourceThreshold) -or ($null -eq $summary.render_shared_resource_p95_ms) -or ($summary.render_shared_resource_p95_ms -le $thresholds.max_render_shared_resource_p95_ms)) -and
    ((-not $hasRenderDrawPresentThreshold) -or ($null -eq $summary.render_draw_present_p95_ms) -or ($summary.render_draw_present_p95_ms -le $thresholds.max_render_draw_present_p95_ms)) -and
    ((-not $hasRenderPresentThreshold) -or ($null -eq $summary.render_present_p95_ms) -or ($summary.render_present_p95_ms -le $thresholds.max_render_present_p95_ms)) -and
    ((-not $hasRenderQueueThreshold) -or ($null -eq $summary.render_queue_replacements) -or ($summary.render_queue_replacements -le $thresholds.max_render_queue_replacements)) -and
    ((-not $hasRenderStaleThreshold) -or ($null -eq $summary.render_stale_frame_drops) -or ($summary.render_stale_frame_drops -le $thresholds.max_render_stale_frame_drops)) -and
    ((-not $hasRenderSkippedThreshold) -or ($null -eq $summary.render_present_skipped_frames) -or ($summary.render_present_skipped_frames -le $thresholds.max_render_present_skipped_frames)) -and
    ((-not $hasRenderSkippedRateThreshold) -or ($null -eq $summary.render_present_skipped_rate) -or ($summary.render_present_skipped_rate -le $thresholds.max_render_present_skipped_rate)) -and
    ($summary.warning_count -le $thresholds.max_warning_count) -and
    ($summary.error_count -le $thresholds.max_error_count)
  )
  if (-not $summary.run_passed -and [string]::IsNullOrWhiteSpace([string]$summary.failure_reason)) {
    $reasons = @()
    if (-not $requiredEvidenceValid) { $reasons += "required evidence missing or non-finite: $($missingEvidence -join ', ')" }
    if (-not $summary.session_established) { $reasons += "session was not established" }
    if (-not $summary.first_frame_seen) { $reasons += "first frame was not observed" }
    if ($null -ne $summary.first_frame_time_ms -and $summary.first_frame_time_ms -gt $thresholds.max_first_frame_time_ms) {
      $reasons += "first frame time $($summary.first_frame_time_ms)ms exceeded $($thresholds.max_first_frame_time_ms)ms"
    }
    if ($summary.fps_observed -lt $thresholds.min_fps_observed) {
      $reasons += "observed FPS $($summary.fps_observed) below $($thresholds.min_fps_observed)"
    }
    if ($null -ne $summary.encode_total_p95_ms -and $summary.encode_total_p95_ms -gt $thresholds.max_encode_total_p95_ms) {
      $reasons += "encode p95 $($summary.encode_total_p95_ms)ms exceeded $($thresholds.max_encode_total_p95_ms)ms"
    }
    if ($null -ne $summary.send_write_p95_ms -and $summary.send_write_p95_ms -gt $thresholds.max_send_write_p95_ms) {
      $reasons += "send p95 $($summary.send_write_p95_ms)ms exceeded $($thresholds.max_send_write_p95_ms)ms"
    }
    if ($null -ne $summary.decode_total_p95_ms -and $summary.decode_total_p95_ms -gt $thresholds.max_decode_total_p95_ms) {
      $reasons += "decode p95 $($summary.decode_total_p95_ms)ms exceeded $($thresholds.max_decode_total_p95_ms)ms"
    }
    if ($hasRenderPresentThreshold -and $null -ne $summary.render_present_p95_ms -and $summary.render_present_p95_ms -gt $thresholds.max_render_present_p95_ms) {
      $reasons += "render present p95 $($summary.render_present_p95_ms)ms exceeded $($thresholds.max_render_present_p95_ms)ms"
    }
    if ($hasRenderExecuteThreshold -and $null -ne $summary.render_execute_p95_ms -and $summary.render_execute_p95_ms -gt $thresholds.max_render_execute_p95_ms) {
      $reasons += "render execute p95 $($summary.render_execute_p95_ms)ms exceeded $($thresholds.max_render_execute_p95_ms)ms"
    }
    if ($hasRenderPrepareWaitThreshold -and $null -ne $summary.render_prepare_wait_p95_ms -and $summary.render_prepare_wait_p95_ms -gt $thresholds.max_render_prepare_wait_p95_ms) {
      $reasons += "render prepare wait p95 $($summary.render_prepare_wait_p95_ms)ms exceeded $($thresholds.max_render_prepare_wait_p95_ms)ms"
    }
    if ($hasRenderSharedResourceThreshold -and $null -ne $summary.render_shared_resource_p95_ms -and $summary.render_shared_resource_p95_ms -gt $thresholds.max_render_shared_resource_p95_ms) {
      $reasons += "render shared resource p95 $($summary.render_shared_resource_p95_ms)ms exceeded $($thresholds.max_render_shared_resource_p95_ms)ms"
    }
    if ($hasRenderDrawPresentThreshold -and $null -ne $summary.render_draw_present_p95_ms -and $summary.render_draw_present_p95_ms -gt $thresholds.max_render_draw_present_p95_ms) {
      $reasons += "render draw/present p95 $($summary.render_draw_present_p95_ms)ms exceeded $($thresholds.max_render_draw_present_p95_ms)ms"
    }
    if ($hasRenderQueueThreshold -and $null -ne $summary.render_queue_replacements -and $summary.render_queue_replacements -gt $thresholds.max_render_queue_replacements) {
      $reasons += "render queue replacements $($summary.render_queue_replacements) exceeded $($thresholds.max_render_queue_replacements)"
    }
    if ($hasRenderStaleThreshold -and $null -ne $summary.render_stale_frame_drops -and $summary.render_stale_frame_drops -gt $thresholds.max_render_stale_frame_drops) {
      $reasons += "render stale frame drops $($summary.render_stale_frame_drops) exceeded $($thresholds.max_render_stale_frame_drops)"
    }
    if ($hasRenderSkippedThreshold -and $null -ne $summary.render_present_skipped_frames -and $summary.render_present_skipped_frames -gt $thresholds.max_render_present_skipped_frames) {
      $reasons += "render present skipped frames $($summary.render_present_skipped_frames) exceeded $($thresholds.max_render_present_skipped_frames)"
    }
    if ($hasRenderSkippedRateThreshold -and $null -ne $summary.render_present_skipped_rate -and $summary.render_present_skipped_rate -gt $thresholds.max_render_present_skipped_rate) {
      $reasons += "render present skipped rate $($summary.render_present_skipped_rate)/s exceeded $($thresholds.max_render_present_skipped_rate)/s"
    }
    if ($summary.warning_count -gt $thresholds.max_warning_count) {
      $reasons += "warning count $($summary.warning_count) exceeded $($thresholds.max_warning_count)"
    }
    if ($summary.error_count -gt $thresholds.max_error_count) {
      $reasons += "error count $($summary.error_count) exceeded $($thresholds.max_error_count)"
    }
    $summary.failure_reason = if ($reasons.Count -gt 0) { $reasons -join "; " } else { "benchmark thresholds were not met" }
  }
  if ($summary.run_passed) {
    $summary.failure_reason = $null
  }
}

$runStatus = if ($summary.run_skipped) { 'SKIP' } elseif ($summary.run_passed) { 'PASS' } else { 'FAIL' }
$summary | Add-Member -Force -NotePropertyName run_status -NotePropertyValue $runStatus
$summary | ConvertTo-Json -Depth 8 | Set-Content $summaryPath -Encoding Ascii

$headers = @(
  'run_status',
  'run_id','scenario','transport','capture_backend','encode_backend','decode_backend','renderer_backend',
  'width','height','fps_target','duration_secs','session_established','first_frame_seen','first_frame_time_ms',
  'probe_complete','fps_observed','bitrate_kbps','target_bitrate_kbps','encoded_fps','decoded_fps',
  'zero_copy_enabled','total_bitstream_bytes','keyframes','dropped_frames',
  'quic_receiver_completed_frames','quic_receiver_expired_frames','quic_receiver_evicted_frames',
  'quic_receiver_duplicate_fragments','quic_receiver_rejected_fragments','quic_receiver_pending_frames',
  'quic_receiver_reassembly_drops','zero_write_access_unit_count',
  'warning_count','error_count','restart_count','encode_total_p95_ms','send_write_p95_ms','decode_total_p95_ms',
  'frame_sink_ingest_p95_ms','render_upload_p95_ms','render_submit_wait_p95_ms','render_execute_p95_ms',
  'render_prepare_wait_p95_ms','render_shared_resource_p95_ms','render_draw_present_p95_ms','render_present_p95_ms',
  'render_submitted_frames','render_uploaded_frames','render_presented_frames','render_present_skipped_frames',
  'render_queue_replacements','render_stale_frame_drops',
  'render_queue_replacement_rate','render_stale_frame_drop_rate','render_present_skipped_rate','render_present_validation_limited_by_display',
  'swap_chain_max_frame_latency','swap_chain_allow_tearing',
  'swap_chain_waitable_object','swap_chain_present_mode','display_refresh_hz','render_thread_priority','render_pixel_format',
  'color_mode','color_pipeline',
  'nvdec_shared_copy_attempts','nvdec_shared_copy_successes','nvdec_shared_copy_failures',
  'nvdec_shared_copy_last_stage','nvdec_shared_copy_last_api','nvdec_shared_copy_last_error',
  'failure_reason','run_skipped','run_passed'
)
$row = [pscustomobject]@{}
foreach ($header in $headers) { $row | Add-Member -NotePropertyName $header -NotePropertyValue $summary.$header }
$row | Export-Csv -Path $csvPath -NoTypeInformation -Encoding Ascii

$report = @(
  "# Transport Benchmark Report",
  "",
  "Run: $($manifest.run_id)",
  "Scenario: $($manifest.scenario)",
  "Transport: $($manifest.transport)",
  "Commit: $($manifest.git_commit)",
  "Resolution: $($manifest.width)x$($manifest.height)@$($manifest.fps)",
  "Duration: $($manifest.duration_secs)s",
  "",
  "## Result",
  "",
  "- Status: $($summary.run_status)",
  "- Skipped: $($summary.run_skipped)",
  "- Session established: $($summary.session_established)",
  "- First frame seen: $($summary.first_frame_seen)",
  "- First frame time ms: $($summary.first_frame_time_ms)",
  "- Probe complete: $($summary.probe_complete)",
  "- Failure reason: $($summary.failure_reason)",
  "",
  "## Metrics",
  "",
  "| Metric | Value |",
  "| --- | --- |",
  "| fps_observed | $($summary.fps_observed) |",
  "| bitrate_kbps | $($summary.bitrate_kbps) |",
  "| target_bitrate_kbps | $($summary.target_bitrate_kbps) |",
  "| encoded_fps | $($summary.encoded_fps) |",
  "| decoded_fps | $($summary.decoded_fps) |",
  "| zero_copy_enabled | $($summary.zero_copy_enabled) |",
  "| total_bitstream_bytes | $($summary.total_bitstream_bytes) |",
  "| encode_total_p95_ms | $($summary.encode_total_p95_ms) |",
  "| send_write_p95_ms | $($summary.send_write_p95_ms) |",
  "| decode_total_p95_ms | $($summary.decode_total_p95_ms) |",
  "| frame_sink_ingest_p95_ms | $($summary.frame_sink_ingest_p95_ms) |",
  "| render_upload_p95_ms | $($summary.render_upload_p95_ms) |",
  "| render_submit_wait_p95_ms | $($summary.render_submit_wait_p95_ms) |",
  "| render_execute_p95_ms | $($summary.render_execute_p95_ms) |",
  "| render_prepare_wait_p95_ms | $($summary.render_prepare_wait_p95_ms) |",
  "| render_shared_resource_p95_ms | $($summary.render_shared_resource_p95_ms) |",
  "| render_draw_present_p95_ms | $($summary.render_draw_present_p95_ms) |",
  "| render_present_p95_ms | $($summary.render_present_p95_ms) |",
  "| render_submitted_frames | $($summary.render_submitted_frames) |",
  "| render_uploaded_frames | $($summary.render_uploaded_frames) |",
  "| render_presented_frames | $($summary.render_presented_frames) |",
  "| render_present_skipped_frames | $($summary.render_present_skipped_frames) |",
  "| render_queue_replacements | $($summary.render_queue_replacements) |",
  "| render_stale_frame_drops | $($summary.render_stale_frame_drops) |",
  "| render_queue_replacement_rate | $($summary.render_queue_replacement_rate) |",
  "| render_stale_frame_drop_rate | $($summary.render_stale_frame_drop_rate) |",
  "| render_present_skipped_rate | $($summary.render_present_skipped_rate) |",
  "| render_present_validation_limited_by_display | $($summary.render_present_validation_limited_by_display) |",
  "| swap_chain_max_frame_latency | $($summary.swap_chain_max_frame_latency) |",
  "| swap_chain_allow_tearing | $($summary.swap_chain_allow_tearing) |",
  "| swap_chain_waitable_object | $($summary.swap_chain_waitable_object) |",
  "| swap_chain_present_mode | $($summary.swap_chain_present_mode) |",
  "| display_refresh_hz | $($summary.display_refresh_hz) |",
  "| render_thread_priority | $($summary.render_thread_priority) |",
  "| render_pixel_format | $($summary.render_pixel_format) |",
  "| color_mode | $($summary.color_mode) |",
  "| color_pipeline | $($summary.color_pipeline) |",
  "| nvdec_shared_copy_attempts | $($summary.nvdec_shared_copy_attempts) |",
  "| nvdec_shared_copy_successes | $($summary.nvdec_shared_copy_successes) |",
  "| nvdec_shared_copy_failures | $($summary.nvdec_shared_copy_failures) |",
  "| nvdec_shared_copy_last_stage | $($summary.nvdec_shared_copy_last_stage) |",
  "| nvdec_shared_copy_last_api | $($summary.nvdec_shared_copy_last_api) |",
  "| nvdec_shared_copy_last_error | $($summary.nvdec_shared_copy_last_error) |",
  "| keyframes | $($summary.keyframes) |",
  "| dropped_frames | $($summary.dropped_frames) |",
  "| quic_receiver_completed_frames | $($summary.quic_receiver_completed_frames) |",
  "| quic_receiver_expired_frames | $($summary.quic_receiver_expired_frames) |",
  "| quic_receiver_evicted_frames | $($summary.quic_receiver_evicted_frames) |",
  "| quic_receiver_duplicate_fragments | $($summary.quic_receiver_duplicate_fragments) |",
  "| quic_receiver_rejected_fragments | $($summary.quic_receiver_rejected_fragments) |",
  "| quic_receiver_pending_frames | $($summary.quic_receiver_pending_frames) |",
  "| quic_receiver_reassembly_drops | $($summary.quic_receiver_reassembly_drops) |",
  "| warning_count | $($summary.warning_count) |",
  "| error_count | $($summary.error_count) |",
  "| restart_count | $($summary.restart_count) |",
  "",
  "## Paths",
  "",
  "- Summary: summary.json",
  "- CSV: summary.csv",
  "- Probe dir: sessions/",
  "- Logs dir: logs/"
) -join [Environment]::NewLine
$report | Set-Content $reportPath -Encoding Ascii
