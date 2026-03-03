$ErrorActionPreference='Stop'
$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$controllerDir=Join-Path $base 'controller-rust'
$agentDir=Join-Path $base 'agent-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.tmp.compare.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force
function SetField($obj,[string]$name,$v){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$v}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $v}}
function StopAll(){ Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust')} | Stop-Process -Force -ErrorAction SilentlyContinue }
function LastNum([string]$line,[string]$key){ $m=[regex]::Match($line,($key+'="?([0-9]+(?:\.[0-9]+)?)"?')); if($m.Success){return [double]$m.Groups[1].Value}; return -1 }
try {
  $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
  $cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240; $cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=240
  $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
  SetField $cfg.capture 'backend' 'dxgi'; SetField $cfg.capture 'encoder' 'nvenc'
  SetField $cfg.capture 'strict_gpu_direct' $true; SetField $cfg.capture 'allow_fallback' $false; SetField $cfg.capture 'allow_encoder_fallback' $false
  SetField $cfg.capture 'queue_strategy' 'drop'; SetField $cfg.capture 'tier_limit_enable' $false
  ($cfg|ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii

  $modes=@('software','d3d11va')
  foreach($mode in $modes){
    StopAll
    $tag='accept.quic.1080p240.capture_present_cmp.'+$mode+'.'+(Get-Date -Format 'HHmmss')
    $slog=Join-Path $base ($tag+'.s.log'); $serr=Join-Path $base ($tag+'.s.err')
    $alog=Join-Path $base ($tag+'.a.log'); $aerr=Join-Path $base ($tag+'.a.err')
    $clog=Join-Path $base ($tag+'.c.log'); $cerr=Join-Path $base ($tag+'.c.err')
    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_QUIC_QUEUE=64&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set MRD_TRANSPORT=quic&& set MRD_DECODER=$mode&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr
    Start-Sleep -Seconds 24
    StopAll

    $content=if(Test-Path $clog){Get-Content $clog}else{@()}
    $backendLine=($content|Select-String 'video decoder initialized'|Select-Object -Last 1).Line
    $decLine=($content|Select-String '\[DECODER-STATS\]'|Select-Object -Last 1).Line
    $preLine=($content|Select-String '\[PRESENT-STATS\]'|Select-Object -Last 1).Line
    if($decLine){ $dec=[regex]::Replace($decLine,"\x1B\[[0-9;]*m","") } else { $dec='' }
    if($preLine){ $pre=[regex]::Replace($preLine,"\x1B\[[0-9;]*m","") } else { $pre='' }
    $fps=if($dec){LastNum $dec 'fps'}else{-1}
    $avgd=if($dec){LastNum $dec 'avg_decode_ms'}else{-1}
    $p95d=if($dec){LastNum $dec 'p95_decode_ms'}else{-1}
    $jit=if($dec){LastNum $dec 'jitter_ms'}else{-1}
    $pavg=if($pre){LastNum $pre 'capture_to_present_avg_ms'}else{-1}
    $pp50=if($pre){LastNum $pre 'capture_to_present_p50_ms'}else{-1}
    $pp95=if($pre){LastNum $pre 'capture_to_present_p95_ms'}else{-1}
    $pp99=if($pre){LastNum $pre 'capture_to_present_p99_ms'}else{-1}
    Write-Output ("mode={0} backend_line={1}" -f $mode,$backendLine)
    Write-Output ("mode={0} decoder fps={1} avg_decode_ms={2} p95_decode_ms={3} jitter_ms={4}" -f $mode,$fps,$avgd,$p95d,$jit)
    Write-Output ("mode={0} present capture_to_present_avg_ms={1} p50={2} p95={3} p99={4}" -f $mode,$pavg,$pp50,$pp95,$pp99)
    Write-Output ("mode={0} logs controller={1} agent={2}" -f $mode,$clog,$alog)
  }
} finally { if(Test-Path $bak){Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue}; StopAll }
