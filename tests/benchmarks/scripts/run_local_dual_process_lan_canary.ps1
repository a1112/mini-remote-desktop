param(
  [string]$RepoRoot = ".",
  [string]$OutputDir = "target/codex-local-dual-process-canary",
  [string[]]$ProfileId,
  [int]$DurationSecs = 30,
  [int]$BitrateMbps = 20,
  [ValidateSet("none", "temporary", "required")]
  [string]$DisplayModePolicy = "temporary",
  [string]$CaptureSourceId = "",
  [string]$CaptureSourceKind = "display_shared",
  [double]$LossPct = 0,
  [int]$BaseDelayMs = 0,
  [int]$JitterMs = 0,
  [int]$MtuBytes = 0,
  [UInt64]$Seed = 0,
  [switch]$NoMotionStimulus,
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

function Test-UdpPortAvailable([int]$Port) {
  $udp = $null
  try {
    $udp = [System.Net.Sockets.UdpClient]::new($Port)
    return $true
  } catch {
    return $false
  } finally {
    if ($udp) { $udp.Close() }
  }
}

function New-DiscoveryPortPair {
  for ($attempt = 0; $attempt -lt 50; $attempt++) {
    $base = 21216 + ((Get-Random -Minimum 0 -Maximum 1000) * 2)
    $controller = $base
    $peer = $base + 1
    if ((Test-UdpPortAvailable $controller) -and (Test-UdpPortAvailable $peer)) {
      return [pscustomobject]@{ controller = $controller; peer = $peer }
    }
  }
  throw "could not find two free UDP discovery ports"
}

function Start-MotionStimulusWindow([string]$Title) {
  $titleLiteral = $Title.Replace("'", "''")
  $script = @'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$title = '__MRD_TITLE__'
$form = New-Object System.Windows.Forms.Form
$form.Text = $title
$form.StartPosition = 'Manual'
$form.Left = 48
$form.Top = 48
$form.Width = 640
$form.Height = 360
$form.TopMost = $true
$form.BackColor = [System.Drawing.Color]::Black

$label = New-Object System.Windows.Forms.Label
$label.Dock = [System.Windows.Forms.DockStyle]::Fill
$label.TextAlign = [System.Drawing.ContentAlignment]::MiddleCenter
$label.Font = New-Object System.Drawing.Font('Segoe UI', 24, [System.Drawing.FontStyle]::Bold)
$label.ForeColor = [System.Drawing.Color]::White
$label.Text = $title
$form.Controls.Add($label)

$script:frame = 0
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 8
$timer.Add_Tick({
  $script:frame += 1
  $r = ($script:frame * 5) % 256
  $g = ($script:frame * 11) % 256
  $b = ($script:frame * 17) % 256
  $form.BackColor = [System.Drawing.Color]::FromArgb($r, $g, $b)
  $label.Text = '{0}  frame {1}' -f $title, $script:frame
  $form.Invalidate()
})
$timer.Start()
[System.Windows.Forms.Application]::Run($form)
'@.Replace("__MRD_TITLE__", $titleLiteral)

  $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($script))
  Start-Process `
    -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-EncodedCommand", $encoded) `
    -WindowStyle Normal `
    -PassThru
}

function ConvertTo-NamedPipeName([string]$Endpoint) {
  $prefix = "\\.\pipe\"
  if ($Endpoint.StartsWith($prefix)) {
    return $Endpoint.Substring($prefix.Length)
  }
  $Endpoint
}

function Read-ExactBytes($Stream, [int]$Length) {
  $buffer = New-Object byte[] $Length
  $offset = 0
  while ($offset -lt $Length) {
    $read = $Stream.Read($buffer, $offset, $Length - $offset)
    if ($read -le 0) {
      throw "stream closed while reading $Length bytes"
    }
    $offset += $read
  }
  $buffer
}

function Wait-IpcServiceHealth([string]$PipeEndpoint, [int]$TimeoutSecs) {
  $pipeName = ConvertTo-NamedPipeName $PipeEndpoint
  $deadline = (Get-Date).AddSeconds($TimeoutSecs)
  $lastError = $null

  while ((Get-Date) -lt $deadline) {
    $client = $null
    try {
      $client = [System.IO.Pipes.NamedPipeClientStream]::new(
        ".",
        $pipeName,
        [System.IO.Pipes.PipeDirection]::InOut,
        [System.IO.Pipes.PipeOptions]::None
      )
      $client.Connect(500)
      $payload = [System.Text.Encoding]::UTF8.GetBytes('{"type":"ServiceHealth"}')
      $length = [System.BitConverter]::GetBytes([UInt32]$payload.Length)
      $client.Write($length, 0, $length.Length)
      $client.Write($payload, 0, $payload.Length)
      $client.Flush()

      $responseLengthBytes = Read-ExactBytes -Stream $client -Length 4
      $responseLength = [System.BitConverter]::ToUInt32($responseLengthBytes, 0)
      $responseBytes = Read-ExactBytes -Stream $client -Length ([int]$responseLength)
      $response = [System.Text.Encoding]::UTF8.GetString($responseBytes) | ConvertFrom-Json
      if ($response.type -eq "ServiceHealth") {
        return $true
      }
      $lastError = "unexpected IPC response type: $($response.type)"
    } catch {
      $lastError = $_.Exception.Message
      Start-Sleep -Milliseconds 250
    } finally {
      if ($client) { $client.Dispose() }
    }
  }

  throw "IPC service health timed out for $PipeEndpoint. Last error: $lastError"
}

function Start-LocalServiceInstance {
  param(
    [Parameter(Mandatory = $true)][string]$ServiceExe,
    [Parameter(Mandatory = $true)][string]$Role,
    [Parameter(Mandatory = $true)][string]$PipeEndpoint,
    [Parameter(Mandatory = $true)][int]$DiscoveryPort,
    [Parameter(Mandatory = $true)][int]$PeerDiscoveryPort,
    [Parameter(Mandatory = $true)][string]$DeviceId,
    [Parameter(Mandatory = $true)][string]$DeviceName,
    [Parameter(Mandatory = $true)][string]$LogsDir,
    [Parameter(Mandatory = $true)]$Impairment
  )

  New-Item -ItemType Directory -Force -Path $LogsDir | Out-Null
  $savedEnv = @{}
  try {
    Set-EnvVar "MRD_SERVICE_IPC_ENDPOINT" $PipeEndpoint $savedEnv
    Set-EnvVar "MRD_LAN_DEVICE_ID" $DeviceId $savedEnv
    Set-EnvVar "MRD_LAN_DEVICE_NAME" $DeviceName $savedEnv
    Set-EnvVar "MRD_LAN_DISCOVERY_PORT" ([string]$DiscoveryPort) $savedEnv
    Set-EnvVar "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS" "127.0.0.1:$PeerDiscoveryPort" $savedEnv
    Set-EnvVar "RUST_LOG" "info" $savedEnv

    if ($Role -eq "peer") {
      Set-EnvVar "MRD_LAN_TEST_IMPAIRMENT_LOSS_PCT" ([string]$Impairment.loss_pct) $savedEnv
      Set-EnvVar "MRD_LAN_TEST_IMPAIRMENT_BASE_DELAY_MS" ([string]$Impairment.base_delay_ms) $savedEnv
      Set-EnvVar "MRD_LAN_TEST_IMPAIRMENT_JITTER_MS" ([string]$Impairment.jitter_ms) $savedEnv
      if ([int]$Impairment.mtu_bytes -gt 0) {
        Set-EnvVar "MRD_LAN_TEST_IMPAIRMENT_MTU_BYTES" ([string]$Impairment.mtu_bytes) $savedEnv
      }
      if ([UInt64]$Impairment.seed -gt 0) {
        Set-EnvVar "MRD_LAN_TEST_IMPAIRMENT_SEED" ([string]$Impairment.seed) $savedEnv
      }
    }

    $stdout = Join-Path $LogsDir "$Role.stdout.log"
    $stderr = Join-Path $LogsDir "$Role.stderr.log"
    $process = Start-Process `
      -FilePath $ServiceExe `
      -WorkingDirectory (Split-Path -Parent $ServiceExe) `
      -RedirectStandardOutput $stdout `
      -RedirectStandardError $stderr `
      -WindowStyle Hidden `
      -PassThru

    [pscustomobject]@{
      role = $Role
      process = $process
      pipe_endpoint = $PipeEndpoint
      discovery_port = $DiscoveryPort
      probe_endpoint = "127.0.0.1:$PeerDiscoveryPort"
      device_id = $DeviceId
      stdout = $stdout
      stderr = $stderr
    }
  } finally {
    Restore-EnvVars $savedEnv
  }
}

function Invoke-LocalDualProcessProfile {
  param(
    [Parameter(Mandatory = $true)][string]$Repo,
    [Parameter(Mandatory = $true)]$Profile,
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [Parameter(Mandatory = $true)][string]$GitCommit,
    [Parameter(Mandatory = $true)]$Impairment,
    [Parameter(Mandatory = $true)][string]$DisplayModePolicy,
    [string]$CaptureSourceId = "",
    [string]$CaptureSourceKind = "display_shared",
    [switch]$NoMotionStimulus,
    [switch]$KeepTauriOpen
  )

  $runStamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $runId = "local-dual-$($Profile.id)-$runStamp-$GitCommit-$([guid]::NewGuid().ToString("N").Substring(0, 8))"
  $rawDir = Join-Path $OutputRoot "raw"
  $runDir = Join-Path $OutputRoot "runs/$runId"
  $logsDir = Join-Path $runDir "logs"
  New-Item -ItemType Directory -Force -Path $rawDir, $logsDir | Out-Null

  $serviceExe = Join-Path $Repo "target/debug/mrd-service.exe"
  if (-not (Test-Path $serviceExe)) {
    throw "mrd-service executable was not found at $serviceExe"
  }
  $runBinDir = Join-Path $runDir "bin"
  New-Item -ItemType Directory -Force -Path $runBinDir | Out-Null
  $runServiceExe = Join-Path $runBinDir "mrd-service.exe"
  Copy-Item -LiteralPath $serviceExe -Destination $runServiceExe -Force

  $ports = New-DiscoveryPortPair
  $controllerPipe = "\\.\pipe\mrd-service-local-controller-$runId"
  $peerPipe = "\\.\pipe\mrd-service-local-peer-$runId"
  $controllerDeviceId = "lan-local-controller-$runId"
  $peerDeviceId = "lan-local-peer-$runId"
  $reportPath = Join-Path $rawDir "local-dual-$($Profile.id).json"
  Remove-Item -LiteralPath $reportPath -Force -ErrorAction SilentlyContinue

  $controller = $null
  $peer = $null
  $tauri = $null
  $motionStimulus = $null
  $savedEnv = @{}
  try {
    $motionStimulusTitle = "MRD Local Dual Motion $runId"
    if (-not $NoMotionStimulus) {
      $motionStimulus = Start-MotionStimulusWindow -Title $motionStimulusTitle
      Start-Sleep -Milliseconds 750
    }

    $controller = Start-LocalServiceInstance `
      -ServiceExe $runServiceExe `
      -Role "controller" `
      -PipeEndpoint $controllerPipe `
      -DiscoveryPort $ports.controller `
      -PeerDiscoveryPort $ports.peer `
      -DeviceId $controllerDeviceId `
      -DeviceName "Local Dual Controller" `
      -LogsDir (Join-Path $logsDir "controller") `
      -Impairment $Impairment

    $peer = Start-LocalServiceInstance `
      -ServiceExe $runServiceExe `
      -Role "peer" `
      -PipeEndpoint $peerPipe `
      -DiscoveryPort $ports.peer `
      -PeerDiscoveryPort $ports.controller `
      -DeviceId $peerDeviceId `
      -DeviceName "Local Dual Peer" `
      -LogsDir (Join-Path $logsDir "peer") `
      -Impairment $Impairment

    Wait-IpcServiceHealth -PipeEndpoint $controllerPipe -TimeoutSecs 20 | Out-Null
    Wait-IpcServiceHealth -PipeEndpoint $peerPipe -TimeoutSecs 20 | Out-Null

    Set-EnvVar "MRD_SERVICE_IPC_ENDPOINT" $controllerPipe $savedEnv
    Set-EnvVar "MRD_SERVICE_BOOTSTRAP_DISABLED" "1" $savedEnv
    Set-EnvVar "MRD_RDESK_SINGLE_INSTANCE_ADDR" ("127.0.0.1:{0}" -f (47650 + (Get-Random -Minimum 0 -Maximum 1000))) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_AUTORUN" "1" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_TARGET_DEVICE_ID" $peerDeviceId $savedEnv
    Set-EnvVar "MRD_LAN_E2E_TRANSPORT" "quic" $savedEnv
    $sampleTimeoutMs = ($Profile.duration_secs * 1000) + 2500
    Set-EnvVar "MRD_LAN_E2E_TIMEOUT_MS" ([string]$sampleTimeoutMs) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_SAMPLE_DURATION_MS" ([string]($Profile.duration_secs * 1000)) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_DECODED_FRAMES" "20" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_MIN_FPS" ([string]([Math]::Max(1, [Math]::Floor($Profile.fps * 0.5)))) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_STOP_ON_COMPLETE" "true" $savedEnv
    Set-EnvVar "MRD_LAN_E2E_REPORT_PATH" $reportPath $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_WIDTH" ([string]$Profile.width) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_HEIGHT" ([string]$Profile.height) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_FPS" ([string]$Profile.fps) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_PROFILE_BITRATE_MBPS" ([string]$Profile.bitrate_mbps) $savedEnv
    Set-EnvVar "MRD_LAN_E2E_DISPLAY_MODE_POLICY" $DisplayModePolicy $savedEnv
    Set-EnvVar "MRD_LAN_E2E_EXPECTED_PEER_BUILD_ID" $GitCommit $savedEnv
    if ($CaptureSourceId.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_CAPTURE_SOURCE_ID" $CaptureSourceId.Trim() $savedEnv
    }
    if ($CaptureSourceKind.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_CAPTURE_SOURCE_KIND" $CaptureSourceKind.Trim() $savedEnv
    }

    $tauriStdout = Join-Path $logsDir "tauri.stdout.log"
    $tauriStderr = Join-Path $logsDir "tauri.stderr.log"
    $tauri = Start-Process `
      -FilePath "cmd.exe" `
      -ArgumentList @("/c", "pnpm", "tauri:dev") `
      -WorkingDirectory (Join-Path $Repo "apps/Rdesk") `
      -RedirectStandardOutput $tauriStdout `
      -RedirectStandardError $tauriStderr `
      -WindowStyle Hidden `
      -PassThru

    $deadline = (Get-Date).AddMilliseconds(($Profile.duration_secs * 1000) + 90000)
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
      if ($tauri.HasExited) {
        break
      }
      if ($controller.process.HasExited -or $peer.process.HasExited) {
        break
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
      $processExitMessage = if ($controller.process.HasExited) {
        "controller mrd-service exited with code $($controller.process.ExitCode)"
      } elseif ($peer.process.HasExited) {
        "peer mrd-service exited with code $($peer.process.ExitCode)"
      } elseif ($tauri -and $tauri.HasExited) {
        "Tauri autorun exited with code $($tauri.ExitCode)"
      } else {
        $null
      }
      $failureReason = if ($processExitMessage) { "service_crash" } else { "transport_timeout" }
      $errorMessage = if ($processExitMessage) {
        "local dual-process LAN E2E autorun exited before writing a completed report: $processExitMessage"
      } else {
        "local dual-process LAN E2E autorun did not produce a completed report before timeout"
      }
      $report = [pscustomobject]@{
        status = "failed"
        failureReason = $failureReason
        errorMessage = $errorMessage
        tauri_exit_code = if ($tauri -and $tauri.HasExited) { $tauri.ExitCode } else { $null }
        controller_exit_code = if ($controller.process.HasExited) { $controller.process.ExitCode } else { $null }
        peer_exit_code = if ($peer.process.HasExited) { $peer.process.ExitCode } else { $null }
        probeSnapshot = $null
        mediaPipelineSnapshot = $null
        sessionSnapshot = $null
      }
      $report | ConvertTo-Json -Depth 16 | Set-Content -Path $reportPath -Encoding Ascii
    }

    $row = Convert-CrossReportToCanaryRow -Profile $Profile -Report $report -ReportPath $reportPath
    $row | Add-Member -Force -NotePropertyName "mode" -NotePropertyValue "local-dual-process"
    $row | Add-Member -Force -NotePropertyName "chain" -NotePropertyValue "local_dual_process/dxgi/nvenc_h264/quic_datagram_media_v3_or_v2/nvdec/d3d11_shared"
    $row | Add-Member -Force -NotePropertyName "controller_pipe" -NotePropertyValue $controllerPipe
    $row | Add-Member -Force -NotePropertyName "peer_pipe" -NotePropertyValue $peerPipe
    $row | Add-Member -Force -NotePropertyName "controller_discovery_port" -NotePropertyValue $ports.controller
    $row | Add-Member -Force -NotePropertyName "peer_discovery_port" -NotePropertyValue $ports.peer
    $row | Add-Member -Force -NotePropertyName "run_dir" -NotePropertyValue $runDir
    $row | Add-Member -Force -NotePropertyName "requested_capture_source_id" -NotePropertyValue $(if ($CaptureSourceId.Trim()) { $CaptureSourceId.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_capture_source_kind" -NotePropertyValue $(if ($CaptureSourceKind.Trim()) { $CaptureSourceKind.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "actual_capture_source_id" -NotePropertyValue $report.captureSource.id
    $row | Add-Member -Force -NotePropertyName "actual_capture_source_kind" -NotePropertyValue $report.captureSource.source_kind
    $row | Add-Member -Force -NotePropertyName "motion_stimulus_title" -NotePropertyValue $(if ($NoMotionStimulus) { $null } else { $motionStimulusTitle })
    $row | Add-Member -Force -NotePropertyName "motion_stimulus_pid" -NotePropertyValue $(if ($motionStimulus) { $motionStimulus.Id } else { $null })
    if ($report.captureSource.source_kind -match "^display") {
      $row | Add-Member -Force -NotePropertyName "fixture_warning" -NotePropertyValue "same_host_display_capture_can_feedback_with_receiver_render_window"
    }
    $row
  } finally {
    Restore-EnvVars $savedEnv
    if ($tauri -and -not $KeepTauriOpen) {
      Stop-ProcessTree -ProcessId $tauri.Id
    }
    if ($controller) {
      Stop-ProcessTree -ProcessId $controller.process.Id
    }
    if ($peer) {
      Stop-ProcessTree -ProcessId $peer.process.Id
    }
    if ($motionStimulus) {
      Stop-ProcessTree -ProcessId $motionStimulus.Id
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
    throw "No local dual-process LAN canary profiles matched: $($requestedProfiles -join ', ')"
  }
}

if (-not $NoBuild) {
  cargo build -p app -p mrd-service
}

$impairment = [pscustomobject]@{
  loss_pct = $LossPct
  base_delay_ms = $BaseDelayMs
  jitter_ms = $JitterMs
  mtu_bytes = $MtuBytes
  seed = $Seed
}

$rows = @()
foreach ($profile in $profiles) {
  Write-Host "Running local dual-process LAN canary $($profile.id)"
  $rows += Invoke-LocalDualProcessProfile `
    -Repo $repo `
    -Profile $profile `
    -OutputRoot $outputRoot `
    -GitCommit $gitCommit `
    -Impairment $impairment `
    -DisplayModePolicy $DisplayModePolicy `
    -CaptureSourceId $CaptureSourceId `
    -CaptureSourceKind $CaptureSourceKind `
    -NoMotionStimulus:$NoMotionStimulus `
    -KeepTauriOpen:$KeepTauriOpen
}

$report = New-PairedLanCanaryReport -Mode "local-dual-process" -Rows $rows -GitCommit $gitCommit
$report | Add-Member -Force -NotePropertyName "chain" -NotePropertyValue "local_dual_process/dxgi/nvenc_h264/quic_datagram_media_v3_or_v2/nvdec/d3d11_shared"
$report | Add-Member -Force -NotePropertyName "test_impairment_config" -NotePropertyValue $impairment
$report | Add-Member -Force -NotePropertyName "capture_source_request" -NotePropertyValue ([pscustomobject]@{
  id = if ($CaptureSourceId.Trim()) { $CaptureSourceId.Trim() } else { $null }
  kind = if ($CaptureSourceKind.Trim()) { $CaptureSourceKind.Trim() } else { $null }
})
$report | Add-Member -Force -NotePropertyName "motion_stimulus" -NotePropertyValue ([pscustomobject]@{
  enabled = -not [bool]$NoMotionStimulus
})

$jsonPath = Join-Path $outputRoot "local-dual-process-canary-report.json"
$markdownPath = Join-Path $outputRoot "local-dual-process-canary-report.md"
Write-CanaryJsonAndMarkdown `
  -Report $report `
  -JsonPath $jsonPath `
  -MarkdownPath $markdownPath `
  -Title "Local Dual-Process LAN Canary Report"

Add-Content -Path $markdownPath -Encoding Ascii -Value @(
  "",
  "## Test Impairment",
  "",
  "- LossPct: $LossPct",
  "- BaseDelayMs: $BaseDelayMs",
  "- JitterMs: $JitterMs",
  "- MtuBytes: $MtuBytes",
  "- Seed: $Seed"
)

Add-Content -Path $markdownPath -Encoding Ascii -Value @(
  "",
  "## Capture Source",
  "",
  "- RequestedSourceId: $(if ($CaptureSourceId.Trim()) { $CaptureSourceId.Trim() } else { '-' })",
  "- RequestedSourceKind: $(if ($CaptureSourceKind.Trim()) { $CaptureSourceKind.Trim() } else { '-' })",
  "- MotionStimulus: $(if ($NoMotionStimulus) { 'disabled' } else { 'enabled' })"
)

$fixtureWarnings = @($rows | Where-Object { $_.fixture_warning } | Select-Object -ExpandProperty fixture_warning -Unique)
if ($fixtureWarnings.Count -gt 0) {
  Add-Content -Path $markdownPath -Encoding Ascii -Value @(
    "",
    "## Fixture Warnings",
    ""
  )
  foreach ($warning in $fixtureWarnings) {
    Add-Content -Path $markdownPath -Encoding Ascii -Value "- $warning"
  }
}

Write-Host "Local dual-process LAN canary report written to $outputRoot"
