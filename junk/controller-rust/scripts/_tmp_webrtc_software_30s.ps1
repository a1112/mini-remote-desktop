param([int]$DurationSec=30)
$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.webrtc_sw.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$captureBackends=@('dxgi','wgc')

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }
function Parse-LastNum([string]$path,[string]$pattern,[string]$key){
  if(!(Test-Path $path)){ return -1.0 }
  $line=(Get-Content $path | Select-String $pattern | Select-Object -Last 1).Line
  if(!$line){ return -1.0 }
  $plain=[regex]::Replace($line,"\x1B\[[0-9;]*m","")
  $m=[regex]::Match($plain,($key + '="?([0-9]+(?:\.[0-9]+)?)"?'))
  if($m.Success){ return [double]$m.Groups[1].Value }
  return -1.0
}

Copy-Item $agentCfgPath $agentBak -Force
try {
  foreach($cap in $captureBackends){
    $cfg=Get-Content $agentCfgPath -Raw | ConvertFrom-Json
    $cfg.capture.fps=180; $cfg.capture.min_fps=180; $cfg.capture.max_fps=180; $cfg.capture.idle_repeat_fps=180
    $cfg.capture.max_fps_mode=$false; $cfg.capture.target_width=2560; $cfg.capture.target_height=1440
    Set-JsonField $cfg.capture 'backend' $cap
    Set-JsonField $cfg.capture 'encoder' 'nvenc'
    Set-JsonField $cfg.capture 'allow_fallback' $false
    Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
    Set-JsonField $cfg.capture 'frame_pacing_enable' $false
    Set-JsonField $cfg.capture 'strict_gpu_direct' ($cap -eq 'dxgi')
    ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

    $tag="accept.webrtc.sw.$cap." + (Get-Date -Format 'HHmmss')
    $slog=Join-Path $base ($tag + '.s.log'); $serr=Join-Path $base ($tag + '.s.err')
    $alog=Join-Path $base ($tag + '.a.log'); $aerr=Join-Path $base ($tag + '.a.err')
    $clog=Join-Path $base ($tag + '.c.log'); $cerr=Join-Path $base ($tag + '.c.err')
    @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object { if(Test-Path $_){ cmd /c del /f /q "$_" | Out-Null } }
    Stop-Mrd

    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $acmd='/c set AGENT_FFMPEG_PATH=' + $ffmpegExe + '&& set AGENT_FPS_MODE=throughput&& set AGENT_QUIC_QUEUE=128&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& "' + $agentExe + '"'
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList $acmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $ccmd='/c set MRD_TRANSPORT=webrtc&& set MRD_DECODER=software&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
    $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds $DurationSec

    if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

    $noOut=(Select-String -Path $clog -Pattern 'decoder no-output streak' | Measure-Object).Count
    $recover=(Select-String -Path $clog -Pattern 'decoder recovery synchronized on keyframe' | Measure-Object).Count
    $pli=(Select-String -Path $alog -Pattern 'rtcp pli' | Measure-Object).Count
    $ctrlFps=Parse-LastNum $clog '\[DECODER-STATS\]' 'fps'
    $sendFps=Parse-LastNum $alog '\[RTCP-PANEL\]' 'send_fps'
    $total=(Get-Content $clog | Select-String 'video frames received so far' | Select-Object -Last 1).Line
    $totalFrames=-1; $decodedFrames=-1
    if($total){
      $plain=[regex]::Replace($total,"\x1B\[[0-9;]*m","")
      $m1=[regex]::Match($plain,'total_frames=([0-9]+)')
      $m2=[regex]::Match($plain,'decoded_frames=([0-9]+)')
      if($m1.Success){$totalFrames=[int64]$m1.Groups[1].Value}
      if($m2.Success){$decodedFrames=[int64]$m2.Groups[1].Value}
    }

    Write-Output ("webrtc+software cap={0} no_output={1} recover={2} pli={3} total={4} decoded={5} ctl_fps={6} send_fps={7}" -f $cap,$noOut,$recover,$pli,$totalFrames,$decodedFrames,[math]::Round($ctrlFps,2),[math]::Round($sendFps,2))
    Write-Output ("logs controller={0} agent={1}" -f $clog,$alog)
  }
}
finally {
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; cmd /c del /f /q "$agentBak" | Out-Null }
  Stop-Mrd
}
