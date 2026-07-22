param(
    [int]$ProbeSecs = 15,
    [double]$MinProbeFps = 110.0,
    [double]$MinAgentSendFps = 115.0,
    [double]$MinAgentUniqueFps = 110.0,
    [int]$Runs = 3,
    [int]$RequiredPasses = 1,
    [string]$AgentBinDir = "",
    [switch]$VerboseProbe
)

$ErrorActionPreference = "Stop"

function Get-ProbeStats([string]$Path) {
    $out = [ordered]@{
        fps = -1.0
        frames = 0
        packets = 0
    }
    if (-not (Test-Path $Path)) { return $out }
    $line = (Get-Content $Path | Select-String "media_stats:" | Select-Object -Last 1).Line
    if (-not $line) { return $out }
    $mFps = [regex]::Match($line, "estimated_fps=([0-9]+(?:\.[0-9]+)?)")
    if ($mFps.Success) { $out.fps = [double]$mFps.Groups[1].Value }
    $mFrames = [regex]::Match($line, "frames=([0-9]+)")
    if ($mFrames.Success) { $out.frames = [int]$mFrames.Groups[1].Value }
    $mPackets = [regex]::Match($line, "packets=([0-9]+)")
    if ($mPackets.Success) { $out.packets = [int]$mPackets.Groups[1].Value }
    return $out
}

