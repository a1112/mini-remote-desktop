$ErrorActionPreference='Stop'
$base=(Resolve-Path (Join-Path $PSScriptRoot '..\\..')).Path
$agentDir=Join-Path $base 'agent-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.protocol_eval.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force

function Get-ProbeStats([string]$Path) {
  $out = [ordered]@{ fps = -1.0; frames = 0 }
  if (-not (Test-Path $Path)) { return $out }
  $line = (Get-Content $Path | Select-String 'media_stats:' | Select-Object -Last 1).Line
  if (-not $line) { return $out }
  $m = [regex]::Match($line, 'estimated_fps=([0-9]+(?:\.[0-9]+)?)')
  if ($m.Success) { $out.fps = [double]$m.Groups[1].Value }
  $m = [regex]::Match($line, 'frames=([0-9]+)')
  if ($m.Success) { $out.frames = [int]$m.Groups[1].Value }
  return $out
}

function Get-AgentStats([string]$Path) {
  $out = [ordered]@{ send = -1.0; unique = -1.0; nvenc = $false; fallback = $false; pc = $false; ice = $false }
  if (-not (Test-Path $Path)) { return $out }
  foreach ($line in Get-Content $Path) {
    $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
    if ($plain -match 'native NVENC pipeline attached') { $out.nvenc = $true }
    if ($plain -match 'native NVENC init failed, using fallback') { $out.fallback = $true }
    if ($plain -match 'peer connection state changed .*state=connected') { $out.pc = $true }
    if ($plain -match 'ice connection state changed .*state=connected') { $out.ice = $true }
    if ($plain -notmatch '\[RTCP-PANEL\]') { continue }
    $m = [regex]::Match($plain, 'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
    if ($m.Success) { $v = [double]$m.Groups[$m.Groups.Count - 1].Value; if ($v -gt $out.send) { $out.send = $v } }
    $m = [regex]::Match($plain, 'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
    if ($m.Success) { $v = [double]$m.Groups[$m.Groups.Count - 1].Value; if ($v -gt $out.unique) { $out.unique = $v } }
  }
  return $out
}

$cases = @(
  @{ name='protocol_resend_240'; fps=1; min=1; max=1; idle=240; maxMode=$true; manual=$true; pace=$false },
  @{ name='normal_120'; fps=120; min=120; max=120; idle=120; maxMode=$true; manual=$true; pace=$false },
  @{ name='normal_240'; fps=240; min=240; max=240; idle=240; maxMode=$true; manual=$true; pace=$false }
)

$runsPerCase = 2
$probeSecs = 12
$results = @()

try {
  foreach ($c in $cases) {
    for ($i = 1; $i -le $runsPerCase; $i++) {
      $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
      $cfg.capture.target_width = 1920
      $cfg.capture.target_height = 1080
      $cfg.capture.encoder = 'auto'
      $cfg.capture.fps = $c.fps
      $cfg.capture.min_fps = $c.min
      $cfg.capture.max_fps = $c.max
      $cfg.capture.idle_repeat_fps = $c.idle
      $cfg.capture.max_fps_mode = $c.maxMode
      $cfg.capture.rtp_use_manual_packetizer = $c.manual
      $cfg.capture.frame_pacing_enable = $c.pace
      ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii

      $tag = ('protocol.eval.{0}.run{1}.{2}' -f $c.name, $i, (Get-Date -Format 'HHmmss'))
      $slog = Join-Path $base ($tag + '.s.log')
      $serr = Join-Path $base ($tag + '.s.err')
      $alog = Join-Path $base ($tag + '.a.log')
      $aerr = Join-Path $base ($tag + '.a.err')
      $plog = Join-Path $base ($tag + '.p.log')
      $perr = Join-Path $base ($tag + '.p.err')

      @($slog,$serr,$alog,$aerr,$plog,$perr) | ForEach-Object { if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue } }
      Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue

      $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
      Start-Sleep -Milliseconds 700
      $agentCmd = "set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`""
      $ap = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
      Start-Sleep -Seconds 2
      $probeCmd = "set PROBE_SECS=$probeSecs&& `"$probeExe`""
      $pp = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $probeCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr

      try { $pp | Wait-Process -Timeout ($probeSecs + 90) } finally {
        if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
        if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }
      }

      $probe = Get-ProbeStats $plog
      $agent = Get-AgentStats $alog
      $row = [pscustomobject]@{
        case = $c.name
        run = $i
        probe_fps = [math]::Round($probe.fps,2)
        send_fps = [math]::Round($agent.send,2)
        unique_fps = [math]::Round($agent.unique,2)
        frames = $probe.frames
        nvenc = $agent.nvenc
        connected = ($agent.pc -and $agent.ice)
      }
      $results += $row
      Write-Output ("case={0} run={1} probe={2} send={3} unique={4} frames={5} nvenc={6} connected={7}" -f $row.case,$row.run,$row.probe_fps,$row.send_fps,$row.unique_fps,$row.frames,$row.nvenc,$row.connected)
    }
  }

  Write-Output '=== summary ==='
  foreach($g in ($results | Group-Object case)) {
    $avgProbe = ($g.Group | Measure-Object -Property probe_fps -Average).Average
    $avgSend = ($g.Group | Measure-Object -Property send_fps -Average).Average
    $avgUnique = ($g.Group | Measure-Object -Property unique_fps -Average).Average
    Write-Output ("case={0} avg_probe={1:N2} avg_send={2:N2} avg_unique={3:N2}" -f $g.Name,$avgProbe,$avgSend,$avgUnique)
  }
}
finally {
  if (Test-Path $bak) { Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue }
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
}


