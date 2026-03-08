$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$controllerCfgPath=Join-Path $controllerDir 'config.json'
$controllerBak=Join-Path $controllerDir ('config.record.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.record.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$recordDir=Join-Path $base ('recordings.accept.' + (Get-Date -Format 'yyyyMMdd_HHmmss'))
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }

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
  ($acfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  $ccfg=Get-Content $controllerCfgPath -Raw | ConvertFrom-Json
  if(-not ($ccfg.PSObject.Properties.Name -contains 'render')){ $ccfg | Add-Member -NotePropertyName 'render' -NotePropertyValue ([pscustomobject]@{}) }
  Set-JsonField $ccfg.render 'sr_mode' 'performance'
  if(-not ($ccfg.PSObject.Properties.Name -contains 'record')){ $ccfg | Add-Member -NotePropertyName 'record' -NotePropertyValue ([pscustomobject]@{}) }
  Set-JsonField $ccfg.record 'enabled' $true
  Set-JsonField $ccfg.record 'output_dir' $recordDir
  Set-JsonField $ccfg.record 'ffmpeg_path' $ffmpegExe
  Set-JsonField $ccfg.record 'segment_seconds' 10
  Set-JsonField $ccfg.record 'input_fps' 180
  Set-JsonField $ccfg.record 'container' 'mp4'
  Set-JsonField $ccfg.record 'queue_depth' 1024
  ($ccfg | ConvertTo-Json -Depth 100) | Set-Content -Path $controllerCfgPath -Encoding Ascii

  $tag='accept.record.2k180.' + (Get-Date -Format 'HHmmss')
  $slog=Join-Path $base ($tag + '.s.log'); $serr=Join-Path $base ($tag + '.s.err')
  $alog=Join-Path $base ($tag + '.a.log'); $aerr=Join-Path $base ($tag + '.a.err')
  $clog=Join-Path $base ($tag + '.c.log'); $cerr=Join-Path $base ($tag + '.c.err')

  Stop-Mrd
  $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700
  $acmd='/c set AGENT_FFMPEG_PATH=' + $ffmpegExe + '&& "' + $agentExe + '"'
  $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList $acmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2
  $ccmd='/c set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set MRD_RENDER_MODE=low_latency&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& "' + $controllerExe + '"'
  $cp=Start-Process -FilePath 'cmd.exe' -ArgumentList $ccmd -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds 40

  if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
  if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
  if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

  $recLine=(Get-Content $clog | Select-String '\[RECORD-STATS\]' | Select-Object -Last 1).Line
  $plain=[regex]::Replace(($recLine|Out-String),"\x1B\[[0-9;]*m","")
  $written=[regex]::Match($plain,'written_frames=([0-9]+)').Groups[1].Value
  if([string]::IsNullOrWhiteSpace($written)){ $written='n/a' }
  $dropped=[regex]::Match($plain,'dropped_frames=([0-9]+)').Groups[1].Value
  if([string]::IsNullOrWhiteSpace($dropped)){ $dropped='n/a' }
  $files=@()
  if(Test-Path $recordDir){ $files=Get-ChildItem -Path $recordDir -File -Filter '*.mp4' | Sort-Object Name }
  $cnt=$files.Count
  $first=if($cnt -gt 0){$files[0].FullName}else{'n/a'}
  Write-Output ("record_test written_frames={0} dropped_frames={1} mp4_count={2} first_file={3} controller_log={4}" -f $written,$dropped,$cnt,$first,$clog)
}
finally {
  if(Test-Path $controllerBak){ Copy-Item $controllerBak $controllerCfgPath -Force; Remove-Item $controllerBak -Force -ErrorAction SilentlyContinue }
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; Remove-Item $agentBak -Force -ErrorAction SilentlyContinue }
  Stop-Mrd
}
