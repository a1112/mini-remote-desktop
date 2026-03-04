$ErrorActionPreference = 'Stop'

$base = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$controllerDir = Join-Path $base 'controller-rust'
$agentDir = Join-Path $base 'agent-rust'
$signalingDir = Join-Path $base 'signaling-rs'

$signalingExe = Join-Path $signalingDir 'target/debug/signaling-rs.exe'
$agentExe = Join-Path $agentDir 'target/debug/agent-rust.exe'
$controllerExe = Join-Path $controllerDir 'target/debug/controller-rust.exe'
$verifyScript = Join-Path $controllerDir 'scripts/verify_control_latency.ps1'
$ffmpegExe = Join-Path $base 'tools/ffmpeg_full_build/bin/ffmpeg.exe'

$durationSec = 25
$rateHz = 250

$tag = "smoke.ctrl.lat.webrtc." + (Get-Date -Format 'HHmmss')
$slog = Join-Path $base ($tag + '.s.log')
$serr = Join-Path $base ($tag + '.s.err')
$alog = Join-Path $base ($tag + '.a.log')
$aerr = Join-Path $base ($tag + '.a.err')
$clog = Join-Path $base ($tag + '.c.log')
$cerr = Join-Path $base ($tag + '.c.err')

Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
  Stop-Process -Force -ErrorAction SilentlyContinue

try {
  $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru `
    -RedirectStandardOutput $slog -RedirectStandardError $serr
  Start-Sleep -Milliseconds 700

  $ap = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`"") `
    -WorkingDirectory $agentDir -PassThru `
    -RedirectStandardOutput $alog -RedirectStandardError $aerr
  Start-Sleep -Seconds 2

  $cp = Start-Process -FilePath 'cmd.exe' `
    -ArgumentList '/c',("set MRD_TRANSPORT=webrtc&& set MRD_DECODER=d3d11va&& set MRD_CTRL_BENCH_ENABLE=1&& set MRD_CTRL_BENCH_RATE_HZ=$rateHz&& set RUST_LOG=controller_rust=info,tokio=warn,webrtc=warn&& `"$controllerExe`"") `
    -WorkingDirectory $controllerDir -PassThru `
    -RedirectStandardOutput $clog -RedirectStandardError $cerr

  Start-Sleep -Seconds $durationSec

  if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
  if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
  if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

  & $verifyScript -LogFile $alog -P95ThresholdMs 12 -P99ThresholdMs 18 -MinSamples 100
  $txLine = (Get-Content $clog | Select-String '\[CTRL-TX\]' | Select-Object -Last 1).Line
  Write-Output ("controller ctrl-tx: {0}" -f $txLine)
  Write-Output ("logs: a={0} c={1} s={2}" -f $alog,$clog,$slog)
}
finally {
  Get-Process | Where-Object { $_.ProcessName -in @('signaling-rs','agent-rust','controller-rust') } |
    Stop-Process -Force -ErrorAction SilentlyContinue
}
