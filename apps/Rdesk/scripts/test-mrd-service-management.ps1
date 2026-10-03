$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'mrd-service-management.ps1')

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -ne $Expected) { throw "$Message (expected $Expected, got $Actual)" }
}

function Get-AceSignature([Security.AccessControl.GenericAce]$Ace) {
    $bytes = [byte[]]::new($Ace.BinaryLength)
    $Ace.GetBinaryForm($bytes, 0)
    return [Convert]::ToBase64String($bytes)
}

function Test-InteractiveGrant {
    $original = 'O:SYG:SYD:PAI(D;;WP;;;IU)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCLCSWLOCRRC;;;IU)S:(AU;SAFA;CC;;;WD)'
    $before = [Security.AccessControl.RawSecurityDescriptor]::new($original)
    $afterSddl = Add-MrdInteractiveServiceStartPermission -Sddl $original
    $after = [Security.AccessControl.RawSecurityDescriptor]::new($afterSddl)
    Assert-Equal $after.DiscretionaryAcl.Count ($before.DiscretionaryAcl.Count + 1) 'Missing start permission must add exactly one ACE'
    Assert-Equal $after.Owner.Value $before.Owner.Value 'Owner must be preserved'
    Assert-Equal $after.Group.Value $before.Group.Value 'Group must be preserved'
    Assert-Equal $after.ControlFlags $before.ControlFlags 'Descriptor flags must be preserved'
    Assert-Equal (Get-AceSignature $after.SystemAcl[0]) (Get-AceSignature $before.SystemAcl[0]) 'Audit ACE must be preserved'
    for ($index = 0; $index -lt $before.DiscretionaryAcl.Count; $index++) {
        Assert-Equal (Get-AceSignature $after.DiscretionaryAcl[$index]) (Get-AceSignature $before.DiscretionaryAcl[$index]) 'Existing deny and allow ACEs must be unchanged'
    }
    $newAce = $after.DiscretionaryAcl[$after.DiscretionaryAcl.Count - 1]
    Assert-Equal $newAce.SecurityIdentifier.Value 'S-1-5-4' 'Only Interactive Users receive the new grant'
    Assert-Equal $newAce.AccessMask 0x10 'Existing read rights mean only SERVICE_START is missing'
    Assert-Equal $newAce.AceQualifier ([Security.AccessControl.AceQualifier]::AccessAllowed) 'New ACE must allow the narrow start right'
    Assert-Equal (Add-MrdInteractiveServiceStartPermission -Sddl $afterSddl) $afterSddl 'Repeating installation must not duplicate grants'
}

function Test-MinimalGrantBeforeInheritedAcl {
    $sddl = Add-MrdInteractiveServiceStartPermission -Sddl 'D:(A;;GA;;;SY)(A;ID;CC;;;IU)'
    $acl = [Security.AccessControl.RawSecurityDescriptor]::new($sddl).DiscretionaryAcl
    Assert-Equal $acl.Count 3 'Missing read and start permissions require one new ACE'
    Assert-Equal $acl[1].AccessMask 0x15 'Grant only query config, query status, and start'
    Assert-Equal $acl[1].AceFlags ([Security.AccessControl.AceFlags]::None) 'Explicit grant must precede inherited ACEs'
    Assert-Equal $acl[2].AceFlags ([Security.AccessControl.AceFlags]::Inherited) 'Inherited ACE must remain intact'
}

function Test-UpgradeOrdering {
    $events = [Collections.Generic.List[string]]::new()
    Invoke-MrdServiceUpgrade -Stop { $events.Add('stop') } -Install { $events.Add('copy') } -Configure { $events.Add('configure') } -Start { $events.Add('start') } -Rollback { $events.Add('rollback') }
    Assert-Equal ($events -join ',') 'stop,copy,configure,start' 'Running service must stop before executable replacement'
}

