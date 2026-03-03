$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$controllerDir = Join-Path $base 'controller-rust'
$agentDir = Join-Path $base 'agent-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$ffmpegExe = Join-Path $base 'tools/ffmpeg-min/ffmpeg.exe'
$agentCfgPath = Join-Path $agentDir 'config.json'
$agentBak = Join-Path $agentDir ('config.dxgi_wgc_cmp.' + (Get-Date -Format 'yyyyMMdd_HHmmss') + '.bak.json')

Copy-Item $agentCfgPath $agentBak -Force

function Set-JsonField($obj, [string]$name, $value) {
  if ($obj.PSObject.Properties.Name -contains $name) {
    $obj.$name = $value
  } else {
    $obj | Add-Member -NotePropertyName $name -NotePropertyValue $value
  }
}

function Stop-MrdProcs {
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Find-TestUfoHwnd {
  Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WinEnumCmp {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
}
"@
  $hits = [System.Collections.Generic.List[object]]::new()
  $cb = [WinEnumCmp+EnumWindowsProc]{
    param([IntPtr]$h,[IntPtr]$l)
    if(-not [WinEnumCmp]::IsWindowVisible($h)){ return $true }
    $len=[WinEnumCmp]::GetWindowTextLength($h)
    if($len -le 0){ return $true }
    $sb = New-Object System.Text.StringBuilder ($len+1)
    [void][WinEnumCmp]::GetWindowText($h,$sb,$sb.Capacity)
    $t=$sb.ToString()
    if($t -match 'testufo|UFO Test|Blur Busters'){
      $hits.Add([pscustomobject]@{Hwnd=('0x{0:X}' -f [int64]$h); Title=$t}) | Out-Null
    }
    return $true
  }
  [WinEnumCmp]::EnumWindows($cb,[IntPtr]::Zero) | Out-Null
  if($hits.Count -eq 0){ return $null }
  return $hits[0]
}

function Focus-Window([string]$hwndHex) {
  if (-not $hwndHex) { return }
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinFocusCmp {
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
  $h = [IntPtr]([Int64]::Parse($hwndHex.Replace('0x',''), [System.Globalization.NumberStyles]::HexNumber))
  [void][WinFocusCmp]::ShowWindowAsync($h, 9) # SW_RESTORE
  Start-Sleep -Milliseconds 120
  [void][WinFocusCmp]::SetForegroundWindow($h)
  Start-Sleep -Milliseconds 250
}

function Parse-AgentStats([string]$path) {
  $out = [ordered]@{
    send_fps = -1.0
    unique_fps = -1.0
    encode_fps = -1.0
    quic_au_sent = 0
    quic_au_dropped = 0
    native_direct_frames = 0
    native_copy_frames = 0
    native_scale_frames = 0
    native_direct_register_failures = 0
    native_acquire_ok = 0
    native_acquire_timeout = 0
    native_acquire_errors = 0
  }
  if (!(Test-Path $path)) { return $out }
  $line = (Get-Content $path | Select-String '\[RTCP-PANEL\]' | Select-Object -Last 1).Line
  if (!$line) { return $out }
  $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
  foreach($k in @('send_fps','unique_send_fps','encode_fps')) {
    $m = [regex]::Match($plain, ($k + '[^0-9]*([0-9]+(?:\.[0-9]+)?)'))
    if ($m.Success) {
      if ($k -eq 'send_fps') { $out.send_fps = [double]$m.Groups[1].Value }
      elseif ($k -eq 'unique_send_fps') { $out.unique_fps = [double]$m.Groups[1].Value }
      else { $out.encode_fps = [double]$m.Groups[1].Value }
    }
  }
  foreach($k in @('quic_au_sent','quic_au_dropped','native_direct_frames','native_copy_frames','native_scale_frames','native_direct_register_failures','native_acquire_ok','native_acquire_timeout','native_acquire_errors')) {
    $m = [regex]::Match($plain, ($k + '=([0-9]+)'))
    if ($m.Success) { $out.$k = [int64]$m.Groups[1].Value }
  }
  return $out
}

function Parse-ControllerStats([string]$path) {
  $out = [ordered]@{
    connected_quic = $false
    fps = -1.0
    avg_decode_ms = -1.0
    p95_decode_ms = -1.0
    jitter_ms = -1.0
    present_avg_ms = -1.0
    present_p95_ms = -1.0
    present_p99_ms = -1.0
  }
  if (!(Test-Path $path)) { return $out }
  $content = Get-Content $path
  if (($content | Select-String 'connected to QUIC media transport' | Select-Object -First 1)) {
    $out.connected_quic = $true
  }
  $dline = ($content | Select-String '\[DECODER-STATS\]' | Select-Object -Last 1).Line
  if ($dline) {
    $plain = [regex]::Replace($dline, "\x1B\[[0-9;]*m", "")
    $m = [regex]::Match($plain, 'fps=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.fps=[double]$m.Groups[1].Value}
    $m = [regex]::Match($plain, 'avg_decode_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.avg_decode_ms=[double]$m.Groups[1].Value}
    $m = [regex]::Match($plain, 'p95_decode_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.p95_decode_ms=[double]$m.Groups[1].Value}
    $m = [regex]::Match($plain, 'jitter_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.jitter_ms=[double]$m.Groups[1].Value}
  }
  $pline = ($content | Select-String '\[PRESENT-STATS\]' | Select-Object -Last 1).Line
  if ($pline) {
    $plain = [regex]::Replace($pline, "\x1B\[[0-9;]*m", "")
    $m = [regex]::Match($plain, 'capture_to_present_avg_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.present_avg_ms=[double]$m.Groups[1].Value}
    $m = [regex]::Match($plain, 'capture_to_present_p95_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.present_p95_ms=[double]$m.Groups[1].Value}
    $m = [regex]::Match($plain, 'capture_to_present_p99_ms=\"?([0-9]+(?:\.[0-9]+)?)\"?'); if($m.Success){$out.present_p99_ms=[double]$m.Groups[1].Value}
  }
  return $out
}

function Apply-Profile($cfg, [string]$backend) {
  $cfg.capture.fps = 240
  $cfg.capture.min_fps = 240
  $cfg.capture.max_fps = 240
  $cfg.capture.max_fps_mode = $true
  $cfg.capture.idle_repeat_fps = 240
  $cfg.capture.target_width = 1920
  $cfg.capture.target_height = 1080
  Set-JsonField $cfg.capture 'backend' $backend
  Set-JsonField $cfg.capture 'encoder' 'nvenc'
  if ($backend -eq 'wgc') {
    # strict_gpu_direct currently forces backend=dxgi in config normalization.
    Set-JsonField $cfg.capture 'strict_gpu_direct' $false
    Set-JsonField $cfg.capture 'allow_fallback' $true
    Set-JsonField $cfg.capture 'allow_encoder_fallback' $true
  } else {
    Set-JsonField $cfg.capture 'strict_gpu_direct' $true
    Set-JsonField $cfg.capture 'allow_fallback' $false
    Set-JsonField $cfg.capture 'allow_encoder_fallback' $false
  }
  Set-JsonField $cfg.capture 'encoder_tune' 'ull'
  Set-JsonField $cfg.capture 'encoder_preset' 'p1'
  Set-JsonField $cfg.capture 'rc_mode' 'cbr'
  Set-JsonField $cfg.capture 'bframes' 0
  Set-JsonField $cfg.capture 'gop' 240
  Set-JsonField $cfg.capture 'frame_pacing_enable' $false
  Set-JsonField $cfg.capture 'network_adapt_enable' $false
  Set-JsonField $cfg.capture 'adapt_enable' $false
  Set-JsonField $cfg.capture 'queue_strategy' 'drop'
  Set-JsonField $cfg.capture 'tier_limit_enable' $false
  Set-JsonField $cfg.capture 'max_frame_latency' 1
  Set-JsonField $cfg.capture 'bitrate_kbps' 20000
  Set-JsonField $cfg.capture 'max_bitrate_kbps' 28000
  $cfg.capture.queue_depth = 2
}

function Run-Case([string]$name, [string]$backend, [string]$wgcHwnd, [int]$seconds = 20) {
  if ($wgcHwnd) { Focus-Window -hwndHex $wgcHwnd }
  $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json
  Apply-Profile -cfg $cfg -backend $backend
  ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

  $tag = "accept.quic.1080p240.same_scene.$name." + (Get-Date -Format 'HHmmss')
  $slog = Join-Path $base ($tag + '.s.log')
  $serr = Join-Path $base ($tag + '.s.err')
  $alog = Join-Path $base ($tag + '.a.log')
  $aerr = Join-Path $base ($tag + '.a.err')
  $clog = Join-Path $base ($tag + '.c.log')
  $cerr = Join-Path $base ($tag + '.c.err')

  @($slog,$serr,$alog,$aerr,$clog,$cerr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
  }

  Stop-MrdProcs
  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
    -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700

  $agentCmd = "set AGENT_FFMPEG_PATH=$ffmpegExe&& set AGENT_QUIC_QUEUE=64&& set AGENT_QUIC_MAX_AU_BYTES=2097152&& "
  if ($backend -eq 'wgc' -and $wgcHwnd) {
    $agentCmd += "set AGENT_WGC_WINDOW_HWND=$wgcHwnd&& "
  }
  $agentCmd += "`"$agentExe`""

  $ap = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $agentCmd `
    -WorkingDirectory $agentDir -PassThru `
    -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $cp = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set MRD_TRANSPORT=quic&& set MRD_DECODER=d3d11va&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
    -WorkingDirectory $controllerDir -PassThru `
    -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds $seconds

  if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
  if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
  if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

  $as = Parse-AgentStats $alog
  $cs = Parse-ControllerStats $clog

  [pscustomobject]@{
    case = $name
    backend = $backend
    quic_connected = $cs.connected_quic
    controller_fps = [math]::Round($cs.fps,2)
    avg_decode_ms = [math]::Round($cs.avg_decode_ms,3)
    p95_decode_ms = [math]::Round($cs.p95_decode_ms,3)
    jitter_ms = [math]::Round($cs.jitter_ms,3)
    present_avg_ms = [math]::Round($cs.present_avg_ms,3)
    present_p95_ms = [math]::Round($cs.present_p95_ms,3)
    present_p99_ms = [math]::Round($cs.present_p99_ms,3)
    encode_fps = [math]::Round($as.encode_fps,2)
    send_fps = [math]::Round($as.send_fps,2)
    unique_fps = [math]::Round($as.unique_fps,2)
    quic_au_sent = $as.quic_au_sent
    quic_au_dropped = $as.quic_au_dropped
    native_direct = $as.native_direct_frames
    native_copy = $as.native_copy_frames
    native_scale = $as.native_scale_frames
    direct_reg_fail = $as.native_direct_register_failures
    acquire_ok = $as.native_acquire_ok
    acquire_timeout = $as.native_acquire_timeout
    acquire_errors = $as.native_acquire_errors
    agent_log = $alog
    controller_log = $clog
  }
}

try {
  $target = Find-TestUfoHwnd
  if (-not $target) {
    throw "未找到 TestUFO 窗口，请先把浏览器 TestUFO 页面置于可见状态"
  }
  Write-Output ("Using TestUFO hwnd={0} title={1}" -f $target.Hwnd, $target.Title)

  $dxgi = Run-Case -name 'dxgi_1080' -backend 'dxgi' -wgcHwnd $null
  $wgc = Run-Case -name 'wgc_1080' -backend 'wgc' -wgcHwnd $target.Hwnd
  $rows = @($dxgi, $wgc)
  $rows | Format-Table -AutoSize
  Write-Output "logs:"
  $rows | ForEach-Object { Write-Output ("{0}: agent={1} controller={2}" -f $_.case,$_.agent_log,$_.controller_log) }
}
finally {
  if (Test-Path $agentBak) {
    Copy-Item $agentBak $agentCfgPath -Force
    Remove-Item $agentBak -Force -ErrorAction SilentlyContinue
  }
  Stop-MrdProcs
}
