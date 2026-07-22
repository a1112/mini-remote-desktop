function Get-PipelineCompareCargoFeatureArgs {
  param([string]$Codec)

  $codecName = $Codec.ToLowerInvariant()
  switch -Regex ($codecName) {
    '^(software_hevc|software-hevc|hevc_software|hevc-software|software_h265|software-h265|h265_software|h265-software|software_hevc_main10|software-hevc-main10|hevc_main10_software|hevc-main10-software|software_av1|software-av1|av1_software|av1-software)$' {
      return @("--features", "production-software-codecs")
    }
    '^(software_vvc|software-vvc|vvc_software|vvc-software|software_h266|software-h266|h266_software|h266-software)$' {
      return @("--features", "production-vvc-software-codec")
    }
    default {
      return @()
    }
  }
}
