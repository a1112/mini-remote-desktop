param(
  [string]$ScenarioPath = "tests/benchmarks/scenarios/quick.transport.json",
  [string]$RepoRoot = "."
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path $RepoRoot).Path
$scenarioFile = Join-Path $repo $ScenarioPath
$scenario = Get-Content $scenarioFile -Raw | ConvertFrom-Json
$gitCommit = (git -C $repo rev-parse --short HEAD).Trim()
$date = Get-Date -Format 'yyyy-MM-dd'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$runId = "$($scenario.scenario)-$($scenario.transport)-$timestamp-$gitCommit"
$runDir = Join-Path $repo ("artifacts/benchmarks/{0}/{1}/{2}" -f $date, $scenario.profile, $runId)
$logsDir = Join-Path $runDir "logs"
$reportsDir = Join-Path $runDir "reports"
$sessionsDir = Join-Path $runDir "sessions"

New-Item -ItemType Directory -Force -Path $logsDir, $reportsDir, $sessionsDir | Out-Null
New-Item -ItemType File -Force -Path (Join-Path $logsDir 'signaling.stdout.log') | Out-Null
New-Item -ItemType File -Force -Path (Join-Path $logsDir 'signaling.stderr.log') | Out-Null

$hostStdout = Join-Path $logsDir 'host.stdout.log'
$hostStderr = Join-Path $logsDir 'host.stderr.log'
$thresholdPath = Join-Path $repo ("tests/benchmarks/thresholds/{0}" -f $scenario.threshold_file)

$env:MRD_BENCH_ARTIFACT_ROOT = $repo
$env:MRD_BENCH_SCENARIO = $scenario.scenario
$env:MRD_BENCH_PROFILE = $scenario.profile
$env:MRD_BENCH_RUN_ID = $runId
$env:MRD_BENCH_DATE = $date
$env:MRD_BENCH_WIDTH = [string]$scenario.width
$env:MRD_BENCH_HEIGHT = [string]$scenario.height
$env:MRD_BENCH_FPS = [string]$scenario.fps
$env:MRD_BENCH_DURATION_SECS = [string]$scenario.duration_secs
$env:MRD_BENCH_GIT_COMMIT = $gitCommit
$env:MRD_BENCH_TRANSPORT = $scenario.transport
$env:MRD_BENCH_CAPTURE_BACKEND = $scenario.capture_backend
$env:MRD_BENCH_ENCODE_BACKEND = $scenario.encode_backend
$env:MRD_BENCH_DECODE_BACKEND = $scenario.decode_backend
$env:MRD_BENCH_RENDERER_BACKEND = $scenario.renderer_backend

$process = Start-Process `
  -FilePath "cargo" `
  -ArgumentList @("test", "-p", "app", "benchmark_run_writes_requested_artifacts", "--", "--nocapture") `
  -WorkingDirectory $repo `
  -RedirectStandardOutput $hostStdout `
  -RedirectStandardError $hostStderr `
  -WindowStyle Hidden `
  -Wait `
  -PassThru

if ($process.ExitCode -ne 0) {
  throw "benchmark cargo test failed with exit code $($process.ExitCode). See $hostStderr"
}

powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tests/benchmarks/scripts/summarize_transport_results.ps1') `
  -RunDir $runDir `
  -ThresholdPath $thresholdPath

Write-Output "Benchmark run completed."
Write-Output "Run directory: $runDir"
Write-Output "Summary: $(Join-Path $runDir 'summary.json')"
