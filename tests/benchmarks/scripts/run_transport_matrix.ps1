param(
  [string]$ScenarioPath = "tests/benchmarks/scenarios/quick.transport.json",
  [string]$RepoRoot = ".",
  [int]$TimeoutSeconds = 300,
  [switch]$Debug
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path $RepoRoot).Path
. (Join-Path $repo 'tests/benchmarks/scripts/transport_matrix_common.ps1')
$scenarioFile = Join-Path $repo $ScenarioPath
$scenario = Get-Content $scenarioFile -Raw | ConvertFrom-Json
$gitCommit = (git -C $repo rev-parse --short HEAD).Trim()
$date = Get-Date -Format 'yyyy-MM-dd'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$cargoProfile = if ($Debug) { "debug" } else { "release" }
$runId = "$($scenario.scenario)-$($scenario.transport)-$cargoProfile-$timestamp-$gitCommit"
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
$cargoArgs = Get-TransportMatrixCargoTestArgs `
  -EncodeBackend $scenario.encode_backend `
  -DecodeBackend $scenario.decode_backend `
  -Release:(-not $Debug)

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
$bitrateBps = Get-TransportMatrixBitrateBps -Scenario $scenario
if ($null -ne $bitrateBps) {
  $env:MRD_BENCH_BITRATE_BPS = $bitrateBps
} else {
  Remove-Item Env:MRD_BENCH_BITRATE_BPS -ErrorAction SilentlyContinue
}
if (@($scenario.PSObject.Properties.Name) -contains "pace_to_fps") {
  $env:MRD_BENCH_PACE_TO_FPS = [string]$scenario.pace_to_fps
} else {
  Remove-Item Env:MRD_BENCH_PACE_TO_FPS -ErrorAction SilentlyContinue
}
if (@($scenario.PSObject.Properties.Name) -contains "color_mode") {
  $env:MRD_BENCH_COLOR_MODE = [string]$scenario.color_mode
} else {
  Remove-Item Env:MRD_BENCH_COLOR_MODE -ErrorAction SilentlyContinue
}
if (@($scenario.PSObject.Properties.Name) -contains "color_pipeline") {
  $env:MRD_BENCH_COLOR_PIPELINE = [string]$scenario.color_pipeline
} else {
  Remove-Item Env:MRD_BENCH_COLOR_PIPELINE -ErrorAction SilentlyContinue
}
$sourceEnvironment = Get-TransportMatrixSourceEnvironment -Scenario $scenario
foreach ($key in @("MRD_BENCH_SOURCE_ID", "MRD_BENCH_DISPLAY_ID")) {
  if ($sourceEnvironment.ContainsKey($key)) {
    Set-Item -Path "Env:$key" -Value $sourceEnvironment[$key]
  } else {
    Remove-Item -Path "Env:$key" -ErrorAction SilentlyContinue
  }
}
$av1Mode = Get-TransportMatrixAv1Mode -Scenario $scenario
if ($null -ne $av1Mode) {
  $env:MRD_BENCH_NVENC_AV1_MODE = $av1Mode
} else {
  Remove-Item Env:MRD_BENCH_NVENC_AV1_MODE -ErrorAction SilentlyContinue
}
$renderEnvironment = Get-TransportMatrixRenderEnvironment -Scenario $scenario
foreach ($key in @("MRD_D3D11_RENDER_WAITABLE_OBJECT", "MRD_RENDER_THREAD_PRIORITY", "MRD_OPENGL_ALLOW_READBACK_FALLBACK")) {
  if ($renderEnvironment.ContainsKey($key)) {
    Set-Item -Path "Env:$key" -Value $renderEnvironment[$key]
  } else {
    Remove-Item -Path "Env:$key" -ErrorAction SilentlyContinue
  }
}

$exitCode = Invoke-TransportMatrixCommand `
  -FilePath "cargo" `
  -ArgumentList $cargoArgs `
  -WorkingDirectory $repo `
  -StdoutPath $hostStdout `
  -StderrPath $hostStderr `
  -TimeoutSeconds $TimeoutSeconds

if ($exitCode.TimedOut) {
  throw "benchmark cargo test timed out after $TimeoutSeconds seconds. See $hostStdout and $hostStderr"
}

if ($exitCode.ExitCode -ne 0) {
  throw "benchmark cargo test failed with exit code $($exitCode.ExitCode). See $hostStderr"
}

$powershell = Resolve-TransportMatrixPowerShellExecutable
& $powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tests/benchmarks/scripts/summarize_transport_results.ps1') `
  -RunDir $runDir `
  -ThresholdPath $thresholdPath

Assert-TransportMatrixSummaryPassed -SummaryPath (Join-Path $runDir 'summary.json')

Write-Output "Benchmark run completed."
Write-Output "Run directory: $runDir"
Write-Output "Summary: $(Join-Path $runDir 'summary.json')"
