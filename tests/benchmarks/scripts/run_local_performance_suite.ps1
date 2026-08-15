param(
  [string]$RepoRoot = ".",
  [string]$OutputRoot = "",
  [string]$PlanPath = "",
  [string]$RunId = "",
  [switch]$SkipEnvironmentProbe
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "local_performance_suite_common.ps1")
. (Join-Path $scriptDir "benchmark_execution_state.ps1")

function Stop-LocalPerformanceProcessTree([int]$ProcessId) {
  $children = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ParentProcessId -eq $ProcessId })
  foreach ($child in $children) {
    Stop-LocalPerformanceProcessTree -ProcessId $child.ProcessId
  }
  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Get-LocalPerformanceEnvironment([string]$ResolvedRepo, [string]$ResolvedRunId) {
  $commit = (& git -C $ResolvedRepo rev-parse --short=12 HEAD).Trim()
  $dirty = @(& git -C $ResolvedRepo status --porcelain).Count -gt 0
  $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
  $computer = Get-CimInstance Win32_ComputerSystem
  $os = Get-CimInstance Win32_OperatingSystem
  $gpuLine = (& nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader,nounits 2>$null |
    Select-Object -First 1)
  $gpuParts = if ($gpuLine) { @($gpuLine -split "," | ForEach-Object { $_.Trim() }) } else { @("unavailable", "unavailable", "0") }
  $gpuMemoryBytes = [uint64]0
  if ($gpuParts.Count -ge 3) {
    $parsedMiB = [uint64]0
    if ([uint64]::TryParse($gpuParts[2], [ref]$parsedMiB)) {
      $gpuMemoryBytes = $parsedMiB * 1MB
    }
  }
  New-LocalPerformanceEnvironmentManifest `
    -RunId $ResolvedRunId `
    -GitCommit $commit `
    -GitDirty $dirty `
    -CpuName $cpu.Name `
    -CpuCores $cpu.NumberOfCores `
    -CpuThreads $cpu.NumberOfLogicalProcessors `
    -RamBytes $computer.TotalPhysicalMemory `
    -OsName $os.Caption `
    -OsBuild $os.BuildNumber `
    -GpuName $gpuParts[0] `
    -GpuDriver $gpuParts[1] `
    -GpuMemoryBytes $gpuMemoryBytes
}

$repo = (Resolve-Path $RepoRoot).Path
if (-not $RunId) {
  $RunId = "local-performance-$((Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss'))"
}
if (-not $OutputRoot) {
  $OutputRoot = Join-Path $repo "artifacts/local-performance/$RunId"
}
$output = [System.IO.Path]::GetFullPath($OutputRoot)
$logs = Join-Path $output "logs"
New-Item -ItemType Directory -Force -Path $logs | Out-Null

$environment = if ($SkipEnvironmentProbe) {
  [pscustomobject]@{
    schema_version = "mrd-local-performance-environment.v1"
    run_id = $RunId
    probe_skipped = $true
  }
} else {
  Get-LocalPerformanceEnvironment -ResolvedRepo $repo -ResolvedRunId $RunId
}
Write-LocalPerformanceJson -InputObject $environment -Path (Join-Path $output "environment.json")

$plan = if ($PlanPath) {
  @(Get-Content -LiteralPath $PlanPath -Raw | ConvertFrom-Json)
} else {
  @(New-LocalPerformanceExecutionPlan -RepoRoot $repo -OutputRoot $output)
}
Write-LocalPerformanceJson -InputObject $plan -Path (Join-Path $output "plan.json")

$hostReadiness = Wait-BenchmarkHostQuiescent -MaxCpuLoadPercent 80 -TimeoutSeconds 30
if (-not $hostReadiness.Ready) {
  throw "benchmark host CPU remained saturated at $($hostReadiness.CpuLoadPercent)% (maximum 80%); stop competing workloads before running performance tests"
}

