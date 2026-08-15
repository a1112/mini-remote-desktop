$ErrorActionPreference = "Stop"

function Get-BenchmarkExecutionStateFlags {
  param([switch]$DisplayRequired)

  [uint32]$flags = 2147483649 # ES_CONTINUOUS | ES_SYSTEM_REQUIRED
  if ($DisplayRequired) {
    $flags = [uint32]($flags -bor [uint32]0x00000002) # ES_DISPLAY_REQUIRED
  }
  return $flags
}

function Initialize-BenchmarkExecutionStateNativeType {
  if ('MrdBenchmark.NativePower' -as [type]) {
    return
  }

  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace MrdBenchmark {
  public static class NativePower {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint SetThreadExecutionState(uint executionState);
  }
}
"@ | Out-Null
}

function Enable-BenchmarkExecutionState {
  param([switch]$DisplayRequired)

  if ($env:OS -ne 'Windows_NT') {
    return $false
  }

  Initialize-BenchmarkExecutionStateNativeType
  $flags = Get-BenchmarkExecutionStateFlags -DisplayRequired:$DisplayRequired
  $previous = [MrdBenchmark.NativePower]::SetThreadExecutionState($flags)
  if ($previous -eq 0) {
    Write-Warning "Unable to keep the benchmark host awake (Win32 error $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))."
    return $false
  }
  return $true
}

function Restore-BenchmarkExecutionState {
  if ($env:OS -ne 'Windows_NT') {
    return
  }

  Initialize-BenchmarkExecutionStateNativeType
  [void][MrdBenchmark.NativePower]::SetThreadExecutionState([uint32]2147483648)
}
