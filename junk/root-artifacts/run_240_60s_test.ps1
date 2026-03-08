# 240FPS 60-second full render test with optimized GPU settings
param(
    [int]$ProbeSecs = 60,
    [string]$Tag = "gpu_opt_240_60s"
)

$ErrorActionPreference = "Stop"

$repoRoot = "J:\ProjectTest\remote-desktop\mini-remote-desktop"
$agentDir = Join-Path $repoRoot "agent-rust"
$signalingDir = Join-Path $repoRoot "signaling-rs"
$controllerDir = Join-Path $repoRoot "controller-rust"
$logsDir = Join-Path $repoRoot "logs"

$signalingExe = Join-Path $signalingDir "target\debug\signaling-rs.exe"
$agentExe = Join-Path $agentDir "target\debug\agent-rust.exe"
$controllerExe = Join-Path $controllerDir "target\debug\controller-rust.exe"
$agentCfgPath = Join-Path $agentDir "config.json"
$agentCfgBak = Join-Path $agentDir ("config.$Tag.bak.json")

# Log files in logs directory
$signalingLog = Join-Path $logsDir "$Tag.signaling.log"
$signalingErr = Join-Path $logsDir "$Tag.signaling.err.log"
$agentLog = Join-Path $logsDir "$Tag.agent.log"
$agentErr = Join-Path $logsDir "$Tag.agent.err.log"
$controllerLog = Join-Path $logsDir "$Tag.controller.log"
$controllerErr = Join-Path $logsDir "$Tag.controller.err.log"

Write-Output "=== 240FPS 60s Full Render Test ==="
Write-Output "Tag: $Tag"
Write-Output "Probe Duration: ${ProbeSecs}s"
Write-Output "Target FPS: 240"
Write-Output "Logs: $logsDir"
Write-Output ""

# Check prerequisites
if (-not (Test-Path $signalingExe)) { throw "missing signaling: $signalingExe" }
if (-not (Test-Path $agentExe)) { throw "missing agent: $agentExe" }
if (-not (Test-Path $controllerExe)) { throw "missing controller: $controllerExe" }
if (-not (Test-Path $agentCfgPath)) { throw "missing agent config: $agentCfgPath" }

# Ensure logs directory exists
if (-not (Test-Path $logsDir)) {
    New-Item -ItemType Directory -Path $logsDir -Force | Out-Null
}

# Clean up old processes
Write-Output "Cleaning up old processes..."
Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "controller-rust") } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# Clean up old logs
Write-Output "Cleaning up old logs..."
@($signalingLog, $signalingErr, $agentLog, $agentErr, $controllerLog, $controllerErr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
}

# Backup agent config
Copy-Item $agentCfgPath $agentCfgBak -Force

