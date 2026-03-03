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
if($hits.Count -eq 0){ Write-Output 'NO_TESTUFO_WINDOW'; exit 0 }
$hits | ConvertTo-Json -Depth 4
