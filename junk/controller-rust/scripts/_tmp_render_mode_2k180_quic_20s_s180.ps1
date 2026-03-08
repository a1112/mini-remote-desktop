$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.render.2k180.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }
function LastLine([string]$path,[string]$pat){ if(!(Test-Path $path)){return ''}; $line=(Get-Content $path | Select-String $pat | Select-Object -Last 1).Line; if(!$line){return ''}; return [regex]::Replace(($line|Out-String),"\x1B\[[0-9;]*m","").Trim() }
function Extract([string]$text,[string]$pat){ $m=[regex]::Match($text,$pat); if($m.Success){return $m.Groups[1].Value}; return 'n/a' }

Copy-Item $agentCfgPath $agentBak -Force
try {
  $cfg=Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  $cfg.capture.target_width=2560; $cfg.capture.target_height=1440
  $cfg.capture.fps=180; $cfg.capture.min_fps=180; $cfg.capture.max_fps=180; $cfg.capture.idle_repeat_fps=180
  Set-JsonField $cfg.capture 'backend' 'dxgi'
  Set-JsonField $cfg.capture 'encoder' 'nvenc'
  Set-JsonField $cfg.capture 'strict_gpu_direct' $true
  Set-JsonField $cfg.capture 'allow_fallback' $false
  Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
  $cfg.capture.queue_depth=8
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  foreach($mode in @('low_latency','smooth')){
    $tag="accept.2k180.quic.$mode." + (Get-Date -Format 'HHmmss')
    $slog=Join-Path $base ($tag + '.s.log'); $serr=Join-Path $base ($tag + '.s.err')
    $alog=Join-Path $base ($tag + '.a.log'); $aerr=Join-Path $base ($tag + '.a.err')
    $clog=Join-Path $base ($tag + '.c.log'); $cerr=Join-Path $base ($tag + '.c.err')
    @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object { if(Test-Path $_){ Remove-Item $_ -Force -ErrorAction SilentlyContinue } }

    Stop-Mrd
    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $acmd='/c set AGENT_FFMPEG_PATH=' + $ffmpegExe + '&& set AGENT_QUIC_DEBUG=1&& set AGENT_QUIC_QUEUE=128&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& "' + $agentExe + '"'
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList $acmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $ccmd='/c set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set MRD_RENDER_MODE=' + $mode + '&& set MRD_SMOOTH_TARGET_FPS=180&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
    $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds 20

    if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

    $rLine=LastLine $clog 'renderer progress'
    $dLine=LastLine $clog '\[DECODER-STATS\]'
    $pLine=LastLine $clog '\[PRESENT-STATS\]'
    $aLine=LastLine $alog '\[RTCP-PANEL\]'

    $modeSeen=Extract $rLine 'render_mode="?([a-z_]+)"?'
    $rep=Extract $rLine 'smooth_repeated_total=([0-9]+)'
    $rfps=Extract $rLine 'rendered_fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $dfps=Extract $dLine 'fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $d95=Extract $dLine 'p95_decode_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $jit=Extract $dLine 'jitter_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $pfps=Extract $pLine 'fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $plat=Extract $pLine 'avg_total_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $send=Extract $aLine 'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'

    Write-Output ("mode={0} logged_mode={1} send_fps={2} decode_fps={3} render_fps={4} present_fps={5} p95_decode_ms={6} jitter_ms={7} avg_total_ms={8} smooth_repeated_total={9}" -f $mode,$modeSeen,$send,$dfps,$rfps,$pfps,$d95,$jit,$plat,$rep)
    Write-Output ("logs agent={0} controller={1} signaling={2}" -f $alog,$clog,$slog)
  }
}
finally {
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; Remove-Item $agentBak -Force -ErrorAction SilentlyContinue }
  Stop-Mrd
}

