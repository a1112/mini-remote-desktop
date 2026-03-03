$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$controllerDir = Join-Path $base 'controller-rust'
$agentDir = Join-Path $base 'agent-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$agentCfgPath = Join-Path $agentDir 'config.json'
$agentBak = Join-Path $agentDir ('config.accept.transportcmp.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
Copy-Item $agentCfgPath $agentBak -Force

function Parse-Agent([string]$path) {
  $out = [ordered]@{
    send_fps = -1.0
    unique_fps = -1.0
    quic_au_sent = 0
    quic_au_dropped = 0
  }
  if (!(Test-Path $path)) { return $out }
  $line = (Get-Content $path | Select-String '\[RTCP-PANEL\]' | Select-Object -Last 1).Line
  if (!$line) { return $out }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  $m = [regex]::Match($plain, 'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
  if ($m.Success) { $out.send_fps = [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
  if ($m.Success) { $out.unique_fps = [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'quic_au_sent=([0-9]+)')
  if ($m.Success) { $out.quic_au_sent = [int64]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'quic_au_dropped=([0-9]+)')
  if ($m.Success) { $out.quic_au_dropped = [int64]$m.Groups[1].Value }
  return $out
}

function Parse-Controller([string]$path) {
  $out = [ordered]@{
    selected_transport = ''
    quic_connected = $false
  }
  if (!(Test-Path $path)) { return $out }
  $content = Get-Content $path
  $line = ($content | Select-String 'received WebRTC answer' | Select-Object -Last 1).Line
  if ($line) {
    $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
    $m = [regex]::Match($plain, 'selected_transport=([^\s]+)')
    if ($m.Success) { $out.selected_transport = $m.Groups[1].Value.Trim('"') }
  }
  if (($content | Select-String 'connected to QUIC media transport' | Select-Object -First 1)) {
    $out.quic_connected = $true
  }
  return $out
}

try {
  $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  $cfg.capture.target_width = 1920
  $cfg.capture.target_height = 1080
  $cfg.capture.fps = 240
  $cfg.capture.min_fps = 240
  $cfg.capture.max_fps = 240
  $cfg.capture.max_fps_mode = $true
  $cfg.capture.idle_repeat_fps = 240
  $cfg.capture.encoder = 'auto'
  $cfg.capture.queue_depth = 8
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  foreach ($transport in @('webrtc','quic')) {
    $tag = "accept.transportcmp.$transport." + (Get-Date -Format 'HHmmss')
    $slog = Join-Path $base ($tag + '.s.log')
    $serr = Join-Path $base ($tag + '.s.err')
    $alog = Join-Path $base ($tag + '.a.log')
    $aerr = Join-Path $base ($tag + '.a.err')
    $clog = Join-Path $base ($tag + '.c.log')
    $cerr = Join-Path $base ($tag + '.c.err')

    Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
      Stop-Process -Force -ErrorAction SilentlyContinue

    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
      -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700

    $ap = Start-Process -FilePath 'cmd.exe' `
      -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_QUIC_QUEUE=128&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& `"$agentExe`"") `
      -WorkingDirectory $agentDir -PassThru `
      -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2

    $cp = Start-Process -FilePath 'cmd.exe' `
      -ArgumentList '/c',("set MRD_TRANSPORT=$transport&& set MRD_DECODER=d3d11va&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
      -WorkingDirectory $controllerDir -PassThru `
      -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds 60

    if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
    if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
    if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

    $as = Parse-Agent $alog
    $cs = Parse-Controller $clog

    Write-Output ("transport={0} selected={1} quic_connected={2} send_fps={3} unique_fps={4} quic_au_sent={5} quic_au_dropped={6}" -f `
      $transport,$cs.selected_transport,$cs.quic_connected,([math]::Round($as.send_fps,2)),([math]::Round($as.unique_fps,2)),$as.quic_au_sent,$as.quic_au_dropped)
    Write-Output ("logs: a={0} c={1} s={2}" -f $alog,$clog,$slog)
  }
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    Remove-Item $agentBak -Force -ErrorAction SilentlyContinue
  }
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

