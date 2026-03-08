$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$controllerCfgPath=Join-Path $controllerDir 'config.json'
$controllerBak=Join-Path $controllerDir ('config.sr.matrix.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.sr.matrix.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }
function LastLine([string]$path,[string]$pat){ if(!(Test-Path $path)){return ''}; $line=(Get-Content $path | Select-String $pat | Select-Object -Last 1).Line; if(!$line){return ''}; return [regex]::Replace(($line|Out-String),"\x1B\[[0-9;]*m","").Trim() }
function Extract([string]$text,[string]$pat){ $m=[regex]::Match($text,$pat); if($m.Success){return $m.Groups[1].Value}; return 'n/a' }

Copy-Item $controllerCfgPath $controllerBak -Force
Copy-Item $agentCfgPath $agentBak -Force
try {
  $acfg=Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  $acfg.capture.target_width=2560; $acfg.capture.target_height=1440
  $acfg.capture.fps=180; $acfg.capture.min_fps=180; $acfg.capture.max_fps=180; $acfg.capture.idle_repeat_fps=180
  Set-JsonField $acfg.capture 'backend' 'dxgi'
  Set-JsonField $acfg.capture 'encoder' 'nvenc'
  Set-JsonField $acfg.capture 'strict_gpu_direct' $true
  Set-JsonField $acfg.capture 'allow_fallback' $false
  Set-JsonField $acfg.capture 'allow_encoder_fallback' $false
  $acfg.capture.queue_depth=8
  ($acfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  foreach($sr in @('off','performance','quality')){
    $ccfg=Get-Content $controllerCfgPath -Raw | ConvertFrom-Json
    if(-not ($ccfg.PSObject.Properties.Name -contains 'render')){ $ccfg | Add-Member -NotePropertyName 'render' -NotePropertyValue ([pscustomobject]@{}) }
    Set-JsonField $ccfg.render 'sr_mode' $sr
    ($ccfg | ConvertTo-Json -Depth 100) | Set-Content -Path $controllerCfgPath -Encoding Ascii

    $tag="accept.2k180.quic.sr40.$sr." + (Get-Date -Format 'HHmmss')
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
    $ccmd='/c set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set MRD_RENDER_MODE=low_latency&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
    $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds 40

    if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

    $rLine=LastLine $clog 'renderer progress'
    $dLine=LastLine $clog '\[DECODER-STATS\]'
    $pLine=LastLine $clog '\[PRESENT-STATS\]'
    $aLine=LastLine $alog '\[RTCP-PANEL\]'

    $srSeen=Extract $rLine 'sr_mode="?([a-z_]+)"?'
    $send=Extract $aLine 'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'
    $dfps=Extract $dLine 'fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $d95=Extract $dLine 'p95_decode_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $jit=Extract $dLine 'jitter_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $rfps=Extract $rLine 'rendered_fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $stale=Extract $rLine 'stale_dropped_total=([0-9]+)'
    $pavg=Extract $pLine 'capture_to_present_avg_ms="?([0-9]+(?:\.[0-9]+)?)"?'
    $p95=Extract $pLine 'capture_to_present_p95_ms="?([0-9]+(?:\.[0-9]+)?)"?'

    Write-Output ("sr={0} sr_seen={1} send_fps={2} decode_fps={3} render_fps={4} p95_decode_ms={5} jitter_ms={6} present_avg_ms={7} present_p95_ms={8} stale_dropped_total={9}" -f $sr,$srSeen,$send,$dfps,$rfps,$d95,$jit,$pavg,$p95,$stale)
    Write-Output ("logs agent={0} controller={1} signaling={2}" -f $alog,$clog,$slog)
  }
}
finally {
  if(Test-Path $controllerBak){ Copy-Item $controllerBak $controllerCfgPath -Force; Remove-Item $controllerBak -Force -ErrorAction SilentlyContinue }
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; Remove-Item $agentBak -Force -ErrorAction SilentlyContinue }
  Stop-Mrd
}
