param(
  [Parameter(Mandatory = $true)]
  [string]$ProbePath
)

$probe = Get-Content $ProbePath -Raw | ConvertFrom-Json
$rows = foreach ($stage in $probe.stages) {
  [pscustomobject]@{
    stage = $stage[0]
    count = $stage[1].count
    p95_ms = $stage[1].p95_ms
    max_ms = $stage[1].max_ms
    bytes = $stage[1].bytes
  }
}

$rows | Format-Table -AutoSize
