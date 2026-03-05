param(
  [int]$DurationSec = 20
)

$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$agentDir = Join-Path $base 'agent-rust'
$controllerDir = Join-Path $base 'controller-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$agentCfgPath = Join-Path $agentDir 'config.json'
$agentBak = Join-Path $agentDir ('config.matrix.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')

$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

$fpsList = @(72, 120, 144, 180, 240)
$resList = @(
  @{ name = '720p'; w = 1280; h = 720 },
  @{ name = '1080p'; w = 1920; h = 1080 }
)

function Stop-Mrd {
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs', 'agent-rust', 'controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Set-JsonField($obj, [string]$name, $value) {
  if ($obj.PSObject.Properties.Name -contains $name) {
    $obj.$name = $value
  } else {
    $obj | Add-Member -NotePropertyName $name -NotePropertyValue $value
  }
}

function Last-Line([string]$path, [string]$pattern) {
  if (!(Test-Path $path)) { return $null }
  (Get-Content $path | Select-String $pattern | Select-Object -Last 1).Line
}

function Parse-Num([string]$line, [string]$key) {
  if (!$line) { return -1.0 }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  $m = [regex]::Match($plain, "$key=""?([0-9]+(?:\.[0-9]+)?)""?")
  if ($m.Success) { return [double]$m.Groups[1].Value }
  $m = [regex]::Match($plain, "$key[^0-9]*([0-9]+(?:\.[0-9]+)?)")
  if ($m.Success) { return [double]$m.Groups[1].Value }
  return -1.0
}

function Parse-Gt100Pct([string]$path) {
  if (!(Test-Path $path)) { return -1.0 }
  $content = Get-Content $path
  $samples = @()
  foreach ($line in ($content | Select-String '\[PRESENT-STATS\]')) {
    $plain = [regex]::Replace($line.Line, "\x1B\[[0-9;]*m", "")
    $m = [regex]::Match($plain, 'capture_to_present_p99_ms="?([0-9]+(?:\.[0-9]+)?)"?')
    if ($m.Success) { $samples += [double]$m.Groups[1].Value }
  }
  if ($samples.Count -eq 0) { return -1.0 }
  $gt = ($samples | Where-Object { $_ -gt 100.0 }).Count
  return [Math]::Round(($gt * 100.0) / [Math]::Max($samples.Count, 1), 2)
}

function Parse-Bool([string]$path, [string]$pattern) {
  if (!(Test-Path $path)) { return $false }
  return ((Get-Content $path | Select-String $pattern | Select-Object -First 1) -ne $null)
}

function Get-CpuUsagePct([datetime]$startTs, [datetime]$endTs, [double]$agentCpuSec, [double]$controllerCpuSec) {
  $duration = [Math]::Max((New-TimeSpan -Start $startTs -End $endTs).TotalSeconds, 0.001)
  $cores = [Math]::Max((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors, 1)
  $cpuSec = [Math]::Max($agentCpuSec + $controllerCpuSec, 0.0)
  return [Math]::Round(($cpuSec / ($duration * $cores)) * 100.0, 2)
}

Copy-Item $agentCfgPath $agentBak -Force

$rows = @()

try {
  foreach ($res in $resList) {
    foreach ($fps in $fpsList) {
      $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json
      $cfg.capture.fps = $fps
      $cfg.capture.min_fps = $fps
      $cfg.capture.max_fps = $fps
      $cfg.capture.idle_repeat_fps = $fps
      $cfg.capture.max_fps_mode = $false
      $cfg.capture.target_width = $res.w
      $cfg.capture.target_height = $res.h
      Set-JsonField $cfg.capture 'backend' 'dxgi'
      Set-JsonField $cfg.capture 'encoder' 'nvenc'
      Set-JsonField $cfg.capture 'strict_gpu_direct' $true
      Set-JsonField $cfg.capture 'allow_fallback' $false
      Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
      Set-JsonField $cfg.capture 'frame_pacing_enable' $false
      Set-JsonField $cfg.capture 'tier_limit_enable' $false
      Set-JsonField $cfg.capture 'queue_strategy' 'drop'
      Set-JsonField $cfg.capture 'queue_depth' 8
      ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

      $tag = "accept.matrix.$($res.name).$fps." + (Get-Date -Format 'HHmmss')
      $slog = Join-Path $base ($tag + '.s.log')
      $serr = Join-Path $base ($tag + '.s.err')
      $alog = Join-Path $base ($tag + '.a.log')
      $aerr = Join-Path $base ($tag + '.a.err')
      $clog = Join-Path $base ($tag + '.c.log')
      $cerr = Join-Path $base ($tag + '.c.err')

      @($slog, $serr, $alog, $aerr, $clog, $cerr) | ForEach-Object {
        if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
      }

      Stop-Mrd

      $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
        -RedirectStandardOutput $slog -RedirectStandardError $serr
      Start-Sleep -Milliseconds 700

      $ap = Start-Process -FilePath 'cmd.exe' `
        -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_FPS_MODE=throughput&& set AGENT_QUIC_QUEUE=128&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& `"$agentExe`"") `
        -WorkingDirectory $agentDir -PassThru `
        -RedirectStandardOutput $alog -RedirectStandardError $aerr
      Start-Sleep -Seconds 2

      $cp = Start-Process -FilePath 'cmd.exe' `
        -ArgumentList '/c',("set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
        -WorkingDirectory $controllerDir -PassThru `
        -RedirectStandardOutput $clog -RedirectStandardError $cerr

      $startTs = Get-Date
      Start-Sleep -Seconds $DurationSec
      $endTs = Get-Date

      $agentCpuSec = 0.0
      $controllerCpuSec = 0.0
      $apProc = Get-Process -Id $ap.Id -ErrorAction SilentlyContinue
      if ($apProc) { $agentCpuSec = [double]$apProc.CPU }
      $cpProc = Get-Process -Id $cp.Id -ErrorAction SilentlyContinue
      if ($cpProc) { $controllerCpuSec = [double]$cpProc.CPU }

      if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
      if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
      if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

      $connected = Parse-Bool $clog 'connected to QUIC media transport'
      $decoderLine = Last-Line $clog '\[DECODER-STATS\]'
      $presentLine = Last-Line $clog '\[PRESENT-STATS\]'
      $fpsAvg = Parse-Num $decoderLine 'fps'
      $p95 = Parse-Num $presentLine 'capture_to_present_p95_ms'
      if ($p95 -lt 0) { $p95 = Parse-Num $decoderLine 'e2e_p95_ms' }
      $p99 = Parse-Num $presentLine 'capture_to_present_p99_ms'
      if ($p99 -lt 0) { $p99 = Parse-Num $decoderLine 'e2e_p99_ms' }
      $gt100 = Parse-Gt100Pct $clog
      $cpuPct = Get-CpuUsagePct $startTs $endTs $agentCpuSec $controllerCpuSec
      $gpuRatio = Parse-Num (Last-Line $clog 'renderer progress') 'gpu_zero_copy_ratio'
      $gpuPct = if ($gpuRatio -ge 0) { [Math]::Round($gpuRatio * 100.0, 2) } else { -1.0 }

      $row = [pscustomobject]@{
        resolution = $res.name
        width = $res.w
        height = $res.h
        target_fps = $fps
        connected = $connected
        fps_avg = [Math]::Round($fpsAvg, 2)
        p95_ms = [Math]::Round($p95, 3)
        p99_ms = [Math]::Round($p99, 3)
        gt100ms = [Math]::Round($gt100, 2)
        cpu_pct = $cpuPct
        gpu_pct = $gpuPct
        agent_log = $alog
        controller_log = $clog
      }
      $rows += $row
      Write-Output ("matrix case res={0} fps={1} connected={2} fps_avg={3} p95={4} p99={5} gt100ms_pct={6} cpu={7} gpu={8}" -f `
        $row.resolution, $row.target_fps, $row.connected, $row.fps_avg, $row.p95_ms, $row.p99_ms, $row.gt100ms, $row.cpu_pct, $row.gpu_pct)
    }
  }

  $outCsv = Join-Path $base ("matrix-regression." + (Get-Date -Format 'yyyyMMdd_HHmmss') + ".csv")
  $rows | Export-Csv -Path $outCsv -NoTypeInformation -Encoding Ascii
  Write-Output ("matrix summary saved: {0}" -f $outCsv)
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    Remove-Item $agentBak -Force -ErrorAction SilentlyContinue
  }
  Stop-Mrd
}
