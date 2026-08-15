$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "benchmark_host_capabilities.ps1")

function Get-LocalPerformanceScenarioPaths {
  param([Parameter(Mandatory = $true)][string]$RepoRoot)

  Get-ChildItem -LiteralPath (Join-Path $RepoRoot "tests/benchmarks/scenarios") `
    -Filter "*.json" -File |
    Sort-Object FullName |
    ForEach-Object { $_.FullName }
}

function Get-LocalPerformanceActiveDisplayCount {
  try {
    Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
    return @([System.Windows.Forms.Screen]::AllScreens).Count
  } catch {
    return [int]::MaxValue
  }
}

function Get-LocalPerformanceScenarioSupportReason {
  param(
    [Parameter(Mandatory = $true)]$Scenario,
    [Parameter(Mandatory = $true)][int]$ActiveDisplayCount,
    [AllowEmptyCollection()][string[]]$AvailableCommands = $null,
    [AllowNull()]$VvcLibraryAvailable = $null,
    [AllowNull()]$DxgiOutputAvailable = $null
  )

  $captureBackend = if ($null -ne $Scenario.PSObject.Properties["capture_backend"]) {
    [string]$Scenario.capture_backend
  } else {
    ""
  }
  if ($captureBackend -eq "dxgi" -and $null -ne $DxgiOutputAvailable -and -not [bool]$DxgiOutputAvailable) {
    return "dxgi_output_unavailable"
  }

  $sourceId = if ($null -ne $Scenario.PSObject.Properties["source_id"]) {
    [string]$Scenario.source_id
  } else {
    ""
  }
  if ($sourceId -match '^windows:display-shared:(\d+)$') {
    $displayIndex = [int]$Matches[1]
    if ($displayIndex -ge $ActiveDisplayCount) {
      return "display_source_unavailable"
    }
  }
  $encodeBackend = if ($null -ne $Scenario.PSObject.Properties["encode_backend"]) {
    [string]$Scenario.encode_backend
  } else {
    ""
  }
  if ($encodeBackend -eq "software_vvc") {
    $pkgConfigAvailable = if ($null -eq $AvailableCommands) {
      $null -ne (Get-Command "pkg-config" -ErrorAction SilentlyContinue)
    } else {
      @($AvailableCommands) -contains "pkg-config"
    }
    $cmakeAvailable = if ($null -eq $AvailableCommands) {
      $null -ne (Get-Command "cmake" -ErrorAction SilentlyContinue)
    } else {
      @($AvailableCommands) -contains "cmake"
    }
    if (-not $pkgConfigAvailable -or -not $cmakeAvailable) {
      return "codec_dependency_unavailable"
    }
    $hasVvcLibrary = if ($null -ne $VvcLibraryAvailable) {
      [bool]$VvcLibraryAvailable
    } else {
      $pkgConfigCommand = if ($env:PKG_CONFIG) { $env:PKG_CONFIG } else { "pkg-config" }
      & $pkgConfigCommand --exists "libvvenc >= 1.13.0" 2>$null
      $LASTEXITCODE -eq 0
    }
    if (-not $hasVvcLibrary) {
      return "codec_dependency_unavailable"
    }
  }
  $null
}

function New-LocalPerformanceEnvironmentManifest {
  param(
    [Parameter(Mandatory = $true)][string]$RunId,
    [Parameter(Mandatory = $true)][string]$GitCommit,
    [bool]$GitDirty,
    [Parameter(Mandatory = $true)][string]$CpuName,
    [int]$CpuCores,
    [int]$CpuThreads,
    [uint64]$RamBytes,
    [Parameter(Mandatory = $true)][string]$OsName,
    [Parameter(Mandatory = $true)][string]$OsBuild,
    [Parameter(Mandatory = $true)][string]$GpuName,
    [Parameter(Mandatory = $true)][string]$GpuDriver,
    [uint64]$GpuMemoryBytes
  )

  [pscustomobject]@{
    schema_version = "mrd-local-performance-environment.v1"
    run_id = $RunId
    captured_at = [DateTimeOffset]::UtcNow.ToString("o")
    git = [pscustomobject]@{
      commit = $GitCommit
      dirty = $GitDirty
    }
    cpu = [pscustomobject]@{
      name = $CpuName
      physical_cores = $CpuCores
      logical_threads = $CpuThreads
    }
    memory = [pscustomobject]@{
      total_bytes = $RamBytes
    }
    os = [pscustomobject]@{
      name = $OsName
      build = $OsBuild
    }
    gpu = [pscustomobject]@{
      name = $GpuName
      driver = $GpuDriver
      memory_bytes = $GpuMemoryBytes
    }
  }
}

function New-LocalPerformanceUnsupportedRow {
  param(
    [Parameter(Mandatory = $true)][string]$Phase,
    [Parameter(Mandatory = $true)][string]$CaseId,
    [Parameter(Mandatory = $true)][string]$ReasonCode
  )

  [pscustomobject]@{
    phase = $Phase
    case_id = $CaseId
    verdict = "UNSUPPORTED"
    reason_code = $ReasonCode
    exit_code = $null
    timed_out = $false
    artifact_path = $null
    duration_ms = 0
  }
}

function Resolve-LocalPerformanceExitCode {
  param([string[]]$Verdicts)

  if (@($Verdicts) -contains "INVALID_ARTIFACT") { return 4 }
  if (@($Verdicts) -contains "INFRA_FAIL") { return 3 }
  if (@($Verdicts) -contains "PRODUCT_FAIL") { return 2 }
  if (@($Verdicts | Where-Object { $_ -notin @("PASS", "UNSUPPORTED") }).Count -gt 0) {
    return 4
  }
  0
}

function Test-LocalPerformanceArtifact {
  param([Parameter(Mandatory = $true)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
  (Get-Item -LiteralPath $Path).Length -gt 0
}

function ConvertTo-LocalPerformanceQuotedPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  "'" + $Path.Replace("'", "''") + "'"
}

function New-LocalPerformanceExecutionPlan {
  param(
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [AllowNull()]$DxgiOutputAvailable = $null
  )

  $componentScript = ConvertTo-LocalPerformanceQuotedPath (
    Join-Path $RepoRoot "tests/component-matrix/scripts/run_component_matrix.ps1"
  )
  $transportScript = ConvertTo-LocalPerformanceQuotedPath (
    Join-Path $RepoRoot "tests/benchmarks/scripts/run_transport_matrix.ps1"
  )
  $canaryScript = ConvertTo-LocalPerformanceQuotedPath (
    Join-Path $RepoRoot "tests/benchmarks/scripts/run_paired_lan_canary.ps1"
  )
  $integrationManifest = ConvertTo-LocalPerformanceQuotedPath (
    Join-Path $RepoRoot "tests/integration/Cargo.toml"
  )
  $canaryOutput = ConvertTo-LocalPerformanceQuotedPath (
    Join-Path $OutputRoot "local-canary"
  )

  $plan = [System.Collections.Generic.List[object]]::new()
  $activeDisplayCount = Get-LocalPerformanceActiveDisplayCount
  if ($null -eq $DxgiOutputAvailable) {
    $DxgiOutputAvailable = Test-BenchmarkDxgiOutputAvailable -RepoRoot $RepoRoot
  }
  $plan.Add([pscustomobject]@{
    phase = "component"
    case_id = "component-matrix"
    command = "& powershell -ExecutionPolicy Bypass -File $componentScript"
    timeout_secs = 1800
  })
  foreach ($testName in @("automated_e2e_matrix", "automated_e2e_pipeline")) {
    $plan.Add([pscustomobject]@{
      phase = "integration"
      case_id = $testName
      command = "& cargo test --release --manifest-path $integrationManifest --test $testName -- --nocapture"
      timeout_secs = 1800
    })
  }
  foreach ($scenarioPath in @(Get-LocalPerformanceScenarioPaths -RepoRoot $RepoRoot)) {
    $scenario = Get-Content -LiteralPath $scenarioPath -Raw | ConvertFrom-Json
    $unsupportedReason = Get-LocalPerformanceScenarioSupportReason `
      -Scenario $scenario `
      -ActiveDisplayCount $activeDisplayCount `
      -DxgiOutputAvailable $DxgiOutputAvailable
    $quotedScenario = ConvertTo-LocalPerformanceQuotedPath $scenarioPath
    $plan.Add([pscustomobject]@{
      phase = "transport"
      case_id = [System.IO.Path]::GetFileNameWithoutExtension($scenarioPath)
      command = "& powershell -ExecutionPolicy Bypass -File $transportScript -ScenarioPath $quotedScenario"
      timeout_secs = 1800
      unsupported_reason = $unsupportedReason
    })
  }
  $plan.Add([pscustomobject]@{
    phase = "local-canary"
    case_id = "all-local-profiles"
    command = "& powershell -ExecutionPolicy Bypass -File $canaryScript -OutputDir $canaryOutput -SkipCross"
    timeout_secs = 7200
    unsupported_reason = if ([bool]$DxgiOutputAvailable) { $null } else { "dxgi_output_unavailable" }
  })
  @($plan)
}

