$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$agentDir = Join-Path $base 'agent-rust'
$controllerDir = Join-Path $base 'controller-rust'
$signalingDir = Join-Path $base 'signaling-rs'
$agentCfg = Join-Path $agentDir 'config.json'
$bak = Join-Path $agentDir ('config.single1080.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'

Copy-Item $agentCfg $bak -Force

function Stop-Mrd {
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Set-JsonField($obj, [string]$name, $value) {
  if ($obj.PSObject.Properties.Name -contains $name) {
    $obj.$name = $value
  } else {
    $obj | Add-Member -NotePropertyName $name -NotePropertyValue $value
  }
}

try {
  $cfg = Get-Content $agentCfg -Raw | ConvertFrom-Json
  $cfg.capture.fps = 240
  $cfg.capture.min_fps = 240
  $cfg.capture.max_fps = 240
  $cfg.capture.max_fps_mode = $true
  $cfg.capture.idle_repeat_fps = 240
  $cfg.capture.target_width = 1920
  $cfg.capture.target_height = 1080
  Set-JsonField $cfg.capture 'backend' 'dxgi'
  Set-JsonField $cfg.capture 'encoder' 'nvenc'
  Set-JsonField $cfg.capture 'strict_gpu_direct' $true
  Set-JsonField $cfg.capture 'allow_fallback' $false
  Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
  Set-JsonField $cfg.capture 'encoder_tune' 'ull'
  Set-JsonField $cfg.capture 'encoder_preset' 'p1'
  Set-JsonField $cfg.capture 'rc_mode' 'cbr'
  Set-JsonField $cfg.capture 'bframes' 0
  Set-JsonField $cfg.capture 'gop' 240
  Set-JsonField $cfg.capture 'frame_pacing_enable' $false
  Set-JsonField $cfg.capture 'network_adapt_enable' $false
  Set-JsonField $cfg.capture 'adapt_enable' $false
  Set-JsonField $cfg.capture 'queue_strategy' 'drop'
  Set-JsonField $cfg.capture 'tier_limit_enable' $false
  Set-JsonField $cfg.capture 'max_frame_latency' 1
  Set-JsonField $cfg.capture 'bitrate_kbps' 20000
  Set-JsonField $cfg.capture 'max_bitrate_kbps' 28000
  $cfg.capture.queue_depth = 2
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfg -Encoding Ascii

  $tag = 'accept.single.1080p240.' + (Get-Date -Format 'HHmmss')
  $slog = Join-Path $base ($tag + '.s.log')
  $serr = Join-Path $base ($tag + '.s.err')
  $alog = Join-Path $base ($tag + '.a.log')
  $aerr = Join-Path $base ($tag + '.a.err')
  $clog = Join-Path $base ($tag + '.c.log')
  $cerr = Join-Path $base ($tag + '.c.err')
  @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
  }

  Stop-Mrd
  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
    -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700
  $ap = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_QUIC_QUEUE=64&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& `"$agentExe`"") `
    -WorkingDirectory $agentDir -PassThru `
    -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2
  $cp = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
    -WorkingDirectory $controllerDir -PassThru `
    -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds 25

  if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
  if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
  if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

  $strip = { param($s) if($null -eq $s){return ''}; [regex]::Replace($s, "\x1B\[[0-9;]*m", "") }
  $aline = (Get-Content $alog | Select-String '\[RTCP-PANEL\]' | Select-Object -Last 1).Line
  $dline = (Get-Content $clog | Select-String '\[DECODER-STATS\]' | Select-Object -Last 1).Line
  $pline = (Get-Content $clog | Select-String '\[PRESENT-STATS\]' | Select-Object -Last 1).Line
  $qconn = ((Get-Content $clog | Select-String 'connected to QUIC media transport' | Select-Object -First 1) -ne $null)

  Write-Output ("quic_connected={0}" -f $qconn)
  Write-Output ("AGENT=" + (& $strip $aline))
  Write-Output ("DECODER=" + (& $strip $dline))
  Write-Output ("PRESENT=" + (& $strip $pline))
  Write-Output ("LOGS agent={0} controller={1} signaling={2}" -f $alog,$clog,$slog)
}
finally {
  if (Test-Path $bak) {
    Copy-Item $bak $agentCfg -Force
    Remove-Item $bak -Force -ErrorAction SilentlyContinue
  }
  Stop-Mrd
}
