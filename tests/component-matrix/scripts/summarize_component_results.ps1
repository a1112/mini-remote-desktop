param(
  [Parameter(Mandatory = $true)]
  [string]$RunDir,
  [string]$ThresholdPath
)

$resultPath = Join-Path $RunDir 'result.json'
$manifestPath = Join-Path $RunDir 'manifest.json'
$summaryPath = Join-Path $RunDir 'summary.csv'
$reportPath = Join-Path $RunDir 'reports/markdown-report.md'
$stdoutPath = Join-Path $RunDir 'logs/component.stdout.log'
$stderrPath = Join-Path $RunDir 'logs/component.stderr.log'

$result = Get-Content $resultPath -Raw | ConvertFrom-Json
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
$warningPattern = '(?i)\bwarning\b|Warning:'
$errorPattern = '(?i)\berror:\b|panic|(^|\s)FAILED(\s|$)'
$warningCount = 0
$errorCount = 0
foreach ($path in @($stdoutPath, $stderrPath)) {
  if (Test-Path $path) {
    $content = Get-Content $path -Raw
    if ($null -eq $content) { $content = "" }
    $warningCount += ([regex]::Matches($content, $warningPattern)).Count
    $errorCount += ([regex]::Matches($content, $errorPattern)).Count
  }
}

$passed = $true
if ($ThresholdPath -and (Test-Path $ThresholdPath)) {
  $thresholds = Get-Content $ThresholdPath -Raw | ConvertFrom-Json
  $passed = (
    (($null -eq $result.success_ratio) -or ($result.success_ratio -ge $thresholds.min_success_ratio)) -and
    (($null -eq $result.latency_ms.p95_ms) -or ($result.latency_ms.p95_ms -le $thresholds.max_latency_p95_ms)) -and
    (($null -eq $result.latency_ms.p99_ms) -or ($result.latency_ms.p99_ms -le $thresholds.max_latency_p99_ms)) -and
    ($result.throughput_fps -ge $thresholds.min_throughput_fps)
  )
}

$summary = [pscustomobject]@{
  run_id = $manifest.run_id
  component = $result.component
  backend = $result.backend
  case_name = $result.case_name
  sample_count = $result.sample_count
  throughput_fps = $result.throughput_fps
  success_ratio = $result.success_ratio
  latency_p50_ms = $result.latency_ms.p50_ms
  latency_p95_ms = $result.latency_ms.p95_ms
  latency_p99_ms = $result.latency_ms.p99_ms
  latency_max_ms = $result.latency_ms.max_ms
  access_unit_bytes_p95 = if ($null -ne $result.access_unit_bytes) { $result.access_unit_bytes.p95 } else { $null }
  written_bytes_p95 = if ($null -ne $result.written_bytes) { $result.written_bytes.p95 } else { $null }
  packets_per_sample_p95 = if ($null -ne $result.packets_per_sample) { $result.packets_per_sample.p95 } else { $null }
  frame_bytes = $result.frame_bytes
  warning_count = $warningCount
  error_count = $errorCount
  passed = $passed
}
$summary | Export-Csv -Path $summaryPath -NoTypeInformation -Encoding Ascii

$report = @(
  "# Component Matrix Report",
  "",
  "Run: $($manifest.run_id)",
  "Component: $($result.component)",
  "Backend: $($result.backend)",
  "Case: $($result.case_name)",
  "",
  "| Metric | Value |",
  "| --- | --- |",
  "| throughput_fps | $($result.throughput_fps) |",
  "| success_ratio | $($result.success_ratio) |",
  "| latency_p50_ms | $($result.latency_ms.p50_ms) |",
  "| latency_p95_ms | $($result.latency_ms.p95_ms) |",
  "| latency_p99_ms | $($result.latency_ms.p99_ms) |",
  "| latency_max_ms | $($result.latency_ms.max_ms) |",
  "| access_unit_bytes_p95 | $(if ($null -ne $result.access_unit_bytes) { $result.access_unit_bytes.p95 } else { '' }) |",
  "| written_bytes_p95 | $(if ($null -ne $result.written_bytes) { $result.written_bytes.p95 } else { '' }) |",
  "| packets_per_sample_p95 | $(if ($null -ne $result.packets_per_sample) { $result.packets_per_sample.p95 } else { '' }) |",
  "| frame_bytes | $($result.frame_bytes) |",
  "| warning_count | $warningCount |",
  "| error_count | $errorCount |",
  "| passed | $passed |"
) -join [Environment]::NewLine
$report | Set-Content $reportPath -Encoding Ascii
