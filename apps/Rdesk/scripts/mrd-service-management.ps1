function ConvertTo-MrdNativeArgument {
    param([AllowEmptyString()][string]$Value)
    # Windows argv parsing doubles backslashes before a quote and before the closing quote.
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}

function Invoke-MrdNativeCommand {
    param([Parameter(Mandatory)][string]$FilePath, [string[]]$Arguments)
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = (@($Arguments | ForEach-Object { ConvertTo-MrdNativeArgument -Value $_ }) -join ' ')
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "Cannot start native command: $FilePath" }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $stdout.GetAwaiter().GetResult()
            Stderr = $stderr.GetAwaiter().GetResult()
        }
    } finally { $process.Dispose() }
}

function Initialize-MrdServiceConfiguration {
    if ('MrdInstaller.ServiceConfiguration' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace MrdInstaller {
    public static class ServiceConfiguration {
        [StructLayout(LayoutKind.Sequential)] private struct PreshutdownInfo { public uint Timeout; }
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern IntPtr OpenSCManager(string machine, string database, uint access);
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern IntPtr OpenService(IntPtr manager, string name, uint access);
        [DllImport("advapi32.dll", EntryPoint="ChangeServiceConfig2W", SetLastError=true)]
        private static extern bool ChangeServiceConfig2(IntPtr service, uint level, ref PreshutdownInfo info);
        [DllImport("advapi32.dll", EntryPoint="QueryServiceConfig2W", SetLastError=true)]
        private static extern bool QueryServiceConfig2(IntPtr service, uint level, IntPtr buffer, uint size, out uint required);
        [DllImport("advapi32.dll")] private static extern bool CloseServiceHandle(IntPtr handle);
        private static uint Apply(string name, uint? timeout) {
            IntPtr manager = OpenSCManager(null, null, 1);
            if (manager == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            IntPtr service = IntPtr.Zero;
            IntPtr buffer = IntPtr.Zero;
            try {
                service = OpenService(manager, name, timeout.HasValue ? 3u : 1u);
                if (service == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                if (timeout.HasValue) {
                    PreshutdownInfo info = new PreshutdownInfo { Timeout = timeout.Value };
                    if (!ChangeServiceConfig2(service, 7, ref info)) throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                buffer = Marshal.AllocHGlobal(4);
                uint required;
                if (!QueryServiceConfig2(service, 7, buffer, 4, out required)) throw new Win32Exception(Marshal.GetLastWin32Error());
                return unchecked((uint)Marshal.ReadInt32(buffer));
            } finally {
                if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer);
                if (service != IntPtr.Zero) CloseServiceHandle(service);
                CloseServiceHandle(manager);
            }
        }
        public static uint GetPreshutdownTimeout(string name) { return Apply(name, null); }
        public static void SetPreshutdownTimeout(string name, uint timeout) {
            if (Apply(name, timeout) != timeout) throw new InvalidOperationException("Service preshutdown timeout readback mismatch");
        }
    }
}
'@
}

function Get-MrdServicePreshutdownTimeout {
    param([Parameter(Mandatory)][string]$ServiceName)
    Initialize-MrdServiceConfiguration
    return [MrdInstaller.ServiceConfiguration]::GetPreshutdownTimeout($ServiceName)
}

function Set-MrdServicePreshutdownTimeout {
    param([Parameter(Mandatory)][string]$ServiceName, [uint32]$Milliseconds = 30000)
    Initialize-MrdServiceConfiguration
    [MrdInstaller.ServiceConfiguration]::SetPreshutdownTimeout($ServiceName, $Milliseconds)
}

function Add-MrdInteractiveServiceStartPermission {
    param([Parameter(Mandatory)][string]$Sddl)
    $descriptor = [Security.AccessControl.RawSecurityDescriptor]::new($Sddl)
    $acl = $descriptor.DiscretionaryAcl
    if ($null -eq $acl) { return $Sddl }

    $interactiveSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-4')
    $required = 0x15 # SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_START
    $granted = 0
    $insertAt = $acl.Count
    for ($index = 0; $index -lt $acl.Count; $index++) {
        $ace = $acl[$index]
        if (([int]$ace.AceFlags -band [int][Security.AccessControl.AceFlags]::Inherited) -ne 0) {
            $insertAt = [Math]::Min($insertAt, $index)
            continue
        }
        if ($ace -is [Security.AccessControl.CommonAce] -and -not $ace.IsCallback -and
            $ace.AceQualifier -eq [Security.AccessControl.AceQualifier]::AccessAllowed -and
            $ace.SecurityIdentifier.Equals($interactiveSid)) {
            $granted = $granted -bor $ace.AccessMask
        }
    }
    $missing = $required -band (-bnot $granted)
    if ($missing -eq 0) { return $Sddl }

    $newAce = [Security.AccessControl.CommonAce]::new(
        [Security.AccessControl.AceFlags]::None,
        [Security.AccessControl.AceQualifier]::AccessAllowed,
        $missing,
        $interactiveSid,
        $false,
        $null)
    $acl.InsertAce($insertAt, $newAce)
    return $descriptor.GetSddlForm([Security.AccessControl.AccessControlSections]::All)
}

function Invoke-MrdServiceUpgrade {
    param(
        [Parameter(Mandatory)][scriptblock]$Stop,
        [Parameter(Mandatory)][scriptblock]$Install,
        [Parameter(Mandatory)][scriptblock]$Configure,
        [Parameter(Mandatory)][scriptblock]$Start,
        [Parameter(Mandatory)][scriptblock]$Rollback
    )
    try {
        & $Stop
        & $Install
        & $Configure
        & $Start
    } catch {
        $upgradeError = $_
        try { & $Rollback }
        catch {
            throw "Service upgrade failed: $($upgradeError.Exception.Message). Rollback failed: $($_.Exception.Message)"
        }
        throw $upgradeError
    }
}
