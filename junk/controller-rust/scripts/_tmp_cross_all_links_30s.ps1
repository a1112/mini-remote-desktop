param(
  [int]$DurationSec = 30
)

$ErrorActionPreference = 'Stop'
$base = 'J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir = Join-Path $base 'agent-rust'
$controllerDir = Join-Path $base 'controller-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$agentCfgPath = Join-Path $agentDir 'config.json'
$agentBak = Join-Path $agentDir ('config.crosscombo.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')

$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

$transports = @('webrtc', 'quic')
$captureBackends = @('dxgi', 'wgc')
$decoders = @('d3d11va', 'mf')

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

function Parse-LastNum([string]$path, [string]$pattern, [string]$key) {
  if (!(Test-Path $path)) { return -1.0 }
  $line = (Get-Content $path | Select-String $pattern | Select-Object -Last 1).Line
  if (!$line) { return -1.0 }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  $m = [regex]::Match($plain, ($key + '="?([0-9]+(?:\.[0-9]+)?)"?'))
  if ($m.Success) { return [double]$m.Groups[1].Value }
  return -1.0
}

function Parse-FrameSeries([string]$path) {
  $out = @()
  if (!(Test-Path $path)) { return $out }
  foreach ($line in (Get-Content $path | Select-String 'video frames received so far')) {
    $plain = [regex]::Replace($line.Line, "\x1B\[[0-9;]*m", "")
    $m1 = [regex]::Match($plain, 'total_frames=([0-9]+)')
    $m2 = [regex]::Match($plain, 'decoded_frames=([0-9]+)')
    if ($m1.Success -and $m2.Success) {
      $out += [pscustomobject]@{ total = [int64]$m1.Groups[1].Value; decoded = [int64]$m2.Groups[1].Value }
    }
  }
  return $out
}

function Parse-Connected([string]$path, [string]$transport) {
  if (!(Test-Path $path)) { return $false }
  $content = Get-Content $path
  if ($transport -eq 'quic') {
    return (($content | Select-String 'connected to QUIC media transport' | Select-Object -First 1) -ne $null)
  }
  $a = ($content | Select-String 'peer connection state changed' | Select-String 'connected' | Select-Object -First 1)
  $b = ($content | Select-String 'starting to read video track' | Select-Object -First 1)
  return ($a -ne $null -and $b -ne $null)
}

Copy-Item $agentCfgPath $agentBak -Force
$rows = @()
try {
  foreach ($transport in $transports) {
    foreach ($captureBackend in $captureBackends) {
      foreach ($decoder in $decoders) {
        $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json
        $cfg.capture.fps = 180
        $cfg.capture.min_fps = 180
        $cfg.capture.max_fps = 180
        $cfg.capture.idle_repeat_fps = 180
        $cfg.capture.max_fps_mode = $false
        $cfg.capture.target_width = 2560
        $cfg.capture.target_height = 1440
        Set-JsonField $cfg.capture 'backend' $captureBackend
        Set-JsonField $cfg.capture 'encoder' 'nvenc'
        Set-JsonField $cfg.capture 'allow_fallback' $false
        Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
        Set-JsonField $cfg.capture 'frame_pacing_enable' $false
        if ($captureBackend -eq 'dxgi') {
          Set-JsonField $cfg.capture 'strict_gpu_direct' $true
        } else {
          Set-JsonField $cfg.capture 'strict_gpu_direct' $false
        }
        ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

        $tag = "accept.cross.$transport.$captureBackend.$decoder." + (Get-Date -Format 'HHmmss')
        $slog = Join-Path $base ($tag + '.s.log')
        $serr = Join-Path $base ($tag + '.s.err')
        $alog = Join-Path $base ($tag + '.a.log')
        $aerr = Join-Path $base ($tag + '.a.err')
        $clog = Join-Path $base ($tag + '.c.log')
        $cerr = Join-Path $base ($tag + '.c.err')

        @($slog, $serr, $alog, $aerr, $clog, $cerr) | ForEach-Object {
          if (Test-Path $_) { cmd /c del /f /q "$_" | Out-Null }
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

        $controllerEnv = "set MRD_TRANSPORT=$transport&& set MRD_DECODER=$decoder&& set MRD_MF_MAX_DRAIN_PER_CALL=4&& set MRD_MF_MAX_DRAIN_SPIN_PER_CALL=32&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`""
        $cp = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $controllerEnv `
          -WorkingDirectory $controllerDir -PassThru `
          -RedirectStandardOutput $clog -RedirectStandardError $cerr

        Start-Sleep -Seconds $DurationSec

        if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
        if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
        if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

        $connected = Parse-Connected $clog $transport
        $series = Parse-FrameSeries $clog
        $finalTotal = -1
        $finalDecoded = -1
        $tailFlat = $false
        if ($series.Count -gt 0) {
          $finalTotal = $series[-1].total
          $finalDecoded = $series[-1].decoded
          if ($series.Count -ge 3) {
            $a = $series[$series.Count - 1].total
            $b = $series[$series.Count - 2].total
            $c = $series[$series.Count - 3].total
            if ($a -eq $b -and $b -eq $c) { $tailFlat = $true }
          }
        }

        $agentSendFps = Parse-LastNum $alog '\[RTCP-PANEL\]' 'send_fps'
        $agentAuSent = Parse-LastNum $alog '\[RTCP-PANEL\]' 'au_sent'
        $controllerFps = Parse-LastNum $clog '\[DECODER-STATS\]' 'fps'

        $status = 'pass'
        if (!$connected) { $status = 'fail_connect' }
        elseif ($finalTotal -lt 120) { $status = 'fail_low_frames' }
        elseif ($tailFlat -and $agentAuSent -gt 300) { $status = 'fail_tail_freeze' }

        $row = [pscustomobject]@{
          transport = $transport
          capture_backend = $captureBackend
          decoder = $decoder
          status = $status
          connected = $connected
          total_frames = $finalTotal
          decoded_frames = $finalDecoded
          controller_fps = [Math]::Round($controllerFps, 2)
          agent_send_fps = [Math]::Round($agentSendFps, 2)
          agent_au_sent = [int64]$agentAuSent
          tail_flat = $tailFlat
          controller_log = $clog
          agent_log = $alog
        }
        $rows += $row
        Write-Output ("cross case transport={0} capture={1} decoder={2} status={3} connected={4} total={5} decoded={6} ctl_fps={7} send_fps={8} tail_flat={9}" -f `
          $row.transport, $row.capture_backend, $row.decoder, $row.status, $row.connected, $row.total_frames, $row.decoded_frames, $row.controller_fps, $row.agent_send_fps, $row.tail_flat)
      }
    }
  }

  $outCsv = Join-Path $base ('cross-regression.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.csv')
  $rows | Export-Csv -Path $outCsv -NoTypeInformation -Encoding Ascii
  Write-Output ("cross summary saved: {0}" -f $outCsv)

  $fails = $rows | Where-Object { $_.status -ne 'pass' }
  Write-Output ("cross failed count: {0}" -f $fails.Count)
  foreach ($f in $fails) {
    Write-Output ("cross failed transport={0} capture={1} decoder={2} status={3} controller_log={4}" -f `
      $f.transport, $f.capture_backend, $f.decoder, $f.status, $f.controller_log)
  }
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    cmd /c del /f /q "$agentBak" | Out-Null
  }
  Stop-Mrd
}
