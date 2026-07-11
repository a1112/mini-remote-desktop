[CmdletBinding()]
param(
  [string]$RepoRoot = ".",
  [string]$OutputDir = "artifacts/e2e/security-negative",
  [string]$CargoCommand = "cargo"
)

$ErrorActionPreference = "Stop"
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
  $PSNativeCommandUseErrorActionPreference = $false
}

$script:CargoCommand = $CargoCommand
$repo = (Resolve-Path $RepoRoot).Path
$outputRoot = if ([System.IO.Path]::IsPathRooted($OutputDir)) {
  $OutputDir
} else {
  Join-Path $repo $OutputDir
}
$policyPath = Join-Path $repo "tests/quality-gates/policies/windows-security-negative.v1.json"
$summaryPath = Join-Path $outputRoot "secure-lan-negative-summary.json"
$invocationId = [Guid]::NewGuid().ToString("N")
$invocationStartedUtc = [DateTime]::UtcNow
$invocationRoot = Join-Path $outputRoot "invocation-$invocationId"

$cases = @(
  [pscustomobject]@{
    id = "untrusted"
    scenario_id = "security.negative.untrusted"
    identity_state = "untrusted"
    authorization_outcome = "denied"
    rejection_reason = "trust_required"
    test_name = "lan_discovery::security_negative_evidence_tests::security_negative_untrusted_emits_authoritative_evidence"
  },
  [pscustomobject]@{
    id = "replay"
    scenario_id = "security.negative.replay"
    identity_state = "trusted"
    authorization_outcome = "denied"
    rejection_reason = "replay_detected"
    test_name = "lan_discovery::security_negative_evidence_tests::security_negative_replay_emits_authoritative_evidence"
  },
  [pscustomobject]@{
    id = "revoked"
    scenario_id = "security.negative.revoked"
    identity_state = "trusted"
    authorization_outcome = "revoked"
    rejection_reason = "grant_revoked"
    test_name = "lan_discovery::security_negative_evidence_tests::security_negative_revoked_emits_authoritative_evidence"
  },
  [pscustomobject]@{
    id = "wrong_scope"
    scenario_id = "security.negative.wrong_scope"
    identity_state = "trusted"
    authorization_outcome = "denied"
    rejection_reason = "scope_denied"
    test_name = "lan_discovery::security_negative_evidence_tests::security_negative_wrong_scope_emits_authoritative_evidence"
  },
  [pscustomobject]@{
    id = "certificate_substitution"
    scenario_id = "security.negative.certificate_substitution"
    identity_state = "trusted"
    authorization_outcome = "denied"
    rejection_reason = "certificate_binding_mismatch"
    test_name = "lan_discovery::security_negative_evidence_tests::security_negative_certificate_substitution_emits_authoritative_evidence"
  }
)

function Write-SecureNegativeUtf8Json {
  param(
    [Parameter(Mandatory = $true)]$InputObject,
    [Parameter(Mandatory = $true)][string]$Path
  )

  $json = ConvertTo-Json -InputObject $InputObject -Depth 16
  $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($Path), $json, $utf8NoBom)
}

function Select-SecureNegativePropertyValue {
  param($Object, [string]$Name, $Fallback)
  if ($null -eq $Object) { return $Fallback }
  $property = $Object.PSObject.Properties[$Name]
  if ($null -eq $property -or $null -eq $property.Value) { return $Fallback }
  $property.Value
}

