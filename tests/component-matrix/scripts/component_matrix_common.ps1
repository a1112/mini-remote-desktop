function Get-ComponentMatrixUnsupportedReason {
  param(
    [Parameter(Mandatory = $true)]$Case,
    [Parameter(Mandatory = $true)][bool]$DxgiOutputAvailable
  )

  $requiresDxgiOutput = $null -ne $Case.PSObject.Properties["requires_dxgi_output"] -and
    [bool]$Case.requires_dxgi_output
  if ($requiresDxgiOutput -and -not $DxgiOutputAvailable) {
    return "dxgi_output_unavailable"
  }
  return $null
}

function Get-CurrentPowerShellExecutable {
  $path = (Get-Process -Id $PID -ErrorAction Stop).Path
  if ([string]::IsNullOrWhiteSpace($path)) {
    throw "Unable to resolve the current PowerShell executable"
  }
  return $path
}

function Stop-ComponentProcessTree {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
  )

  $children = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ParentProcessId -eq $ProcessId })
  foreach ($child in $children) {
    Stop-ComponentProcessTree -ProcessId $child.ProcessId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Invoke-ComponentMatrixCommand {
  param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory = $true)]
    [string]$WorkingDirectory,
    [Parameter(Mandatory = $true)]
    [string]$StdoutPath,
    [Parameter(Mandatory = $true)]
    [string]$StderrPath,
    [int]$TimeoutSeconds = 300
  )

  New-Item -ItemType File -Force -Path $StdoutPath | Out-Null
  New-Item -ItemType File -Force -Path $StderrPath | Out-Null

  $job = Start-Job -ScriptBlock {
    param($FilePath, $ArgumentList, $WorkingDirectory, $StdoutPath, $StderrPath)
    Set-Location $WorkingDirectory
    & $FilePath @ArgumentList > $StdoutPath 2> $StderrPath
    if ($null -ne $LASTEXITCODE) {
      return $LASTEXITCODE
    }
    return 0
  } -ArgumentList $FilePath, $ArgumentList, $WorkingDirectory, $StdoutPath, $StderrPath

  $completed = Wait-Job -Job $job -Timeout ([Math]::Max(1, $TimeoutSeconds))
  if ($null -eq $completed) {
    $childProcesses = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
      Where-Object { $_.ParentProcessId -eq $job.ChildJobs[0].ProcessId })
    foreach ($child in $childProcesses) {
      Stop-ComponentProcessTree -ProcessId $child.ProcessId
    }
    Stop-Job -Job $job -ErrorAction SilentlyContinue
    Remove-Job -Job $job -Force -ErrorAction SilentlyContinue
    return [pscustomobject]@{
      ExitCode = 124
      TimedOut = $true
    }
  }

  $output = @(Receive-Job -Job $job)
  Remove-Job -Job $job -Force -ErrorAction SilentlyContinue
  $exitCode = if ($output.Count -gt 0) { [int]$output[-1] } else { 0 }
  return [pscustomobject]@{
    ExitCode = $exitCode
    TimedOut = $false
  }
}

function Invoke-ComponentMatrixSummaryIfAvailable {
  param(
    [Parameter(Mandatory = $true)]
    [string]$RunDir,
    [string]$ThresholdPath,
    [Parameter(Mandatory = $true)]
    [string]$SummarizerPath
  )

  $resultPath = Join-Path $RunDir "result.json"
  if (-not (Test-Path $resultPath)) {
    return $false
  }

  $args = @("-ExecutionPolicy", "Bypass", "-File", $SummarizerPath, "-RunDir", $RunDir)
  if ($ThresholdPath) {
    $args += @("-ThresholdPath", $ThresholdPath)
  }
  & (Get-CurrentPowerShellExecutable) @args
  if ($LASTEXITCODE -ne 0) {
    throw "component summary failed with exit code $LASTEXITCODE for $RunDir"
  }
  return $true
}
