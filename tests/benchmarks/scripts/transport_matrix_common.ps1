function Get-TransportMatrixCargoFeatureArgs {
  param([string]$DecodeBackend)

  $decode = $DecodeBackend.ToLowerInvariant()
  switch -Regex ($decode) {
    '^(software_hevc|hevc_software|software_hevc_main10|hevc_main10_software|software_av1|av1_software)$' {
      return @("--features", "production-software-codecs")
    }
    '^(software_vvc|vvc_software|software_h266|h266_software)$' {
      return @("--features", "production-vvc-software-codec")
    }
    default {
      return @()
    }
  }
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
    [string]$StderrPath
  )

  $previousLocation = (Get-Location).Path
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    Set-Location $WorkingDirectory
    $ErrorActionPreference = "Continue"
    & $FilePath @ArgumentList > $StdoutPath 2> $StderrPath
    return $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
    Set-Location $previousLocation
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