function Get-SecureNegativeArtifactFailures {
  param(
    [Parameter(Mandatory = $true)]$Artifact,
    [Parameter(Mandatory = $true)]$Case,
    [Parameter(Mandatory = $true)][string]$ExpectedRunId
  )

  $failures = @()
  if ([string]$Artifact.schema_version -ne "remote-experience-run.v2") {
    $failures += "schema_version_mismatch"
  }
  if ([string]$Artifact.run_id -ne $ExpectedRunId) {
    $failures += "invocation_id_mismatch"
  }
  if ([string]$Artifact.scenario.id -ne $Case.scenario_id) {
    $failures += "scenario_id_mismatch"
  }
  if ([string]$Artifact.security.attempt_kind -ne $Case.id) {
    $failures += "attempt_kind_mismatch"
  }
  if ([string]$Artifact.security.identity_state -ne $Case.identity_state) {
    $failures += "identity_state_mismatch"
  }
  if ([string]$Artifact.security.authorization_outcome -ne $Case.authorization_outcome) {
    $failures += "authorization_outcome_mismatch"
  }
  if (-not [bool](Select-SecureNegativePropertyValue $Artifact.security "rejected" $false)) {
    $failures += "attempt_not_rejected"
  }
  if ([string](Select-SecureNegativePropertyValue $Artifact.security "rejection_reason" "") -ne $Case.rejection_reason) {
    $failures += "rejection_reason_mismatch"
  }
  if (-not [bool](Select-SecureNegativePropertyValue $Artifact.security "cleanup_completed" $false)) {
    $failures += "cleanup_not_completed"
  }
  if ([string](Select-SecureNegativePropertyValue $Artifact.route "selected" "") -ne "none") {
    $failures += "route_started"
  }
  if ([bool](Select-SecureNegativePropertyValue $Artifact.security "quic_peer_authenticated" $false)) {
    $failures += "quic_peer_authenticated_after_rejection"
  }
  if ([bool](Select-SecureNegativePropertyValue $Artifact.security "control_input_authenticated" $false)) {
    $failures += "control_input_authenticated_after_rejection"
  }

  foreach ($counter in @(
    "sender_tasks_started",
    "receiver_tasks_started",
    "media_packets_sent",
    "media_frames_presented",
    "control_events_injected"
  )) {
    $value = Select-SecureNegativePropertyValue $Artifact.side_effects $counter $null
    if ($null -eq $value -or [int64]$value -ne 0) {
      $failures += "$($counter)_not_zero"
    }
  }

  $auditIds = @($Artifact.audit_event_ids)
  if ($auditIds.Count -lt 1 -or @($auditIds | Where-Object { -not ([string]$_).Trim() }).Count -gt 0) {
    $failures += "audit_event_missing"
  }
  if ([string]$Artifact.producer_status -ne "completed") {
    $failures += "producer_not_completed"
  }
  if ([string]$Artifact.gate_status -ne "PASS") {
    $failures += "artifact_gate_status_not_pass"
  }
  if ($null -ne $Artifact.present.visible_first_frame_ms) {
    $failures += "rejected_attempt_reported_first_frame"
  }

  @($failures)
}

function Invoke-SecurityNegativeProductCase {
  param(
    [Parameter(Mandatory = $true)]$Case,
    [Parameter(Mandatory = $true)][string]$ArtifactPath
  )

  $previous = @{}
  foreach ($name in @(
    "MRD_SECURITY_NEGATIVE_INVOCATION_ID",
    "MRD_SECURITY_NEGATIVE_CASE",
    "MRD_SECURITY_NEGATIVE_ARTIFACT"
  )) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
  }
  [Environment]::SetEnvironmentVariable("MRD_SECURITY_NEGATIVE_INVOCATION_ID", $invocationId, "Process")
  [Environment]::SetEnvironmentVariable("MRD_SECURITY_NEGATIVE_CASE", $Case.id, "Process")
  [Environment]::SetEnvironmentVariable("MRD_SECURITY_NEGATIVE_ARTIFACT", $ArtifactPath, "Process")

  $startedAt = Get-Date
  $exitCode = 127
  $invocationError = $null
  try {
    Write-Host "Running authoritative secure LAN negative case: $($Case.id)"
    & $script:CargoCommand test -p mrd-service --lib $Case.test_name -- --exact --nocapture | Out-Host
    $exitCode = $LASTEXITCODE
  } catch {
    $invocationError = $_.Exception.Message
  } finally {
    foreach ($name in $previous.Keys) {
      [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process")
    }
  }

  [pscustomobject]@{
    id = $Case.id
    command = "cargo test -p mrd-service --lib $($Case.test_name) -- --exact --nocapture"
    test_name = $Case.test_name
    exit_code = $exitCode
    duration_ms = [int64]((Get-Date) - $startedAt).TotalMilliseconds
    passed = ($exitCode -eq 0)
    error = $invocationError
  }
}

$preflightFailures = @()
if (-not (Test-Path -LiteralPath $policyPath -PathType Leaf)) {
  $preflightFailures += "policy_missing: $policyPath"
}
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
Remove-Item -LiteralPath $summaryPath -Force -ErrorAction SilentlyContinue
if (Test-Path -LiteralPath $invocationRoot) {
  $preflightFailures += "invocation_directory_collision: $invocationRoot"
} else {
  New-Item -ItemType Directory -Path $invocationRoot | Out-Null
}