function Invoke-LocalPerformancePlan {
  param(
    [Parameter(Mandatory = $true)][object[]]$Plan,
    [Parameter(Mandatory = $true)][scriptblock]$Executor
  )

  foreach ($item in $Plan) {
    $unsupportedReason = if ($null -ne $item.PSObject.Properties["unsupported_reason"]) {
      [string]$item.unsupported_reason
    } else {
      ""
    }
    if ($unsupportedReason) {
      New-LocalPerformanceUnsupportedRow `
        -Phase ([string]$item.phase) `
        -CaseId ([string]$item.case_id) `
        -ReasonCode $unsupportedReason
      continue
    }
    $result = & $Executor $item
    $verdict = if ([bool]$result.timed_out) {
      "INFRA_FAIL"
    } elseif (
      $null -ne $result.PSObject.Properties["artifact_valid"] -and
      -not [bool]$result.artifact_valid
    ) {
      "INVALID_ARTIFACT"
    } else {
      switch ([int]$result.exit_code) {
        0 { "PASS" }
        2 { "PRODUCT_FAIL" }
        3 { "INFRA_FAIL" }
        4 { "INVALID_ARTIFACT" }
        default { "INFRA_FAIL" }
      }
    }
    [pscustomobject]@{
      phase = [string]$item.phase
      case_id = [string]$item.case_id
      verdict = $verdict
      exit_code = $result.exit_code
      timed_out = [bool]$result.timed_out
      artifact_path = $result.artifact_path
      duration_ms = [uint64]$result.duration_ms
    }
  }
}

function Write-LocalPerformanceJson {
  param(
    [Parameter(Mandatory = $true)]$InputObject,
    [Parameter(Mandatory = $true)][string]$Path,
    [int]$Depth = 24
  )

  $json = ConvertTo-Json -InputObject $InputObject -Depth $Depth
  $encoding = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($Path), $json, $encoding)
}
