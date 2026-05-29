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
$errorPattern = '(?i)\berror:\b|panic|(^|\s)FAILED(\s|$)'
$restartPattern = '(?i)\brestart\b|recreated'

$warningCount = 0
$errorCount = 0
$restartCount = 0
foreach ($path in @($hostStdout, $hostStderr, $signalingStdout, $signalingStderr)) {
  if (Test-Path $path) {
    $content = Get-Content $path -Raw
    if ($null -eq $content) { $content = "" }
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

if ((-not $summary.run_skipped) -and $ThresholdPath -and (Test-Path $ThresholdPath)) {
  $thresholds = Get-Content $ThresholdPath -Raw | ConvertFrom-Json
  $summary.run_passed = (
    $summary.run_passed -and
    $summary.session_established -and
    $summary.first_frame_seen -and
    ($summary.first_frame_time_ms -le $thresholds.max_first_frame_time_ms) -and
    ($summary.fps_observed -ge $thresholds.min_fps_observed) -and
    (($null -eq $summary.encode_total_p95_ms) -or ($summary.encode_total_p95_ms -le $thresholds.max_encode_total_p95_ms)) -and
    (($null -eq $summary.send_write_p95_ms) -or ($summary.send_write_p95_ms -le $thresholds.max_send_write_p95_ms)) -and
    (($null -eq $summary.decode_total_p95_ms) -or ($summary.decode_total_p95_ms -le $thresholds.max_decode_total_p95_ms)) -and
    ($summary.warning_count -le $thresholds.max_warning_count) -and
    ($summary.error_count -le $thresholds.max_error_count)
  )
  if (-not $summary.run_passed -and [string]::IsNullOrWhiteSpace([string]$summary.failure_reason)) {
    $reasons = @()
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
    if ($summary.warning_count -gt $thresholds.max_warning_count) {
      $reasons += "warning count $($summary.warning_count) exceeded $($thresholds.max_warning_count)"
    }
    if ($summary.error_count -gt $thresholds.max_error_count) {
      $reasons += "error count $($summary.error_count) exceeded $($thresholds.max_error_count)"
    }
    $summary.failure_reason = if ($reasons.Count -gt 0) { $reasons -join "; " } else { "benchmark thresholds were not met" }
  }
}

$summary | ConvertTo-Json -Depth 8 | Set-Content $summaryPath -Encoding Ascii

$headers = @(
  'run_id','scenario','transport','capture_backend','encode_backend','decode_backend','renderer_backend',
  'width','height','fps_target','duration_secs','session_established','first_frame_seen','first_frame_time_ms',
  'probe_complete','fps_observed','bitrate_kbps','keyframes','dropped_frames',
  'quic_receiver_completed_frames','quic_receiver_expired_frames','quic_receiver_evicted_frames',
  'quic_receiver_duplicate_fragments','quic_receiver_rejected_fragments','quic_receiver_pending_frames',
  'quic_receiver_reassembly_drops','zero_write_access_unit_count',
  'warning_count','error_count','restart_count','encode_total_p95_ms','send_write_p95_ms','decode_total_p95_ms',
  'frame_sink_ingest_p95_ms','render_upload_p95_ms','render_present_p95_ms',
  'render_submitted_frames','render_uploaded_frames','render_presented_frames','render_present_skipped_frames',
  'render_queue_replacements','render_stale_frame_drops',
  'swap_chain_waitable_object','swap_chain_present_mode','display_refresh_hz','render_thread_priority',
  'failure_reason','run_skipped','run_passed'
)
$row = [pscustomobject]@{}
foreach ($header in $headers) { $row | Add-Member -NotePropertyName $header -NotePropertyValue $summary.$header }
$row | Export-Csv -Path $csvPath -NoTypeInformation -Encoding Ascii

$status = if ($summary.run_skipped) { 'SKIP' } elseif ($summary.run_passed) { 'PASS' } else { 'FAIL' }
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
  "- Status: $status",
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
  "| encode_total_p95_ms | $($summary.encode_total_p95_ms) |",
  "| send_write_p95_ms | $($summary.send_write_p95_ms) |",
  "| decode_total_p95_ms | $($summary.decode_total_p95_ms) |",
  "| frame_sink_ingest_p95_ms | $($summary.frame_sink_ingest_p95_ms) |",
  "| render_upload_p95_ms | $($summary.render_upload_p95_ms) |",
  "| render_present_p95_ms | $($summary.render_present_p95_ms) |",
  "| render_submitted_frames | $($summary.render_submitted_frames) |",
  "| render_uploaded_frames | $($summary.render_uploaded_frames) |",
  "| render_presented_frames | $($summary.render_presented_frames) |",
  "| render_present_skipped_frames | $($summary.render_present_skipped_frames) |",
  "| render_queue_replacements | $($summary.render_queue_replacements) |",
  "| render_stale_frame_drops | $($summary.render_stale_frame_drops) |",
  "| swap_chain_waitable_object | $($summary.swap_chain_waitable_object) |",
  "| swap_chain_present_mode | $($summary.swap_chain_present_mode) |",
  "| display_refresh_hz | $($summary.display_refresh_hz) |",
  "| render_thread_priority | $($summary.render_thread_priority) |",
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
