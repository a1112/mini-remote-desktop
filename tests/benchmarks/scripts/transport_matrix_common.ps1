function Get-TransportMatrixCargoFeatureArgs {
  param(
    [string]$EncodeBackend = "",
    [string]$DecodeBackend = ""
  )

  $encode = $EncodeBackend.ToLowerInvariant()
  $decode = $DecodeBackend.ToLowerInvariant()
  $softwareCodecsPattern = '^(software_hevc|hevc_software|software_hevc_main10|hevc_main10_software|software_av1|av1_software)$'
  $vvcPattern = '^(software_vvc|vvc_software|software_h266|h266_software|software-vvc|vvc-software|software-h266|h266-software|vvenc|vvc|h266|h\.266)$'
  $features = @()

  switch -Regex ($decode) {
    $softwareCodecsPattern {
      $features += "production-software-codecs"
    }
    $vvcPattern {
      $features += "production-vvc-software-codec"
    }
  }

  if ($features -notcontains "production-vvc-software-codec" -and $encode -match $vvcPattern) {
    $features += "mrd-encode-vvenc/software-vvenc"
  }

  if ($features.Count -eq 0) {
    return @()
  }

  $uniqueFeatures = @()
  foreach ($feature in $features) {
    if ($uniqueFeatures -notcontains $feature) {
      $uniqueFeatures += $feature
    }
  }
  return @("--features", ($uniqueFeatures -join ","))
}

function Resolve-TransportMatrixPowerShellExecutable {
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

function Get-TransportMatrixCargoTestArgs {
  param(
    [string]$EncodeBackend = "",
    [string]$DecodeBackend = "",
    [bool]$Release = $true
  )

  $args = @("test")
  if ($Release) {
    $args += "--release"
  }
  $args += @("-p", "app")
  $args += Get-TransportMatrixCargoFeatureArgs -EncodeBackend $EncodeBackend -DecodeBackend $DecodeBackend
  $args += @("benchmark_run_writes_requested_artifacts", "--", "--nocapture")
  return $args
}

function Get-TransportMatrixBitrateBps {
  param([object]$Scenario)

  $propertyNames = @($Scenario.PSObject.Properties.Name)
  if ($propertyNames -contains "bitrate_bps" -and $null -ne $Scenario.bitrate_bps) {
    $bitrateBps = [int64]$Scenario.bitrate_bps
    if ($bitrateBps -le 0) {
      throw "scenario bitrate_bps must be greater than zero"
    }
    return [string]$bitrateBps
  }

  if ($propertyNames -contains "bitrate_mbps" -and $null -ne $Scenario.bitrate_mbps) {
    $bitrateMbps = [double]$Scenario.bitrate_mbps
    if ($bitrateMbps -le 0) {
      throw "scenario bitrate_mbps must be greater than zero"
    }
    return [string][int64]($bitrateMbps * 1000000)
  }

  return $null
}

function Get-TransportMatrixSourceEnvironment {
  param([object]$Scenario)

  $result = @{}
  $propertyNames = @($Scenario.PSObject.Properties.Name)
  if (
    $propertyNames -contains "source_id" -and
    -not [string]::IsNullOrWhiteSpace([string]$Scenario.source_id)
  ) {
    $result.MRD_BENCH_SOURCE_ID = ([string]$Scenario.source_id).Trim()
  }
  if (
    $propertyNames -contains "display_id" -and
    -not [string]::IsNullOrWhiteSpace([string]$Scenario.display_id)
  ) {
    $result.MRD_BENCH_DISPLAY_ID = ([string]$Scenario.display_id).Trim()
  }
  return $result
}

function Get-TransportMatrixAv1Mode {
  param([object]$Scenario)

  $propertyNames = @($Scenario.PSObject.Properties.Name)
  $encodeBackend = if ($propertyNames -contains "encode_backend") {
    ([string]$Scenario.encode_backend).ToLowerInvariant()
  } else {
    ""
  }
  if ($encodeBackend -notin @("nvenc_av1", "av1_nvenc", "nvenc-av1")) {
    return $null
  }

  if (
    $propertyNames -contains "av1_mode" -and
    -not [string]::IsNullOrWhiteSpace([string]$Scenario.av1_mode)
  ) {
    $mode = ([string]$Scenario.av1_mode).Trim()
    if ($mode -notin @("low_latency", "ultra_low_latency", "high_refresh")) {
      throw "scenario av1_mode must be low_latency, ultra_low_latency, or high_refresh"
    }
    return $mode
  }

  $fps = if ($propertyNames -contains "fps" -and $null -ne $Scenario.fps) {
    [int]$Scenario.fps
  } else {
    0
  }
  if ($fps -ge 120) {
    return "high_refresh"
  }

  return $null
}

function Get-TransportMatrixRenderEnvironment {
  param([object]$Scenario)

  $result = @{}
  $propertyNames = @($Scenario.PSObject.Properties.Name)
  $rendererBackend = if ($propertyNames -contains "renderer_backend") {
    ([string]$Scenario.renderer_backend).ToLowerInvariant()
  } else {
    ""
  }
  $fps = if ($propertyNames -contains "fps" -and $null -ne $Scenario.fps) {
    [int]$Scenario.fps
  } else {
    0
  }
  $highRefreshD3d11 = $fps -ge 120 -and $rendererBackend -match '^d3d11'

  if ($propertyNames -contains "d3d11_waitable_object") {
    $result.MRD_D3D11_RENDER_WAITABLE_OBJECT = if ($Scenario.d3d11_waitable_object) { "1" } else { "0" }
  } elseif ($highRefreshD3d11) {
    $result.MRD_D3D11_RENDER_WAITABLE_OBJECT = "1"
  }
  if (
    $propertyNames -contains "render_thread_priority" -and
    -not [string]::IsNullOrWhiteSpace([string]$Scenario.render_thread_priority)
  ) {
    $result.MRD_RENDER_THREAD_PRIORITY = [string]$Scenario.render_thread_priority
  } elseif ($highRefreshD3d11) {
    $result.MRD_RENDER_THREAD_PRIORITY = "above_normal"
  }
  if ($propertyNames -contains "opengl_allow_readback_fallback") {
    $result.MRD_OPENGL_ALLOW_READBACK_FALLBACK = if ($Scenario.opengl_allow_readback_fallback) { "1" } else { "0" }
  }
  return $result
}

function Get-TransportChildProcessIds {
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

function Stop-TransportProcessTree {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
  )

  foreach ($childProcessId in @(Get-TransportChildProcessIds -ParentProcessId $ProcessId)) {
    Stop-TransportProcessTree -ProcessId $childProcessId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Invoke-TransportMatrixCommand {
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
      foreach ($childProcessId in @(Get-TransportChildProcessIds -ParentProcessId $jobProcessId)) {
        Stop-TransportProcessTree -ProcessId $childProcessId
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

function Assert-TransportMatrixSummaryPassed {
  param(
    [Parameter(Mandatory = $true)]
    [string]$SummaryPath
  )

  $summary = Get-Content $SummaryPath -Raw | ConvertFrom-Json
  if ($summary.run_skipped) {
    return
  }
  if (-not $summary.run_passed) {
    throw "transport matrix failed thresholds for $($summary.scenario)/$($summary.profile). See $SummaryPath"
  }
}
