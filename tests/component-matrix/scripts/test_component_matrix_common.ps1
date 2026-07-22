$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "component_matrix_common.ps1")

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

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-component-common-{0}" -f ([guid]::NewGuid()))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $stdout = Join-Path $tmp "stdout.log"
  $stderr = Join-Path $tmp "stderr.log"
  $result = Invoke-ComponentMatrixCommand `
    -FilePath "powershell" `
    -ArgumentList @("-NoProfile", "-Command", "Write-Output 'stdout-ok'; [Console]::Error.WriteLine('stderr-ok'); exit 7") `
    -WorkingDirectory $tmp `
    -StdoutPath $stdout `
    -StderrPath $stderr `
    -TimeoutSeconds 30

  Assert-Equal $result.ExitCode 7 "Invoke-ComponentMatrixCommand returns native exit code"
  Assert-True (-not $result.TimedOut) "Non-timeout command should not be marked timed out"
  Assert-True ((Get-Content $stdout -Raw) -match "stdout-ok") "stdout should be captured"
  Assert-True ((Get-Content $stderr -Raw) -match "stderr-ok") "stderr should be captured"

  $timeoutStdout = Join-Path $tmp "timeout.stdout.log"
  $timeoutStderr = Join-Path $tmp "timeout.stderr.log"
  $timeout = Invoke-ComponentMatrixCommand `
    -FilePath "powershell" `
    -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Seconds 10") `
    -WorkingDirectory $tmp `
    -StdoutPath $timeoutStdout `
    -StderrPath $timeoutStderr `
    -TimeoutSeconds 1

  Assert-Equal $timeout.ExitCode 124 "Timed out command should return 124"
  Assert-True $timeout.TimedOut "Timed out command should report TimedOut"

  $runDir = Join-Path $tmp "run"
  New-Item -ItemType Directory -Force -Path (Join-Path $runDir "logs") | Out-Null
  New-Item -ItemType Directory -Force -Path (Join-Path $runDir "reports") | Out-Null
  [ordered]@{
    run_id = "decode.ffmpeg_h264-test"
    component = "decode"
    crate = "mrd-decode"
    backend = "ffmpeg_h264"
    case_name = "decode.ffmpeg_h264"
    sample_count = 60
    git_commit = "abc123"
    timestamp = "20260529-000000"
  } | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $runDir "manifest.json") -Encoding Ascii
  [ordered]@{
    component = "Decode"
    backend = "ffmpeg_h264"
    case_name = "decode.ffmpeg_h264"
    sample_count = 60
    duration_sec = 1.0
    success_count = 60
    failure_count = 0
    throughput_fps = 60.0
    latency_ms = [ordered]@{
      count = 60
      p50_ms = 0.2
      p95_ms = 0.4
      p99_ms = 0.8
      max_ms = 1.0
    }
    success_ratio = 1.0
    frame_bytes = $null
  } | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $runDir "result.json") -Encoding Ascii
  New-Item -ItemType File -Force -Path (Join-Path $runDir "logs/component.stdout.log") | Out-Null
  New-Item -ItemType File -Force -Path (Join-Path $runDir "logs/component.stderr.log") | Out-Null

  $null = Invoke-ComponentMatrixSummaryIfAvailable `
    -RunDir $runDir `
    -ThresholdPath (Join-Path $scriptDir "../thresholds/decode.json") `
    -SummarizerPath (Join-Path $scriptDir "summarize_component_results.ps1")

  Assert-True (Test-Path (Join-Path $runDir "summary.csv")) "summary.csv should be written when result.json exists"
  $summary = Import-Csv (Join-Path $runDir "summary.csv")
  Assert-Equal $summary.passed "True" "summary should evaluate thresholds"

  foreach ($fixtureName in @('threshold-failure-result.json', 'null-latency-result.json')) {
    $fixtureRun = Join-Path $tmp ([IO.Path]::GetFileNameWithoutExtension($fixtureName))
    New-Item -ItemType Directory -Force -Path (Join-Path $fixtureRun 'logs'), (Join-Path $fixtureRun 'reports') | Out-Null
    Copy-Item (Join-Path $scriptDir "../fixtures/$fixtureName") (Join-Path $fixtureRun 'result.json')
    Copy-Item (Join-Path $runDir 'manifest.json') (Join-Path $fixtureRun 'manifest.json')
    New-Item -ItemType File -Force -Path (Join-Path $fixtureRun 'logs/component.stdout.log'), (Join-Path $fixtureRun 'logs/component.stderr.log') | Out-Null
    & powershell -ExecutionPolicy Bypass -File (Join-Path $scriptDir 'summarize_component_results.ps1') -RunDir $fixtureRun -ThresholdPath (Join-Path $scriptDir '../thresholds/decode.json')
    $fixtureSummary = Import-Csv (Join-Path $fixtureRun 'summary.csv')
    Assert-Equal $fixtureSummary.passed "False" "$fixtureName should fail closed"
  }
} finally {
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
