param(
  [Parameter(Mandatory = $true)]
  [string]$CasePath,
  [string]$RepoRoot = ".",
  [int]$TimeoutSeconds = 300
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "component_matrix_common.ps1")
. (Join-Path $scriptDir "../../benchmarks/scripts/benchmark_host_capabilities.ps1")

$repo = (Resolve-Path $RepoRoot).Path
$caseFile = Join-Path $repo $CasePath
$case = Get-Content $caseFile -Raw | ConvertFrom-Json
$requiresDxgiOutput = $null -ne $case.PSObject.Properties["requires_dxgi_output"] -and
  [bool]$case.requires_dxgi_output
$dxgiOutputAvailable = if ($requiresDxgiOutput) {
  Test-BenchmarkDxgiOutputAvailable -RepoRoot $repo
} else {
  $true
}
$unsupportedReason = Get-ComponentMatrixUnsupportedReason `
  -Case $case `
  -DxgiOutputAvailable $dxgiOutputAvailable
if ($unsupportedReason) {
  Write-Output "Component case skipped: $($case.case_name) ($unsupportedReason)"
  exit 0
}
$gitCommit = (git -C $repo rev-parse --short HEAD).Trim()
$date = Get-Date -Format 'yyyy-MM-dd'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$runId = "$($case.case_name)-$timestamp-$gitCommit"
$runDir = Join-Path $repo ("artifacts/component-matrix/{0}/{1}/{2}" -f $date, $case.component, $runId)
$logsDir = Join-Path $runDir "logs"
$reportsDir = Join-Path $runDir "reports"
$null = New-Item -ItemType Directory -Force -Path $logsDir, $reportsDir

$stdoutPath = Join-Path $logsDir "component.stdout.log"
$stderrPath = Join-Path $logsDir "component.stderr.log"
$resultPath = Join-Path $runDir "result.json"
$manifestPath = Join-Path $runDir "manifest.json"
$thresholdPath = Join-Path $repo ("tests/component-matrix/thresholds/{0}" -f $case.threshold_file)

$manifest = [pscustomobject]@{
  run_id = $runId
  component = $case.component
  crate = $case.crate
  backend = $case.backend
  case_name = $case.case_name
  sample_count = $case.sample_count
  git_commit = $gitCommit
  timestamp = $timestamp
}
$manifest | ConvertTo-Json -Depth 6 | Set-Content $manifestPath -Encoding Ascii

$env:MRD_COMPONENT_CASE_NAME = $case.case_name
$env:MRD_COMPONENT_SAMPLES = [string]$case.sample_count
$env:MRD_COMPONENT_RESULT_PATH = $resultPath
$env:MRD_COMPONENT_BACKEND = $case.backend

$result = Invoke-ComponentMatrixCommand `
  -FilePath "cargo" `
  -ArgumentList @("test", "-p", $case.crate, $case.test_name, "--", "--ignored", "--nocapture") `
  -WorkingDirectory $repo `
  -StdoutPath $stdoutPath `
  -StderrPath $stderrPath `
  -TimeoutSeconds $TimeoutSeconds

$summaryWritten = Invoke-ComponentMatrixSummaryIfAvailable `
  -RunDir $runDir `
  -ThresholdPath $thresholdPath `
  -SummarizerPath (Join-Path $repo 'tests/component-matrix/scripts/summarize_component_results.ps1')

if ($result.ExitCode -ne 0) {
  if ($result.TimedOut) {
    Write-Error "component test timed out after ${TimeoutSeconds}s. See $stderrPath"
    exit 3
  }
  Write-Error "component test failed with exit code $($result.ExitCode). See $stderrPath"
  exit 2
}

if (-not $summaryWritten) {
  Write-Error "component test did not write result.json at $resultPath"
  exit 2
}

$summary = Import-Csv (Join-Path $runDir 'summary.csv')
if ($summary.passed -ne 'True') {
  Write-Error "component quality gate failed for $($case.case_name). See $runDir/summary.csv and reports/markdown-report.md"
  exit 2
}

Write-Output "Component case completed."
Write-Output "Run directory: $runDir"
