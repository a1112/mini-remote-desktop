$ErrorActionPreference='Stop'
$base=(Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$agentDir=Join-Path $base 'agent-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.accept_1080p240.tier.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force
function GP($p){$o=[ordered]@{fps=-1.0;frames=0};$l=(Get-Content $p|Select-String 'media_stats:'|Select-Object -Last 1).Line;if($l){$m=[regex]::Match($l,'estimated_fps=([0-9]+(?:\.[0-9]+)?)');if($m.Success){$o.fps=[double]$m.Groups[1].Value};$m=[regex]::Match($l,'frames=([0-9]+)');if($m.Success){$o.frames=[int]$m.Groups[1].Value}};$o}
function GA($p){$o=[ordered]@{send=-1.0;unique=-1.0;tier='';reason='';switch=''}; foreach($line in Get-Content $p){$t=[regex]::Replace($line,"\x1B\[[0-9;]*m",""); if($t -notmatch '\[RTCP-PANEL\]'){continue}; $m=[regex]::Match($t,'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.send){$o.send=$v}}; $m=[regex]::Match($t,'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.unique){$o.unique=$v}}; $m=[regex]::Match($t,'tier_level=([0-9]+)'); if($m.Success){$o.tier=$m.Groups[1].Value}; $m=[regex]::Match($t,'tier_reason="([^"]+)"'); if($m.Success){$o.reason=$m.Groups[1].Value}; $m=[regex]::Match($t,'tier_switch_count=([0-9]+)'); if($m.Success){$o.switch=$m.Groups[1].Value} }; $o}
try {
$cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
$cfg.capture.target_width=1920; $cfg.capture.target_height=1080
$cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240
$cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=240
$cfg.capture.encoder='auto'
if (-not ($cfg.capture.PSObject.Properties.Name -contains 'tier_limit_enable')) {
  $cfg.capture | Add-Member -NotePropertyName tier_limit_enable -NotePropertyValue $true
} else {
  $cfg.capture.tier_limit_enable=$true
}
($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii
for($i=1;$i -le 3;$i++){
  $tag='accept.1080p240.tier.run'+$i+'.'+(Get-Date -Format 'HHmmss')
  $slog=Join-Path $base ($tag+'.s.log');$serr=Join-Path $base ($tag+'.s.err');$alog=Join-Path $base ($tag+'.a.log');$aerr=Join-Path $base ($tag+'.a.err');$plog=Join-Path $base ($tag+'.p.log');$perr=Join-Path $base ($tag+'.p.err')
  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -EA SilentlyContinue
  $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700
  $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2
  $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=12&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
  try{$pp|Wait-Process -Timeout 120}finally{if(Get-Process -Id $ap.Id -EA SilentlyContinue){Stop-Process -Id $ap.Id -Force -EA SilentlyContinue}; if(Get-Process -Id $sp.Id -EA SilentlyContinue){Stop-Process -Id $sp.Id -Force -EA SilentlyContinue}}
  $p=GP $plog; $a=GA $alog
  Write-Output ("run={0} probe={1:N2} send={2:N2} unique={3:N2} frames={4} tier={5} reason={6} switches={7}" -f $i,$p.fps,$a.send,$a.unique,$p.frames,$a.tier,$a.reason,$a.switch)
}
} finally {
Copy-Item $bak $cfgPath -Force
Remove-Item $bak -Force -EA SilentlyContinue
Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -EA SilentlyContinue
}


