[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$SourceDirectory,
    [string]$InstallDirectory,
    [string]$DataDirectory,
    [switch]$SkipStart
)

$ErrorActionPreference = 'Stop'
if (-not $SourceDirectory) { $SourceDirectory = Join-Path $PSScriptRoot '..\..\..\target\release' }
if (-not $InstallDirectory) { $InstallDirectory = Join-Path $env:ProgramFiles 'MiniRemoteDesktop' }
if (-not $DataDirectory) { $DataDirectory = Join-Path $env:ProgramData 'MiniRemoteDesktop' }
$serviceName = 'MiniRemoteDesktop'
$serviceSid = 'S-1-5-80-1879472017-33930626-126605267-2295067401-1052995421'
$serviceExe = Join-Path $InstallDirectory 'mrd-service.exe'
$agentExe = Join-Path $InstallDirectory 'mrd-session-agent.exe'
$sourceService = Join-Path $SourceDirectory 'mrd-service.exe'
$sourceAgent = Join-Path $SourceDirectory 'mrd-session-agent.exe'

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Installing MiniRemoteDesktop requires an elevated PowerShell session.'
    }
}

function Invoke-Sc {
    param([Parameter(Mandatory)][string[]]$Arguments)
    & "$env:SystemRoot\System32\sc.exe" @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "sc.exe failed ($LASTEXITCODE): $($Arguments -join ' ')"
    }
}

function Set-ProtectedDataAcl {
    param([Parameter(Mandatory)][string]$Path)
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
    $inheritance = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $propagation = [Security.AccessControl.PropagationFlags]::None
    $allow = [Security.AccessControl.AccessControlType]::Allow
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new('S-1-5-18'),
        [Security.AccessControl.FileSystemRights]::FullControl,
        $inheritance, $propagation, $allow))
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'),
        [Security.AccessControl.FileSystemRights]::FullControl,
        $inheritance, $propagation, $allow))
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new($serviceSid),
        [Security.AccessControl.FileSystemRights]0x1301bf,
        $inheritance, $propagation, $allow))
    Set-Acl -LiteralPath $Path -AclObject $acl
}

if (-not $WhatIfPreference) {
    Assert-Administrator
    foreach ($source in @($sourceService, $sourceAgent)) {
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Required release binary was not found: $source"
        }
    }
}

if ($PSCmdlet.ShouldProcess($InstallDirectory, 'Create protected installation directory and copy service binaries')) {
    New-Item -ItemType Directory -Path $InstallDirectory -Force | Out-Null
    Copy-Item -LiteralPath $sourceService -Destination $serviceExe -Force
    Copy-Item -LiteralPath $sourceAgent -Destination $agentExe -Force
}

if ($PSCmdlet.ShouldProcess($DataDirectory, 'Create machine data directory with SYSTEM, Administrators, and per-service SID ACL')) {
    New-Item -ItemType Directory -Path $DataDirectory -Force | Out-Null
    Set-ProtectedDataAcl -Path $DataDirectory
}

$existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
$binaryPath = '"{0}" --service' -f $serviceExe
if ($existing) {
    if ($PSCmdlet.ShouldProcess($serviceName, 'Update Windows machine service configuration')) {
        if ($existing.Status -ne 'Stopped') {
            Stop-Service -Name $serviceName -Force
            $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
        Invoke-Sc @('config', $serviceName, "binPath= $binaryPath", 'start= auto', 'obj= LocalSystem', 'DisplayName= Mini Remote Desktop Service')
    }
} elseif ($PSCmdlet.ShouldProcess($serviceName, 'Create non-interactive LocalSystem Windows machine service')) {
    Invoke-Sc @('create', $serviceName, "binPath= $binaryPath", 'type= own', 'start= auto', 'obj= LocalSystem', 'DisplayName= Mini Remote Desktop Service')
}

if ($PSCmdlet.ShouldProcess($serviceName, 'Configure service SID, recovery, preshutdown, and description')) {
    Invoke-Sc @('sidtype', $serviceName, 'unrestricted')
    Invoke-Sc @('failure', $serviceName, 'reset= 86400', 'actions= restart/5000/restart/15000/none/0')
    Invoke-Sc @('failureflag', $serviceName, '1')
    Invoke-Sc @('preshutdown', $serviceName, '30000')
    Invoke-Sc @('description', $serviceName, 'Mini Remote Desktop machine service and interactive Session Agent supervisor')
}

if ($PSCmdlet.ShouldProcess('Windows Event Log', "Register event source $serviceName")) {
    if (-not [Diagnostics.EventLog]::SourceExists($serviceName)) {
        New-EventLog -LogName Application -Source $serviceName
    }
}

if (-not $SkipStart -and $PSCmdlet.ShouldProcess($serviceName, 'Start Windows machine service')) {
    Start-Service -Name $serviceName
    (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
}

Write-Host "MiniRemoteDesktop service installation configured at $InstallDirectory"