function Get-AgentStats([string]$Path) {
    $out = [ordered]@{
        max_send_fps = -1.0
        max_unique_fps = -1.0
        nvenc_attached = $false
        nvenc_fallback = $false
        pc_connected = $false
        ice_connected = $false
    }
    if (-not (Test-Path $Path)) { return $out }

    foreach ($line in Get-Content $Path) {
        $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
        if ($plain -match "native NVENC pipeline attached") { $out.nvenc_attached = $true }
        if ($plain -match "native NVENC init failed, using fallback") { $out.nvenc_fallback = $true }
        if ($plain -match "peer connection state changed .*state=connected") { $out.pc_connected = $true }
        if ($plain -match "ice connection state changed .*state=connected") { $out.ice_connected = $true }
        if ($plain -notmatch "\[RTCP-PANEL\]") { continue }

        $mSend = [regex]::Match($plain, "send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)")
        if ($mSend.Success) {
            $v = [double]$mSend.Groups[$mSend.Groups.Count - 1].Value
            if ($v -gt $out.max_send_fps) { $out.max_send_fps = $v }
        }
        $mUnique = [regex]::Match($plain, "unique_send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)")
        if ($mUnique.Success) {
            $v = [double]$mUnique.Groups[$mUnique.Groups.Count - 1].Value
            if ($v -gt $out.max_unique_fps) { $out.max_unique_fps = $v }
        }
    }
    return $out
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..")).Path
$agentDir = Join-Path $repoRoot "agent-rust"
$signalingDir = Join-Path $repoRoot "signaling-rs"
$binDir = if ([string]::IsNullOrWhiteSpace($AgentBinDir)) {
    Join-Path $agentDir "target\debug"
} else {
    $AgentBinDir
}

$signalingExe = Join-Path $signalingDir "target-codex-hotfix\debug\signaling-rs.exe"
$agentExe = Join-Path $binDir "agent-rust.exe"
$probeExe = Join-Path $binDir "m2_offer_probe.exe"
$ffmpegExe = Join-Path $repoRoot "tools\ffmpeg_full_build\\bin\\ffmpeg.exe"
$cfgPath = Join-Path $agentDir "config.json"
$cfgBak = Join-Path $agentDir ("config.accept_1080p120.{0}.bak.json" -f (Get-Date -Format "yyyyMMdd_HHmmss"))

if (-not (Test-Path $signalingExe)) { throw "missing signaling exe: $signalingExe" }
if (-not (Test-Path $agentExe)) { throw "missing agent exe: $agentExe" }
if (-not (Test-Path $probeExe)) { throw "missing probe exe: $probeExe" }
if (-not (Test-Path $ffmpegExe)) { throw "missing ffmpeg: $ffmpegExe" }
if (-not (Test-Path $cfgPath)) { throw "missing config: $cfgPath" }

if ($RequiredPasses -lt 1) { $RequiredPasses = 1 }
if ($Runs -lt $RequiredPasses) { $Runs = $RequiredPasses }

$tag = "accept_1080p120"
Copy-Item $cfgPath $cfgBak -Force

$passCount = 0
$bestProbeFps = -1.0
$bestSendFps = -1.0
$bestUniqueFps = -1.0

try {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $cfg.capture.fps = 120
    $cfg.capture.min_fps = 120
    $cfg.capture.max_fps = 120
    $cfg.capture.encoder = "auto"
    $cfg.capture.target_width = 1920
    $cfg.capture.target_height = 1080
    $cfg.capture.max_fps_mode = $true
    $cfg.capture.idle_repeat_fps = 120
    ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii

    for ($i = 1; $i -le $Runs; $i++) {
        $runTag = "$tag.run$i"
        $signalingLog = Join-Path $repoRoot "$runTag.signaling.log"
        $signalingErr = Join-Path $repoRoot "$runTag.signaling.err.log"
        $agentLog = Join-Path $repoRoot "$runTag.agent.log"
        $agentErr = Join-Path $repoRoot "$runTag.agent.err.log"
        $probeLog = Join-Path $repoRoot "$runTag.probe.log"
        $probeErr = Join-Path $repoRoot "$runTag.probe.err.log"

        @($signalingLog, $signalingErr, $agentLog, $agentErr, $probeLog, $probeErr) | ForEach-Object {
            if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
        }
        Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "m2_offer_probe") } | Stop-Process -Force -ErrorAction SilentlyContinue

        $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $signalingLog -RedirectStandardError $signalingErr
        Start-Sleep -Milliseconds 700
        $agentCmd = "set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`""
        $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $agentLog -RedirectStandardError $agentErr
        Start-Sleep -Seconds 2

        $probeArgText = ""
        if ($VerboseProbe) { $probeArgText = " --verbose" }
        $probeCmd = "set PROBE_SECS=$ProbeSecs&& `"$probeExe`"$probeArgText"
        $pp = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $probeCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $probeLog -RedirectStandardError $probeErr

        try {
            $pp | Wait-Process -Timeout ($ProbeSecs + 90)
        } finally {
            if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
            if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }
        }

        $probe = Get-ProbeStats $probeLog
        $agent = Get-AgentStats $agentLog

        if ($probe.fps -gt $bestProbeFps) { $bestProbeFps = $probe.fps }
        if ($agent.max_send_fps -gt $bestSendFps) { $bestSendFps = $agent.max_send_fps }
        if ($agent.max_unique_fps -gt $bestUniqueFps) { $bestUniqueFps = $agent.max_unique_fps }

        $connected = $agent.pc_connected -and $agent.ice_connected
        $nvencOk = $agent.nvenc_attached -and (-not $agent.nvenc_fallback)
        $fpsOk = ($probe.fps -ge $MinProbeFps) -and ($agent.max_send_fps -ge $MinAgentSendFps) -and ($agent.max_unique_fps -ge $MinAgentUniqueFps)
        $runPass = $connected -and $nvencOk -and $fpsOk -and ($probe.frames -gt 0)
        if ($runPass) { $passCount++ }

        Write-Output ("run={0} probe_fps={1:N2} send_fps={2:N2} unique_fps={3:N2} frames={4} nvenc_attached={5} nvenc_fallback={6} connected={7} pass={8}" -f `
            $i, $probe.fps, $agent.max_send_fps, $agent.max_unique_fps, $probe.frames, $agent.nvenc_attached, $agent.nvenc_fallback, $connected, $runPass)

        if ($passCount -ge $RequiredPasses) {
            Write-Output "1080p120 acceptance passed"
            exit 0
        }
    }

    Write-Output ("best_probe_fps={0:N2}" -f $bestProbeFps)
    Write-Output ("best_agent_send_fps={0:N2}" -f $bestSendFps)
    Write-Output ("best_agent_unique_fps={0:N2}" -f $bestUniqueFps)
    Write-Error ("1080p120 acceptance failed: pass_count={0}/{1}" -f $passCount, $RequiredPasses)
    exit 1
}
finally {
    if (Test-Path $cfgBak) {
        Copy-Item $cfgBak $cfgPath -Force
        Remove-Item $cfgBak -Force -ErrorAction SilentlyContinue
    }
}


