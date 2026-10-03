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
        if (($ace.AceFlags -band [Security.AccessControl.AceFlags]::Inherited) -ne 0) {
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