$executor = {
  param($Item)

  $safeId = ([string]$Item.case_id) -replace "[^A-Za-z0-9_.-]", "_"
  $stdoutPath = Join-Path $logs "$safeId.stdout.log"
  $stderrPath = Join-Path $logs "$safeId.stderr.log"
  $repoQuoted = $repo.Replace("'", "''")
  $stdoutQuoted = $stdoutPath.Replace("'", "''")
  $stderrQuoted = $stderrPath.Replace("'", "''")
  $source = @"
Set-Location -LiteralPath '$repoQuoted'
`$ErrorActionPreference = 'Continue'
& {
  $($Item.command)
} 1> '$stdoutQuoted' 2> '$stderrQuoted'
`$childExitCode = if (`$null -eq `$LASTEXITCODE) { 0 } else { [int]`$LASTEXITCODE }
exit `$childExitCode
"@
  [System.IO.File]::WriteAllText(
    (Join-Path $logs "$safeId.command.ps1"),
    $source,
    (New-Object System.Text.UTF8Encoding($false))
  )
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($source))
  $started = [DateTimeOffset]::UtcNow
  $process = Start-Process -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-EncodedCommand", $encoded) `
    -WindowStyle Hidden `
    -PassThru
  $completed = $process.WaitForExit([int]$Item.timeout_secs * 1000)
  if (-not $completed) {
    Stop-LocalPerformanceProcessTree -ProcessId $process.Id
  }
  $durationMs = [uint64]([DateTimeOffset]::UtcNow - $started).TotalMilliseconds
  $exitCode = if ($completed) { $process.ExitCode } else { $null }
  $expectedArtifact = if ($null -ne $Item.PSObject.Properties["expected_artifact"]) {
    [string]$Item.expected_artifact
  } else {
    ""
  }
  $artifactPath = if ($expectedArtifact) { $expectedArtifact } else { $stdoutPath }
  $artifactValid = if ($completed -and $exitCode -eq 0 -and $expectedArtifact) {
    Test-LocalPerformanceArtifact -Path $artifactPath
  } else {
    $true
  }
  [pscustomobject]@{
    exit_code = $exitCode
    timed_out = -not $completed
    artifact_path = $artifactPath
    artifact_valid = $artifactValid
    duration_ms = $durationMs
  }
}

$executionStateHeld = Enable-BenchmarkExecutionState -DisplayRequired
try {
  $rows = @(Invoke-LocalPerformancePlan -Plan $plan -Executor $executor)
} finally {
  if ($executionStateHeld) {
    Restore-BenchmarkExecutionState
  }
}
$exitCode = Resolve-LocalPerformanceExitCode -Verdicts @($rows.verdict)
$summary = [pscustomobject]@{
  schema_version = "mrd-local-performance-summary.v1"
  run_id = $RunId
  completed_at = [DateTimeOffset]::UtcNow.ToString("o")
  verdict = switch ($exitCode) {
    0 { "PASS" }
    2 { "PRODUCT_FAIL" }
    3 { "INFRA_FAIL" }
    4 { "INVALID_ARTIFACT" }
  }
  exit_code = $exitCode
  rows = $rows
}
Write-LocalPerformanceJson -InputObject $summary -Path (Join-Path $output "summary.json")

$markdown = @(
  "# Local Performance Suite",
  "",
  "- Run: $RunId",
  "- Verdict: $($summary.verdict)",
  "- Rows: $($rows.Count)",
  "",
  "| Phase | Case | Verdict | Duration ms | Artifact |",
  "| --- | --- | --- | ---: | --- |"
)
foreach ($row in $rows) {
  $markdown += "| $($row.phase) | $($row.case_id) | $($row.verdict) | $($row.duration_ms) | $($row.artifact_path) |"
}
$markdown -join "`n" | Set-Content -Path (Join-Path $output "summary.md") -Encoding UTF8

Write-Output "Local performance suite completed: $($summary.verdict)"
Write-Output "Summary: $(Join-Path $output 'summary.json')"
exit $exitCode
