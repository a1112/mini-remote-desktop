$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "local_performance_suite_common.ps1")

function Assert-Equal($Actual, $Expected, [string]$Message) {
  if ($Actual -ne $Expected) {
    throw "$Message. Expected '$Expected', got '$Actual'."
  }
}

function Assert-True([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

$repo = [System.IO.Path]::GetFullPath((Join-Path $scriptDir "../../.."))
$scenarios = @(Get-LocalPerformanceScenarioPaths -RepoRoot $repo)
$expected = @(Get-ChildItem (Join-Path $repo "tests/benchmarks/scenarios") -Filter "*.json" -File).Count
Assert-Equal $scenarios.Count $expected "Every local transport scenario is discovered"
Assert-True ($scenarios[0] -lt $scenarios[-1]) "Scenario paths are deterministic and sorted"
Assert-True (-not (@($scenarios) -match "public_route")) "Peer/TURN-only canaries are excluded"
Assert-True (Test-BenchmarkCpuLoadAcceptable -CpuLoadPercent 79 -MaxCpuLoadPercent 80) "Host load below the ceiling is accepted"
Assert-True (-not (Test-BenchmarkCpuLoadAcceptable -CpuLoadPercent 95 -MaxCpuLoadPercent 80)) "Saturated hosts are rejected"

$missingDisplayReason = Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ source_id = "windows:display-shared:1" }) `
  -ActiveDisplayCount 1
Assert-Equal $missingDisplayReason "display_source_unavailable" "Inactive display sources are classified as unsupported"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ capture_backend = "dxgi"; encode_backend = "nvenc" }) `
  -ActiveDisplayCount 1 `
  -DxgiOutputAvailable $false) "dxgi_output_unavailable" "Detached DXGI output is unsupported"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ source_id = "windows:display-shared:0" }) `
  -ActiveDisplayCount 1) $null "Active display sources remain supported"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ encode_backend = "software_vvc" }) `
  -ActiveDisplayCount 1 `
  -AvailableCommands @()) "codec_dependency_unavailable" "Missing VVC build dependencies are explicit"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ encode_backend = "software_vvc" }) `
  -ActiveDisplayCount 1 `
  -AvailableCommands @("pkg-config")) "codec_dependency_unavailable" "Missing CMake keeps VVC explicitly unsupported"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ encode_backend = "software_vvc" }) `
  -ActiveDisplayCount 1 `
  -AvailableCommands @("pkg-config", "cmake") `
  -VvcLibraryAvailable $true) $null "VVC remains runnable when all build dependencies are available"
Assert-Equal (Get-LocalPerformanceScenarioSupportReason `
  -Scenario ([pscustomobject]@{ encode_backend = "software_vvc" }) `
  -ActiveDisplayCount 1 `
  -AvailableCommands @("pkg-config", "cmake") `
  -VvcLibraryAvailable $false) "codec_dependency_unavailable" "Missing libvvenc is explicitly unsupported"

$manifest = New-LocalPerformanceEnvironmentManifest `
  -RunId "local-1" `
  -GitCommit "abc123" `
  -GitDirty $true `
  -CpuName "cpu" `
  -CpuCores 14 `
  -CpuThreads 20 `
  -RamBytes 64000000000 `
  -OsName "Windows" `
  -OsBuild "26300" `
  -GpuName "gpu" `
  -GpuDriver "620.02" `
  -GpuMemoryBytes 16000000000
Assert-Equal $manifest.schema_version "mrd-local-performance-environment.v1" "Manifest schema is stable"
Assert-Equal $manifest.cpu.logical_threads 20 "CPU topology is retained"
Assert-Equal $manifest.gpu.driver "620.02" "GPU driver is retained"
Assert-True $manifest.git.dirty "Dirty worktree state is explicit"

$unsupported = New-LocalPerformanceUnsupportedRow `
  -Phase "transport" `
  -CaseId "software-vvc" `
  -ReasonCode "codec_feature_unavailable"
Assert-Equal $unsupported.verdict "UNSUPPORTED" "Unsupported is not reported as PASS"
Assert-Equal $unsupported.reason_code "codec_feature_unavailable" "Unsupported reason is stable"

Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @("PASS")) 0 "PASS exits zero"
Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @("PASS", "UNSUPPORTED")) 0 "Honest unsupported rows do not fail supported rows"
Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @("PRODUCT_FAIL")) 2 "Product failure exits two"
Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @("INFRA_FAIL")) 3 "Infrastructure failure exits three"
Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @("INVALID_ARTIFACT")) 4 "Invalid artifacts dominate"

$plan = @(New-LocalPerformanceExecutionPlan -RepoRoot $repo -OutputRoot "C:\perf-output" -DxgiOutputAvailable $false)
Assert-True ($plan.Count -ge ($expected + 4)) "Plan includes scenarios plus component, integration, and canary phases"
Assert-True (@($plan | Where-Object { $_.phase -eq "transport" }).Count -eq $expected) "Every transport scenario is planned once"
Assert-True (-not (@($plan | Where-Object { $_.command -match "-Debug" }).Count)) "Performance plan never enables debug mode"
Assert-True (@($plan | Where-Object { $_.phase -eq "integration" -and $_.command -match "--release" }).Count -ge 2) "Integration tests use release mode"
Assert-Equal $plan[-1].unsupported_reason "dxgi_output_unavailable" "Local canary is skipped without DXGI output"

