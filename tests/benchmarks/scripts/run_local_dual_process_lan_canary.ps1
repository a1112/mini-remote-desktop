param(
  [string]$RepoRoot = ".",
  [string]$OutputDir = "target/codex-local-dual-process-canary",
  [string[]]$ProfileId,
  [int]$DurationSecs = 30,
  [int]$BitrateMbps = 20,
  [ValidateSet("none", "temporary", "required")]
  [string]$DisplayModePolicy = "temporary",
  [ValidateSet("h264", "hevc", "av1")]
  [string]$Codec = "h264",
  [string]$CodecProfile = "",
  [ValidateSet("low_latency", "ultra_low_latency", "high_refresh")]
  [string]$Av1Mode = "high_refresh",
  [int]$BitDepth = 0,
  [string]$ChromaSubsampling = "",
  [string]$PixelFormat = "",
  [bool]$HdrEnabled = $false,
  [string]$CaptureSourceId = "",
  [string]$CaptureSourceKind = "display_shared",
  [string]$RenderDisplaySourceId = "",
  [int]$RenderMaxFps = 0,
  [ValidateSet("blocking", "nonblocking", "waitable")]
  [string]$RenderPresentMode = "blocking",
  [double]$LossPct = 0,
  [int]$BaseDelayMs = 0,
  [int]$JitterMs = 0,
  [int]$MtuBytes = 0,
  [UInt64]$Seed = 0,
  [string]$ServiceExePath = "",
  [int]$TauriStartupGraceSecs = 90,
  [switch]$NoMotionStimulus,
  [switch]$NoBuild,
  [switch]$NoRenderProfileCap,
  [switch]$ShowTauriWindow,
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

function Start-MotionStimulusWindow([string]$Title, [int]$Width = 640, [int]$Height = 360) {
  $titleLiteral = $Title.Replace("'", "''")
  $script = @'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$title = '__MRD_TITLE__'
$width = __MRD_WIDTH__
$height = __MRD_HEIGHT__
$form = New-Object System.Windows.Forms.Form
$form.Text = $title
$form.StartPosition = 'Manual'
$form.Left = 48
$form.Top = 48
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
$form.ClientSize = New-Object System.Drawing.Size($width, $height)
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
'@.Replace("__MRD_TITLE__", $titleLiteral).Replace("__MRD_WIDTH__", [string]$Width).Replace("__MRD_HEIGHT__", [string]$Height)

  $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($script))
  Start-Process `
    -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-EncodedCommand", $encoded) `
    -WindowStyle Normal `
    -PassThru
}

function Resolve-ProcessWindowCaptureSourceId($Process, [int]$TimeoutSecs = 5) {
  if (-not $Process) { return "" }
  $deadline = (Get-Date).AddSeconds($TimeoutSecs)
  while ((Get-Date) -lt $deadline) {
    try {
      $refreshed = Get-Process -Id $Process.Id -ErrorAction Stop
      if ($refreshed.MainWindowHandle -and $refreshed.MainWindowHandle.ToInt64() -ne 0) {
        return ("windows:window:0x{0:X}" -f $refreshed.MainWindowHandle.ToInt64())
      }
    } catch {
      return ""
    }
    Start-Sleep -Milliseconds 100
  }
  ""
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
    [Parameter(Mandatory = $true)]$Impairment,
    [string]$ServiceBuildId = "",
    [int]$RenderMaxFps = 0
  )

  New-Item -ItemType Directory -Force -Path $LogsDir | Out-Null
  $savedEnv = @{}
  try {
    Set-EnvVar "MRD_SERVICE_IPC_ENDPOINT" $PipeEndpoint $savedEnv
    Set-EnvVar "MRD_LAN_DEVICE_ID" $DeviceId $savedEnv
    Set-EnvVar "MRD_LAN_DEVICE_NAME" $DeviceName $savedEnv
    Set-EnvVar "MRD_LAN_DISCOVERY_PORT" ([string]$DiscoveryPort) $savedEnv
    Set-EnvVar "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS" "127.0.0.1:$PeerDiscoveryPort" $savedEnv
    # The local dual-process runner already talks to both services through
    # isolated named pipes. Disable the localhost web bridge so the two service
    # instances do not contend for the default 127.0.0.1:9532 listener.
    Set-EnvVar "MRD_WEB_BRIDGE_ENABLED" "false" $savedEnv
    Set-EnvVar "RUST_LOG" "info" $savedEnv
    if ($ServiceBuildId.Trim()) {
      Set-EnvVar "MRD_SERVICE_BUILD_ID" $ServiceBuildId.Trim() $savedEnv
    }
    if ($RenderMaxFps -gt 0) {
      Set-EnvVar "MRD_LAN_RENDER_MAX_FPS" ([string]$RenderMaxFps) $savedEnv
    }

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
    [string]$ServiceExePath = "",
    [string]$CaptureSourceId = "",
    [string]$CaptureSourceKind = "display_shared",
    [int]$RenderMaxFps = 0,
    [ValidateSet("blocking", "nonblocking", "waitable")]
    [string]$RenderPresentMode = "blocking",
    [int]$TauriStartupGraceSecs = 90,
    [switch]$NoMotionStimulus,
    [switch]$NoRenderProfileCap,
    [switch]$NoBuild,
    [switch]$ShowTauriWindow,
    [switch]$KeepTauriOpen
  )

  $runStamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $runId = "local-dual-$($Profile.id)-$runStamp-$GitCommit-$([guid]::NewGuid().ToString("N").Substring(0, 8))"
  $rawDir = Join-Path $OutputRoot "raw"
  $runDir = Join-Path $OutputRoot "runs/$runId"
  $logsDir = Join-Path $runDir "logs"
  New-Item -ItemType Directory -Force -Path $rawDir, $logsDir | Out-Null

  $serviceExe = if ($ServiceExePath.Trim()) {
    if ([System.IO.Path]::IsPathRooted($ServiceExePath)) {
      $ServiceExePath
    } else {
      Join-Path $Repo $ServiceExePath
    }
  } else {
    Join-Path $Repo "target/debug/mrd-service.exe"
  }
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
      $motionStimulus = Start-MotionStimulusWindow `
        -Title $motionStimulusTitle `
        -Width ([Math]::Max(640, [int]$Profile.width)) `
        -Height ([Math]::Max(360, [int]$Profile.height))
      Start-Sleep -Milliseconds 750
    }
    $effectiveCaptureSourceId = $CaptureSourceId
    if (
      -not $effectiveCaptureSourceId.Trim() -and
      $CaptureSourceKind.Trim() -eq "window" -and
      $motionStimulus
    ) {
      $effectiveCaptureSourceId = Resolve-ProcessWindowCaptureSourceId -Process $motionStimulus
      if ($effectiveCaptureSourceId.Trim()) {
        Write-Host "Using motion stimulus capture source $effectiveCaptureSourceId"
      }
    }

    switch ($RenderPresentMode) {
      "blocking" {
        Set-EnvVar "MRD_D3D11_RENDER_PRESENT_BLOCKING" "true" $savedEnv
        Set-EnvVar "MRD_D3D11_RENDER_WAITABLE_OBJECT" "false" $savedEnv
        Set-EnvVar "MRD_RENDER_THREAD_PRIORITY" "normal" $savedEnv
      }
      "waitable" {
        Set-EnvVar "MRD_D3D11_RENDER_PRESENT_BLOCKING" "false" $savedEnv
        Set-EnvVar "MRD_D3D11_RENDER_WAITABLE_OBJECT" "true" $savedEnv
        Set-EnvVar "MRD_RENDER_THREAD_PRIORITY" "above_normal" $savedEnv
      }
      default {
        Set-EnvVar "MRD_D3D11_RENDER_PRESENT_BLOCKING" "false" $savedEnv
        Set-EnvVar "MRD_D3D11_RENDER_WAITABLE_OBJECT" "false" $savedEnv
        Set-EnvVar "MRD_RENDER_THREAD_PRIORITY" "normal" $savedEnv
      }
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
      -Impairment $Impairment `
      -ServiceBuildId $GitCommit `
      -RenderMaxFps $RenderMaxFps

    $peer = Start-LocalServiceInstance `
      -ServiceExe $runServiceExe `
      -Role "peer" `
      -PipeEndpoint $peerPipe `
      -DiscoveryPort $ports.peer `
      -PeerDiscoveryPort $ports.controller `
      -DeviceId $peerDeviceId `
      -DeviceName "Local Dual Peer" `
      -LogsDir (Join-Path $logsDir "peer") `
      -Impairment $Impairment `
      -ServiceBuildId $GitCommit `
      -RenderMaxFps $RenderMaxFps

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
    Set-EnvVar "MRD_LAN_E2E_PROFILE_CODEC" $Codec $savedEnv
    Set-EnvVar "MRD_BENCH_NVENC_AV1_MODE" $Av1Mode $savedEnv
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
    if ($NoRenderProfileCap) {
      Set-EnvVar "MRD_LAN_E2E_RENDER_PROFILE_CAP" "false" $savedEnv
    }
    if ($Profile.adaptive) {
      Set-EnvVar "MRD_LAN_E2E_ADAPTIVE" "true" $savedEnv
    }
    if ($effectiveCaptureSourceId.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_CAPTURE_SOURCE_ID" $effectiveCaptureSourceId.Trim() $savedEnv
    }
    if ($CaptureSourceKind.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_CAPTURE_SOURCE_KIND" $CaptureSourceKind.Trim() $savedEnv
    }
    if ($RenderDisplaySourceId.Trim()) {
      Set-EnvVar "MRD_LAN_E2E_RENDER_DISPLAY_SOURCE_ID" $RenderDisplaySourceId.Trim() $savedEnv
    }
    $tauriEnvPlan = New-LocalDualProcessTauriEnvPlan `
      -OutputRoot $OutputRoot `
      -ServiceExe $runServiceExe `
      -WorkspaceTargetDir (Join-Path $Repo "target") `
      -NoBuild:$NoBuild
    foreach ($envVar in $tauriEnvPlan.PSObject.Properties) {
      Set-EnvVar $envVar.Name ([string]$envVar.Value) $savedEnv
    }

    $tauriStdout = Join-Path $logsDir "tauri.stdout.log"
    $tauriStderr = Join-Path $logsDir "tauri.stderr.log"
    $tauriWindowStyle = if ($ShowTauriWindow) { "Normal" } else { "Hidden" }
    $tauri = Start-Process `
      -FilePath "cmd.exe" `
      -ArgumentList @("/c", "pnpm", "tauri:dev") `
      -WorkingDirectory (Join-Path $Repo "apps/Rdesk") `
      -RedirectStandardOutput $tauriStdout `
      -RedirectStandardError $tauriStderr `
      -WindowStyle $tauriWindowStyle `
      -PassThru

    $startupGraceMs = [Math]::Max(1, $TauriStartupGraceSecs) * 1000
    $deadline = (Get-Date).AddMilliseconds(($Profile.duration_secs * 1000) + $startupGraceMs)
    $report = $null
    while ((Get-Date) -lt $deadline) {
      if (Test-Path $reportPath) {
        try {
          $reportCandidate = Get-Content $reportPath -Raw | ConvertFrom-Json
          $normalizedReport = Resolve-LanE2EAutomationReport -Report $reportCandidate
          if ($normalizedReport.status -in @("completed", "failed", "skipped")) {
            $report = $reportCandidate
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

    $automationReport = Resolve-LanE2EAutomationReport -Report $report
    $row = Convert-CrossReportToCanaryRow -Profile $Profile -Report $automationReport -ReportPath $reportPath
    $serviceBoundary = New-LocalDualProcessServiceBoundaryEvidence `
      -Controller $controller `
      -Peer $peer `
      -Report $automationReport `
      -ReportPath $reportPath `
      -RunDir $runDir
    $row | Add-Member -Force -NotePropertyName "mode" -NotePropertyValue "local-dual-process"
    $backends = Get-CanaryCodecBackends -Codec $Codec
    $row | Add-Member -Force -NotePropertyName "chain" -NotePropertyValue "local_dual_process/dxgi/$($backends.encoder)/quic_datagram_media_v3_or_v2/$($backends.decoder)/d3d11_shared"
    $row | Add-Member -Force -NotePropertyName "service_boundary" -NotePropertyValue $serviceBoundary
    $row | Add-Member -Force -NotePropertyName "service_boundary_gate" -NotePropertyValue $serviceBoundary.gate
    if (-not $serviceBoundary.gate.passed) {
      $row | Add-Member -Force -NotePropertyName "status" -NotePropertyValue "failed"
      $row | Add-Member -Force -NotePropertyName "classification" -NotePropertyValue "service_boundary_failed"
      $row | Add-Member -Force -NotePropertyName "error_message" -NotePropertyValue ("service boundary gate failed: {0}" -f (($serviceBoundary.gate.failures | ForEach-Object { [string]$_ }) -join ", "))
    }
    $row | Add-Member -Force -NotePropertyName "controller_pipe" -NotePropertyValue $controllerPipe
    $row | Add-Member -Force -NotePropertyName "peer_pipe" -NotePropertyValue $peerPipe
    $row | Add-Member -Force -NotePropertyName "controller_discovery_port" -NotePropertyValue $ports.controller
    $row | Add-Member -Force -NotePropertyName "peer_discovery_port" -NotePropertyValue $ports.peer
    $row | Add-Member -Force -NotePropertyName "run_dir" -NotePropertyValue $runDir
    $row | Add-Member -Force -NotePropertyName "requested_capture_source_id" -NotePropertyValue $(if ($effectiveCaptureSourceId.Trim()) { $effectiveCaptureSourceId.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_capture_source_kind" -NotePropertyValue $(if ($CaptureSourceKind.Trim()) { $CaptureSourceKind.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_codec" -NotePropertyValue $Codec
    $row | Add-Member -Force -NotePropertyName "tauri_window_visible" -NotePropertyValue ([bool]$ShowTauriWindow)
    $row | Add-Member -Force -NotePropertyName "requested_codec_profile" -NotePropertyValue $(if ($CodecProfile.Trim()) { $CodecProfile.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_bit_depth" -NotePropertyValue $(if ($BitDepth -gt 0) { $BitDepth } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_chroma_subsampling" -NotePropertyValue $(if ($ChromaSubsampling.Trim()) { $ChromaSubsampling.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_pixel_format" -NotePropertyValue $(if ($PixelFormat.Trim()) { $PixelFormat.Trim() } else { $null })
    $row | Add-Member -Force -NotePropertyName "requested_hdr_enabled" -NotePropertyValue $HdrEnabled
    $row | Add-Member -Force -NotePropertyName "active_codec" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_codec
    $row | Add-Member -Force -NotePropertyName "active_codec_profile" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_codec_profile
    $row | Add-Member -Force -NotePropertyName "active_bit_depth" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_bit_depth
    $row | Add-Member -Force -NotePropertyName "active_chroma_subsampling" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_chroma_subsampling
    $row | Add-Member -Force -NotePropertyName "active_pixel_format" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_pixel_format
    $row | Add-Member -Force -NotePropertyName "active_hdr_enabled" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_hdr_enabled
    $row | Add-Member -Force -NotePropertyName "active_width" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_width
    $row | Add-Member -Force -NotePropertyName "active_height" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_height
    $row | Add-Member -Force -NotePropertyName "active_fps" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_fps
    $row | Add-Member -Force -NotePropertyName "active_bitrate_mbps" -NotePropertyValue $automationReport.mediaPipelineSnapshot.active_bitrate_mbps
    $row | Add-Member -Force -NotePropertyName "actual_capture_source_id" -NotePropertyValue $automationReport.captureSource.id
    $row | Add-Member -Force -NotePropertyName "actual_capture_source_kind" -NotePropertyValue $automationReport.captureSource.source_kind
    $row | Add-Member -Force -NotePropertyName "motion_stimulus_title" -NotePropertyValue $(if ($NoMotionStimulus) { $null } else { $motionStimulusTitle })
    $row | Add-Member -Force -NotePropertyName "motion_stimulus_pid" -NotePropertyValue $(if ($motionStimulus) { $motionStimulus.Id } else { $null })
    if ($automationReport.captureSource.source_kind -match "^display") {
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
  $savedBuildEnv = @{}
  try {
    Set-EnvVar "GIT_COMMIT" $gitCommit $savedBuildEnv
    Set-EnvVar "CARGO_TARGET_DIR" (Join-Path $repo "target") $savedBuildEnv
    cargo build -p mrd-service
    cargo build -p app --no-default-features
  } finally {
    Restore-EnvVars $savedBuildEnv
  }
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
    -ServiceExePath $ServiceExePath `
    -CaptureSourceId $CaptureSourceId `
    -CaptureSourceKind $CaptureSourceKind `
    -RenderMaxFps $RenderMaxFps `
    -RenderPresentMode $RenderPresentMode `
    -TauriStartupGraceSecs $TauriStartupGraceSecs `
    -NoMotionStimulus:$NoMotionStimulus `
    -NoRenderProfileCap:$NoRenderProfileCap `
    -NoBuild:$NoBuild `
    -ShowTauriWindow:$ShowTauriWindow `
    -KeepTauriOpen:$KeepTauriOpen
}

$report = New-PairedLanCanaryReport -Mode "local-dual-process" -Rows $rows -GitCommit $gitCommit -Codec $Codec
$backends = Get-CanaryCodecBackends -Codec $Codec
$report | Add-Member -Force -NotePropertyName "chain" -NotePropertyValue "local_dual_process/dxgi/$($backends.encoder)/quic_datagram_media_v3_or_v2/$($backends.decoder)/d3d11_shared"
$serviceBoundaryFailures = @(
  $rows |
    Where-Object { $_.service_boundary_gate -and -not $_.service_boundary_gate.passed } |
    ForEach-Object {
      [pscustomobject]@{
        id = $_.id
        failures = @($_.service_boundary_gate.failures)
        run_dir = $_.run_dir
      }
    }
)
$report | Add-Member -Force -NotePropertyName "service_boundary_gate" -NotePropertyValue ([pscustomobject]@{
  passed = ($serviceBoundaryFailures.Count -eq 0)
  failure_count = $serviceBoundaryFailures.Count
  failures = @($serviceBoundaryFailures)
})
$localGateFailures = @($report.rows | Where-Object { $_.status -ne "completed" })
$localGateFailures += @($report.service_boundary_gate.failures)
$report | Add-Member -Force -NotePropertyName "gate" -NotePropertyValue ([pscustomobject]@{
  passed = ($localGateFailures.Count -eq 0)
  verdict = if ($localGateFailures.Count -eq 0) { "PASS" } else { "PRODUCT_FAIL" }
  failures = @($localGateFailures)
})
$report | Add-Member -Force -NotePropertyName "test_impairment_config" -NotePropertyValue $impairment
$report | Add-Member -Force -NotePropertyName "capture_source_request" -NotePropertyValue ([pscustomobject]@{
  id = if ($CaptureSourceId.Trim()) { $CaptureSourceId.Trim() } else { $null }
  kind = if ($CaptureSourceKind.Trim()) { $CaptureSourceKind.Trim() } else { $null }
})
$report | Add-Member -Force -NotePropertyName "render_display_source_request" -NotePropertyValue $(if ($RenderDisplaySourceId.Trim()) { $RenderDisplaySourceId.Trim() } else { $null })
$report | Add-Member -Force -NotePropertyName "render_max_fps_override" -NotePropertyValue $(if ($RenderMaxFps -gt 0) { $RenderMaxFps } else { $null })
$report | Add-Member -Force -NotePropertyName "render_present_mode_request" -NotePropertyValue $RenderPresentMode
$report | Add-Member -Force -NotePropertyName "tauri_window_visible" -NotePropertyValue ([bool]$ShowTauriWindow)
$report | Add-Member -Force -NotePropertyName "codec_request" -NotePropertyValue ([pscustomobject]@{
  codec = $Codec
  codec_profile = if ($CodecProfile.Trim()) { $CodecProfile.Trim() } else { $null }
  av1_mode = $Av1Mode
  bit_depth = if ($BitDepth -gt 0) { $BitDepth } else { $null }
  chroma_subsampling = if ($ChromaSubsampling.Trim()) { $ChromaSubsampling.Trim() } else { $null }
  pixel_format = if ($PixelFormat.Trim()) { $PixelFormat.Trim() } else { $null }
  hdr_enabled = $HdrEnabled
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
  "## Service Boundary Gate",
  "",
  "- Passed: $($report.service_boundary_gate.passed)",
  "- FailureCount: $($report.service_boundary_gate.failure_count)"
)
foreach ($failure in @($report.service_boundary_gate.failures)) {
  Add-Content -Path $markdownPath -Encoding Ascii -Value "- $($failure.id): $($failure.failures -join ', ')"
}

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

Add-Content -Path $markdownPath -Encoding Ascii -Value @(
  "",
  "## Codec Request",
  "",
  "- Codec: $Codec",
  "- CodecProfile: $(if ($CodecProfile.Trim()) { $CodecProfile.Trim() } else { '-' })",
  "- BitDepth: $(if ($BitDepth -gt 0) { $BitDepth } else { '-' })",
  "- ChromaSubsampling: $(if ($ChromaSubsampling.Trim()) { $ChromaSubsampling.Trim() } else { '-' })",
  "- PixelFormat: $(if ($PixelFormat.Trim()) { $PixelFormat.Trim() } else { '-' })",
  "- HdrEnabled: $HdrEnabled"
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
if (-not $report.gate.passed) {
  Write-Error ("Local dual-process LAN canary gate failed with $($report.gate.failures.Count) failure(s)")
  exit 2
}
