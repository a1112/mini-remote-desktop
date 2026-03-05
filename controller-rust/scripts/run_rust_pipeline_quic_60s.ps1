param(
  [int]$DurationSec = 60,
  [string]$Transport = "quic",
  [string]$Decoder = "d3d11va",
  [int]$AgentCaptureFps = 0,
  [int]$AgentCaptureMinFps = 0,
  [int]$AgentCaptureMaxFps = 0,
  [int]$AgentCaptureBitrateKbps = 0,
  [int]$AgentCaptureMaxBitrateKbps = 0,
  [int]$AgentCaptureNetworkFloorKbps = 0,
  [int]$AgentCaptureNetworkCeilingKbps = 0,
  [string]$AgentCaptureEncoderPreset = "",
  [string]$AgentCaptureEncoderTune = "",
  [string]$AgentCapturePerfProfile = "",
  [string]$AgentCaptureProfileTemplate = "",
  [string]$AgentCaptureFpsMode = "",
  [string]$RenderMode = "",
  [string]$RenderDropOld = "",
  [int]$RenderMaxAgeMs = 0,
  [string]$PresentAdaptive = "",
  [int]$PresentAdaptiveMinFps = 0,
  [int]$PresentAdaptiveMaxFps = 0,
  [int]$PresentTargetFps = 0,
  [string]$DecodeSelect = "",
  [int]$DecodeAdaptiveMaxAgeMs = 0,
  [int]$TargetFps = 0,
  [int]$QuicRxQueue = 0,
  [string]$Tag = ("rust.pipeline." + (Get-Date -Format "yyyyMMdd_HHmmss"))
)

$ErrorActionPreference = "Stop"

$base = "J:/ProjectTest/remote-desktop/mini-remote-desktop"
$signalingDir = Join-Path $base "signaling-rs"
$agentDir = Join-Path $base "agent-rust"
$controllerDir = Join-Path $base "controller-rust"

$signalingExe = Join-Path $signalingDir "target/debug/signaling-rs.exe"
$agentExe = Join-Path $agentDir "target/debug/agent-rust.exe"
$controllerExe = Join-Path $controllerDir "target/debug/controller-rust.exe"
$ffmpegExe = Join-Path $base "tools/ffmpeg_full_build/bin/ffmpeg.exe"

$slog = Join-Path $base ($Tag + ".s.log")
$serr = Join-Path $base ($Tag + ".s.err")
$alog = Join-Path $base ($Tag + ".a.log")
$aerr = Join-Path $base ($Tag + ".a.err")
$clog = Join-Path $base ($Tag + ".c.log")
$cerr = Join-Path $base ($Tag + ".c.err")

function Stop-Mrd {
  Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "controller-rust") } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Remove-Ansi([string]$s) {
  if ($null -eq $s) { return "" }
  return [regex]::Replace($s, "\x1B\[[0-9;]*m", "")
}

function Last-Line([string]$path, [string]$needle) {
  if (!(Test-Path $path)) { return "" }
  $line = (Get-Content $path | Select-String $needle | Select-Object -Last 1).Line
  return (Remove-Ansi $line)
}

function Parse-KeyNum([string]$line, [string]$key) {
  if ([string]::IsNullOrWhiteSpace($line)) { return [double]::NaN }
  $m = [regex]::Match($line, ($key + '="?([0-9]+(?:\.[0-9]+)?)"?'))
  if ($m.Success) { return [double]$m.Groups[1].Value }
  return [double]::NaN
}

function Judge([string]$name, [double]$value, [double]$limit, [string]$op) {
  $ok = $false
  if ($op -eq "<=") { $ok = ($value -le $limit) }
  elseif ($op -eq ">=") { $ok = ($value -ge $limit) }
  $status = if ($ok) { "PASS" } else { "FAIL" }
  [pscustomobject]@{ item = $name; value = $value; limit = $limit; op = $op; status = $status }
}

Stop-Mrd

