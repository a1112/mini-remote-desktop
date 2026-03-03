$ErrorActionPreference='Stop'
$base=(Resolve-Path (Join-Path $PSScriptRoot '..\\..')).Path
$agentDir=Join-Path $base 'agent-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.accept_1080p240.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force

function GP($p){
  $o=[ordered]@{fps=-1.0;frames=0}
  if(!(Test-Path $p)){return $o}
  $l=(Get-Content $p|Select-String 'media_stats:'|Select-Object -Last 1).Line
  if(!$l){return $o}
  $m=[regex]::Match($l,'estimated_fps=([0-9]+(?:\.[0-9]+)?)')
  if($m.Success){$o.fps=[double]$m.Groups[1].Value}
  $m=[regex]::Match($l,'frames=([0-9]+)')
  if($m.Success){$o.frames=[int]$m.Groups[1].Value}
  $o
}
function GA($p){
  $o=[ordered]@{send=-1.0;unique=-1.0;cfg=-1;nvenc=$false;fallback=$false;pc=$false;ice=$false;wgc=$false}
  if(!(Test-Path $p)){return $o}
  foreach($line in Get-Content $p){
    $t=[regex]::Replace($line,"\x1B\[[0-9;]*m","")
    if($t -match 'capture configuration\s+fps=([0-9]+)'){ $o.cfg=[int]$matches[1] }
    if($t -match 'capture backend selected: wgc'){ $o.wgc=$true }
    if($t -match 'native NVENC pipeline attached' -or $t -match 'WGC native NVENC texture pipeline attached'){ $o.nvenc=$true }
    if($t -match 'native NVENC init failed, using fallback' -or $t -match 'WGC native NVENC init failed, using fallback'){ $o.fallback=$true }
    if($t -match 'peer connection state changed .*state=connected'){ $o.pc=$true }
    if($t -match 'ice connection state changed .*state=connected'){ $o.ice=$true }
    if($t -notmatch '\[RTCP-PANEL\]'){continue}
    $m=[regex]::Match($t,'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
    if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.send){$o.send=$v}}
    $m=[regex]::Match($t,'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)')
    if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.unique){$o.unique=$v}}
  }
  $o
}

$runs=3
$probeSecs=15
$results=@()
try {
  $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
  $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
  $cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240
  $cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=240
  $cfg.capture.backend='wgc'
  $cfg.capture.encoder='nvenc'
  if ($null -eq $cfg.capture.PSObject.Properties['strict_gpu_direct']) {
    $cfg.capture | Add-Member -NotePropertyName strict_gpu_direct -NotePropertyValue $true
  } else {
    $cfg.capture.strict_gpu_direct=$true
  }
  ($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii

  for($i=1; $i -le $runs; $i++) {
    $tag=('accept.1080p240.run{0}.{1}' -f $i,(Get-Date -Format 'HHmmss'))
    $slog=Join-Path $base ($tag+'.s.log'); $serr=Join-Path $base ($tag+'.s.err')
    $alog=Join-Path $base ($tag+'.a.log'); $aerr=Join-Path $base ($tag+'.a.err')
    $plog=Join-Path $base ($tag+'.p.log'); $perr=Join-Path $base ($tag+'.p.err')
    @($slog,$serr,$alog,$aerr,$plog,$perr) | % { if(Test-Path $_){Remove-Item $_ -Force -ErrorAction SilentlyContinue} }
    Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue

    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=$probeSecs&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
    try { $pp | Wait-Process -Timeout ($probeSecs+90) } finally {
      if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
      if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}
    }

    $p=GP $plog
    $a=GA $alog
    $okBase = ($p.frames -gt 0 -and $a.nvenc -and -not $a.fallback -and $a.pc -and $a.ice)
    $ok240 = ($p.fps -ge 230 -and $a.unique -ge 230)
    $r=[pscustomobject]@{run=$i; cfg_fps=$a.cfg; probe_fps=[math]::Round($p.fps,2); send_fps=[math]::Round($a.send,2); unique_fps=[math]::Round($a.unique,2); frames=$p.frames; nvenc=$a.nvenc; wgc=$a.wgc; connected=($a.pc -and $a.ice); base_ok=$okBase; pass_1080p240=($okBase -and $ok240)}
    $results += $r
    Write-Output ("run={0} cfg={1} probe={2} send={3} unique={4} frames={5} nvenc={6} wgc={7} connected={8} base_ok={9} pass_1080p240={10}" -f $r.run,$r.cfg_fps,$r.probe_fps,$r.send_fps,$r.unique_fps,$r.frames,$r.nvenc,$r.wgc,$r.connected,$r.base_ok,$r.pass_1080p240)
  }

  Write-Output '=== summary ==='
  $bestProbe=($results | Sort-Object probe_fps -Descending | Select-Object -First 1)
  $bestUnique=($results | Sort-Object unique_fps -Descending | Select-Object -First 1)
  $passCount=($results | ? { $_.pass_1080p240 }).Count
  Write-Output ("best_probe_fps={0} run={1}" -f $bestProbe.probe_fps,$bestProbe.run)
  Write-Output ("best_unique_fps={0} run={1}" -f $bestUnique.unique_fps,$bestUnique.run)
  Write-Output ("pass_1080p240={0}/{1}" -f $passCount,$runs)
}
finally {
  if(Test-Path $bak){Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue}
  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
}
