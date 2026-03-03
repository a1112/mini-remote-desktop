$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$controllerDir = Join-Path $base 'controller-rust'
$agentDir = Join-Path $base 'agent-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$signalingExe = Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$agentCfgPath = Join-Path $agentDir 'config.json'
$agentBak = Join-Path $agentDir ('config.accept.decodecmp.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')

Copy-Item $agentCfgPath $agentBak -Force

function Parse-ControllerStats([string]$path) {
  $out = [ordered]@{
    backend = ''
    fps = -1.0
    avg_decode_ms = -1.0
    p95_decode_ms = -1.0
    jitter_ms = -1.0
  }
  if (!(Test-Path $path)) { return $out }
  $line = (Get-Content $path | Select-String '\[DECODER-STATS\]' | Select-Object -Last 1).Line
  if (!$line) { return $out }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  $m = [regex]::Match($plain, 'backend=([^\s]+)')
  if ($m.Success) { $out.backend = $m.Groups[1].Value }
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
  $cfg.capture.target_width = 1920
  $cfg.capture.target_height = 1080
  $cfg.capture.fps = 240
  $cfg.capture.min_fps = 240
  $cfg.capture.max_fps = 240
  $cfg.capture.max_fps_mode = $true
  $cfg.capture.idle_repeat_fps = 240
  $cfg.capture.encoder = 'auto'
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  $modes = @('software', 'd3d11va')
  $results = @()

  foreach ($mode in $modes) {
    $tag = "accept.decodecmp.$mode." + (Get-Date -Format 'HHmmss')
    $slog = Join-Path $base ($tag + '.s.log')
    $serr = Join-Path $base ($tag + '.s.err')
    $alog = Join-Path $base ($tag + '.a.log')
    $aerr = Join-Path $base ($tag + '.a.err')
    $clog = Join-Path $base ($tag + '.c.log')
    $cerr = Join-Path $base ($tag + '.c.err')

    @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object {
      if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
    }

    Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | `
      Stop-Process -Force -ErrorAction SilentlyContinue

    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
      -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700

    $ap = Start-Process -FilePath 'cmd.exe' `
      -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") `
      -WorkingDirectory $agentDir -PassThru `
      -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2

    $cp = Start-Process -FilePath 'cmd.exe' `
      -ArgumentList '/c',("set MRD_DECODER=$mode&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
      -WorkingDirectory $controllerDir -PassThru `
      -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds 18

    if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
    if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
    if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

    $cs = Parse-ControllerStats $clog
    $row = [pscustomobject]@{
      mode = $mode
      backend = $cs.backend
      fps = [math]::Round($cs.fps, 2)
      avg_decode_ms = [math]::Round($cs.avg_decode_ms, 3)
      p95_decode_ms = [math]::Round($cs.p95_decode_ms, 3)
      jitter_ms = [math]::Round($cs.jitter_ms, 3)
      controller_log = $clog
    }
    $results += $row
    Write-Output ("mode={0} backend={1} fps={2} avg_decode_ms={3} p95_decode_ms={4} jitter_ms={5}" -f `
      $row.mode, $row.backend, $row.fps, $row.avg_decode_ms, $row.p95_decode_ms, $row.jitter_ms)
  }

  Write-Output '=== decode compare summary ==='
  $results | ForEach-Object {
    Write-Output ("mode={0} backend={1} fps={2} avg={3} p95={4} jitter={5} log={6}" -f `
      $_.mode,$_.backend,$_.fps,$_.avg_decode_ms,$_.p95_decode_ms,$_.jitter_ms,$_.controller_log)
  }
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    Remove-Item $agentBak -Force -ErrorAction SilentlyContinue
  }
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | `
    Stop-Process -Force -ErrorAction SilentlyContinue
}
