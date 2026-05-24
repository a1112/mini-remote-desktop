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

function Assert-TransportMatrixSummaryPassed {
  param(
    [Parameter(Mandatory = $true)]
    [string]$SummaryPath
  )

  $summary = Get-Content $SummaryPath -Raw | ConvertFrom-Json
  if (-not $summary.run_passed) {
    throw "transport matrix failed thresholds for $($summary.scenario)/$($summary.profile). See $SummaryPath"
  }
}
