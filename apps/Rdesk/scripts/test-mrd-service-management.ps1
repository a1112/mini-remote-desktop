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

$tests = @('Test-InteractiveGrant', 'Test-MinimalGrantBeforeInheritedAcl', 'Test-UpgradeOrdering', 'Test-UpgradeRollback', 'Test-RollbackFailureIsVisible')
$failures = 0
foreach ($test in $tests) {
    try { & $test; Write-Output "PASS $test" }
    catch { $failures++; Write-Output "FAIL $test`: $($_.Exception.Message)" }
}
if ($failures) { throw "$failures service management test(s) failed" }
Write-Output "$($tests.Count) service management tests passed"
