$ErrorActionPreference = "Stop"

function Test-BenchmarkCpuLoadAcceptable {
  param(
    [Parameter(Mandatory = $true)][double]$CpuLoadPercent,
    [double]$MaxCpuLoadPercent = 80
  )

  (-not [double]::IsNaN($CpuLoadPercent)) -and
    (-not [double]::IsInfinity($CpuLoadPercent)) -and
    $CpuLoadPercent -ge 0 -and
    $CpuLoadPercent -le $MaxCpuLoadPercent
}

function Get-BenchmarkCpuLoadPercent {
  try {
    $sample = Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 2 |
      Select-Object -ExpandProperty CounterSamples |
      Select-Object -Last 1
    return [math]::Round([double]$sample.CookedValue, 1)
  } catch {
    return [double](Get-CimInstance Win32_Processor |
      Measure-Object -Property LoadPercentage -Average |
      Select-Object -ExpandProperty Average)
  }
}

function Wait-BenchmarkHostQuiescent {
  param(
    [double]$MaxCpuLoadPercent = 80,
    [int]$TimeoutSeconds = 30
  )

  $deadline = [DateTimeOffset]::UtcNow.AddSeconds([Math]::Max(0, $TimeoutSeconds))
  do {
    $load = Get-BenchmarkCpuLoadPercent
    if (Test-BenchmarkCpuLoadAcceptable -CpuLoadPercent $load -MaxCpuLoadPercent $MaxCpuLoadPercent) {
      return [pscustomobject]@{ Ready = $true; CpuLoadPercent = $load }
    }
    if ([DateTimeOffset]::UtcNow -ge $deadline) {
      return [pscustomobject]@{ Ready = $false; CpuLoadPercent = $load }
    }
    Start-Sleep -Seconds 5
  } while ($true)
}

function Test-BenchmarkDxgiOutputAvailable {
  param([Parameter(Mandatory = $true)][string]$RepoRoot)

  if ($env:OS -ne "Windows_NT" -or $null -eq (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    return $false
  }

  $previousSamples = $env:MRD_COMPONENT_SAMPLES
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    $env:MRD_COMPONENT_SAMPLES = "1"
    $ErrorActionPreference = "Continue"
    & cargo test --quiet --manifest-path (Join-Path $RepoRoot "Cargo.toml") `
      -p mrd-capture-dxgi --test perf_capture `
      perf_dxgi_shared_texture_capture_reports_latency_distribution -- --ignored 1>$null 2>$null
    return $LASTEXITCODE -eq 0
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
    if ($null -eq $previousSamples) {
      Remove-Item Env:MRD_COMPONENT_SAMPLES -ErrorAction SilentlyContinue
    } else {
      $env:MRD_COMPONENT_SAMPLES = $previousSamples
    }
  }
}
