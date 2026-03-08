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

if ($ThresholdPath -and (Test-Path $ThresholdPath)) {
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
}

$summary | ConvertTo-Json -Depth 8 | Set-Content $summaryPath -Encoding Ascii

$headers = @(
  'run_id','scenario','transport','capture_backend','encode_backend','decode_backend','renderer_backend',
  'width','height','fps_target','duration_secs','session_established','first_frame_seen','first_frame_time_ms',
  'probe_complete','fps_observed','bitrate_kbps','keyframes','dropped_frames','zero_write_access_unit_count',
  'warning_count','error_count','restart_count','encode_total_p95_ms','send_write_p95_ms','decode_total_p95_ms',
  'frame_sink_ingest_p95_ms','render_upload_p95_ms','render_present_p95_ms','run_passed'
)
$row = [pscustomobject]@{}
foreach ($header in $headers) { $row | Add-Member -NotePropertyName $header -NotePropertyValue $summary.$header }
$row | Export-Csv -Path $csvPath -NoTypeInformation -Encoding Ascii

$status = if ($summary.run_passed) { 'PASS' } else { 'FAIL' }
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
  "- Session established: $($summary.session_established)",
  "- First frame seen: $($summary.first_frame_seen)",
  "- First frame time ms: $($summary.first_frame_time_ms)",
  "- Probe complete: $($summary.probe_complete)",
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
  "| keyframes | $($summary.keyframes) |",
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
