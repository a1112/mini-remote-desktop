param(
  [string]$Base = 'J:/ProjectTest/remote-desktop/mini-remote-desktop',
  [int]$DurationSec = 40
)
$ErrorActionPreference = 'Stop'

$cfgPath = Join-Path $Base 'agent-rust/config.json'
$bak = Join-Path $Base ('agent-rust/config.lowlat1080p.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingDir = Join-Path $Base 'signaling-rs'
$agentDir = Join-Path $Base 'agent-rust'
$controllerDir = Join-Path $Base 'controller-rust'
$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $Base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd {
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs', 'agent-rust', 'controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Set-Capture([int]$fps) {
  $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
  $cfg.capture.target_width = 1920
  $cfg.capture.target_height = 1080
  $cfg.capture.fps = $fps
  $cfg.capture.min_fps = $fps
  $cfg.capture.max_fps = $fps
  $cfg.capture.idle_repeat_fps = $fps
  $cfg.capture.max_fps_mode = $false
  $cfg.capture.frame_pacing_enable = $false
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii
}

function Last-Line([string]$path, [string]$pat) {
  $line = (Get-Content $path | Select-String $pat | Select-Object -Last 1).Line
  if (-not $line) { return '' }
  return [regex]::Replace($line, '\x1B\[[0-9;]*m', '')
}

function Num([string]$line, [string]$key) {
  $m = [regex]::Match($line, ($key + '="?([0-9]+(?:\.[0-9]+)?)"?'))
  if ($m.Success) { return [double]$m.Groups[1].Value }
  return [double]::NaN
}

function Run-Case([int]$fps, [int]$queue, [int]$renderMaxAgeMs) {
  Set-Capture -fps $fps
  Stop-Mrd
  $tag = ('rust.lowlat1080p.f' + $fps + '.q' + $queue + '.r' + $renderMaxAgeMs + '.' + (Get-Date -Format 'HHmmss'))
  $slog = Join-Path $Base ($tag + '.s.log')
  $serr = Join-Path $Base ($tag + '.s.err')
  $alog = Join-Path $Base ($tag + '.a.log')
  $aerr = Join-Path $Base ($tag + '.a.err')
  $clog = Join-Path $Base ($tag + '.c.log')
  $cerr = Join-Path $Base ($tag + '.c.err')

  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 800

  $agentCmd = @(
    "set AGENT_FFMPEG_PATH=$ffmpegExe",
    "set AGENT_QUIC_QUEUE=$queue",
    "set AGENT_QUIC_PACE_ENABLE=1",
    "set AGENT_QUIC_PACE_MODE=manual",
    "set AGENT_QUIC_PACE_INTERVAL_MS=1",
    "set AGENT_QUIC_PACE_BURST=2",
    'set RUST_LOG=agent_rust=info',
    "`"$agentExe`""
  ) -join '&& '
  $ap = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $controllerCmd = @(
    'set MRD_TRANSPORT=quic',
    'set MRD_DECODER=d3d11va',
    'set MRD_RECORD_ENABLE=0',
    "set MRD_RENDER_MAX_AGE_MS=$renderMaxAgeMs",
    'set MRD_RENDER_DROP_OLD=1',
    "set MRD_PRESENT_TARGET_FPS=$fps",
    'set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn',
    "`"$controllerExe`""
  ) -join '&& '
  $cp = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $controllerCmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds $DurationSec
  Stop-Mrd

  $dec = Last-Line $clog '\[PIPELINE-STATS\].*side.*controller_decode'
  $ren = Last-Line $clog '\[PIPELINE-STATS\].*side.*controller_render'
  $agent = Last-Line $alog '\[PIPELINE-STATS\].*side.*agent'
  $conn = (Get-Content $clog | Select-String 'connected to QUIC media transport' | Select-Object -First 1) -ne $null

  [pscustomobject]@{
    fps = $fps
    quic_queue = $queue
    render_max_age_ms = $renderMaxAgeMs
    connected = $conn
    fps_decode = Num $dec 'fps_decode'
    fps_render = Num $ren 'fps_render'
    decode_p95_ms = Num $dec 'stage_decode_p95_ms'
    e2e_p95_ms = Num $dec 'overall_e2e_p95_ms'
    e2e_p99_ms = Num $dec 'overall_e2e_p99_ms'
    present_p95_ms = Num $ren 'stage_render_present_p95_ms'
    present_call_p95_ms = Num $ren 'present_call_p95_ms'
    agent_send_jitter_ms = Num $agent 'stage_send_interval_jitter_ms'
    agent_queue_std_ms = Num $agent 'stage_queue_wait_std_ms'
    controller_log = $clog
    agent_log = $alog
  }
}

Copy-Item $cfgPath $bak -Force
try {
  $profiles = @(
    @{ q = 512; r = 8 },
    @{ q = 256; r = 8 },
    @{ q = 256; r = 6 }
  )
  $fpsSet = @(120, 144)
  $rows = @()
  foreach ($f in $fpsSet) {
    foreach ($p in $profiles) {
      $rows += Run-Case -fps $f -queue $p.q -renderMaxAgeMs $p.r
    }
  }

  $rows |
    Sort-Object e2e_p95_ms, e2e_p99_ms |
    Format-Table fps,quic_queue,render_max_age_ms,connected,fps_decode,fps_render,decode_p95_ms,e2e_p95_ms,e2e_p99_ms,present_p95_ms,present_call_p95_ms,agent_send_jitter_ms,agent_queue_std_ms -AutoSize |
    Out-String -Width 300
  'JSON=' + ($rows | ConvertTo-Json -Depth 6 -Compress)
}
finally {
  if (Test-Path $bak) {
    Copy-Item $bak $cfgPath -Force
    Remove-Item $bak -Force
  }
  Stop-Mrd
}