function Test-UpgradeRollback {
    $events = [Collections.Generic.List[string]]::new()
    $caught = $null
    try {
        Invoke-MrdServiceUpgrade -Stop { $events.Add('stop') } -Install { $events.Add('copy') } -Configure { $events.Add('configure') } -Start { $events.Add('start'); throw 'start failed' } -Rollback { $events.Add('rollback') }
    } catch { $caught = $_.Exception.Message }
    Assert-Equal $caught 'start failed' 'Upgrade failure must be reported'
    Assert-Equal ($events -join ',') 'stop,copy,configure,start,rollback' 'Failure must restore the previous service'
}

function Test-RollbackFailureIsVisible {
    $caught = $null
    try {
        Invoke-MrdServiceUpgrade -Stop { throw 'stop failed' } -Install {} -Configure {} -Start {} -Rollback { throw 'restore failed' }
    } catch { $caught = $_.Exception.Message }
    if (-not $caught -or -not $caught.Contains('stop failed') -or -not $caught.Contains('restore failed')) {
        throw "Upgrade and rollback failure must both be reported: $caught"
    }
}

function Test-NativeArgumentsRoundTrip {
    $fixtureDirectory = Join-Path ([IO.Path]::GetTempPath()) ('mrd-native-argv-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
    try {
        $fixtureSource = Join-Path $fixtureDirectory 'Arguments.cs'
        $fixtureExecutable = Join-Path $fixtureDirectory 'Arguments.exe'
        @'
using System;
using System.Text;
class Arguments {
    public static void Main(string[] args) {
        foreach (string arg in args) Console.WriteLine(Convert.ToBase64String(Encoding.UTF8.GetBytes(arg)));
    }
}
'@ | Set-Content -LiteralPath $fixtureSource -Encoding UTF8
        $compiler = Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
        & $compiler /nologo /target:exe ("/out:$fixtureExecutable") $fixtureSource
        if ($LASTEXITCODE -ne 0) { throw 'Cannot compile native argument fixture' }
        $expected = @('config', 'MiniRemoteDesktop', 'binPath=', '"C:\Program Files\MiniRemoteDesktop\mrd-service.exe" --service', 'DisplayName=', 'Mini Remote Desktop Service', '', 'C:\trailing\', 'quoted\"value')
        $result = Invoke-MrdNativeCommand -FilePath $fixtureExecutable -Arguments $expected
        Assert-Equal $result.ExitCode 0 'Native fixture must exit successfully'
        $actual = @($result.Stdout -split '\r?\n' | Select-Object -SkipLast 1)
        Assert-Equal $actual.Count $expected.Count 'Native option/value boundaries must be preserved'
        for ($index = 0; $index -lt $expected.Count; $index++) {
            $decoded = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($actual[$index]))
            Assert-Equal $decoded $expected[$index] "Native argument $index must survive Windows command-line parsing"
        }
    } finally {
        # Delete only the freshly created fixture under the resolved system temp directory.
        $resolvedFixture = [IO.Path]::GetFullPath($fixtureDirectory)
        $resolvedTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        if (-not $resolvedFixture.StartsWith($resolvedTemp, [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture cleanup is outside temp' }
        Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
    }
}

function Test-PreshutdownQueryUsesServiceApi {
    # EventLog is a standard Windows service; this probe changes no configuration.
    $timeout = Get-MrdServicePreshutdownTimeout -ServiceName 'EventLog'
    Assert-Equal ($timeout -is [uint32]) $true 'Service API must return a typed timeout'
}

$tests = @('Test-InteractiveGrant', 'Test-MinimalGrantBeforeInheritedAcl', 'Test-UpgradeOrdering', 'Test-UpgradeRollback', 'Test-RollbackFailureIsVisible', 'Test-NativeArgumentsRoundTrip', 'Test-PreshutdownQueryUsesServiceApi')
$failures = 0
foreach ($test in $tests) {
    try { & $test; Write-Output "PASS $test" }
    catch { $failures++; Write-Output "FAIL $test`: $($_.Exception.Message)" }
}
if ($failures) { throw "$failures service management test(s) failed" }
Write-Output "$($tests.Count) service management tests passed"
