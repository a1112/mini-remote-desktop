param(
  [Parameter(Mandatory = $true)]
  [string]$CasePath,
  [string]$RepoRoot = "."
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path $RepoRoot).Path
$caseFile = Join-Path $repo $CasePath
$case = Get-Content $caseFile -Raw | ConvertFrom-Json
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

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = "cargo"
$psi.WorkingDirectory = $repo
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.UseShellExecute = $false
$psi.Arguments = "test -p $($case.crate) $($case.test_name) -- --ignored --nocapture"
$psi.Environment["MRD_COMPONENT_CASE_NAME"] = $case.case_name
$psi.Environment["MRD_COMPONENT_SAMPLES"] = [string]$case.sample_count
$psi.Environment["MRD_COMPONENT_RESULT_PATH"] = $resultPath

$process = New-Object System.Diagnostics.Process
$process.StartInfo = $psi
$null = $process.Start()
$stdout = $process.StandardOutput.ReadToEnd()
$stderr = $process.StandardError.ReadToEnd()
$process.WaitForExit()

Set-Content -Path $stdoutPath -Value $stdout -Encoding Ascii
Set-Content -Path $stderrPath -Value $stderr -Encoding Ascii

if ($process.ExitCode -ne 0) {
  throw "component test failed with exit code $($process.ExitCode). See $stderrPath"
}

powershell -ExecutionPolicy Bypass -File (Join-Path $repo 'tests/component-matrix/scripts/summarize_component_results.ps1') `
  -RunDir $runDir `
  -ThresholdPath $thresholdPath

Write-Output "Component case completed."
Write-Output "Run directory: $runDir"
