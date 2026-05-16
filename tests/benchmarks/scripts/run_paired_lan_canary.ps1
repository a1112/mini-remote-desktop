param(
  [string]$RepoRoot = ".",
  [string]$OutputDir = "target/codex-matrix-compare",
  [string]$TargetDeviceId,
  [int]$DurationSecs = 30,
  [int]$BitrateMbps = 20,
  [double]$RatioThreshold = 0.8,
  [switch]$SkipLocal,
  [switch]$SkipCross,
  [switch]$NoBuild,
  [switch]$KeepTauriOpen
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "paired_lan_canary_common.ps1")

function Resolve-RepoPath([string]$Path) {
  (Resolve-Path $Path).Path
}

function Set-EnvVar([string]$Name, [string]$Value, [hashtable]$Saved) {
  if (-not $Saved.ContainsKey($Name)) {
    $Saved[$Name] = [Environment]::GetEnvironmentVariable($Name, "Process")
  }
  [Environment]::SetEnvironmentVariable($Name, $Value, "Process")
}

function Restore-EnvVars([hashtable]$Saved) {
  foreach ($name in $Saved.Keys) {
    [Environment]::SetEnvironmentVariable($name, $Saved[$name], "Process")
  }
}

function Stop-ProcessTree([int]$ProcessId) {
  $children = Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $ProcessId }
  foreach ($child in $children) {
    Stop-ProcessTree -ProcessId $child.ProcessId
  }
  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Invoke-LocalCanaryProfile($Repo, $Profile, $GitCommit) {
  $timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $date = Get-Date -Format 'yyyy-MM-dd'
  $runId = "paired-local-$($Profile.id)-$timestamp-$GitCommit"
  $runDir = Join-Path $Repo ("artifacts/benchmarks/{0}/paired-lan-canary/{1}" -f $date, $runId)
  $logsDir = Join-Path $runDir "logs"
  New-Item -ItemType Directory -Force -Path $logsDir | Out-Null
  New-Item -ItemType File -Force -Path (Join-Path $logsDir 'signaling.stdout.log'), (Join-Path $logsDir 'signaling.stderr.log') | Out-Null

  $savedEnv = @{}
  try {
    Set-EnvVar "MRD_BENCH_ARTIFACT_ROOT" $Repo $savedEnv
    Set-EnvVar "MRD_BENCH_SCENARIO" "paired.local.canary" $savedEnv
    Set-EnvVar "MRD_BENCH_PROFILE" "paired-lan-canary" $savedEnv
    Set-EnvVar "MRD_BENCH_RUN_ID" $runId $savedEnv
    Set-EnvVar "MRD_BENCH_DATE" $date $savedEnv
    Set-EnvVar "MRD_BENCH_WIDTH" ([string]$Profile.width) $savedEnv
    Set-EnvVar "MRD_BENCH_HEIGHT" ([string]$Profile.height) $savedEnv
    Set-EnvVar "MRD_BENCH_FPS" ([string]$Profile.fps) $savedEnv
    Set-EnvVar "MRD_BENCH_DURATION_SECS" ([string]$Profile.duration_secs) $savedEnv
    Set-EnvVar "MRD_BENCH_GIT_COMMIT" $GitCommit $savedEnv
    Set-EnvVar "MRD_BENCH_TRANSPORT" "quic_datagram" $savedEnv
    Set-EnvVar "MRD_BENCH_CAPTURE_BACKEND" "dxgi" $savedEnv
    Set-EnvVar "MRD_BENCH_ENCODE_BACKEND" "nvenc_h264" $savedEnv
    Set-EnvVar "MRD_BENCH_DECODE_BACKEND" "nvdec" $savedEnv
    Set-EnvVar "MRD_BENCH_RENDERER_BACKEND" "d3d11_shared" $savedEnv

    $stdout = Join-Path $logsDir "host.stdout.log"
    $stderr = Join-Path $logsDir "host.stderr.log"
    $process = Start-Process -FilePath "cargo" -ArgumentList @("test", "-p", "app", "benchmark_run_writes_requested_artifacts", "--", "--nocapture") -WorkingDirectory $Repo -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) {
      throw "local canary cargo test failed for $($Profile.id), see $stderr"
    }

    powershell -ExecutionPolicy Bypass -File (Join-Path $Repo "tests/benchmarks/scripts/summarize_transport_results.ps1") -RunDir $runDir
    $summaryPath = Join-Path $runDir "summary.json"
    $summary = Get-Content $summaryPath -Raw | ConvertFrom-Json
    Convert-LocalSummaryToCanaryRow -Profile $Profile -Summary $summary -SummaryPath $summaryPath
  } finally {
    Restore-EnvVars $savedEnv
  }
}

