$ErrorActionPreference='Stop'
$base=(Resolve-Path '..').Path
$agentDir=(Resolve-Path '.').Path
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.modes.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force

function Run-Mode([string]$name,[string]$backend,[bool]$strict){
  $tag = "accept.1080p240.$name." + (Get-Date -Format 'HHmmss')
  $slog=Join-Path $base ($tag+'.s.log'); $serr=Join-Path $base ($tag+'.s.err')
  $alog=Join-Path $base ($tag+'.a.log'); $aerr=Join-Path $base ($tag+'.a.err')
  $plog=Join-Path $base ($tag+'.p.log'); $perr=Join-Path $base ($tag+'.p.err')

  $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
  $cfg.capture.backend=$backend
  $cfg.capture.encoder='nvenc'
  $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
  $cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240
  $cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=240
  if ($null -eq $cfg.capture.PSObject.Properties['strict_gpu_direct']) {
    $cfg.capture | Add-Member -NotePropertyName strict_gpu_direct -NotePropertyValue $strict
  } else { $cfg.capture.strict_gpu_direct=$strict }
  if($strict){ $cfg.capture.allow_fallback=$false; $cfg.capture.allow_encoder_fallback=$false }
  else { $cfg.capture.allow_fallback=$true; $cfg.capture.allow_encoder_fallback=$true }
  ($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii

  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
  $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700
  $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2
  $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=15&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
  try { $pp | Wait-Process -Timeout 120 } finally {
    if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
    if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}
  }

  $probe=(Get-Content $plog | Select-String 'media_stats:' | Select-Object -Last 1).Line
  $agent=(Get-Content $alog)
  $backendLine=($agent | Select-String 'capture backend selected:' | Select-Object -Last 1).Line
  $nvencLine=($agent | Select-String 'native NVENC pipeline attached|WGC native NVENC texture pipeline attached' | Select-Object -Last 1).Line
  $errLine=($agent | Select-String 'native NVENC encode failed|WGC native NVENC encode failed' | Select-Object -Last 1).Line
  Write-Output "mode=$name tag=$tag"
  Write-Output "backend_line=$backendLine"
  Write-Output "nvenc_line=$nvencLine"
  Write-Output "err_line=$errLine"
  Write-Output "probe=$probe"
}

try {
  Run-Mode -name 'dxgi.strict' -backend 'dxgi' -strict $true
  Run-Mode -name 'wgc' -backend 'wgc' -strict $false
}
finally {
  if (Test-Path $bak) { Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue }
  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
}


