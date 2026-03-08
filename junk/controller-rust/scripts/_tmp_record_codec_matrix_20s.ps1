$ErrorActionPreference='Stop'
$base='J:/ProjectTest/remote-desktop/mini-remote-desktop'
$agentDir=Join-Path $base 'agent-rust'
$controllerDir=Join-Path $base 'controller-rust'
$signalingDir=Join-Path $base 'signaling-rs'
$controllerCfgPath=Join-Path $controllerDir 'config.json'
$controllerBak=Join-Path $controllerDir ('config.record.codec.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$agentCfgPath=Join-Path $agentDir 'config.json'
$agentBak=Join-Path $agentDir ('config.record.codec.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')
$signalingExe=Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe=Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

function Stop-Mrd { Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } | Stop-Process -Force -ErrorAction SilentlyContinue }
function Set-JsonField($obj,[string]$name,$value){ if($obj.PSObject.Properties.Name -contains $name){$obj.$name=$value}else{$obj|Add-Member -NotePropertyName $name -NotePropertyValue $value} }
function LastLine([string]$path,[string]$pat){ if(!(Test-Path $path)){return ''}; $line=(Get-Content $path | Select-String $pat | Select-Object -Last 1).Line; if(!$line){return ''}; return [regex]::Replace(($line|Out-String),"\x1B\[[0-9;]*m","").Trim() }
function Extract([string]$text,[string]$pat){ $m=[regex]::Match($text,$pat); if($m.Success){return $m.Groups[1].Value}; return 'n/a' }

$codecs=@('copy','libx264','h264_nvenc','libx265','hevc_nvenc')
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

  foreach($codec in $codecs){
    $dir=Join-Path $base ('recordings.codec.' + $codec + '.' + (Get-Date -Format 'HHmmss'))
    $ccfg=Get-Content $controllerCfgPath -Raw | ConvertFrom-Json
    if(-not ($ccfg.PSObject.Properties.Name -contains 'render')){ $ccfg | Add-Member -NotePropertyName 'render' -NotePropertyValue ([pscustomobject]@{}) }
    Set-JsonField $ccfg.render 'sr_mode' 'performance'
    if(-not ($ccfg.PSObject.Properties.Name -contains 'record')){ $ccfg | Add-Member -NotePropertyName 'record' -NotePropertyValue ([pscustomobject]@{}) }
    Set-JsonField $ccfg.record 'enabled' $true
    Set-JsonField $ccfg.record 'output_dir' $dir
    Set-JsonField $ccfg.record 'ffmpeg_path' $ffmpegExe
    Set-JsonField $ccfg.record 'segment_seconds' 10
    Set-JsonField $ccfg.record 'input_fps' 180
    Set-JsonField $ccfg.record 'container' 'mp4'
    Set-JsonField $ccfg.record 'video_codec' $codec
    Set-JsonField $ccfg.record 'video_preset' 'p4'
    Set-JsonField $ccfg.record 'video_crf' 23
    Set-JsonField $ccfg.record 'video_bitrate_kbps' 12000
    Set-JsonField $ccfg.record 'queue_depth' 1024
    ($ccfg | ConvertTo-Json -Depth 100) | Set-Content -Path $controllerCfgPath -Encoding Ascii

    $tag='accept.record.codec.' + $codec + '.' + (Get-Date -Format 'HHmmss')
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

    Start-Sleep -Seconds 20

    if(Get-Process -Id $cp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $cp.Id -Force}
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}

    $r=LastLine $clog '\[RECORD-STATS\]'
    $d=LastLine $clog '\[DECODER-STATS\]'
    $written=Extract $r 'written_frames=([0-9]+)'
    $drop=Extract $r 'dropped_frames=([0-9]+)'
    $wf=Extract $r 'write_failures=([0-9]+)'
    $aw=Extract $r 'avg_write_us="?([0-9]+(?:\.[0-9]+)?)"?'
    $fps=Extract $d 'fps="?([0-9]+(?:\.[0-9]+)?)"?'
    $files=@(); if(Test-Path $dir){ $files=Get-ChildItem -Path $dir -File -Filter '*.mp4' }
    $ok=if($files.Count -gt 0 -and $written -ne 'n/a' -and $written -ne '0'){'ok'}else{'fail'}
    Write-Output ("codec={0} status={1} decode_fps={2} written_frames={3} dropped_frames={4} write_failures={5} avg_write_us={6} mp4_count={7} dir={8} clog={9}" -f $codec,$ok,$fps,$written,$drop,$wf,$aw,$files.Count,$dir,$clog)
  }
}
finally {
  if(Test-Path $controllerBak){ Copy-Item $controllerBak $controllerCfgPath -Force; Remove-Item $controllerBak -Force -ErrorAction SilentlyContinue }
  if(Test-Path $agentBak){ Copy-Item $agentBak $agentCfgPath -Force; Remove-Item $agentBak -Force -ErrorAction SilentlyContinue }
  Stop-Mrd
}
