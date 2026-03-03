$ErrorActionPreference='Stop'
$base=(Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$agentDir=Join-Path $base 'agent-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.protocol_mode_cmp.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force

function GP($p){$o=[ordered]@{fps=-1.0;frames=0};$l=(Get-Content $p|Select-String 'media_stats:'|Select-Object -Last 1).Line;if($l){$m=[regex]::Match($l,'estimated_fps=([0-9]+(?:\.[0-9]+)?)');if($m.Success){$o.fps=[double]$m.Groups[1].Value};$m=[regex]::Match($l,'frames=([0-9]+)');if($m.Success){$o.frames=[int]$m.Groups[1].Value}};$o}
function GA($p){$o=[ordered]@{send=-1.0;unique=-1.0}; foreach($line in Get-Content $p){$t=[regex]::Replace($line,"\x1B\[[0-9;]*m",""); if($t -notmatch '\[RTCP-PANEL\]'){continue}; $m=[regex]::Match($t,'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.send){$o.send=$v}}; $m=[regex]::Match($t,'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[$m.Groups.Count-1].Value; if($v -gt $o.unique){$o.unique=$v}} }; $o}

$cases=@(
  @{name='manual_rtp'; manual=$true; pace=$false},
  @{name='sample_track'; manual=$false; pace=$false}
)
try {
  foreach($c in $cases){
    $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
    $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
    $cfg.capture.encoder='auto'; $cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240
    $cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=240
    $cfg.capture.rtp_use_manual_packetizer=$c.manual; $cfg.capture.frame_pacing_enable=$c.pace
    ($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii
    $tag='protocol.cmp.'+$c.name+'.'+(Get-Date -Format 'HHmmss')
    $slog=Join-Path $base ($tag+'.s.log');$serr=Join-Path $base ($tag+'.s.err');$alog=Join-Path $base ($tag+'.a.log');$aerr=Join-Path $base ($tag+'.a.err');$plog=Join-Path $base ($tag+'.p.log');$perr=Join-Path $base ($tag+'.p.err')
    Get-Process|?{$_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe')}|Stop-Process -Force -ErrorAction SilentlyContinue
    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=12&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
    try{$pp|Wait-Process -Timeout 120}finally{if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}; if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}}
    $p=GP $plog; $a=GA $alog
    Write-Output ("case={0} probe={1:N2} send={2:N2} unique={3:N2} frames={4}" -f $c.name,$p.fps,$a.send,$a.unique,$p.frames)
  }
} finally { Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue; Get-Process|?{$_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe')}|Stop-Process -Force -ErrorAction SilentlyContinue }
