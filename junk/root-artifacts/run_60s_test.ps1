# 60-second test script with optimized GPU settings
param(
    [int]$ProbeSecs = 60,
    [string]$Tag = "gpu_opt_60s"
)

$ErrorActionPreference = "Stop"

$repoRoot = "J:\ProjectTest\remote-desktop\mini-remote-desktop"
$agentDir = Join-Path $repoRoot "agent-rust"
$signalingDir = Join-Path $repoRoot "signaling-rs"

$signalingExe = Join-Path $signalingDir "target\debug\signaling-rs.exe"
$agentExe = Join-Path $agentDir "target\debug\agent-rust.exe"
$probeExe = Join-Path $agentDir "target\debug\m2_offer_probe.exe"
$cfgPath = Join-Path $agentDir "config.json"
$cfgBak = Join-Path $agentDir ("config.$Tag.bak.json")

# Log files
$signalingLog = Join-Path $repoRoot "$Tag.signaling.log"
$signalingErr = Join-Path $repoRoot "$Tag.signaling.err.log"
$agentLog = Join-Path $repoRoot "$Tag.agent.log"
$agentErr = Join-Path $repoRoot "$Tag.agent.err.log"
$probeLog = Join-Path $repoRoot "$Tag.probe.log"
$probeErr = Join-Path $repoRoot "$Tag.probe.err.log"

Write-Output "=== 60s GPU Optimization Test ==="
Write-Output "Tag: $Tag"
Write-Output "Probe Duration: ${ProbeSecs}s"
Write-Output ""

# Check prerequisites
if (-not (Test-Path $signalingExe)) { throw "missing signaling: $signalingExe" }
if (-not (Test-Path $agentExe)) { throw "missing agent: $agentExe" }
if (-not (Test-Path $probeExe)) { throw "missing probe: $probeExe" }
if (-not (Test-Path $cfgPath)) { throw "missing config: $cfgPath" }

# Clean up old processes and logs
Write-Output "Cleaning up old processes..."
Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "m2_offer_probe") } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

Write-Output "Cleaning up old logs..."
@($signalingLog, $signalingErr, $agentLog, $agentErr, $probeLog, $probeErr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
}

# Backup config
Copy-Item $cfgPath $cfgBak -Force

try {
    # Apply optimized configuration
    Write-Output "Applying optimized GPU configuration..."
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json

    # Save original values
    $origFps = $cfg.capture.fps
    $origMinFps = $cfg.capture.min_fps
    $origMaxFps = $cfg.capture.max_fps
    $origWidth = $cfg.capture.target_width
    $origHeight = $cfg.capture.target_height

    # Apply test config
    $cfg.capture.fps = 120
    $cfg.capture.min_fps = 120
    $cfg.capture.max_fps = 120
    $cfg.capture.target_width = 1920
    $cfg.capture.target_height = 1080
    $cfg.capture.encoder = "auto"
    $cfg.capture.max_fps_mode = $true
    $cfg.capture.idle_repeat_fps = 120

    ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii

    # Start signaling server
    Write-Output "Starting signaling server..."
    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $signalingLog -RedirectStandardError $signalingErr
    Start-Sleep -Milliseconds 700

    # Create a batch file to set environment variables and start agent
    Write-Output "Starting agent with GPU optimization settings..."
    $agentBat = Join-Path $agentDir "agent_gpu_env.bat"
    @"
@echo off
set MRD_STRICT_GPU_MODE=1
set MRD_D3D11_KEYED_TIMEOUT_MS=2
set MRD_D3D11_KEYED_TIMEOUT_MS_FALLBACK=8
set MRD_ENABLE_SHARED_KEYED=1
set MRD_SHARED_KEYED_SLOTS=8
set RUST_LOG=agent_rust=info
"$agentExe"
"@ | Set-Content -Path $agentBat -Encoding Ascii

    $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentBat -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $agentLog -RedirectStandardError $agentErr
    Start-Sleep -Seconds 2

    # Start probe for 60 seconds
    Write-Output "Starting ${ProbeSecs}s probe test..."
    $probeBat = Join-Path $agentDir "probe_gpu_env.bat"
    @"
@echo off
set PROBE_SECS=$ProbeSecs
"$probeExe" --verbose
"@ | Set-Content -Path $probeBat -Encoding Ascii

    $p = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $probeBat -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $probeLog -RedirectStandardError $probeErr

    # Wait for probe to complete
    try {
        $p | Wait-Process -Timeout ($ProbeSecs + 90)
        $exitCode = $p.ExitCode
        Write-Output "Probe completed with exit code: $exitCode"
    } finally {
        Write-Output "Stopping all processes..."
        if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
        if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }
    }

    # Collect results
    Write-Output ""
    Write-Output "=== Test Results ==="

    # Parse probe results
    if (Test-Path $probeLog) {
        $probeLines = Get-Content $probeLog | Select-String "media_stats:"
        if ($probeLines) {
            $lastLine = $probeLines[-1].Line
            Write-Output "Probe final stats: $lastLine"

            # Extract FPS
            $m = [regex]::Match($lastLine, "estimated_fps=([0-9]+(?:\.[0-9]+)?)")
            if ($m.Success) {
                Write-Output "  Estimated FPS: $($m.Groups[1].Value)"
            }
            $m = [regex]::Match($lastLine, "frames=(\d+)")
            if ($m.Success) {
                Write-Output "  Total Frames: $($m.Groups[1].Value)"
            }
        }
    }

    # Parse agent results
    if (Test-Path $agentLog) {
        $agentLines = Get-Content $agentLog | Select-String "\[RTCP-PANEL\]"
        if ($agentLines.Count -gt 0) {
            $lastStats = $agentLines[-1].Line
            # Strip ANSI codes
            $lastStats = [regex]::Replace($lastStats, "\x1B\[[0-9;]*m", "")
            Write-Output ""
            Write-Output "Agent final stats: $lastStats"
        }
    }

    # Check GPU path usage
    if (Test-Path $agentLog) {
        $gpuPath = Select-String -Path $agentLog -Pattern "gpu_zero_copy_ratio" | Select-Object -Last 1
        if ($gpuPath) {
            $gpuLine = [regex]::Replace($gpuPath.Line, "\x1B\[[0-9;]*m", "")
            Write-Output ""
            Write-Output "GPU Path: $gpuLine"
        }
    }

    # Check decode stats
    if (Test-Path $probeLog) {
        $decodeLines = Get-Content $probeLog | Select-String "decode_"
        if ($decodeLines.Count -gt 0) {
            Write-Output ""
            Write-Output "Decode Stats:"
            $decodeLines[-5..-1] | ForEach-Object {
                $line = [regex]::Replace($_.Line, "\x1B\[[0-9;]*m", "")
                Write-Output "  $line"
            }
        }
    }

    Write-Output ""
    Write-Output "=== Log Files ==="
    Write-Output "Signaling: $signalingLog"
    Write-Output "Agent: $agentLog"
    Write-Output "Probe: $probeLog"

    exit 0
}
finally {
    # Restore original config
    if (Test-Path $cfgBak) {
        Copy-Item $cfgBak $cfgPath -Force
        Remove-Item $cfgBak -Force -ErrorAction SilentlyContinue
        Write-Output "Configuration restored"
    }

    # Clean up batch files
    Remove-Item (Join-Path $agentDir "agent_gpu_env.bat") -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $agentDir "probe_gpu_env.bat") -Force -ErrorAction SilentlyContinue

    # Final cleanup
    Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "m2_offer_probe") } | Stop-Process -Force -ErrorAction SilentlyContinue
}
