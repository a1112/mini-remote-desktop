$ErrorActionPreference='Stop'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WinEnum {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
}
"@

function Find-TestUfoHwnd {
  $hits = [System.Collections.Generic.List[object]]::new()
  $cb = [WinEnum+EnumWindowsProc]{
    param([IntPtr]$h,[IntPtr]$l)
    if(-not [WinEnum]::IsWindowVisible($h)){ return $true }
    $len=[WinEnum]::GetWindowTextLength($h)
    if($len -le 0){ return $true }
    $sb = New-Object System.Text.StringBuilder ($len+1)
    [void][WinEnum]::GetWindowText($h,$sb,$sb.Capacity)
    $t=$sb.ToString()
    if($t -match 'testufo|UFO Test|Blur Busters'){
      $hits.Add([pscustomobject]@{Hwnd=('0x{0:X}' -f [int64]$h); Title=$t}) | Out-Null
    }
    return $true
  }
  [WinEnum]::EnumWindows($cb,[IntPtr]::Zero) | Out-Null
  if($hits.Count -eq 0){ return $null }
  return $hits[0]
}

function Parse-ProbeLine($plog){
  $line=(Get-Content $plog | Select-String 'media_stats:' | Select-Object -Last 1).Line
  if(-not $line){
    return [pscustomobject]@{fps=0.0;frames=0;ppf=0.0;line=''}
  }
  $fps=0.0; $frames=0; $ppf=0.0
  $m=[regex]::Match($line,'estimated_fps=([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$fps=[double]$m.Groups[1].Value}
  $m=[regex]::Match($line,'frames=([0-9]+)'); if($m.Success){$frames=[int]$m.Groups[1].Value}
  $m=[regex]::Match($line,'packets_per_frame=([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$ppf=[double]$m.Groups[1].Value}
  return [pscustomobject]@{fps=$fps;frames=$frames;ppf=$ppf;line=$line}
}

function Parse-AgentStats($alog){
  $lines=Get-Content $alog
  $rebind=($lines | Select-String 'wgc capture session rebound').Count
  $fallbackKeep=($lines | Select-String 'wgc rebind fallback').Count
  $encErr=($lines | Select-String 'WGC native NVENC encode failed').Count
  $maxSend=0.0; $maxUnique=0.0; $maxEncode=0.0
  foreach($l in $lines){
    if($l -notmatch '\[RTCP-PANEL\]'){ continue }
    $m=[regex]::Match($l,'send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[1].Value; if($v -gt $maxSend){$maxSend=$v}}
    $m=[regex]::Match($l,'unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[1].Value; if($v -gt $maxUnique){$maxUnique=$v}}
    $m=[regex]::Match($l,'encode_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)'); if($m.Success){$v=[double]$m.Groups[1].Value; if($v -gt $maxEncode){$maxEncode=$v}}
  }
  return [pscustomobject]@{rebind=$rebind;fallbackKeep=$fallbackKeep;encErr=$encErr;maxSend=$maxSend;maxUnique=$maxUnique;maxEncode=$maxEncode}
}

$target=Find-TestUfoHwnd
if($null -eq $target){ throw 'TestUFO window not found. Please keep browser window open.' }

$base=(Resolve-Path '..').Path
$agentDir=(Resolve-Path '.').Path
$signalingDir=Join-Path $base 'signaling-rs'
$signalingExe=Join-Path $signalingDir 'target-codex-hotfix/debug/signaling-rs.exe'
$agentExe=Join-Path $agentDir 'target/debug/agent-rust.exe'
$probeExe=Join-Path $agentDir 'target/debug/m2_offer_probe.exe'
$ffmpegExe=Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$cfgPath=Join-Path $agentDir 'config.json'
$bak=Join-Path $agentDir ('config.wgc.curve.'+(Get-Date -Format 'yyyyMMdd_HHmmss')+'.bak.json')
Copy-Item $cfgPath $bak -Force

$tiers=@(120,60)
$runs=3
$probeSecs=20
$results=[System.Collections.Generic.List[object]]::new()

try {
  foreach($tier in $tiers){
    for($i=1; $i -le $runs; $i++){
      $tag=('accept.wgc.testufo.{0}.run{1}.{2}' -f $tier,$i,(Get-Date -Format 'HHmmss'))
      $slog=Join-Path $base ($tag+'.s.log'); $serr=Join-Path $base ($tag+'.s.err')
      $alog=Join-Path $base ($tag+'.a.log'); $aerr=Join-Path $base ($tag+'.a.err')
      $plog=Join-Path $base ($tag+'.p.log'); $perr=Join-Path $base ($tag+'.p.err')

      $cfg=Get-Content $cfgPath -Raw | ConvertFrom-Json
      $cfg.capture.backend='wgc'
      $cfg.capture.encoder='nvenc'
      $cfg.capture.target_width=1920; $cfg.capture.target_height=1080
      $cfg.capture.fps=$tier; $cfg.capture.min_fps=$tier; $cfg.capture.max_fps=$tier
      $cfg.capture.max_fps_mode=$true; $cfg.capture.idle_repeat_fps=$tier
      $cfg.capture.allow_fallback=$true
      $cfg.capture.allow_encoder_fallback=$true
      if ($null -eq $cfg.capture.PSObject.Properties['strict_gpu_direct']) {
        $cfg.capture | Add-Member -NotePropertyName strict_gpu_direct -NotePropertyValue $false
      } else { $cfg.capture.strict_gpu_direct=$false }
      ($cfg|ConvertTo-Json -Depth 100)|Set-Content -Path $cfgPath -Encoding Ascii

      Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
      $sp=Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $slog -RedirectStandardError $serr
      Start-Sleep -Milliseconds 700
      $ap=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set AGENT_WGC_WINDOW_HWND=$($target.Hwnd)&& set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $alog -RedirectStandardError $aerr
      Start-Sleep -Seconds 2
      $pp=Start-Process -FilePath 'cmd.exe' -ArgumentList '/c',("set PROBE_SECS=$probeSecs&& `"$probeExe`"") -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $plog -RedirectStandardError $perr
      try { $pp | Wait-Process -Timeout ($probeSecs+120) } finally {
        if(Get-Process -Id $ap.Id -ErrorAction SilentlyContinue){Stop-Process -Id $ap.Id -Force}
        if(Get-Process -Id $sp.Id -ErrorAction SilentlyContinue){Stop-Process -Id $sp.Id -Force}
      }

      $p=Parse-ProbeLine $plog
      $a=Parse-AgentStats $alog
      $eff=[math]::Round((100.0*$p.fps/[double]$tier),2)
      $o=[pscustomobject]@{
        tier=$tier; run=$i; fps=[math]::Round($p.fps,2); frames=$p.frames; ppf=[math]::Round($p.ppf,2);
        eff_pct=$eff; max_send=[math]::Round($a.maxSend,2); max_unique=[math]::Round($a.maxUnique,2); max_encode=[math]::Round($a.maxEncode,2);
        rebind=$a.rebind; fallback_keep=$a.fallbackKeep; enc_err=$a.encErr; tag=$tag
      }
      $results.Add($o) | Out-Null
      Write-Output ("tier={0} run={1} fps={2} eff={3}% frames={4} ppf={5} send={6} unique={7} encode={8} rebind={9} err={10} tag={11}" -f $o.tier,$o.run,$o.fps,$o.eff_pct,$o.frames,$o.ppf,$o.max_send,$o.max_unique,$o.max_encode,$o.rebind,$o.enc_err,$o.tag)
    }
  }

  Write-Output '=== summary ==='
  foreach($tier in $tiers){
    $set=@($results | Where-Object { $_.tier -eq $tier })
    $avgFps=[math]::Round((($set | Measure-Object -Property fps -Average).Average),2)
    $bestFps=[math]::Round((($set | Measure-Object -Property fps -Maximum).Maximum),2)
    $avgEff=[math]::Round((($set | Measure-Object -Property eff_pct -Average).Average),2)
    $avgRebind=[math]::Round((($set | Measure-Object -Property rebind -Average).Average),2)
    $sumErr=($set | Measure-Object -Property enc_err -Sum).Sum
    Write-Output ("tier={0} avg_fps={1} best_fps={2} avg_eff={3}% avg_rebind={4} sum_encode_err={5}" -f $tier,$avgFps,$bestFps,$avgEff,$avgRebind,$sumErr)
  }
}
finally {
  if(Test-Path $bak){ Copy-Item $bak $cfgPath -Force; Remove-Item $bak -Force -ErrorAction SilentlyContinue }
  Get-Process | ? { $_.ProcessName -in @('signaling-rs','agent-rust','m2_offer_probe') } | Stop-Process -Force -ErrorAction SilentlyContinue
}
