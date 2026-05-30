$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "transport_matrix_common.ps1")
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..\..")).Path

function Assert-ArrayEqual([object[]]$Actual, [object[]]$Expected, [string]$Message) {
  if ($Actual.Count -ne $Expected.Count) {
    throw "$Message. Expected $($Expected.Count) item(s), got $($Actual.Count)"
  }
  for ($i = 0; $i -lt $Expected.Count; $i++) {
    if ($Actual[$i] -ne $Expected[$i]) {
      throw "$Message. Item $i expected '$($Expected[$i])', got '$($Actual[$i])'"
    }
  }
}

function Assert-Throws([scriptblock]$Action, [string]$Pattern, [string]$Message) {
  try {
    & $Action
  } catch {
    if ($_.Exception.Message -match $Pattern) {
      return
    }
    throw "$Message. Threw unexpected message: $($_.Exception.Message)"
  }
  throw "$Message. Expected an exception matching '$Pattern'"
}

$hevcArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_hevc"
Assert-ArrayEqual $hevcArgs @("--features", "production-software-codecs") "HEVC software matrix enables production software codecs"

$main10Args = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_hevc_main10"
Assert-ArrayEqual $main10Args @("--features", "production-software-codecs") "HEVC Main10 software matrix enables production software codecs"

$av1Args = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_av1"
Assert-ArrayEqual $av1Args @("--features", "production-software-codecs") "AV1 software matrix enables production software codecs"

$vvcArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "software_vvc"
Assert-ArrayEqual $vvcArgs @("--features", "production-vvc-software-codec") "VVC software matrix enables VVC software codec"

$vvcEncodeOnlyArgs = Get-TransportMatrixCargoFeatureArgs -EncodeBackend "software_vvc" -DecodeBackend "none"
Assert-ArrayEqual $vvcEncodeOnlyArgs @("--features", "mrd-encode-vvenc/software-vvenc") "VVC encode-only matrix enables VVenC encoder feature"

$vvcEncodeDecodeArgs = Get-TransportMatrixCargoFeatureArgs -EncodeBackend "software_vvc" -DecodeBackend "software_vvc"
Assert-ArrayEqual $vvcEncodeDecodeArgs @("--features", "production-vvc-software-codec") "VVC encode/decode matrix uses aggregate VVC software codec feature"

$noneArgs = Get-TransportMatrixCargoFeatureArgs -DecodeBackend "nvdec"
Assert-ArrayEqual $noneArgs @() "Hardware decode matrix does not enable software codec features"

$bitrateBps = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_bps = 12000000 })
if ($bitrateBps -ne "12000000") {
  throw "bitrate_bps scenario field should pass through unchanged"
}

$bitrateMbps = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_mbps = 12 })
if ($bitrateMbps -ne "12000000") {
  throw "bitrate_mbps scenario field should convert to bps"
}

$noBitrate = Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ profile = "default" })
if ($null -ne $noBitrate) {
  throw "scenario without bitrate should leave MRD_BENCH_BITRATE_BPS unset"
}

Assert-Throws { Get-TransportMatrixBitrateBps -Scenario ([pscustomobject]@{ bitrate_bps = 0 }) } "greater than zero" "Zero bitrate_bps must be rejected"

$renderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
  d3d11_waitable_object = $true
  render_thread_priority = "above_normal"
})
if ($renderEnv.MRD_D3D11_RENDER_WAITABLE_OBJECT -ne "1") { throw "waitable render scenario should set waitable env" }
if ($renderEnv.MRD_RENDER_THREAD_PRIORITY -ne "above_normal") { throw "render scenario should set thread priority env" }

$defaultRenderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{ profile = "default" })
if ($defaultRenderEnv.ContainsKey("MRD_D3D11_RENDER_WAITABLE_OBJECT")) { throw "default scenario should not set waitable env" }
if ($defaultRenderEnv.ContainsKey("MRD_RENDER_THREAD_PRIORITY")) { throw "default scenario should not set render priority env" }

$scenarioSpecs = @(
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.h264_software.2k.json"
    profile = "transport-quic-openh264-h264-software-2k"
    transport = "quic_quinn"
    encode = "openh264"
    decode = "h264_software"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.ffmpeg_h264.2k.json"
    profile = "transport-quic-openh264-ffmpeg-h264-2k"
    transport = "quic_quinn"
    encode = "openh264"
    decode = "ffmpeg_h264"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.openh264.nvdec.2k.json"
    profile = "transport-quic-openh264-nvdec-2k"
    transport = "quic_quinn"
    encode = "openh264"
    decode = "nvdec"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.quic.nvenc.nvdec.2k.json"
    profile = "transport-quic-nvenc-nvdec-2k"
    transport = "quic_quinn"
    encode = "nvenc"
    decode = "nvdec"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.2k144.waitable.json"
    profile = "transport-webrtc-nvenc-h264-nvdec-2k144-waitable"
    transport = "webrtc"
    encode = "nvenc"
    decode = "nvdec"
  }
)

