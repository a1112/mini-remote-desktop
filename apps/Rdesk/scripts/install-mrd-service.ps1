[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$SourceDirectory,
    [string]$InstallDirectory,
    [string]$DataDirectory,
    [switch]$SkipStart
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'mrd-service-management.ps1')
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

function Get-ServiceSddl {
    $lines = @(Invoke-Sc @('sdshow', $serviceName))
    $descriptors = @($lines | ForEach-Object { $_.ToString().Trim() } | Where-Object { $_ -match '^(O:|G:|D:|S:)' })
    if ($descriptors.Count -ne 1) { throw "Could not read the security descriptor of $serviceName." }
    return $descriptors[0]
}

if (-not $WhatIfPreference) {
    Assert-Administrator
    foreach ($source in @($sourceService, $sourceAgent)) {
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Required release binary was not found: $source"
        }
    }
}

if ($PSCmdlet.ShouldProcess($serviceName, "Install or update background service in $InstallDirectory; back up existing binaries, preserve startup configuration, and grant interactive start access")) {
    $existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    $wasRunning = $existing -and $existing.Status -eq 'Running'
    $existingConfig = if ($existing) {
        Get-CimInstance -ClassName Win32_Service -Filter "Name='$serviceName'"
    } else { $null }
    if ($existing -and -not $existingConfig) { throw 'Cannot snapshot the installed service configuration; upgrade has not started.' }
    $upgradeState = @{
        CreatedService = $false
        OldSddl = $null
        Backups = @()
    }
    $binaryPath = '"{0}" --service' -f $serviceExe
    $backupDirectory = Join-Path $InstallDirectory ('service-backup-{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'), [Guid]::NewGuid().ToString('N').Substring(0, 8))

    Invoke-MrdServiceUpgrade -Stop {
        New-Item -ItemType Directory -Path $InstallDirectory -Force | Out-Null
        if ($existing) {
            $upgradeState.OldSddl = Get-ServiceSddl
            New-Item -ItemType Directory -Path $backupDirectory -Force | Out-Null
            $existingConfig | Select-Object Name, PathName, StartMode | ConvertTo-Json |
                Set-Content -LiteralPath (Join-Path $backupDirectory 'service-config.json') -Encoding UTF8
            $upgradeState.OldSddl | Set-Content -LiteralPath (Join-Path $backupDirectory 'service-sddl.txt') -Encoding ASCII
        }
        foreach ($installedBinary in @($serviceExe, $agentExe)) {
            if (Test-Path -LiteralPath $installedBinary -PathType Leaf) {
                New-Item -ItemType Directory -Path $backupDirectory -Force | Out-Null
                $backupBinary = Join-Path $backupDirectory (Split-Path -Leaf $installedBinary)
                Copy-Item -LiteralPath $installedBinary -Destination $backupBinary -Force
                $upgradeState.Backups += @{ Installed = $installedBinary; Backup = $backupBinary }
            }
        }
        if ($existing -and $existing.Status -ne 'Stopped') {
            Stop-Service -Name $serviceName -Force
            $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
    } -Install {
        # Executables are replaced only after the old process releases them.
        Copy-Item -LiteralPath $sourceService -Destination $serviceExe -Force
        Copy-Item -LiteralPath $sourceAgent -Destination $agentExe -Force
        New-Item -ItemType Directory -Path $DataDirectory -Force | Out-Null
        Set-ProtectedDataAcl -Path $DataDirectory
    } -Configure {
        if ($existing) {
            # Updating must preserve the user's autostart setting and service account.
            Invoke-Sc @('config', $serviceName, "binPath= $binaryPath", 'DisplayName= Mini Remote Desktop Service')
        } else {
            Invoke-Sc @('create', $serviceName, "binPath= $binaryPath", 'type= own', 'start= auto', 'obj= LocalSystem', 'DisplayName= Mini Remote Desktop Service')
            $upgradeState.CreatedService = $true
        }
        Invoke-Sc @('sidtype', $serviceName, 'unrestricted')
        Invoke-Sc @('failure', $serviceName, 'reset= 86400', 'actions= restart/5000/restart/15000/none/0')
        Invoke-Sc @('failureflag', $serviceName, '1')
        Invoke-Sc @('preshutdown', $serviceName, '30000')
        Invoke-Sc @('description', $serviceName, 'Mini Remote Desktop machine service and interactive Session Agent supervisor')
        $oldSddl = Get-ServiceSddl
        $newSddl = Add-MrdInteractiveServiceStartPermission -Sddl $oldSddl
        if ($newSddl -ne $oldSddl) { Invoke-Sc @('sdset', $serviceName, $newSddl) }
        if (-not [Diagnostics.EventLog]::SourceExists($serviceName)) {
            New-EventLog -LogName Application -Source $serviceName
        }
    } -Start {
        if (-not $SkipStart) {
            Start-Service -Name $serviceName
            (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
        }
    } -Rollback {
        $current = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
        if ($current -and $current.Status -ne 'Stopped') {
            Stop-Service -Name $serviceName -Force
            $current.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
        foreach ($backup in $upgradeState.Backups) {
            Copy-Item -LiteralPath $backup.Backup -Destination $backup.Installed -Force
        }
        if ($existing -and $existingConfig) {
            $oldStartType = switch ($existingConfig.StartMode) { 'Auto' { 'auto' }; 'Manual' { 'demand' }; 'Disabled' { 'disabled' }; default { throw "Unknown original startup mode: $($existingConfig.StartMode)" } }
            Invoke-Sc @('config', $serviceName, "binPath= $($existingConfig.PathName)", "start= $oldStartType")
            if ($upgradeState.OldSddl) { Invoke-Sc @('sdset', $serviceName, $upgradeState.OldSddl) }
            if ($wasRunning) {
                Start-Service -Name $serviceName
                (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
            }
        } elseif ($upgradeState.CreatedService) {
            Invoke-Sc @('delete', $serviceName)
        }
        Write-Warning "Service upgrade failed; previous binaries and startup configuration restored. Backup: $backupDirectory"
    }
    Write-Host "MiniRemoteDesktop service installation configured at $InstallDirectory"
    if (Test-Path -LiteralPath $backupDirectory) { Write-Host "Previous installation backup: $backupDirectory" }
}
