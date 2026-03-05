param([int]$DurationSec=30)
$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.webrtc_only.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$captureBackends=@('dxgi','wgc')
$decoders=@('d3d11va','mf')

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }

Copy-Item $agentCfgPath $agentBak -Force
$rows=@()
try {
  foreach($cap in $captureBackends){
    foreach($dec in $decoders){
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

      $tag="accept.webrtc.only.$cap.$dec." + (Get-Date -Format 'HHmmss')
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
      $ccmd='/c set MRD_TRANSPORT=webrtc&& set MRD_DECODER=' + $dec + '&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
      $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr
      Start-Sleep -Seconds $DurationSec

      if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
      if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
      if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

      $noOut=(Select-String -Path $clog -Pattern 'decoder no-output streak' | Measure-Object).Count
      $recover=(Select-String -Path $clog -Pattern 'decoder recovery synchronized on keyframe' | Measure-Object).Count
      $pli=(Select-String -Path $alog -Pattern 'rtcp pli' | Measure-Object).Count
      $idrWarn=(Select-String -Path $alog -Pattern 'force_idr requested but encoded AU still has no IDR' | Measure-Object).Count
      $nvencRecreate=(Select-String -Path $alog -Pattern 'recreated native NVENC pipeline due to prolonged missing IDR' | Measure-Object).Count
      $fallback=(Select-String -Path $clog -Pattern 'falling back to d3d11va' | Measure-Object).Count
      $rows += [pscustomobject]@{capture=$cap;decoder=$dec;no_output=$noOut;recover=$recover;pli=$pli;force_no_idr_warn=$idrWarn;nvenc_recreate=$nvencRecreate;fallback=$fallback;clog=$clog;alog=$alog}
      Write-Output ("webrtc-only cap={0} dec={1} no_output={2} recover={3} pli={4} no_idr_warn={5} nvenc_recreate={6} fallback={7}" -f $cap,$dec,$noOut,$recover,$pli,$idrWarn,$nvencRecreate,$fallback)
    }
  }
}
finally {
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; cmd /c del /f /q "$agentBak" | Out-Null }
  Stop-Mrd
}
