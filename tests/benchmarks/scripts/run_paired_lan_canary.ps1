param(
  [string]$RepoRoot = ".",
  [string]$OutputDir = "target/codex-matrix-compare",
  [string]$TargetDeviceId,
  [string]$TargetAddress,
  [string[]]$ProfileId,
  [int]$DurationSecs = 30,
  [int]$BitrateMbps = 20,
  [double]$RatioThreshold = 0.8,
  [ValidateSet("none", "temporary", "required")]
  [string]$DisplayModePolicy = "temporary",
  [ValidateSet("h264", "hevc")]
  [string]$Codec = "h264",
  [string]$CodecProfile = "",
  [int]$BitDepth = 0,
  [string]$ChromaSubsampling = "",
  [string]$PixelFormat = "",
  [bool]$HdrEnabled = $false,
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

function Invoke-LanDiscoveryDiagnostics($OutputRoot, [string]$TargetAddress) {
  $targets = @("255.255.255.255")
  if ($TargetAddress) {
    $targets = @($TargetAddress) + $targets
  }
  try {
    $targets += Get-NetIPAddress -AddressFamily IPv4 |
      Where-Object { $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254*" -and $_.PrefixLength -lt 32 } |
      ForEach-Object { Get-IPv4BroadcastAddress -IPAddress $_.IPAddress -PrefixLength $_.PrefixLength }
  } catch {
    $targets += @()
  }
  $targets = @($targets | Where-Object { $_ } | Select-Object -Unique)

  $ping = $null
  if ($TargetAddress) {
    try {
      $pingSamples = @(Test-Connection -ComputerName $TargetAddress -Count 2 -ErrorAction Stop)
      $ping = [pscustomobject]@{
        target = $TargetAddress
        succeeded = $true
        response_time_ms = @($pingSamples | ForEach-Object { $_.ResponseTime })
      }
    } catch {
      $ping = [pscustomobject]@{
        target = $TargetAddress
        succeeded = $false
        error = $_.Exception.Message
      }
    }
  }

  $udpResponses = @()
  $sentTargets = @()
  $udpError = $null
  try {
    $udp = [System.Net.Sockets.UdpClient]::new(0)
    $udp.EnableBroadcast = $true
    $udp.Client.ReceiveTimeout = 1000
    $packet = @{
      type = "probe"
      magic = "mrd-lan-discovery-v1"
      app_id = "rdesk"
      instance_id = "paired-canary-probe-$([guid]::NewGuid().ToString("N"))"
      device_id = $null
      timestamp_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    } | ConvertTo-Json -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($packet)
    foreach ($target in $targets) {
      try {
        [void]$udp.Send($bytes, $bytes.Length, $target, 21116)
        $sentTargets += $target
      } catch {
        $udpResponses += [pscustomobject]@{
          target = $target
          error = $_.Exception.Message
        }
      }
    }

    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline) {
      try {
        $remote = [System.Net.IPEndPoint]::new([System.Net.IPAddress]::Any, 0)
        $data = $udp.Receive([ref]$remote)
        $udpResponses += [pscustomobject]@{
          remote = $remote.ToString()
          payload = [System.Text.Encoding]::UTF8.GetString($data)
        }
      } catch [System.Net.Sockets.SocketException] {
      }
    }
    $udp.Close()
  } catch {
    $udpError = $_.Exception.Message
  }

  $diagnostics = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString("o")
    target_device_id = $TargetDeviceId
    target_address = $TargetAddress
    udp_port = 21116
    probe_targets = $targets
    sent_targets = $sentTargets
    ping = $ping
    udp_response_count = @($udpResponses | Where-Object { $_.remote }).Count
    udp_responses = $udpResponses
    udp_error = $udpError
  }

  $jsonPath = Join-Path $OutputRoot "lan-discovery-diagnostics.json"
  $markdownPath = Join-Path $OutputRoot "lan-discovery-diagnostics.md"
  ConvertTo-Json -InputObject $diagnostics -Depth 16 | Set-Content -Path $jsonPath -Encoding Ascii
  $pingStatus = if ($ping -and $ping.succeeded) { "reachable" } elseif ($ping) { "failed" } else { "not tested" }
  $lines = @(
    "# LAN Discovery Diagnostics",
    "",
    "- Target device: $TargetDeviceId",
    "- Target address: $TargetAddress",
    "- UDP port: 21116",
    "- Ping: $pingStatus",
    "- UDP responses: $($diagnostics.udp_response_count)",
    "- Probe targets: $($targets -join ', ')",
    ""
  )
  if ($udpResponses.Count -gt 0) {
    $lines += "| Remote | Target | Error |"
    $lines += "| --- | --- | --- |"
    foreach ($response in $udpResponses) {
      $lines += "| $($response.remote) | $($response.target) | $($response.error) |"
    }
  }
  $lines -join "`n" | Set-Content -Path $markdownPath -Encoding Ascii
  $diagnostics
}