foreach ($spec in $scenarioSpecs) {
  $scenarioPath = Join-Path $repoRoot $spec.path
  if (-not (Test-Path $scenarioPath)) {
    throw "Expected formal 2K transport scenario at $($spec.path)"
  }
  $scenario = Get-Content $scenarioPath -Raw | ConvertFrom-Json
  if ($scenario.profile -ne $spec.profile) { throw "$($spec.path) profile mismatch" }
  if ($scenario.transport -ne $spec.transport) { throw "$($spec.path) should use $($spec.transport)" }
  if ($scenario.encode_backend -ne $spec.encode) { throw "$($spec.path) encode backend mismatch" }
  if ($scenario.decode_backend -ne $spec.decode) { throw "$($spec.path) decode backend mismatch" }
  if ($scenario.width -ne 2560 -or $scenario.height -ne 1440) { throw "$($spec.path) should be 2560x1440" }
  if ($spec.path -like "*.2k144.waitable.json") {
    if ($scenario.fps -ne 144) { throw "$($spec.path) should target 144fps" }
    if (-not $scenario.d3d11_waitable_object) { throw "$($spec.path) should enable waitable object" }
    if ($scenario.render_thread_priority -ne "above_normal") { throw "$($spec.path) should set render thread priority" }
  } elseif ($scenario.fps -ne 30) {
    throw "$($spec.path) should target 30fps"
  }
}

$processTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-process-{0}" -f ([guid]::NewGuid()))
New-Item -ItemType Directory -Force -Path $processTmp | Out-Null
try {
  $stdout = Join-Path $processTmp "stdout.log"
  $stderr = Join-Path $processTmp "stderr.log"
  $exitCode = Invoke-TransportMatrixCommand `
    -FilePath "powershell" `
    -ArgumentList @("-NoProfile", "-Command", "Write-Output 'stdout-ok'; [Console]::Error.WriteLine('stderr-ok'); exit 7") `
    -WorkingDirectory $processTmp `
    -StdoutPath $stdout `
    -StderrPath $stderr

  if ($exitCode.ExitCode -ne 7) { throw "Invoke-TransportMatrixCommand should return the native exit code" }
  if ($exitCode.TimedOut) { throw "Invoke-TransportMatrixCommand should not mark a completed command as timed out" }
  if ((Get-Content $stdout -Raw) -notmatch "stdout-ok") { throw "Invoke-TransportMatrixCommand should capture stdout" }
  if ((Get-Content $stderr -Raw) -notmatch "stderr-ok") { throw "Invoke-TransportMatrixCommand should capture stderr" }

  $timeoutResult = Invoke-TransportMatrixCommand `
    -FilePath "powershell" `
    -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Seconds 5; Write-Output 'too-late'; exit 0") `
    -WorkingDirectory $processTmp `
    -StdoutPath $stdout `
    -StderrPath $stderr `
    -TimeoutSeconds 1

  if ($timeoutResult.ExitCode -ne 124) { throw "Invoke-TransportMatrixCommand should use 124 for timeouts" }
  if (-not $timeoutResult.TimedOut) { throw "Invoke-TransportMatrixCommand should mark timed out commands" }
} finally {
  Remove-Item $processTmp -Recurse -Force -ErrorAction SilentlyContinue
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-summary-{0}.json" -f ([guid]::NewGuid()))
try {
  @{ run_passed = $false; scenario = "quick.transport"; profile = "hevc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-Throws { Assert-TransportMatrixSummaryPassed -SummaryPath $tmp } "failed thresholds" "Failed matrix summary must throw"

  @{ run_passed = $true; scenario = "quick.transport"; profile = "hevc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-TransportMatrixSummaryPassed -SummaryPath $tmp

  @{ run_passed = $false; run_skipped = $true; scenario = "quick.transport"; profile = "vvc" } |
    ConvertTo-Json |
    Set-Content -Path $tmp -Encoding Ascii
  Assert-TransportMatrixSummaryPassed -SummaryPath $tmp
} finally {
  Remove-Item $tmp -ErrorAction SilentlyContinue
}

$summaryTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-summary-run-{0}" -f ([guid]::NewGuid()))
try {
  New-Item -ItemType Directory -Force -Path (Join-Path $summaryTmp "logs") | Out-Null
  New-Item -ItemType Directory -Force -Path (Join-Path $summaryTmp "reports") | Out-Null

  [ordered]@{
    run_id = "quick.transport-webrtc-test"
    scenario = "quick.transport"
    transport = "webrtc"
    capture_backend = "dxgi"
    encode_backend = "nvenc"
    decode_backend = "nvdec"
    renderer_backend = "d3d11_shared"
    width = 2560
    height = 1440
    fps_target = 144
    duration_secs = 20
    session_established = $true
    first_frame_seen = $true
    first_frame_time_ms = 100.0
    probe_complete = $true
    fps_observed = 143.5
    bitrate_kbps = 30000.0
    keyframes = 0
    dropped_frames = 0
    quic_receiver_completed_frames = $null
    quic_receiver_expired_frames = $null
    quic_receiver_evicted_frames = $null
    quic_receiver_duplicate_fragments = $null
    quic_receiver_rejected_fragments = $null
    quic_receiver_pending_frames = $null
    quic_receiver_reassembly_drops = $null
    zero_write_access_unit_count = 0
    warning_count = 0
    error_count = 0
    restart_count = 0
    encode_total_p95_ms = 0.4
    send_write_p95_ms = 0.8
    decode_total_p95_ms = 1.5
    frame_sink_ingest_p95_ms = 2.0
    render_upload_p95_ms = 0.2
    render_present_p95_ms = 8.0
    render_submitted_frames = 2849
    render_uploaded_frames = 2841
    render_presented_frames = 2839
    render_present_skipped_frames = 2
    render_queue_replacements = 7
    render_stale_frame_drops = 7
    swap_chain_waitable_object = $true
    swap_chain_present_mode = "waitable"
    display_refresh_hz = 144
    render_thread_priority = "above_normal"
    failure_reason = $null
    run_skipped = $false
    run_passed = $true
  } | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $summaryTmp "summary.json") -Encoding Ascii

  [ordered]@{
    run_id = "quick.transport-webrtc-test"
    scenario = "quick.transport"
    transport = "webrtc"
    width = 2560
    height = 1440
    fps = 144
    duration_secs = 20
    git_commit = "abc123"
  } | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $summaryTmp "manifest.json") -Encoding Ascii

  & (Join-Path $scriptDir "summarize_transport_results.ps1") -RunDir $summaryTmp

  $schema = Get-Content (Join-Path $repoRoot "tests/benchmarks/schemas/benchmark-result.schema.json") -Raw | ConvertFrom-Json
  $schemaProperties = @($schema.properties.PSObject.Properties.Name)
  $summarized = Get-Content (Join-Path $summaryTmp "summary.json") -Raw | ConvertFrom-Json
  $extraProperties = @($summarized.PSObject.Properties.Name | Where-Object { $_ -notin $schemaProperties })
  if ($extraProperties.Count -gt 0) {
    throw "benchmark summary schema is missing properties: $($extraProperties -join ', ')"
  }

  $csv = Import-Csv (Join-Path $summaryTmp "summary.csv")
  if ($csv.run_status -ne "PASS") { throw "summary CSV must expose PASS run_status" }
  if ($csv.render_queue_replacements -ne "7") { throw "summary CSV must include render queue replacements" }
  if ($csv.render_stale_frame_drops -ne "7") { throw "summary CSV must include render stale frame drops" }
  if ($csv.render_queue_replacement_rate -ne "0.35") { throw "summary CSV must include render queue replacement rate" }
  if ($csv.render_stale_frame_drop_rate -ne "0.35") { throw "summary CSV must include render stale frame drop rate" }
  if ($csv.render_present_skipped_rate -ne "0.1") { throw "summary CSV must include render skipped frame rate" }
  if ($csv.swap_chain_present_mode -ne "waitable") { throw "summary CSV must include swapchain present mode" }
  if ($csv.display_refresh_hz -ne "144") { throw "summary CSV must include display refresh hz" }
  $report = Get-Content (Join-Path $summaryTmp "reports/markdown-report.md") -Raw
  if ($report -notmatch "swap_chain_present_mode \\| waitable") { throw "markdown report must include swapchain present mode" }

  $thresholdPath = Join-Path $summaryTmp "strict-thresholds.json"
  [ordered]@{
    max_first_frame_time_ms = 5000
    min_fps_observed = 120.0
    max_encode_total_p95_ms = 8.0
    max_send_write_p95_ms = 8.0
    max_decode_total_p95_ms = 8.0
    max_render_present_p95_ms = 7.0
    max_render_queue_replacements = 3
    max_render_stale_frame_drops = 3
    max_render_present_skipped_frames = 1
    max_warning_count = 20
    max_error_count = 0
  } | ConvertTo-Json -Depth 8 | Set-Content -Path $thresholdPath -Encoding Ascii

  & (Join-Path $scriptDir "summarize_transport_results.ps1") -RunDir $summaryTmp -ThresholdPath $thresholdPath
  $strict = Get-Content (Join-Path $summaryTmp "summary.json") -Raw | ConvertFrom-Json
  if ($strict.run_passed) { throw "strict render threshold should fail the summary" }
  if ($strict.failure_reason -notmatch "render present p95") { throw "failure reason should include render present threshold" }
  if ($strict.failure_reason -notmatch "render queue replacements") { throw "failure reason should include render queue replacement threshold" }
  $strictCsv = Import-Csv (Join-Path $summaryTmp "summary.csv")
  if ($strictCsv.run_status -ne "FAIL") { throw "strict threshold CSV should expose FAIL run_status" }
} finally {
  Remove-Item $summaryTmp -Recurse -Force -ErrorAction SilentlyContinue
}
