$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.render.mode.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }

Copy-Item $agentCfgPath $agentBak -Force
try {
  $cfg=Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  $cfg.capture.fps=60; $cfg.capture.min_fps=60; $cfg.capture.max_fps=60; $cfg.capture.idle_repeat_fps=60
  $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
  Set-JsonField $cfg.capture 'backend' 'wgc'
  Set-JsonField $cfg.capture 'encoder' 'nvenc'
  Set-JsonField $cfg.capture 'strict_gpu_direct' $false
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  foreach($mode in @('low_latency','smooth')){
    $tag="accept.render.mode.$mode." + (Get-Date -Format 'HHmmss')
    $slog=Join-Path $base ($tag + '.s.log'); $serr=Join-Path $base ($tag + '.s.err')
    $alog=Join-Path $base ($tag + '.a.log'); $aerr=Join-Path $base ($tag + '.a.err')
    $clog=Join-Path $base ($tag + '.c.log'); $cerr=Join-Path $base ($tag + '.c.err')
    @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object { if(Test-Path $_){ cmd /c del /f /q "$_" | Out-Null } }
    Stop-Mrd

    $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
    Start-Sleep -Milliseconds 700
    $acmd='/c set AGENT_FFMPEG_PATH=' + $ffmpegExe + '&& "' + $agentExe + '"'
    $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList $acmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
    Start-Sleep -Seconds 2
    $ccmd='/c set MRD_TRANSPORT=webrtc&& set MRD_DECODER=d3d11va&& set MRD_RENDER_MODE=' + $mode + '&& set MRD_SR_MODE=quality&& set MRD_SMOOTH_TARGET_FPS=120&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
    $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

    Start-Sleep -Seconds 12
    if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

    $line=(Get-Content $clog | Select-String 'renderer progress' | Select-Object -Last 1).Line
    $plain=[regex]::Replace(($line|Out-String),"\x1B\[[0-9;]*m","")
    $mMode=[regex]::Match($plain,'render_mode="?([a-z_]+)"?')
    $mRep=[regex]::Match($plain,'smooth_repeated_total=([0-9]+)')
    $mRfps=[regex]::Match($plain,'rendered_fps="?([0-9]+(?:\.[0-9]+)?)"?')
    $modeSeen=if($mMode.Success){$mMode.Groups[1].Value}else{'n/a'}
    $rep=if($mRep.Success){$mRep.Groups[1].Value}else{'n/a'}
    $rfps=if($mRfps.Success){$mRfps.Groups[1].Value}else{'n/a'}
    Write-Output ("render-mode-check mode={0} logged_mode={1} smooth_repeated_total={2} rendered_fps={3} clog={4}" -f $mode,$modeSeen,$rep,$rfps,$clog)
  }
}
finally {
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; cmd /c del /f /q "$agentBak" | Out-Null }
  Stop-Mrd
}