function Invoke-CrossCanaryProfile($Repo, $Profile, $OutputRoot, $TargetDeviceId, [int]$TimeoutMs, [switch]$KeepTauriOpen) {
  $reportPath = Join-Path $OutputRoot ("raw/cross-$($Profile.id).json")
  $logsDir = Join-Path $OutputRoot "logs"
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $reportPath), $logsDir | Out-Null
  Remove-Item -LiteralPath $reportPath -Force -ErrorAction SilentlyContinue

  $savedEnv = @{}
  $process = $null
  try {
    Set-EnvVar "MRD_LAN_E2E_AUTORUN" "1" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_TRANSPORT" "quic" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_TIMEOUT_MS" ([string]$TimeoutMs) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_DECODED_FRAMES" "20" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_FPS" ([string]([Math]::Max(1, [Math]::Floor($Profile.fps * 0.5)))) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_STOP_ON_COMPLETE" "true" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_REPORT_PATH" $reportPath $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_WIDTH" ([string]$Profile.width) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_HEIGHT" ([string]$Profile.height) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_FPS" ([string]$Profile.fps) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_BITRATE_MBPS" ([string]$Profile.bitrate_mbps) $savedEnv
    if ($TargetDeviceId) {
      Set-EnvVar "MRD_LAN_E2E_TARGET_DEVICE_ID" $TargetDeviceId $savedEnv
    }

    $stdout = Join-Path $logsDir "cross-$($Profile.id).stdout.log"
    $stderr = Join-Path $logsDir "cross-$($Profile.id).stderr.log"
    $process = Start-Process -FilePath "cmd.exe" -ArgumentList @("/c", "pnpm", "tauri:dev") -WorkingDirectory (Join-Path $Repo "apps/Rdesk") -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru

    $deadline = (Get-Date).AddMilliseconds($TimeoutMs + 60000)
    $report = $null
    while ((Get-Date) -lt $deadline) {
      if (Test-Path $reportPath) {
        try {
          $report = Get-Content $reportPath -Raw | ConvertFrom-Json
          if ($report.status -in @("completed", "failed", "skipped")) {
            break
          }
        } catch {
          Start-Sleep -Milliseconds 500
        }
      }
      if ($process.HasExited) {
        break
      }
      Start-Sleep -Seconds 1
    }

    if (-not $report) {
      $report = [pscustomobject]@{
        status = "failed"
        failureReason = "transport_timeout"
        errorMessage = "LAN E2E autorun did not produce a completed report before timeout"
        probeSnapshot = $null
        mediaPipelineSnapshot = $null
        sessionSnapshot = $null
      }
    }

    Convert-CrossReportToCanaryRow -Profile $Profile -Report $report -ReportPath $reportPath
  } finally {
    Restore-EnvVars $savedEnv
    if ($process -and -not $KeepTauriOpen) {
      Stop-ProcessTree -ProcessId $process.Id
    }
  }
}

$repo = Resolve-RepoPath $RepoRoot
$outputRoot = Join-Path $repo $OutputDir
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$gitCommit = (git -C $repo rev-parse --short=12 HEAD).Trim()
$profiles = Get-PairedLanCanaryProfiles -DurationSecs $DurationSecs -BitrateMbps $BitrateMbps

if (-not $NoBuild) {
  cargo build -p app -p mrd-service
}

$localRows = @()
if (-not $SkipLocal) {
  foreach ($profile in $profiles) {
    Write-Host "Running local canary $($profile.id)"
    $localRows += Invoke-LocalCanaryProfile -Repo $repo -Profile $profile -GitCommit $gitCommit
  }
}

$crossRows = @()
if (-not $SkipCross) {
  $timeoutMs = ($DurationSecs * 1000) + 30000
  foreach ($profile in $profiles) {
    Write-Host "Running cross-device canary $($profile.id)"
    $crossRows += Invoke-CrossCanaryProfile -Repo $repo -Profile $profile -OutputRoot $outputRoot -TargetDeviceId $TargetDeviceId -TimeoutMs $timeoutMs -KeepTauriOpen:$KeepTauriOpen
  }
}

$localReport = New-PairedLanCanaryReport -Mode "local" -Rows $localRows -GitCommit $gitCommit
$crossReport = New-PairedLanCanaryReport -Mode "cross" -Rows $crossRows -GitCommit $gitCommit
$comparisonRows = @(Compare-PairedLanCanaryRows -LocalRows $localRows -CrossRows $crossRows -RatioThreshold $RatioThreshold)

Write-CanaryJsonAndMarkdown -Report $localReport -JsonPath (Join-Path $outputRoot "local-canary-report.json") -MarkdownPath (Join-Path $outputRoot "local-canary-report.md") -Title "Local Canary Report"
Write-CanaryJsonAndMarkdown -Report $crossReport -JsonPath (Join-Path $outputRoot "cross-device-canary-report.json") -MarkdownPath (Join-Path $outputRoot "cross-device-canary-report.md") -Title "Cross-Device Canary Report"
ConvertTo-Json -InputObject $comparisonRows -Depth 16 | Set-Content -Path (Join-Path $outputRoot "matrix-comparison-report.json") -Encoding Ascii
Write-PairedLanComparisonMarkdown -Rows $comparisonRows -MarkdownPath (Join-Path $outputRoot "matrix-comparison-report.md") -GitCommit $gitCommit

Write-Host "Paired LAN canary reports written to $outputRoot"
