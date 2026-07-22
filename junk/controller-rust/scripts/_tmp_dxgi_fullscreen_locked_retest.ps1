$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$inner = Join-Path $PSScriptRoot 'accept_1080p240_quic_gpu_direct_extreme_tmp.ps1'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WinProbe {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@

function Find-TestUfoWindow {
  $hits = [System.Collections.Generic.List[object]]::new()
  $cb = [WinProbe+EnumWindowsProc]{
    param([IntPtr]$h,[IntPtr]$l)
    if(-not [WinProbe]::IsWindowVisible($h)){ return $true }
    $len=[WinProbe]::GetWindowTextLength($h)
    if($len -le 0){ return $true }
    $sb = New-Object System.Text.StringBuilder ($len+1)
    [void][WinProbe]::GetWindowText($h,$sb,$sb.Capacity)
    $t=$sb.ToString()
    if($t -match 'testufo|UFO Test|Blur Busters'){
      $hits.Add([pscustomobject]@{Handle=$h; Hwnd=('0x{0:X}' -f [int64]$h); Title=$t}) | Out-Null
    }
    return $true
  }
  [WinProbe]::EnumWindows($cb,[IntPtr]::Zero) | Out-Null
  if($hits.Count -eq 0){ return $null }
  return $hits[0]
}

function Focus-Window($handle) {
  [void][WinProbe]::ShowWindowAsync($handle, 9) # SW_RESTORE
  Start-Sleep -Milliseconds 120
  [void][WinProbe]::SetForegroundWindow($handle)
  Start-Sleep -Milliseconds 280
}

function Parse-CaseValue([string[]]$lines, [string]$case, [string]$key) {
  $line = $lines | Select-String ("case={0} .*{1}=(-?[0-9]+(?:\.[0-9]+)?)" -f $case, $key) | Select-Object -Last 1
  if(-not $line){ return $null }
  $m=[regex]::Match($line.Line, ("{0}=(-?[0-9]+(?:\.[0-9]+)?)" -f $key))
  if(-not $m.Success){ return $null }
  return [double]$m.Groups[1].Value
}

$target = Find-TestUfoWindow
if(-not $target){
  throw "未找到 TestUFO 窗口，请先确保浏览器页面可见且前台。"
}

Write-Output ("Using TestUFO: hwnd={0} title={1}" -f $target.Hwnd, $target.Title)

$runs = @()
for($i=1; $i -le 3; $i++){
  Focus-Window $target.Handle
  $outLog = Join-Path $base ("accept.locked.retest.run{0}.{1}.out.log" -f $i, (Get-Date -Format 'HHmmss'))
  $errLog = Join-Path $base ("accept.locked.retest.run{0}.{1}.err.log" -f $i, (Get-Date -Format 'HHmmss'))
  $proc = Start-Process -FilePath 'powershell' -ArgumentList @('-ExecutionPolicy','Bypass','-File', $inner) `
    -WorkingDirectory $base -PassThru -RedirectStandardOutput $outLog -RedirectStandardError $errLog

  $total=0
  $fg=0
  while(-not $proc.HasExited){
    Focus-Window $target.Handle
    $cur=[WinProbe]::GetForegroundWindow()
    $total++
    if($cur -eq $target.Handle){ $fg++ }
    Start-Sleep -Milliseconds 200
  }

  $lines = @()
  if(Test-Path $outLog){ $lines = Get-Content $outLog }
  $noscaleSend = Parse-CaseValue $lines 'extreme_noscale' 'send_fps'
  $noscaleCtl = Parse-CaseValue $lines 'extreme_noscale' 'controller_fps'
  $scaleSend = Parse-CaseValue $lines 'extreme_1080' 'send_fps'
  $scaleCtl = Parse-CaseValue $lines 'extreme_1080' 'controller_fps'
  $ratio = if($total -gt 0){ [math]::Round($fg*1.0/$total,4) } else { 0.0 }
  $valid = $ratio -ge 0.95
  $runs += [pscustomobject]@{
    run = $i
    fg_ratio = $ratio
    valid = $valid
    noscale_send_fps = $noscaleSend
    noscale_controller_fps = $noscaleCtl
    scale1080_send_fps = $scaleSend
    scale1080_controller_fps = $scaleCtl
    out_log = $outLog
    err_log = $errLog
  }
  Write-Output ("run={0} fg_ratio={1} valid={2} noscale_send={3} noscale_ctl={4} 1080_send={5} 1080_ctl={6}" -f `
    $i,$ratio,$valid,$noscaleSend,$noscaleCtl,$scaleSend,$scaleCtl)
}

Write-Output "==== SUMMARY ===="
$runs | Format-Table -AutoSize

$validRuns = $runs | Where-Object { $_.valid -eq $true -and $_.scale1080_send_fps -ne $null }
if($validRuns.Count -gt 0){
  $max = ($validRuns | Measure-Object -Property scale1080_send_fps -Maximum).Maximum
  $min = ($validRuns | Measure-Object -Property scale1080_send_fps -Minimum).Minimum
  $avg = [math]::Round((($validRuns | Measure-Object -Property scale1080_send_fps -Average).Average),2)
  Write-Output ("valid_runs={0} scale1080_send_fps min={1} max={2} avg={3}" -f $validRuns.Count,$min,$max,$avg)
} else {
  Write-Output "valid_runs=0 (foreground ratio < 0.95)"
}
