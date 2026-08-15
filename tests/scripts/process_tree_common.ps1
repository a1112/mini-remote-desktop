function Get-ProcessChildIdsCrossPlatform {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
  )

  $isWindows = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
  if ($isWindows -and (Get-Command Get-CimInstance -ErrorAction SilentlyContinue)) {
    return @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
      Where-Object { [int]$_.ParentProcessId -eq $ProcessId } |
      ForEach-Object { [int]$_.ProcessId })
  }

  # PowerShell Core on Linux does not ship the CIM cmdlets. The procfs children
  # file provides the same parent/child relationship for timeout cleanup.
  $childrenPath = "/proc/$ProcessId/task/$ProcessId/children"
  if (-not (Test-Path -LiteralPath $childrenPath)) {
    return @()
  }

  $raw = Get-Content -LiteralPath $childrenPath -Raw -ErrorAction SilentlyContinue
  if ([string]::IsNullOrWhiteSpace($raw)) {
    return @()
  }

  $childIds = @()
  foreach ($token in ($raw -split "\s+")) {
    $childId = 0
    if ([int]::TryParse($token, [ref]$childId) -and $childId -gt 0) {
      $childIds += $childId
    }
  }
  return $childIds
}

function Stop-ProcessTreeCrossPlatform {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
  )

  foreach ($childId in @(Get-ProcessChildIdsCrossPlatform -ProcessId $ProcessId)) {
    Stop-ProcessTreeCrossPlatform -ProcessId $childId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}
