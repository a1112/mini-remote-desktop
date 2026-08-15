[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$InstallDirectory,
    [string]$DataDirectory,
    [switch]$PurgeData
)

$ErrorActionPreference = 'Stop'
if (-not $InstallDirectory) { $InstallDirectory = Join-Path $env:ProgramFiles 'MiniRemoteDesktop' }
if (-not $DataDirectory) { $DataDirectory = Join-Path $env:ProgramData 'MiniRemoteDesktop' }
$serviceName = 'MiniRemoteDesktop'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Uninstalling MiniRemoteDesktop requires an elevated PowerShell session.'
    }
}

if (-not $WhatIfPreference) {
    Assert-Administrator
}

$service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($service -and $service.Status -ne 'Stopped' -and $PSCmdlet.ShouldProcess($serviceName, 'Stop service and wait for deterministic Agent cleanup')) {
    Stop-Service -Name $serviceName
    $service.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(45))
}

if ($service -and $PSCmdlet.ShouldProcess($serviceName, 'Delete Windows service registration')) {
    & "$env:SystemRoot\System32\sc.exe" delete $serviceName
    if ($LASTEXITCODE -ne 0) {
        throw "sc.exe delete failed with exit code $LASTEXITCODE"
    }
}

if ($PSCmdlet.ShouldProcess('Windows Event Log', "Remove event source $serviceName")) {
    if ([Diagnostics.EventLog]::SourceExists($serviceName)) {
        Remove-EventLog -Source $serviceName
    }
}

if ((Test-Path -LiteralPath $InstallDirectory) -and $PSCmdlet.ShouldProcess($InstallDirectory, 'Remove installed service binaries')) {
    $resolved = [IO.Path]::GetFullPath($InstallDirectory)
    $programFiles = [IO.Path]::GetFullPath($env:ProgramFiles).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($programFiles, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove an installation directory outside Program Files: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

if ($PurgeData -and (Test-Path -LiteralPath $DataDirectory) -and $PSCmdlet.ShouldProcess($DataDirectory, 'Permanently remove trust, device, and diagnostic data')) {
    $resolved = [IO.Path]::GetFullPath($DataDirectory)
    $programData = [IO.Path]::GetFullPath($env:ProgramData).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($programData, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a data directory outside ProgramData: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
} elseif (-not $PurgeData) {
    Write-Host "Preserved machine trust and configuration data at $DataDirectory (use -PurgeData to remove)."
}

Write-Host 'MiniRemoteDesktop service uninstalled.'
