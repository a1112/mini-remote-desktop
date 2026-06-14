function Get-ComponentChildProcessIds {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ParentProcessId
  )

  if (Get-Command Get-CimInstance -ErrorAction SilentlyContinue) {
    return @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
      Where-Object { $_.ParentProcessId -eq $ParentProcessId } |
      ForEach-Object { [int]$_.ProcessId })
  }

  $pgrep = Get-Command pgrep -ErrorAction SilentlyContinue
  if ($pgrep) {
    return @(& $pgrep.Source -P $ParentProcessId 2> $null |
      Where-Object { $_ -match '^\d+$' } |
      ForEach-Object { [int]$_ })
  }

  return @()
}

function Stop-ComponentProcessTree {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
  )

  foreach ($childProcessId in @(Get-ComponentChildProcessIds -ParentProcessId $ProcessId)) {
    Stop-ComponentProcessTree -ProcessId $childProcessId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Resolve-ComponentMatrixPowerShellExecutable {
  $currentProcess = Get-Process -Id $PID -ErrorAction SilentlyContinue
  if ($currentProcess -and -not [string]::IsNullOrWhiteSpace($currentProcess.Path)) {
    return $currentProcess.Path
  }

  foreach ($candidate in @("pwsh", "powershell", "powershell.exe")) {
    $command = Get-Command $candidate -ErrorAction SilentlyContinue
    if ($command -and -not [string]::IsNullOrWhiteSpace($command.Source)) {
      return $command.Source
    }
  }

  throw "Unable to find a PowerShell executable. Install PowerShell 7 (pwsh) or Windows PowerShell."
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
    $jobProcessId = $job.ChildJobs[0].ProcessId
    if ($null -ne $jobProcessId) {
      foreach ($childProcessId in @(Get-ComponentChildProcessIds -ParentProcessId $jobProcessId)) {
        Stop-ComponentProcessTree -ProcessId $childProcessId
      }
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
  $powershell = Resolve-ComponentMatrixPowerShellExecutable
  & $powershell @args
  if ($LASTEXITCODE -ne 0) {
    throw "component summary failed with exit code $LASTEXITCODE for $RunDir"
  }
  return $true
}
