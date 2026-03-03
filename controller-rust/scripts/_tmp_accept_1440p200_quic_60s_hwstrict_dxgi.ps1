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
$agentBak = Join-Path $agentDir ('config.accept.quic1440p200.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')

Copy-Item $agentCfgPath $agentBak -Force

function Parse-AgentStats([string]$path) {
  $out = [ordered]@{
    send_fps = -1.0
    unique_fps = -1.0
    quic_au_sent = 0
    quic_au_dropped = 0
    quic_bytes_sent = 0
    native_direct_frames = 0
    native_copy_frames = 0
    native_scale_frames = 0
    native_direct_register_failures = 0
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
  $m = [regex]::Match($plain, 'quic_bytes_sent=([0-9]+)')
  if ($m.Success) { $out.quic_bytes_sent = [int64]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'native_direct_frames=([0-9]+)')
  if ($m.Success) { $out.native_direct_frames = [int64]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'native_copy_frames=([0-9]+)')
  if ($m.Success) { $out.native_copy_frames = [int64]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'native_scale_frames=([0-9]+)')
  if ($m.Success) { $out.native_scale_frames = [int64]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'native_direct_register_failures=([0-9]+)')
  if ($m.Success) { $out.native_direct_register_failures = [int64]$m.Groups[1].Value }
  return $out
}

function Set-JsonField($obj, [string]$name, $value) {
  if ($obj.PSObject.Properties.Name -contains $name) {
    $obj.$name = $value
  } else {
    $obj | Add-Member -NotePropertyName $name -NotePropertyValue $value
  }
}

function Parse-ControllerStats([string]$path) {
  $out = [ordered]@{
    connected_quic = $false
    fps = -1.0
    avg_decode_ms = -1.0
    p95_decode_ms = -1.0
    jitter_ms = -1.0
  }
  if (!(Test-Path $path)) { return $out }
  $content = Get-Content $path
  if (($content | Select-String 'connected to QUIC media transport' | Select-Object -First 1)) {
    $out.connected_quic = $true
  }
  $line = ($content | Select-String '\[DECODER-STATS\]' | Select-Object -Last 1).Line
  if (!$line) { return $out }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  $m = [regex]::Match($plain, 'fps=\"?([0-9]+(?:\.[0-9]+)?)\"?')
  if ($m.Success) { $out.fps = [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'avg_decode_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?')
  if ($m.Success) { $out.avg_decode_ms = [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'p95_decode_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?')
  if ($m.Success) { $out.p95_decode_ms = [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, 'jitter_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?')
  if ($m.Success) { $out.jitter_ms = [double]$m.Groups[1].Value }
  return $out
}

try {
  $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  # Strict GPU-direct trial: keep source resolution to avoid scale/copy path.
  $cfg.capture.target_width = 2560
  $cfg.capture.target_height = 1440
  $cfg.capture.fps = 200
  $cfg.capture.min_fps = 200
  $cfg.capture.max_fps = 200
  $cfg.capture.max_fps_mode = $true
  $cfg.capture.idle_repeat_fps = 200
  Set-JsonField $cfg.capture 'backend' 'dxgi'
  Set-JsonField $cfg.capture 'encoder' 'nvenc'
  Set-JsonField $cfg.capture 'strict_gpu_direct' $true
  Set-JsonField $cfg.capture 'allow_fallback' $false
  Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
  $cfg.capture.queue_depth = 8
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  $tag = "accept.quic.1440p200." + (Get-Date -Format 'HHmmss')
  $slog = Join-Path $base ($tag + '.s.log')
  $serr = Join-Path $base ($tag + '.s.err')
  $alog = Join-Path $base ($tag + '.a.log')
  $aerr = Join-Path $base ($tag + '.a.err')
  $clog = Join-Path $base ($tag + '.c.log')
  $cerr = Join-Path $base ($tag + '.c.err')

  @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
  }

  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue

  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
    -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700

  $ap = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_CAPTURE_BACKEND_FORCE=dxgi&& set AGENT_DXGI_OUTPUT_INDEX=0&& set AGENT_QUIC_DEBUG=1&& set AGENT_QUIC_QUEUE=128&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& `"$agentExe`"") `
    -WorkingDirectory $agentDir -PassThru `
    -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $cp = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set MRD_REQUIRE_D3D11VA=1&& set MRD_HARDWARE_FAIL_FAST=1&& set MRD_TRY_MF_FALLBACK=0&& set MRD_DISABLE_DECODE_RECOVER=1&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
    -WorkingDirectory $controllerDir -PassThru `
    -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds 60

  if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
  if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
  if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

  $as = Parse-AgentStats $alog
  $cs = Parse-ControllerStats $clog

  Write-Output ("quic_connected={0} controller_fps={1} avg_decode_ms={2} p95_decode_ms={3} jitter_ms={4}" -f `
    $cs.connected_quic,([math]::Round($cs.fps,2)),([math]::Round($cs.avg_decode_ms,3)),([math]::Round($cs.p95_decode_ms,3)),([math]::Round($cs.jitter_ms,3)))
  Write-Output ("agent_send_fps={0} unique_fps={1} quic_au_sent={2} quic_au_dropped={3} quic_bytes_sent={4}" -f `
    ([math]::Round($as.send_fps,2)),([math]::Round($as.unique_fps,2)),$as.quic_au_sent,$as.quic_au_dropped,$as.quic_bytes_sent)
  Write-Output ("agent_native_direct={0} native_copy={1} native_scale={2} native_direct_reg_fail={3}" -f `
    $as.native_direct_frames,$as.native_copy_frames,$as.native_scale_frames,$as.native_direct_register_failures)
  Write-Output ("logs: agent={0} controller={1} signaling={2}" -f $alog,$clog,$slog)
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    Remove-Item $agentBak -Force -ErrorAction SilentlyContinue
  }
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}







