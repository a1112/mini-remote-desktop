param(
  [Parameter(Mandatory = $true)]
  [string]$LogFile,
  [double]$P95ThresholdMs = 12.0,
  [double]$P99ThresholdMs = 18.0,
  [int]$MinSamples = 100
)

if (!(Test-Path $LogFile)) {
  Write-Error "Log file not found: $LogFile"
  exit 2
}

$lines = Get-Content $LogFile | Where-Object { $_ -match "\[CTRL-LAT\]" }
if (-not $lines -or $lines.Count -eq 0) {
  Write-Error "No [CTRL-LAT] lines found in log: $LogFile"
  exit 2
}

$worstP95 = 0.0
$worstP99 = 0.0
$maxSamples = 0

foreach ($line in $lines) {
  if ($line -match "one_way_p95_ms[=\s]+(?<p95>[0-9]+(\.[0-9]+)?)") {
    $p95 = [double]$Matches["p95"]
    if ($p95 -gt $worstP95) { $worstP95 = $p95 }
  }
  if ($line -match "one_way_p99_ms[=\s]+(?<p99>[0-9]+(\.[0-9]+)?)") {
    $p99 = [double]$Matches["p99"]
    if ($p99 -gt $worstP99) { $worstP99 = $p99 }
  }
  if ($line -match "samples[=\s]+(?<n>[0-9]+)") {
    $n = [int]$Matches["n"]
    if ($n -gt $maxSamples) { $maxSamples = $n }
  }
}

Write-Host "control-latency summary:"
Write-Host "  worst one_way_p95_ms = $worstP95"
Write-Host "  worst one_way_p99_ms = $worstP99"
Write-Host "  max samples window   = $maxSamples"
Write-Host "  thresholds           = p95<$P95ThresholdMs p99<$P99ThresholdMs"

if ($maxSamples -lt $MinSamples) {
  Write-Error "Insufficient samples: $maxSamples < $MinSamples"
  exit 1
}

if ($worstP95 -ge $P95ThresholdMs) {
  Write-Error "SLO failed: one_way_p95_ms=$worstP95 (threshold $P95ThresholdMs)"
  exit 1
}

if ($worstP99 -ge $P99ThresholdMs) {
  Write-Error "SLO failed: one_way_p99_ms=$worstP99 (threshold $P99ThresholdMs)"
  exit 1
}

Write-Host "SLO PASS"
exit 0

