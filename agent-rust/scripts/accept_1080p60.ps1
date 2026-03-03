param(
    [int]$ProbeSecs = 15,
    [double]$MinProbeFps = 45.0,
    [double]$MinAgentSendFps = 45.0,
    [string]$AgentBinDir = "",
    [switch]$VerboseProbe
)

$ErrorActionPreference = "Stop"

function Get-ProbeFps([string]$Path) {
    if (-not (Test-Path $Path)) { return -1.0 }
    $line = (Get-Content $Path | Select-String "media_stats:" | Select-Object -Last 1).Line
    if (-not $line) { return -1.0 }
    $m = [regex]::Match($line, "estimated_fps=([0-9]+(?:\.[0-9]+)?)")
    if (-not $m.Success) { return -1.0 }
    return [double]$m.Groups[1].Value
}

function Get-AgentMaxSendFps([string]$Path) {
    if (-not (Test-Path $Path)) { return -1.0 }
    $max = -1.0
    foreach ($line in Get-Content $Path) {
        $plain = [regex]::Replace($line, "\x1B\[[0-9;]*m", "")
        if ($plain -notmatch "\[RTCP-PANEL\]") { continue }
        $m = [regex]::Match($plain, "send_fps[^0-9]*([0-9]+(?:\.[0-9]+)?)")
        if ($m.Success) {
            $v = [double]$m.Groups[$m.Groups.Count - 1].Value
            if ($v -gt $max) { $max = $v }
        }
    }
    return $max
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..")).Path
$agentDir = Join-Path $repoRoot "agent-rust"
$signalingDir = Join-Path $repoRoot "signaling-rs"

$signalingExe = Join-Path $signalingDir "target-codex-hotfix\debug\signaling-rs.exe"
$binDir = if ([string]::IsNullOrWhiteSpace($AgentBinDir)) {
    Join-Path $agentDir "target\debug"
} else {
    $AgentBinDir
}
$agentExe = Join-Path $binDir "agent-rust.exe"
$probeExe = Join-Path $binDir "m2_offer_probe.exe"
$ffmpegExe = Join-Path $repoRoot "tools\ffmpeg-min\ffmpeg.exe"
$cfgPath = Join-Path $agentDir "config.json"
$cfgBak = Join-Path $agentDir ("config.accept_1080p60.{0}.bak.json" -f (Get-Date -Format "yyyyMMdd_HHmmss"))

if (-not (Test-Path $signalingExe)) { throw "missing signaling exe: $signalingExe" }
if (-not (Test-Path $agentExe)) { throw "missing agent exe: $agentExe" }
if (-not (Test-Path $probeExe)) { throw "missing probe exe: $probeExe" }
if (-not (Test-Path $ffmpegExe)) { throw "missing ffmpeg: $ffmpegExe" }
if (-not (Test-Path $cfgPath)) { throw "missing config: $cfgPath" }

$tag = "accept_1080p60"
$signalingLog = Join-Path $repoRoot "$tag.signaling.log"
$signalingErr = Join-Path $repoRoot "$tag.signaling.err.log"
$agentLog = Join-Path $repoRoot "$tag.agent.log"
$agentErr = Join-Path $repoRoot "$tag.agent.err.log"
$probeLog = Join-Path $repoRoot "$tag.probe.log"
$probeErr = Join-Path $repoRoot "$tag.probe.err.log"

@($signalingLog, $signalingErr, $agentLog, $agentErr, $probeLog, $probeErr) | ForEach-Object {
    if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
}

Copy-Item $cfgPath $cfgBak -Force

try {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $cfg.capture.fps = 60
    $cfg.capture.min_fps = 60
    $cfg.capture.max_fps = 60
    $cfg.capture.encoder = "auto"
    $cfg.capture.target_width = 1920
    $cfg.capture.target_height = 1080
    $cfg.capture.max_fps_mode = $true
    $cfg.capture.idle_repeat_fps = 60
    ($cfg | ConvertTo-Json -Depth 100) | Set-Content -Path $cfgPath -Encoding Ascii

    Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "m2_offer_probe") } | Stop-Process -Force -ErrorAction SilentlyContinue
    @($signalingLog, $signalingErr, $agentLog, $agentErr, $probeLog, $probeErr) | ForEach-Object {
        if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
    }

    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $signalingLog -RedirectStandardError $signalingErr
    Start-Sleep -Milliseconds 700
    $agentCmd = "set AGENT_FFMPEG_PATH=$ffmpegExe&& `"$agentExe`""
    $ap = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $agentCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $agentLog -RedirectStandardError $agentErr
    Start-Sleep -Seconds 2

    $probeArgText = ""
    if ($VerboseProbe) { $probeArgText = " --verbose" }
    $probeCmd = "set PROBE_SECS=$ProbeSecs&& `"$probeExe`"$probeArgText"
    $p = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $probeCmd -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $probeLog -RedirectStandardError $probeErr

    try {
        $p | Wait-Process -Timeout ($ProbeSecs + 90)
    } finally {
        if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
        if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }
    }

    $probeFps = Get-ProbeFps $probeLog
    $agentMaxSendFps = Get-AgentMaxSendFps $agentLog

    $probePass = $probeFps -ge $MinProbeFps
    $agentPass = $agentMaxSendFps -ge $MinAgentSendFps
    $pass = $probePass -and $agentPass

    Write-Output ("probe_estimated_fps={0:N2} (min={1:N2}) pass={2}" -f $probeFps, $MinProbeFps, $probePass)
    Write-Output ("agent_max_send_fps={0:N2} (min={1:N2}) pass={2}" -f $agentMaxSendFps, $MinAgentSendFps, $agentPass)

    if (-not $pass) {
        Write-Error "1080p60 acceptance failed"
        exit 1
    }

    Write-Output "1080p60 acceptance passed"
    exit 0
}
finally {
    if (Test-Path $cfgBak) {
        Copy-Item $cfgBak $cfgPath -Force
        Remove-Item $cfgBak -Force -ErrorAction SilentlyContinue
    }
}
