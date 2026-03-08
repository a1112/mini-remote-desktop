param([int]$DurationSec=20)
$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.swenc.range.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$captureBackends=@('dxgi','wgc')
$profiles=@(
  @{w=1920; h=1080; fps=30},
  @{w=1920; h=1080; fps=60},
  @{w=2560; h=1440; fps=30}
)

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
$rows=@()
try {
  foreach($cap in $captureBackends){
    foreach($p in $profiles){
      $cfg=Get-Content $agentCfgPath -Raw | ConvertFrom-Json
      $cfg.capture.fps=$p.fps; $cfg.capture.min_fps=$p.fps; $cfg.capture.max_fps=$p.fps; $cfg.capture.idle_repeat_fps=$p.fps
      $cfg.capture.max_fps_mode=$false; $cfg.capture.target_width=$p.w; $cfg.capture.target_height=$p.h
      Set-JsonField $cfg.capture 'backend' $cap
      Set-JsonField $cfg.capture 'encoder' 'openh264'
      Set-JsonField $cfg.capture 'allow_fallback' $true
      Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
      Set-JsonField $cfg.capture 'frame_pacing_enable' $false
      Set-JsonField $cfg.capture 'strict_gpu_direct' $false
      ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

      $tag="accept.swenc.range.webrtc.$cap.$($p.w)x$($p.h).$($p.fps)." + (Get-Date -Format 'HHmmss')
      $slog=Join-Path $base ($tag + '.s.log'); $serr=Join-Path $base ($tag + '.s.err')
      $alog=Join-Path $base ($tag + '.a.log'); $aerr=Join-Path $base ($tag + '.a.err')
      $clog=Join-Path $base ($tag + '.c.log'); $cerr=Join-Path $base ($tag + '.c.err')
      @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object { if(Test-Path $_){ cmd /c del /f /q "$_" | Out-Null } }
      Stop-Mrd

      $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
      Start-Sleep -Milliseconds 700
      $acmd='/c set AGENT_FFMPEG_PATH=' + $ffmpegExe + '&& set AGENT_FPS_MODE=throughput&& "' + $agentExe + '"'
      $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList $acmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
      Start-Sleep -Seconds 2
      $ccmd='/c set MRD_TRANSPORT=webrtc&& set MRD_DECODER=d3d11va&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
      $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

      Start-Sleep -Seconds $DurationSec

      if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
      if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
      if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

      $encodeFps=Parse-LastNum $alog '\[RTCP-PANEL\]' 'encode_fps'
      $sendFps=Parse-LastNum $alog '\[RTCP-PANEL\]' 'send_fps'
      $ctlFps=Parse-LastNum $clog '\[DECODER-STATS\]' 'fps'
      $pli=(Select-String -Path $alog -Pattern 'rtcp pli' | Measure-Object).Count
      $noOut=(Select-String -Path $clog -Pattern 'decoder no-output streak' | Measure-Object).Count
      $line=(Get-Content $clog | Select-String 'video frames received so far' | Select-Object -Last 1).Line
      $totalFrames=-1; $decodedFrames=-1
      if($line){
        $plain=[regex]::Replace($line,"\x1B\[[0-9;]*m","")
        $m1=[regex]::Match($plain,'total_frames=([0-9]+)'); $m2=[regex]::Match($plain,'decoded_frames=([0-9]+)')
        if($m1.Success){$totalFrames=[int64]$m1.Groups[1].Value}
        if($m2.Success){$decodedFrames=[int64]$m2.Groups[1].Value}
      }

      $target=[double]$p.fps
      $usable=($sendFps -ge ($target*0.8) -and $ctlFps -ge ($target*0.7) -and $decodedFrames -ge ($target*$DurationSec*0.6))
      $row=[pscustomobject]@{
        capture=$cap; width=$p.w; height=$p.h; fps_target=$p.fps;
        encode_fps=[math]::Round($encodeFps,2); send_fps=[math]::Round($sendFps,2); ctl_fps=[math]::Round($ctlFps,2);
        total_frames=$totalFrames; decoded_frames=$decodedFrames; pli=$pli; no_output=$noOut; usable=$usable;
        agent_log=$alog; controller_log=$clog
      }
      $rows += $row
      Write-Output ("swenc-range cap={0} {1}x{2}@{3} encode_fps={4} send_fps={5} ctl_fps={6} pli={7} no_output={8} usable={9}" -f $row.capture,$row.width,$row.height,$row.fps_target,$row.encode_fps,$row.send_fps,$row.ctl_fps,$row.pli,$row.no_output,$row.usable)
    }
  }

  $outCsv=Join-Path $base ('swenc-usable-range.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.csv')
  $rows | Export-Csv -Path $outCsv -NoTypeInformation -Encoding Ascii
  Write-Output ("swenc usable range csv: {0}" -f $outCsv)
}
finally {
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; cmd /c del /f /q "$agentBak" | Out-Null }
  Stop-Mrd
}
