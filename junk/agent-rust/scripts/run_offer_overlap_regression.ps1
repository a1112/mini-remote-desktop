param(
    [int]$Runs = 1,
    [int]$DelayMs = 1200,
    [int]$ProbeSecs = 15,
    [switch]$VerboseProbe
)

$ErrorActionPreference = "Stop"

function Get-FramesFromLog([string]$Path) {
    if (-not (Test-Path $Path)) { return -1 }
    $line = (Get-Content $Path | Select-String "media_stats:" | Select-Object -Last 1).Line
    if (-not $line) { return -1 }
    $m = [regex]::Match($line, "frames=(\d+)")
    if (-not $m.Success) { return -1 }
    return [int]$m.Groups[1].Value
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..")).Path
$agentDir = Join-Path $repoRoot "agent-rust"
$signalingDir = Join-Path $repoRoot "signaling-rs"

$signalingExe = Join-Path $signalingDir "target-codex-hotfix\debug\signaling-rs.exe"
$agentExe = Join-Path $agentDir "target\debug\agent-rust.exe"
$probeExe = Join-Path $agentDir "target\debug\m2_offer_probe.exe"

if (-not (Test-Path $signalingExe)) { throw "missing signaling exe: $signalingExe" }
if (-not (Test-Path $agentExe)) { throw "missing agent exe: $agentExe" }
if (-not (Test-Path $probeExe)) { throw "missing probe exe: $probeExe" }

$failCount = 0

for ($i = 1; $i -le $Runs; $i++) {
    $tag = "overlap_r${i}"
    $signalingLog = Join-Path $repoRoot "$tag.signaling.log"
    $signalingErr = Join-Path $repoRoot "$tag.signaling.err.log"
    $agentLog = Join-Path $repoRoot "$tag.agent.log"
    $agentErr = Join-Path $repoRoot "$tag.agent.err.log"
    $p1Log = Join-Path $repoRoot "$tag.p1.log"
    $p1Err = Join-Path $repoRoot "$tag.p1.err.log"
    $p2Log = Join-Path $repoRoot "$tag.p2.log"
    $p2Err = Join-Path $repoRoot "$tag.p2.err.log"

    Get-Process | Where-Object { $_.ProcessName -in @("signaling-rs", "agent-rust", "m2_offer_probe") } | Stop-Process -Force -ErrorAction SilentlyContinue
    @($signalingLog, $signalingErr, $agentLog, $agentErr, $p1Log, $p1Err, $p2Log, $p2Err) | ForEach-Object {
        if (Test-Path $_) { Remove-Item $_ -Force -ErrorAction SilentlyContinue }
    }

    $sp = Start-Process -FilePath $signalingExe -WorkingDirectory $signalingDir -PassThru -RedirectStandardOutput $signalingLog -RedirectStandardError $signalingErr
    Start-Sleep -Milliseconds 700
    $ap = Start-Process -FilePath $agentExe -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $agentLog -RedirectStandardError $agentErr
    Start-Sleep -Seconds 2

    $probeArgText = ""
    if ($VerboseProbe) { $probeArgText = " --verbose" }
    $probeCmd1 = "set PROBE_SECS=$ProbeSecs&& `"$probeExe`"$probeArgText"
    $p1 = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $probeCmd1 -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $p1Log -RedirectStandardError $p1Err
    Start-Sleep -Milliseconds $DelayMs
    $probeCmd2 = "set PROBE_SECS=$ProbeSecs&& `"$probeExe`"$probeArgText"
    $p2 = Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $probeCmd2 -WorkingDirectory $agentDir -PassThru -RedirectStandardOutput $p2Log -RedirectStandardError $p2Err

    try {
        $p1 | Wait-Process -Timeout ($ProbeSecs + 90)
        $p2 | Wait-Process -Timeout ($ProbeSecs + 90)
    } finally {
        if (Get-Process -Id $ap.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ap.Id -Force }
        if (Get-Process -Id $sp.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $sp.Id -Force }
    }

    $p1Frames = Get-FramesFromLog $p1Log
    $p2Frames = Get-FramesFromLog $p2Log
    $ok = ($p1Frames -eq 0) -and ($p2Frames -gt 0)

    Write-Output ("run={0} p1_frames={1} p2_frames={2} pass={3}" -f $i, $p1Frames, $p2Frames, $ok)
    if (-not $ok) { $failCount++ }
}

if ($failCount -gt 0) {
    Write-Error ("overlap regression failed: {0}/{1} runs" -f $failCount, $Runs)
    exit 1
}

Write-Output ("overlap regression passed: {0}/{0} runs" -f $Runs)
exit 0