$calls = [System.Collections.Generic.List[string]]::new()
$fakePlan = @(
  [pscustomobject]@{ phase = "one"; case_id = "pass"; command = "pass"; timeout_secs = 1 },
  [pscustomobject]@{ phase = "two"; case_id = "fail"; command = "fail"; timeout_secs = 1 },
  [pscustomobject]@{ phase = "two"; case_id = "unsupported"; command = "must-not-run"; timeout_secs = 1; unsupported_reason = "display_source_unavailable" },
  [pscustomobject]@{ phase = "three"; case_id = "timeout"; command = "timeout"; timeout_secs = 1 }
)
$fakeExecutor = {
  param($Item)
  $calls.Add([string]$Item.case_id)
  switch ($Item.case_id) {
    "pass" { [pscustomobject]@{ exit_code = 0; timed_out = $false; artifact_path = "pass.json"; duration_ms = 1 } }
    "fail" { [pscustomobject]@{ exit_code = 2; timed_out = $false; artifact_path = "fail.json"; duration_ms = 2 } }
    "timeout" { [pscustomobject]@{ exit_code = $null; timed_out = $true; artifact_path = $null; duration_ms = 1000 } }
  }
}
$runRows = @(Invoke-LocalPerformancePlan -Plan $fakePlan -Executor $fakeExecutor)
Assert-Equal $calls.Count 3 "Runner continues after a product failure"
Assert-Equal ($calls -join ",") "pass,fail,timeout" "Runner executes serially"
Assert-Equal $runRows[0].verdict "PASS" "Zero exit is PASS"
Assert-Equal $runRows[1].verdict "PRODUCT_FAIL" "Exit two is product failure"
Assert-Equal $runRows[2].verdict "UNSUPPORTED" "Unsupported preconditions are explicit"
Assert-Equal $runRows[3].verdict "INFRA_FAIL" "Timeout is infrastructure failure"
Assert-Equal (Resolve-LocalPerformanceExitCode -Verdicts @($runRows.verdict)) 3 "Timeout dominates aggregate exit"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) "mrd-local-performance-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  $artifact = Join-Path $tmp "summary.json"
  Set-Content -Path $artifact -Value "{}" -Encoding Ascii
  Assert-True (Test-LocalPerformanceArtifact -Path $artifact) "Non-empty artifact is valid"
  Assert-True (-not (Test-LocalPerformanceArtifact -Path (Join-Path $tmp "missing.json"))) "Missing artifact is invalid"
} finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force
}

$runnerTmp = Join-Path ([System.IO.Path]::GetTempPath()) "mrd-local-runner-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $runnerTmp | Out-Null
try {
  $passScript = Join-Path $runnerTmp "pass.ps1"
  $failScript = Join-Path $runnerTmp "fail.ps1"
  $timeoutScript = Join-Path $runnerTmp "timeout.ps1"
  $silentScript = Join-Path $runnerTmp "silent.ps1"
  Set-Content $passScript '[Console]::Error.WriteLine("progress"); Set-Content -Path $args[0] -Value "{}" -Encoding Ascii; exit 0' -Encoding Ascii
  Set-Content $failScript 'Set-Content -Path $args[0] -Value "{}" -Encoding Ascii; exit 2' -Encoding Ascii
  Set-Content $timeoutScript 'Start-Sleep -Seconds 3; exit 0' -Encoding Ascii
  Set-Content $silentScript 'exit 0' -Encoding Ascii
  $passArtifact = Join-Path $runnerTmp "pass.json"
  $failArtifact = Join-Path $runnerTmp "fail.json"
  $planPath = Join-Path $runnerTmp "plan.json"
  @(
    [pscustomobject]@{ phase="fake"; case_id="pass"; command="& '$passScript' '$passArtifact'"; timeout_secs=15; expected_artifact=$passArtifact },
    [pscustomobject]@{ phase="fake"; case_id="fail"; command="& '$failScript' '$failArtifact'"; timeout_secs=15; expected_artifact=$failArtifact },
    [pscustomobject]@{ phase="fake"; case_id="timeout"; command="& '$timeoutScript'"; timeout_secs=1; expected_artifact=$null },
    [pscustomobject]@{ phase="fake"; case_id="silent"; command="& '$silentScript'"; timeout_secs=15 }
  ) | ConvertTo-Json | Set-Content -Path $planPath -Encoding Ascii
  $runner = Join-Path $scriptDir "run_local_performance_suite.ps1"
  & powershell -ExecutionPolicy Bypass -File $runner -RepoRoot $repo -OutputRoot $runnerTmp -PlanPath $planPath -SkipEnvironmentProbe
  Assert-Equal $LASTEXITCODE 3 "Runner returns aggregate infrastructure exit"
  $summaryPath = Join-Path $runnerTmp "summary.json"
  Assert-True (Test-Path $summaryPath) "Runner writes aggregate summary"
  $summary = Get-Content $summaryPath -Raw | ConvertFrom-Json
  Assert-Equal @($summary.rows).Count 4 "Runner records every child result"
  Assert-Equal $summary.rows[0].verdict "PASS" "Runner preserves child PASS"
  Assert-Equal $summary.rows[1].verdict "PRODUCT_FAIL" "Runner preserves child product failure"
  Assert-Equal $summary.rows[2].verdict "INFRA_FAIL" "Runner classifies timeout honestly"
  Assert-Equal $summary.rows[3].verdict "PASS" "Runner permits silent commands without declared artifacts"
} finally {
  Remove-Item -LiteralPath $runnerTmp -Recurse -Force
}

Write-Output "Local performance suite contract tests passed."