try {
  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 800

  $agentCmd = @(
    "set AGENT_FFMPEG_PATH=$ffmpegExe",
    "set AGENT_QUIC_QUEUE=512",
    "set AGENT_WEBTRANSPORT_QUEUE=256",
    "set AGENT_QUIC_PACE_ENABLE=1",
    "set AGENT_QUIC_PACE_MODE=manual",
    "set AGENT_QUIC_PACE_INTERVAL_MS=1",
    "set AGENT_QUIC_PACE_BURST=2",
    "set RUST_LOG=agent_rust=info",
    "`"$agentExe`""
  ) -join "&& "
  if ($AgentCaptureFps -gt 0) { $agentCmd = "set AGENT_CAPTURE_FPS=$AgentCaptureFps&& " + $agentCmd }
  if ($AgentCaptureMinFps -gt 0) { $agentCmd = "set AGENT_CAPTURE_MIN_FPS=$AgentCaptureMinFps&& " + $agentCmd }
  if ($AgentCaptureMaxFps -gt 0) { $agentCmd = "set AGENT_CAPTURE_MAX_FPS=$AgentCaptureMaxFps&& " + $agentCmd }
  if ($AgentCaptureBitrateKbps -gt 0) { $agentCmd = "set AGENT_CAPTURE_BITRATE_KBPS=$AgentCaptureBitrateKbps&& " + $agentCmd }
  if ($AgentCaptureMaxBitrateKbps -gt 0) { $agentCmd = "set AGENT_CAPTURE_MAX_BITRATE_KBPS=$AgentCaptureMaxBitrateKbps&& " + $agentCmd }
  if ($AgentCaptureNetworkFloorKbps -gt 0) { $agentCmd = "set AGENT_CAPTURE_NETWORK_FLOOR_KBPS=$AgentCaptureNetworkFloorKbps&& " + $agentCmd }
  if ($AgentCaptureNetworkCeilingKbps -gt 0) { $agentCmd = "set AGENT_CAPTURE_NETWORK_CEILING_KBPS=$AgentCaptureNetworkCeilingKbps&& " + $agentCmd }
  if (![string]::IsNullOrWhiteSpace($AgentCaptureEncoderPreset)) { $agentCmd = "set AGENT_CAPTURE_ENCODER_PRESET=$AgentCaptureEncoderPreset&& " + $agentCmd }
  if (![string]::IsNullOrWhiteSpace($AgentCaptureEncoderTune)) { $agentCmd = "set AGENT_CAPTURE_ENCODER_TUNE=$AgentCaptureEncoderTune&& " + $agentCmd }
  if (![string]::IsNullOrWhiteSpace($AgentCapturePerfProfile)) { $agentCmd = "set AGENT_CAPTURE_PERF_PROFILE=$AgentCapturePerfProfile&& " + $agentCmd }
  if (![string]::IsNullOrWhiteSpace($AgentCaptureProfileTemplate)) { $agentCmd = "set AGENT_CAPTURE_PROFILE_TEMPLATE=$AgentCaptureProfileTemplate&& " + $agentCmd }
  if (![string]::IsNullOrWhiteSpace($AgentCaptureFpsMode)) { $agentCmd = "set AGENT_CAPTURE_FPS_MODE=$AgentCaptureFpsMode&& " + $agentCmd }
  $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $controllerCmd = @(
    "set MRD_TRANSPORT=$Transport",
    "set MRD_DECODER=$Decoder",
    "set MRD_RECORD_ENABLE=0",
    "set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn",
    "`"$controllerExe`""
  ) -join "&& "
  if (![string]::IsNullOrWhiteSpace($RenderMode)) { $controllerCmd = "set MRD_RENDER_MODE=$RenderMode&& " + $controllerCmd }
  if (![string]::IsNullOrWhiteSpace($RenderDropOld)) { $controllerCmd = "set MRD_RENDER_DROP_OLD=$RenderDropOld&& " + $controllerCmd }
  if ($RenderMaxAgeMs -gt 0) { $controllerCmd = "set MRD_RENDER_MAX_AGE_MS=$RenderMaxAgeMs&& " + $controllerCmd }
  if (![string]::IsNullOrWhiteSpace($PresentAdaptive)) { $controllerCmd = "set MRD_PRESENT_ADAPTIVE=$PresentAdaptive&& " + $controllerCmd }
  if ($PresentAdaptiveMinFps -gt 0) { $controllerCmd = "set MRD_PRESENT_ADAPTIVE_MIN_FPS=$PresentAdaptiveMinFps&& " + $controllerCmd }
  if ($PresentAdaptiveMaxFps -gt 0) { $controllerCmd = "set MRD_PRESENT_ADAPTIVE_MAX_FPS=$PresentAdaptiveMaxFps&& " + $controllerCmd }
  if ($PresentTargetFps -gt 0) { $controllerCmd = "set MRD_PRESENT_TARGET_FPS=$PresentTargetFps&& " + $controllerCmd }
  if (![string]::IsNullOrWhiteSpace($DecodeSelect)) { $controllerCmd = "set MRD_DECODE_SELECT=$DecodeSelect&& " + $controllerCmd }
  if ($DecodeAdaptiveMaxAgeMs -gt 0) { $controllerCmd = "set MRD_DECODE_ADAPTIVE_MAX_AGE_MS=$DecodeAdaptiveMaxAgeMs&& " + $controllerCmd }
  if ($TargetFps -gt 0) { $controllerCmd = "set MRD_TARGET_FPS=$TargetFps&& " + $controllerCmd }
  if ($QuicRxQueue -gt 0) { $controllerCmd = "set MRD_QUIC_RX_QUEUE=$QuicRxQueue&& " + $controllerCmd }
  $cp = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $controllerCmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds $DurationSec
}
finally {
  Stop-Mrd
}

