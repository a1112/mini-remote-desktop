param(
  [int]$DurationSec = 60,
  [string]$Transport = "quic",
  [string]$Decoder = "d3d11va",
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
  $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $controllerCmd = @(
    "set MRD_TRANSPORT=$Transport",
    "set MRD_DECODER=$Decoder",
    "set MRD_RECORD_ENABLE=0",
    "set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn",
    "`"$controllerExe`""
  ) -join "&& "
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
