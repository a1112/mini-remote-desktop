$ErrorActionPreference='Stop'
$base=(Resolve-Path '..').Path
$agentDir=(Resolve-Path '.').Path
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.wgc.stress2.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force
$tag='accept.wgc.stress.dynamic.'+(Get-Date -Format 'HHmmss')
$slog=Join-Path $base ($tag+'.s.log'); $serr=Join-Path $base ($tag+'.s.err')
$alog=Join-Path $base ($tag+'.a.log'); $aerr=Join-Path $base ($tag+'.a.err')
$plog=Join-Path $base ($tag+'.p.log'); $perr=Join-Path $base ($tag+'.p.err')

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@

$np = $null
$sp = $null
$ap = $null
$pp = $null
$ps = $null
try {
  $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
  $cfg.capture.backend='wgc'
  $cfg.capture.encoder='nvenc'
  $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
  $cfg.capture.fps=240; $cfg.capture.min_fps=240; $cfg.capture.max_fps=240
  $cfg.capture.allow_fallback=$true
  $cfg.capture.allow_encoder_fallback=$true
  if ($null -eq $cfg.capture.PSObject.Properties['strict_gpu_direct']) {
    $cfg.capture | Add-Member -NotePropertyName strict_gpu_direct -NotePropertyValue $false
  } else { $cfg.capture.strict_gpu_direct=$false }
  ($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii

  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe','notepad') } | Stop-Process -Force -ErrorAction SilentlyContinue

  $np = Start-Process notepad -PassThru
  Start-Sleep -Seconds 1
  $np.Refresh()
  [WinApi]::SetForegroundWindow([IntPtr]::new([int64]$np.MainWindowHandle)) | Out-Null

  $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700
  $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $j = Start-Job -ScriptBlock {
    param($h)
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
    $ptr=[IntPtr]::new($h)
    Start-Sleep -Seconds 5
    [WinApi]::ShowWindow($ptr,6) | Out-Null
    Start-Sleep -Seconds 3
    [WinApi]::ShowWindow($ptr,9) | Out-Null
    [WinApi]::SetForegroundWindow($ptr) | Out-Null
    Start-Sleep -Seconds 3
    [WinApi]::ShowWindow($ptr,6) | Out-Null
    Start-Sleep -Seconds 3
    [WinApi]::ShowWindow($ptr,9) | Out-Null
  } -ArgumentList ([int64]$np.MainWindowHandle)

  $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=20&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
  $pp | Wait-Process -Timeout 180
  Receive-Job $j | Out-Null
  Remove-Job $j -Force

  $probe=(Get-Content $plog | Select-String 'media_stats:' | Select-Object -Last 1).Line
  $agent=Get-Content $alog
  $rebindCount=($agent | Select-String 'wgc capture session rebound').Count
  $fallbackCount=($agent | Select-String 'wgc rebind fallback').Count
  $errCount=($agent | Select-String 'WGC native NVENC encode failed').Count
  $backend=($agent | Select-String 'capture backend selected:' | Select-Object -Last 1).Line
  $pipe=($agent | Select-String 'WGC native NVENC texture pipeline attached' | Select-Object -Last 1).Line

  Write-Output "TAG=$tag"
  Write-Output "PROBE=$probe"
  Write-Output "BACKEND=$backend"
  Write-Output "PIPE=$pipe"
  Write-Output "REBINDS=$rebindCount FALLBACK_KEEP=$fallbackCount ENCODE_ERR=$errCount"
}
finally {
  foreach($p in @($pp,$ap,$sp,$np,$ps)){
    if($p -and (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)){ Stop-Process -Id $p.Id -Force }
  }
  if (Test-Path $bak) { Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue }
  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe','notepad') } | Stop-Process -Force -ErrorAction SilentlyContinue
}


