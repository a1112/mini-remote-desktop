param(
  [string]$Base = 'J:/ProjectTest/remote-desktop/mini-remote-desktop'
)
$ErrorActionPreference = 'Stop'
$cfgPath = Join-Path $Base 'agent-rust/config.json'
$bak = Join-Path $Base ('agent-rust/config.compare1080p.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$runner = Join-Path $Base 'controller-rust/scripts/run_rust_pipeline_quic_60s.ps1'

function Set-CaptureConfig([int]$w,[int]$h,[int]$fps) {
  $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
  if (-not $cfg.capture) { throw 'capture config missing' }
  $cfg.capture.target_width = $w
  $cfg.capture.target_height = $h
  $cfg.capture.fps = $fps
  $cfg.capture.min_fps = $fps
  $cfg.capture.max_fps = $fps
  $cfg.capture.idle_repeat_fps = $fps
  $cfg.capture.max_fps_mode = $false
  $cfg.capture.frame_pacing_enable = $false
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii
}

function Last-Line([string]$path,[string]$pat) {
  $line = (Get-Content $path | Select-String $pat | Select-Object -Last 1).Line
  if (-not $line) { return '' }
  return [regex]::Replace($line, '\x1B\[[0-9;]*m', '')
}

function Num([string]$line,[string]$key) {
  $m = [regex]::Match($line, ($key + '="?([0-9]+(?:\.[0-9]+)?)"?'))
  if ($m.Success) { return [double]$m.Groups[1].Value }
  return [double]::NaN
}

function Run-Case([string]$name,[int]$w,[int]$h,[int]$fps) {
  Set-CaptureConfig -w $w -h $h -fps $fps
  $tag = ('rust.' + $name + '.' + (Get-Date -Format 'HHmmss'))
  $out = powershell -ExecutionPolicy Bypass -File $runner -DurationSec 60 -Tag $tag
  $clog = ($out | Select-String '^controller_log=').ToString().Split('=', 2)[1]
  $alog = ($out | Select-String '^agent_log=').ToString().Split('=', 2)[1]
  $d = Last-Line $clog '\[PIPELINE-STATS\].*side="controller_decode"'
  $r = Last-Line $clog '\[PIPELINE-STATS\].*side="controller_render"'
  $a = Last-Line $alog '\[PIPELINE-STATS\].*side="agent"'
  [pscustomobject]@{
    case = $name
    resolution = ($w.ToString() + 'x' + $h.ToString())
    fps_cfg = $fps
    fps_decode = Num $d 'fps_decode'
    fps_render = Num $r 'fps_render'
    decode_p95_ms = Num $d 'stage_decode_p95_ms'
    e2e_p95_ms = Num $d 'overall_e2e_p95_ms'
    e2e_p99_ms = Num $d 'overall_e2e_p99_ms'
    present_p95_ms = Num $r 'stage_render_present_p95_ms'
    present_call_p95_ms = Num $r 'present_call_p95_ms'
    agent_send_jitter_ms = Num $a 'stage_send_interval_jitter_ms'
    agent_queue_std_ms = Num $a 'stage_queue_wait_std_ms'
    controller_log = $clog
    agent_log = $alog
  }
}

Copy-Item $cfgPath $bak -Force
try {
  $twoK = Run-Case -name '2k' -w 2560 -h 1440 -fps 144
  $fhd = Run-Case -name '1080p' -w 1920 -h 1080 -fps 144
  $rows = @($twoK, $fhd)
  $rows | Format-Table case,resolution,fps_cfg,fps_decode,fps_render,decode_p95_ms,e2e_p95_ms,e2e_p99_ms,present_p95_ms,present_call_p95_ms,agent_send_jitter_ms,agent_queue_std_ms -AutoSize | Out-String -Width 260
  'JSON=' + ($rows | ConvertTo-Json -Depth 6 -Compress)
}
finally {
  if (Test-Path $bak) {
    Copy-Item $bak $cfgPath -Force
    Remove-Item $bak -Force
  }
}