function Get-IPv4BroadcastAddress([string]$IPAddress, [int]$PrefixLength) {
  $bytes = [System.Net.IPAddress]::Parse($IPAddress).GetAddressBytes()
  $mask = [byte[]]@(0, 0, 0, 0)
  for ($i = 0; $i -lt 4; $i++) {
    $remaining = $PrefixLength - ($i * 8)
    if ($remaining -ge 8) {
      $mask[$i] = 255
    } elseif ($remaining -gt 0) {
      $mask[$i] = [byte](256 - (1 -shl (8 - $remaining)))
    } else {
      $mask[$i] = 0
    }
  }
  $broadcast = [byte[]]@(0, 0, 0, 0)
  for ($i = 0; $i -lt 4; $i++) {
    $broadcast[$i] = [byte]($bytes[$i] -bor ((-bnot $mask[$i]) -band 255))
  }
  [System.Net.IPAddress]::new($broadcast).ToString()
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

function Invoke-CrossCanaryProfile($Repo, $Profile, $OutputRoot, $TargetDeviceId, [int]$TimeoutMs, [string]$DisplayModePolicy, [string]$GitCommit, [string]$Codec, [string]$CodecProfile, [int]$BitDepth, [string]$ChromaSubsampling, [string]$PixelFormat, [bool]$HdrEnabled, [switch]$KeepTauriOpen) {
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
    Set-EnvVar "MRD_LAN_E2E_MIN_SAMPLE_DURATION_MS" ([string]($Profile.duration_secs * 1000)) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_DECODED_FRAMES" "20" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_FPS" ([string]([Math]::Max(1, [Math]::Floor($Profile.fps * 0.5)))) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_STOP_ON_COMPLETE" "true" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_REPORT_PATH" $reportPath $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_WIDTH" ([string]$Profile.width) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_HEIGHT" ([string]$Profile.height) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_FPS" ([string]$Profile.fps) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_BITRATE_MBPS" ([string]$Profile.bitrate_mbps) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_CODEC" $Codec $savedEnv
    if ($CodecProfile.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_PROFILE_CODEC_PROFILE" $CodecProfile.Trim() $savedEnv
    }
    if ($BitDepth -gt 0) {
      Set-EnvVar "MRD_LAN_E2E_PROFILE_BIT_DEPTH" ([string]$BitDepth) $savedEnv
    }
    if ($ChromaSubsampling.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_PROFILE_CHROMA_SUBSAMPLING" $ChromaSubsampling.Trim() $savedEnv
    }
    if ($PixelFormat.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_PROFILE_PIXEL_FORMAT" $PixelFormat.Trim() $savedEnv
    }
    Set-EnvVar "MRD_LAN_E2E_PROFILE_HDR_ENABLED" ([string]$HdrEnabled).ToLowerInvariant() $savedEnv
    Set-EnvVar "MRD_LAN_E2E_DISPLAY_MODE_POLICY" $DisplayModePolicy $savedEnv
    Set-EnvVar "MRD_LAN_E2E_EXPECTED_PEER_BUILD_ID" $GitCommit $savedEnv
    if ($Profile.adaptive) {
      Set-EnvVar "MRD_LAN_E2E_ADAPTIVE" "true" $savedEnv
    }
    if ($TargetDeviceId) {
      Set-EnvVar "MRD_LAN_E2E_TARGET_DEVICE_ID" $TargetDeviceId $savedEnv
    }

    $stdout = Join-Path $logsDir "cross-$($Profile.id).stdout.log"
    $stderr = Join-Path $logsDir "cross-$($Profile.id).stderr.log"
    $process = Start-Process -FilePath "cmd.exe" -ArgumentList @("/c", "pnpm", "tauri:dev") -WorkingDirectory (Join-Path $Repo "apps/Rdesk") -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru

    $deadline = (Get-Date).AddMilliseconds($TimeoutMs + 60000)
    $processExitGraceDeadline = $null
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
        if ($null -eq $processExitGraceDeadline) {
          $processExitGraceDeadline = (Get-Date).AddSeconds(15)
        }
        if ((Get-Date) -ge $processExitGraceDeadline) {
          break
        }
      }
      Start-Sleep -Seconds 1
    }

    if (-not $report -and (Test-Path $reportPath)) {
      try {
        $report = Get-Content $reportPath -Raw | ConvertFrom-Json
      } catch {
        $report = $null
      }
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

    $row = Convert-CrossReportToCanaryRow -Profile $Profile -Report $report -ReportPath $reportPath -RequestedCodec $Codec
    $row | Add-Member -Force -NotePropertyName "requested_codec_profile" -NotePropertyValue $(if ($CodecProfile.Trim()) { $CodecProfile.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_bit_depth" -NotePropertyValue $(if ($BitDepth -gt 0) { $BitDepth } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_chroma_subsampling" -NotePropertyValue $(if ($ChromaSubsampling.Trim()) { $ChromaSubsampling.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_pixel_format" -NotePropertyValue $(if ($PixelFormat.Trim()) { $PixelFormat.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_hdr_enabled" -NotePropertyValue $HdrEnabled
    $row
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
if ($ProfileId -and $ProfileId.Count -gt 0) {
  $requestedProfiles = @(
    $ProfileId |
      ForEach-Object { $_ -split "," } |
      ForEach-Object { $_.Trim() } |
      Where-Object { $_ }
  )
  $profiles = @($profiles | Where-Object { $requestedProfiles -contains $_.id })
  if ($profiles.Count -eq 0) {
    throw "No paired LAN canary profiles matched: $($requestedProfiles -join ', ')"
  }
}

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
  $discoveryDiagnostics = Invoke-LanDiscoveryDiagnostics -OutputRoot $outputRoot -TargetAddress $TargetAddress
  $effectiveTargetDeviceId = Resolve-PairedLanCanaryTargetDeviceId -Diagnostics $discoveryDiagnostics -RequestedTargetDeviceId $TargetDeviceId
  if (-not $TargetDeviceId -and $effectiveTargetDeviceId) {
    Write-Host "Auto-selected LAN target $effectiveTargetDeviceId from discovery diagnostics"
  }
  $timeoutMs = ($DurationSecs * 1000) + 30000
  foreach ($profile in $profiles) {
    Write-Host "Running cross-device canary $($profile.id)"
    $crossRows += Invoke-CrossCanaryProfile -Repo $repo -Profile $profile -OutputRoot $outputRoot -TargetDeviceId $effectiveTargetDeviceId -TimeoutMs $timeoutMs -DisplayModePolicy $DisplayModePolicy -GitCommit $gitCommit -Codec $Codec -CodecProfile $CodecProfile -BitDepth $BitDepth -ChromaSubsampling $ChromaSubsampling -PixelFormat $PixelFormat -HdrEnabled $HdrEnabled -KeepTauriOpen:$KeepTauriOpen
  }
}

$localReport = New-PairedLanCanaryReport -Mode "local" -Rows $localRows -GitCommit $gitCommit -Codec "h264"
$crossReport = New-PairedLanCanaryReport -Mode "cross" -Rows $crossRows -GitCommit $gitCommit -Codec $Codec
$crossReport | Add-Member -Force -NotePropertyName "codec_request" -NotePropertyValue ([pscustomobject]@{
  codec = $Codec
  codec_profile = if ($CodecProfile.Trim()) { $CodecProfile.Trim() } else { $null }
  bit_depth = if ($BitDepth -gt 0) { $BitDepth } else { $null }
  chroma_subsampling = if ($ChromaSubsampling.Trim()) { $ChromaSubsampling.Trim() } else { $null }
  pixel_format = if ($PixelFormat.Trim()) { $PixelFormat.Trim() } else { $null }
  hdr_enabled = $HdrEnabled
})
$comparisonRows = @(Compare-PairedLanCanaryRows -LocalRows $localRows -CrossRows $crossRows -RatioThreshold $RatioThreshold)

Write-CanaryJsonAndMarkdown -Report $localReport -JsonPath (Join-Path $outputRoot "local-canary-report.json") -MarkdownPath (Join-Path $outputRoot "local-canary-report.md") -Title "Local Canary Report"
Write-CanaryJsonAndMarkdown -Report $crossReport -JsonPath (Join-Path $outputRoot "cross-device-canary-report.json") -MarkdownPath (Join-Path $outputRoot "cross-device-canary-report.md") -Title "Cross-Device Canary Report"
ConvertTo-Json -InputObject $comparisonRows -Depth 16 | Set-Content -Path (Join-Path $outputRoot "matrix-comparison-report.json") -Encoding Ascii
Write-PairedLanComparisonMarkdown -Rows $comparisonRows -MarkdownPath (Join-Path $outputRoot "matrix-comparison-report.md") -GitCommit $gitCommit

Write-Host "Paired LAN canary reports written to $outputRoot"