$connected = ((Get-Content $clog | Select-String "connected to QUIC media transport" | Select-Object -First 1) -ne $null)

$agentLine = Last-Line $alog '\[PIPELINE-STATS\].*side.*agent'
$decLine = Last-Line $clog '\[PIPELINE-STATS\].*side.*controller_decode'
$renLine = Last-Line $clog '\[PIPELINE-STATS\].*side.*controller_render'
$preLine = Last-Line $clog '\[PRESENT-STATS\]'

$rows = @()
$rows += Judge "connected_quic" ([double]([int]$connected)) 1 ">="
$rows += Judge "decode_p95_ms" (Parse-KeyNum $decLine "stage_decode_p95_ms") 6.0 "<="
$rows += Judge "decode_e2e_p95_ms" (Parse-KeyNum $decLine "overall_e2e_p95_ms") 30.0 "<="
$rows += Judge "decode_e2e_p99_ms" (Parse-KeyNum $decLine "overall_e2e_p99_ms") 45.0 "<="
$rows += Judge "render_present_p95_ms" (Parse-KeyNum $renLine "stage_render_present_p95_ms") 32.0 "<="
$rows += Judge "present_call_p95_ms" (Parse-KeyNum $renLine "present_call_p95_ms") 1.0 "<="
$rows += Judge "agent_send_interval_jitter_ms" (Parse-KeyNum $agentLine "stage_send_interval_jitter_ms") 2.0 "<="
$rows += Judge "agent_queue_wait_std_ms" (Parse-KeyNum $agentLine "stage_queue_wait_std_ms") 2.0 "<="

$pass = ($rows | Where-Object { $_.status -eq "FAIL" }).Count -eq 0

Write-Output ("tag={0}" -f $Tag)
Write-Output ("status={0}" -f ($(if ($pass) { "PASS" } else { "FAIL" })))
Write-Output ("controller_log={0}" -f $clog)
Write-Output ("agent_log={0}" -f $alog)
Write-Output ""
Write-Output "metrics:"
$rows | Format-Table -AutoSize | Out-String -Width 220 | Write-Output
Write-Output "last_agent_pipeline:"
Write-Output $agentLine
Write-Output "last_decode_pipeline:"
Write-Output $decLine
Write-Output "last_render_pipeline:"
Write-Output $renLine
Write-Output "last_present_stats:"
Write-Output $preLine