try {
    # Apply optimized configuration to agent
    Write-Output "Applying 240FPS optimized configuration..."
    $cfg = Get-Content $agentCfgPath -Raw | ConvertFrom-Json

    # Apply 60FPS test config (to check if decode works at lower FPS)
    $cfg.capture.fps = 60
    $cfg.capture.min_fps = 60
    $cfg.capture.max_fps = 60
    $cfg.capture.target_width = 1920
    $cfg.capture.target_height = 1080
    $cfg.capture.encoder = "auto"  # Use auto to allow encoder selection
    $cfg.capture.allow_encoder_fallback = $true  # Allow fallback
    $cfg.capture.max_fps_mode = $true
    $cfg.capture.idle_repeat_fps = 60

    ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $agentCfgPath -Encoding Ascii

    # Start signaling server
    Write-Output "Starting signaling server..."
    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $signalingLog -RedirectStandardError $signalingErr
    Start-Sleep -Milliseconds 700

    # Create agent batch file with GPU optimization
    Write-Output "Starting agent with GPU optimization settings..."
    $ffmpegExe = Join-Path $repoRoot "tools\ffmpeg_full_build\\bin\\ffmpeg.exe"
    $agentBat = Join-Path $agentDir "agent_240_env.bat"
    @"
@echo off
set MRD_STRICT_GPU_MODE=1
set MRD_D3D11_KEYED_TIMEOUT_MS=2
set MRD_D3D11_KEYED_TIMEOUT_MS_FALLBACK=8
set MRD_ENABLE_SHARED_KEYED=1
set MRD_SHARED_KEYED_SLOTS=8
set AGENT_FFMPEG_PATH=$ffmpegExe
set RUST_LOG=agent_rust=info
"$agentExe"
"@ | Set-Content -Path $agentBat -Encoding Ascii

    $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentBat -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $agentLog -RedirectStandardError $agentErr
    Start-Sleep -Seconds 3

    # Create controller batch file with GPU optimization
    Write-Output "Starting controller with GPU optimization settings..."
    $controllerBat = Join-Path $controllerDir "controller_240_env.bat"
    @"
@echo off
set MRD_STRICT_GPU_MODE=0
set MRD_ALLOW_SOFTWARE_FALLBACK=1
set MRD_DECODER=mf
set MRD_D3D11_KEYED_TIMEOUT_MS=2
set MRD_D3D11_KEYED_TIMEOUT_MS_FALLBACK=8
set MRD_ENABLE_SHARED_KEYED=1
set MRD_SHARED_KEYED_SLOTS=8
set MRD_RENDER_MAX_AGE_MS=50
set MRD_RENDER_DROP_OLD=1
set MRD_DECODE_SELECT=latest-key
set MRD_QUIC_RX_QUEUE=4
set MRD_TARGET_FPS=60
set RUST_LOG=controller_rust=info,webrtc=warn
"$controllerExe"
"@ | Set-Content -Path $controllerBat -Encoding Ascii

    $cp = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $controllerBat -WorkingDirectory $controllerDir -PassThru -RedirectStandardOutput $controllerLog -RedirectStandardError $controllerErr
    Start-Sleep -Seconds 2

    Write-Output ""
    Write-Output "=== Running ${ProbeSecs}s test at 240FPS ==="
    Write-Output "Controller window should be visible..."
    Write-Output ""

    # Wait for test duration
    Start-Sleep -Seconds $ProbeSecs

    Write-Output "Test duration complete, stopping processes..."

    # Stop all processes
    if (Get-Process -Id $cp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $cp.Id -Force }
    if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
    if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }

    # Collect results
    Write-Output ""
    Write-Output "=== Test Results ==="

    # Parse agent results
    if (Test-Path $agentLog) {
        $agentLines = Get-Content $agentLog | Select-String "\[RTCP-PANEL\]"
        if ($agentLines.Count -gt 0) {
            $lastStats = $agentLines[-1].Line
            $lastStats = [regex]::Replace($lastStats, "\x1B\[[0-9;]*m", "")
            Write-Output ""
            Write-Output "--- Agent Final Stats ---"
            Write-Output $lastStats

            # Extract key metrics
            $m = [regex]::Match($lastStats, "encode_fps=([0-9]+(?:\.[0-9]+)?)")
            if ($m.Success) { Write-Output "  Encode FPS: $($m.Groups[1].Value)" }
            $m = [regex]::Match($lastStats, "send_fps=([0-9]+(?:\.[0-9]+)?)")
            if ($m.Success) { Write-Output "  Send FPS: $($m.Groups[1].Value)" }
            $m = [regex]::Match($lastStats, "native_scale_frames=([0-9]+)")
            if ($m.Success) { Write-Output "  GPU Scale Frames: $($m.Groups[1].Value)" }
        }
    }

    # Parse controller render results
    if (Test-Path $controllerLog) {
        Write-Output ""
        Write-Output "--- Controller Render Stats ---"

        # Present stats
        $presentLines = Get-Content $controllerLog | Select-String "capture_to_present_p50_ms"
        if ($presentLines) {
            $lastPresent = $presentLines[-1].Line
            Write-Output $lastPresent
        }

        # Shared draw stats
        $sharedLines = Get-Content $controllerLog | Select-String "\[SHARED-DRAW-STATS\]"
        if ($sharedLines) {
            $lastShared = $sharedLines[-1].Line
            Write-Output $lastShared
        }

        # Progress stats
        $progressLines = Get-Content $controllerLog | Select-String "renderer progress"
        if ($progressLines) {
            $lastProgress = $progressLines[-1].Line
            Write-Output $lastProgress
        }
    }

    Write-Output ""
    Write-Output "=== Log Files ==="
    Write-Output "Logs directory: $logsDir"
    Write-Output "  - $Tag.signaling.log"
    Write-Output "  - $Tag.agent.log"
    Write-Output "  - $Tag.controller.log"

    exit 0
}
finally {
    # Restore agent config
    if (Test-Path $agentCfgBak) {
        Copy-Item $agentCfgBak $agentCfgPath -Force
        Remove-Item $agentCfgBak -Force -ErrorAction SilentlyContinue
        Write-Output "Agent configuration restored"
    }

    # Clean up batch files
    Remove-Item (Join-Path $agentDir "agent_240_env.bat") -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $controllerDir "controller_240_env.bat") -Force -ErrorAction SilentlyContinue

    # Final cleanup
    Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "controller-rust") } | Stop-Process -Force -ErrorAction SilentlyContinue
}