$productTestResults = @()
$results = @()
foreach ($case in $cases) {
  $artifactPath = Join-Path $invocationRoot "$($case.id).artifact.json"
  $evaluationPath = Join-Path $invocationRoot "$($case.id).evaluation.json"
  $expectedRunId = "security-negative-$invocationId-$($case.id)"
  $caseStartedUtc = [DateTime]::UtcNow
  Remove-Item -LiteralPath $artifactPath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $evaluationPath -Force -ErrorAction SilentlyContinue

  $productResult = Invoke-SecurityNegativeProductCase -Case $case -ArtifactPath $artifactPath
  $productTestResults += $productResult
  $failures = @()
  if (-not $productResult.passed) {
    $failures += "product_test_exit_$($productResult.exit_code)"
  }

  $artifact = $null
  if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
    $failures += "case_evidence_missing"
  } else {
    $artifactFile = Get-Item -LiteralPath $artifactPath
    if ($artifactFile.LastWriteTimeUtc -lt $caseStartedUtc) {
      $failures += "artifact_not_fresh"
    }
    try {
      $artifact = Get-Content -LiteralPath $artifactPath -Raw | ConvertFrom-Json
      $failures += @(Get-SecureNegativeArtifactFailures `
        -Artifact $artifact `
        -Case $case `
        -ExpectedRunId $expectedRunId)
    } catch {
      $failures += "artifact_unreadable: $($_.Exception.Message)"
    }
  }

  $qualityGateExitCode = $null
  $evaluation = $null
  if ($failures.Count -eq 0) {
    try {
      & $script:CargoCommand run -q -p mrd-quality-gate --bin mrd-quality-gate -- `
        --artifact $artifactPath `
        --policy $policyPath `
        --output $evaluationPath
      $qualityGateExitCode = $LASTEXITCODE
    } catch {
      $qualityGateExitCode = 127
      $failures += "quality_gate_invocation_failed: $($_.Exception.Message)"
    }
    if (Test-Path -LiteralPath $evaluationPath -PathType Leaf) {
      try {
        $evaluation = Get-Content -LiteralPath $evaluationPath -Raw | ConvertFrom-Json
      } catch {
        $failures += "evaluation_unreadable: $($_.Exception.Message)"
      }
    } else {
      $failures += "evaluation_missing"
    }
    if ($qualityGateExitCode -ne 0) {
      $failures += "quality_gate_exit_$qualityGateExitCode"
    }
    if ($null -eq $evaluation -or [string]$evaluation.verdict -ne "PASS") {
      $failures += "quality_gate_verdict_not_pass"
    }
  }

  $result = [pscustomobject]@{
    id = $case.id
    scenario_id = $case.scenario_id
    invocation_id = $invocationId
    artifact = $artifactPath
    evaluation = $evaluationPath
    product_test_exit_code = $productResult.exit_code
    quality_gate_exit_code = $qualityGateExitCode
    verdict = if ($evaluation) { [string]$evaluation.verdict } else { $null }
    passed = ($failures.Count -eq 0)
    failures = @($failures)
  }
  $results += $result
  if ($result.passed) {
    Write-Host "PASS secure LAN negative case: $($case.id)"
  } else {
    Write-Warning "FAIL secure LAN negative case: $($case.id) ($($failures -join ', '))"
  }
}

$expectedArtifactNames = @($cases | ForEach-Object { "$($_.id).artifact.json" } | Sort-Object)
$actualArtifactNames = @(
  Get-ChildItem -LiteralPath $invocationRoot -File -Filter "*.artifact.json" |
    ForEach-Object { $_.Name } |
    Sort-Object
)
if (($actualArtifactNames -join "|") -ne ($expectedArtifactNames -join "|")) {
  $preflightFailures += "invocation_artifact_set_mismatch"
}

$failedProductSuites = @($productTestResults | Where-Object { -not $_.passed })
$failedCases = @($results | Where-Object { -not $_.passed })
$suitePassed = $preflightFailures.Count -eq 0 -and $failedProductSuites.Count -eq 0 -and $failedCases.Count -eq 0
$summary = [pscustomobject]@{
  schema_version = "secure-lan-negative-suite.v2"
  generated_at = (Get-Date).ToUniversalTime().ToString("o")
  invocation_id = $invocationId
  invocation_started_at = $invocationStartedUtc.ToString("o")
  invocation_root = $invocationRoot
  policy = $policyPath
  passed = $suitePassed
  verdict = if ($suitePassed) { "PASS" } else { "PRODUCT_FAIL" }
  preflight_failures = @($preflightFailures)
  product_test_suites = @($productTestResults)
  cases = @($results)
}
Write-SecureNegativeUtf8Json -InputObject $summary -Path $summaryPath

if (-not $suitePassed) {
  [Console]::Error.WriteLine("Secure LAN negative suite failed; see $summaryPath")
  exit 2
}

Write-Host "Secure LAN negative suite passed; evidence written to $invocationRoot"
