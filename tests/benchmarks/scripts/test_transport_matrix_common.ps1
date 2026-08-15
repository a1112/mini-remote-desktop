$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "transport_matrix_common.ps1")
. (Join-Path $scriptDir "benchmark_execution_state.ps1")
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

if ((Get-BenchmarkExecutionStateFlags -DisplayRequired) -ne [uint32]2147483651) {
  throw "Display-required benchmark execution state must keep both system and display awake"
}
if ((Get-BenchmarkExecutionStateFlags) -ne [uint32]2147483649) {
  throw "Default benchmark execution state must keep the system awake"
}

$relativeScenario = Resolve-BenchmarkPath `
  -RepoRoot $repoRoot `
  -Path "tests/benchmarks/scenarios/quick.transport.json"
$expectedRelativeScenario = [System.IO.Path]::GetFullPath(
  (Join-Path $repoRoot "tests/benchmarks/scenarios/quick.transport.json")
)
if ($relativeScenario -ne $expectedRelativeScenario) {
  throw "Relative benchmark paths must resolve under the repository root"
}
$absoluteOutput = [System.IO.Path]::GetFullPath(
  (Join-Path ([System.IO.Path]::GetTempPath()) "mrd-benchmark-absolute")
)
if ((Resolve-BenchmarkPath -RepoRoot $repoRoot -Path $absoluteOutput) -ne $absoluteOutput) {
  throw "Absolute benchmark paths must not be joined to the repository root"
}
if ((Get-TransportMatrixTimeoutSeconds -Scenario ([pscustomobject]@{ duration_secs = 20 })) -ne 440) {
  throw "Quick transport timeout must retain a cold release-build allowance"
}
if ((Get-TransportMatrixTimeoutSeconds -Scenario ([pscustomobject]@{ duration_secs = 180 })) -ne 600) {
  throw "Stress transport timeout must include scenario duration and build allowance"
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

$releaseCargoArgs = Get-TransportMatrixCargoTestArgs -EncodeBackend "nvenc_av1" -DecodeBackend "nvdec_av1"
Assert-ArrayEqual $releaseCargoArgs @("test", "--release", "-p", "app", "benchmark_run_writes_requested_artifacts", "--", "--nocapture") "Transport benchmarks should default to release cargo tests"

$debugCargoArgs = Get-TransportMatrixCargoTestArgs -EncodeBackend "nvenc_av1" -DecodeBackend "nvdec_av1" -Release:$false
Assert-ArrayEqual $debugCargoArgs @("test", "-p", "app", "benchmark_run_writes_requested_artifacts", "--", "--nocapture") "Transport benchmarks should allow debug cargo tests for local debugging"

$vvcCargoArgs = Get-TransportMatrixCargoTestArgs -EncodeBackend "software_vvc" -DecodeBackend "software_vvc"
Assert-ArrayEqual $vvcCargoArgs @("test", "--release", "-p", "app", "--features", "production-vvc-software-codec", "benchmark_run_writes_requested_artifacts", "--", "--nocapture") "Transport benchmark cargo args should preserve codec feature flags"

$explicitAv1Mode = Get-TransportMatrixAv1Mode -Scenario ([pscustomobject]@{ encode_backend = "nvenc_av1"; av1_mode = "ultra_low_latency"; fps = 144 })
if ($explicitAv1Mode -ne "ultra_low_latency") { throw "transport matrix should preserve explicit AV1 mode" }

$highRefreshAv1Mode = Get-TransportMatrixAv1Mode -Scenario ([pscustomobject]@{ encode_backend = "nvenc_av1"; fps = 144 })
if ($highRefreshAv1Mode -ne "high_refresh") { throw "transport matrix should default high-refresh AV1 runs to high_refresh mode" }

$defaultAv1Mode = Get-TransportMatrixAv1Mode -Scenario ([pscustomobject]@{ encode_backend = "nvenc_av1"; fps = 60 })
if ($null -ne $defaultAv1Mode) { throw "transport matrix should leave non-high-refresh AV1 mode at harness default" }

$nonAv1Mode = Get-TransportMatrixAv1Mode -Scenario ([pscustomobject]@{ encode_backend = "nvenc"; fps = 144 })
if ($null -ne $nonAv1Mode) { throw "transport matrix should not set AV1 mode for non-AV1 encoders" }

$threshold2k144 = Get-Content (Join-Path $repoRoot "tests/benchmarks/thresholds/transport.2k144.json") -Raw | ConvertFrom-Json
foreach ($thresholdName in @(
  "max_render_execute_p95_ms",
  "max_render_prepare_wait_p95_ms",
  "max_render_shared_resource_p95_ms",
  "max_render_draw_present_p95_ms"
)) {
  if (-not ($threshold2k144.PSObject.Properties.Name -contains $thresholdName)) {
    throw "transport.2k144.json must include $thresholdName"
  }
  if ([double]$threshold2k144.$thresholdName -le 0) {
    throw "transport.2k144.json $thresholdName must be positive"
  }
}

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

$sourceEnv = Get-TransportMatrixSourceEnvironment -Scenario ([pscustomobject]@{
  source_id = " windows:display-shared:1 "
  display_id = "windows:display-shared:1"
})
if ($sourceEnv.MRD_BENCH_SOURCE_ID -ne "windows:display-shared:1") { throw "scenario source_id should set MRD_BENCH_SOURCE_ID" }
if ($sourceEnv.MRD_BENCH_DISPLAY_ID -ne "windows:display-shared:1") { throw "scenario display_id should set MRD_BENCH_DISPLAY_ID" }

$emptySourceEnv = Get-TransportMatrixSourceEnvironment -Scenario ([pscustomobject]@{ profile = "default"; source_id = "" })
if ($emptySourceEnv.ContainsKey("MRD_BENCH_SOURCE_ID")) { throw "empty scenario source_id should leave MRD_BENCH_SOURCE_ID unset" }

$renderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
  d3d11_waitable_object = $true
  render_thread_priority = "above_normal"
})
if ($renderEnv.MRD_D3D11_RENDER_WAITABLE_OBJECT -ne "1") { throw "waitable render scenario should set waitable env" }
if ($renderEnv.MRD_RENDER_THREAD_PRIORITY -ne "above_normal") { throw "render scenario should set thread priority env" }

$defaultRenderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{ profile = "default" })
if ($defaultRenderEnv.ContainsKey("MRD_D3D11_RENDER_WAITABLE_OBJECT")) { throw "default scenario should not set waitable env" }
if ($defaultRenderEnv.ContainsKey("MRD_RENDER_THREAD_PRIORITY")) { throw "default scenario should not set render priority env" }

$highRefreshRenderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
  renderer_backend = "d3d11_shared"
  fps = 144
})
if ($highRefreshRenderEnv.MRD_D3D11_RENDER_WAITABLE_OBJECT -ne "1") { throw "high refresh D3D11 scenario should default to waitable env" }
if ($highRefreshRenderEnv.MRD_RENDER_THREAD_PRIORITY -ne "above_normal") { throw "high refresh D3D11 scenario should default render priority env" }

$explicitNonWaitableRenderEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
  renderer_backend = "d3d11_shared"
  fps = 144
  d3d11_waitable_object = $false
  render_thread_priority = "normal"
})
if ($explicitNonWaitableRenderEnv.MRD_D3D11_RENDER_WAITABLE_OBJECT -ne "0") { throw "explicit high refresh waitable=false should be preserved" }
if ($explicitNonWaitableRenderEnv.MRD_RENDER_THREAD_PRIORITY -ne "normal") { throw "explicit high refresh render priority should be preserved" }

$transportMatrixScript = Get-Content (Join-Path $scriptDir "run_transport_matrix.ps1") -Raw
if ($transportMatrixScript -notmatch "MRD_BENCH_COLOR_MODE") { throw "run_transport_matrix.ps1 must pass color_mode to the benchmark harness" }
if ($transportMatrixScript -notmatch "MRD_BENCH_COLOR_PIPELINE") { throw "run_transport_matrix.ps1 must pass color_pipeline to the benchmark harness" }

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
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.av1_nvdec.2k144.waitable.json"
    profile = "transport-webrtc-nvenc-av1-nvdec-2k144-waitable"
    transport = "webrtc"
    encode = "nvenc_av1"
    decode = "nvdec_av1"
    av1_mode = "high_refresh"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.4k120.waitable.json"
    profile = "transport-webrtc-nvenc-h264-nvdec-4k120-waitable"
    transport = "webrtc"
    encode = "nvenc"
    decode = "nvdec"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.4k120.waitable.grayscale.json"
    profile = "transport-webrtc-nvenc-h264-nvdec-4k120-waitable-grayscale"
    transport = "webrtc"
    encode = "nvenc"
    decode = "nvdec"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "grayscale"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.4k120.waitable.monochrome.json"
    profile = "transport-webrtc-nvenc-h264-nvdec-4k120-waitable-monochrome"
    transport = "webrtc"
    encode = "nvenc"
    decode = "nvdec"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "monochrome"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.4k120.waitable.low_chroma.json"
    profile = "transport-webrtc-nvenc-h264-nvdec-4k120-waitable-low-chroma"
    transport = "webrtc"
    encode = "nvenc"
    decode = "nvdec"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "low_chroma"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.json"
    profile = "transport-webrtc-nvenc-hevc-nvdec-4k120-waitable"
    transport = "webrtc"
    encode = "nvenc_hevc"
    decode = "nvdec_hevc"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.grayscale.json"
    profile = "transport-webrtc-nvenc-hevc-nvdec-4k120-waitable-grayscale"
    transport = "webrtc"
    encode = "nvenc_hevc"
    decode = "nvdec_hevc"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "grayscale"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.monochrome.json"
    profile = "transport-webrtc-nvenc-hevc-nvdec-4k120-waitable-monochrome"
    transport = "webrtc"
    encode = "nvenc_hevc"
    decode = "nvdec_hevc"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "monochrome"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_nvdec.4k120.waitable.low_chroma.json"
    profile = "transport-webrtc-nvenc-hevc-nvdec-4k120-waitable-low-chroma"
    transport = "webrtc"
    encode = "nvenc_hevc"
    decode = "nvdec_hevc"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
    color_mode = "low_chroma"
    color_pipeline = "sdr8"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.hevc_main10_nvdec.4k120.waitable.json"
    profile = "transport-webrtc-nvenc-hevc-main10-nvdec-4k120-waitable"
    transport = "webrtc"
    encode = "nvenc_hevc_main10"
    decode = "nvdec_hevc_main10"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    threshold_file = "transport.4k120.json"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.av1_nvdec.4k120.waitable.json"
    profile = "transport-webrtc-nvenc-av1-nvdec-4k120-waitable"
    transport = "webrtc"
    encode = "nvenc_av1"
    decode = "nvdec_av1"
    width = 3840
    height = 2160
    fps = 120
    bitrate_bps = 120000000
    av1_mode = "high_refresh"
    threshold_file = "transport.4k120.json"
  },
  [pscustomobject]@{
    path = "tests/benchmarks/scenarios/quick.transport.webrtc.software_vvc.2k144.json"
    profile = "transport-webrtc-software-vvc-2k144"
    transport = "webrtc"
    encode = "software_vvc"
    decode = "ffmpeg_vvc"
    threshold_file = "transport.software-vvc.2k144.json"
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
  if ($spec.PSObject.Properties.Name -contains "threshold_file" -and $scenario.threshold_file -ne $spec.threshold_file) { throw "$($spec.path) threshold file mismatch" }
  if ($spec.PSObject.Properties.Name -contains "av1_mode" -and $scenario.av1_mode -ne $spec.av1_mode) { throw "$($spec.path) AV1 mode mismatch" }
  $expectedWidth = if ($spec.PSObject.Properties.Name -contains "width") { [int]$spec.width } else { 2560 }
  $expectedHeight = if ($spec.PSObject.Properties.Name -contains "height") { [int]$spec.height } else { 1440 }
  if ($scenario.width -ne $expectedWidth -or $scenario.height -ne $expectedHeight) { throw "$($spec.path) should be ${expectedWidth}x${expectedHeight}" }
  if ($spec.PSObject.Properties.Name -contains "bitrate_bps" -and $scenario.bitrate_bps -ne $spec.bitrate_bps) { throw "$($spec.path) bitrate mismatch" }
  if ($spec.path -like "*.2k144*.json") {
    if ($scenario.fps -ne 144) { throw "$($spec.path) should target 144fps" }
    if ($spec.path -like "*.2k144.waitable.json") {
      if (-not $scenario.d3d11_waitable_object) { throw "$($spec.path) should enable waitable object" }
      if ($scenario.render_thread_priority -ne "above_normal") { throw "$($spec.path) should set render thread priority" }
    }
  } elseif ($spec.path -like "*.4k120*.json") {
    if ($scenario.fps -ne 120) { throw "$($spec.path) should target 120fps" }
    if ($scenario.source_id -ne "windows:display-shared:1") { throw "$($spec.path) should target the 4K120 capture source" }
    if ($scenario.display_id -ne "windows:display-shared:1") { throw "$($spec.path) should target the 4K120 render display" }
    if (-not $scenario.d3d11_waitable_object) { throw "$($spec.path) should enable waitable object" }
    if ($scenario.render_thread_priority -ne "above_normal") { throw "$($spec.path) should set render thread priority" }
  } elseif ($scenario.fps -ne 30) {
    throw "$($spec.path) should target 30fps"
  }
}

$processTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mrd-transport-process-{0}" -f ([guid]::NewGuid()))
New-Item -ItemType Directory -Force -Path $processTmp | Out-Null
try {
  $powerShellHost = Get-CurrentPowerShellExecutable
  if (-not (Test-Path $powerShellHost)) { throw "Current PowerShell executable should resolve to an existing file" }
  $stdout = Join-Path $processTmp "stdout.log"
  $stderr = Join-Path $processTmp "stderr.log"
  $exitCode = Invoke-TransportMatrixCommand `
    -FilePath $powerShellHost `
    -ArgumentList @("-NoProfile", "-Command", "Start-Sleep -Milliseconds 250; Write-Output 'stdout-ok'; Write-Output ([System.Diagnostics.Process]::GetCurrentProcess().PriorityClass); [Console]::Error.WriteLine('stderr-ok'); exit 7") `
    -WorkingDirectory $processTmp `
    -StdoutPath $stdout `
    -StderrPath $stderr

  if ($exitCode.ExitCode -ne 7) { throw "Invoke-TransportMatrixCommand should return the native exit code" }
  if ($exitCode.TimedOut) { throw "Invoke-TransportMatrixCommand should not mark a completed command as timed out" }
  if ((Get-Content $stdout -Raw) -notmatch "stdout-ok") { throw "Invoke-TransportMatrixCommand should capture stdout" }
  if (
    [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT -and
    (Get-Content $stdout -Raw) -notmatch "AboveNormal"
  ) {
    throw "Invoke-TransportMatrixCommand should run benchmark commands above normal priority on Windows"
  }
  if ((Get-Content $stderr -Raw) -notmatch "stderr-ok") { throw "Invoke-TransportMatrixCommand should capture stderr" }

  $timeoutResult = Invoke-TransportMatrixCommand `
    -FilePath $powerShellHost `
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
    target_bitrate_kbps = 120000.0
    encoded_fps = 143.9
    decoded_fps = 143.5
    zero_copy_enabled = $true
    total_bitstream_bytes = 75000000
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
    render_submit_wait_p95_ms = 0.03
    render_execute_p95_ms = 0.17
    render_prepare_wait_p95_ms = 0.01
    render_shared_resource_p95_ms = 0.06
    render_draw_present_p95_ms = 0.11
    render_present_p95_ms = 8.0
    render_submitted_frames = 2849
    render_uploaded_frames = 2841
    render_presented_frames = 2839
    render_present_skipped_frames = 2
    render_queue_replacements = 7
    render_stale_frame_drops = 7
    swap_chain_max_frame_latency = 1
    swap_chain_allow_tearing = $true
    swap_chain_waitable_object = $true
    swap_chain_present_mode = "waitable"
    display_refresh_hz = 144
    render_thread_priority = "above_normal"
    render_pixel_format = "D3D11SharedP010"
    color_mode = "grayscale"
    color_pipeline = "sdr8"
    nvdec_shared_copy_attempts = 2800
    nvdec_shared_copy_successes = 2799
    nvdec_shared_copy_failures = 1
    nvdec_shared_copy_last_stage = "success"
    nvdec_shared_copy_last_api = "cuda-d3d11-copy"
    nvdec_shared_copy_last_error = "shared texture copy succeeded"
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
  New-Item -ItemType Directory -Force -Path (Join-Path $summaryTmp "logs") | Out-Null
  @"
warning: failed to save last-use data
database or disk is full
     Running unittests src\main.rs (target\release\deps\app-test.exe)
"@ | Set-Content -Path (Join-Path $summaryTmp "logs/host.stderr.log") -Encoding Ascii

  & (Join-Path $scriptDir "summarize_transport_results.ps1") -RunDir $summaryTmp

  $schema = Get-Content (Join-Path $repoRoot "tests/benchmarks/schemas/benchmark-result.schema.json") -Raw | ConvertFrom-Json
  $schemaProperties = @($schema.properties.PSObject.Properties.Name)
  $summarized = Get-Content (Join-Path $summaryTmp "summary.json") -Raw | ConvertFrom-Json
  if ($summarized.error_count -ne 0) { throw "cargo cache warning text must not be counted as benchmark errors" }
  if ($summarized.warning_count -ne 0) { throw "cargo build diagnostics must not be counted as runtime warnings" }
  $extraProperties = @($summarized.PSObject.Properties.Name | Where-Object { $_ -notin $schemaProperties })
  if ($extraProperties.Count -gt 0) {
    throw "benchmark summary schema is missing properties: $($extraProperties -join ', ')"
  }

  $csv = Import-Csv (Join-Path $summaryTmp "summary.csv")
  if ($csv.run_status -ne "PASS") { throw "summary CSV must expose PASS run_status" }
  if ($csv.target_bitrate_kbps -ne "120000") { throw "summary CSV must include target bitrate" }
  if ($csv.encoded_fps -ne "143.9") { throw "summary CSV must include encoded FPS" }
  if ($csv.decoded_fps -ne "143.5") { throw "summary CSV must include decoded FPS" }
  if ($csv.zero_copy_enabled -ne "True") { throw "summary CSV must include zero-copy status" }
  if ($csv.total_bitstream_bytes -ne "75000000") { throw "summary CSV must include bitstream byte count" }
  if ($csv.render_queue_replacements -ne "7") { throw "summary CSV must include render queue replacements" }
  if ($csv.render_stale_frame_drops -ne "7") { throw "summary CSV must include render stale frame drops" }
  if ($csv.render_queue_replacement_rate -ne "0.35") { throw "summary CSV must include render queue replacement rate" }
  if ($csv.render_stale_frame_drop_rate -ne "0.35") { throw "summary CSV must include render stale frame drop rate" }
  if ($csv.render_present_skipped_rate -ne "0.1") { throw "summary CSV must include render skipped frame rate" }
  if ($csv.render_submit_wait_p95_ms -ne "0.03") { throw "summary CSV must include render submit wait p95" }
  if ($csv.render_execute_p95_ms -ne "0.17") { throw "summary CSV must include render execute p95" }
  if ($csv.render_prepare_wait_p95_ms -ne "0.01") { throw "summary CSV must include render prepare wait p95" }
  if ($csv.render_shared_resource_p95_ms -ne "0.06") { throw "summary CSV must include render shared resource p95" }
  if ($csv.render_draw_present_p95_ms -ne "0.11") { throw "summary CSV must include render draw present p95" }
  if ($csv.swap_chain_max_frame_latency -ne "1") { throw "summary CSV must include swapchain max frame latency" }
  if ($csv.swap_chain_allow_tearing -ne "True") { throw "summary CSV must include swapchain tearing policy" }
  if ($csv.swap_chain_present_mode -ne "waitable") { throw "summary CSV must include swapchain present mode" }
  if ($csv.display_refresh_hz -ne "144") { throw "summary CSV must include display refresh hz" }
  if ($csv.render_pixel_format -ne "D3D11SharedP010") { throw "summary CSV must include render pixel format" }
  if ($csv.color_mode -ne "grayscale") { throw "summary CSV must include color mode" }
  if ($csv.color_pipeline -ne "sdr8") { throw "summary CSV must include color pipeline" }
  if ($csv.nvdec_shared_copy_attempts -ne "2800") { throw "summary CSV must include NVDEC shared copy attempts" }
  if ($csv.nvdec_shared_copy_successes -ne "2799") { throw "summary CSV must include NVDEC shared copy successes" }
  if ($csv.nvdec_shared_copy_failures -ne "1") { throw "summary CSV must include NVDEC shared copy failures" }
  if ($csv.nvdec_shared_copy_last_stage -ne "success") { throw "summary CSV must include NVDEC shared copy last stage" }
  if ($csv.nvdec_shared_copy_last_api -ne "cuda-d3d11-copy") { throw "summary CSV must include NVDEC shared copy last API" }
  if ($csv.nvdec_shared_copy_last_error -ne "shared texture copy succeeded") { throw "summary CSV must include NVDEC shared copy last error" }
  $report = Get-Content (Join-Path $summaryTmp "reports/markdown-report.md") -Raw
  if ($report -notmatch "target_bitrate_kbps \\| 120000") { throw "markdown report must include target bitrate" }
  if ($report -notmatch "encoded_fps \\| 143.9") { throw "markdown report must include encoded FPS" }
  if ($report -notmatch "zero_copy_enabled \\| True") { throw "markdown report must include zero-copy status" }
  if ($report -notmatch "total_bitstream_bytes \\| 75000000") { throw "markdown report must include bitstream bytes" }
  if ($report -notmatch "swap_chain_max_frame_latency \\| 1") { throw "markdown report must include swapchain max frame latency" }
  if ($report -notmatch "swap_chain_allow_tearing \\| True") { throw "markdown report must include swapchain tearing policy" }
  if ($report -notmatch "swap_chain_present_mode \\| waitable") { throw "markdown report must include swapchain present mode" }
  if ($report -notmatch "render_pixel_format \\| D3D11SharedP010") { throw "markdown report must include render pixel format" }
  if ($report -notmatch "color_mode \\| grayscale") { throw "markdown report must include color mode" }
  if ($report -notmatch "color_pipeline \\| sdr8") { throw "markdown report must include color pipeline" }
  if ($report -notmatch "nvdec_shared_copy_attempts \\| 2800") { throw "markdown report must include NVDEC shared copy attempts" }
  if ($report -notmatch "nvdec_shared_copy_failures \\| 1") { throw "markdown report must include NVDEC shared copy failures" }
  if ($report -notmatch "nvdec_shared_copy_last_error \\| shared texture copy succeeded") { throw "markdown report must include NVDEC shared copy last error" }

  $thresholdPath = Join-Path $summaryTmp "strict-thresholds.json"
  $strictInput = Get-Content (Join-Path $summaryTmp "summary.json") -Raw | ConvertFrom-Json
  $strictInput | Add-Member -Force -NotePropertyName encode_total_p95_ms -NotePropertyValue $null
  $strictInput | Add-Member -Force -NotePropertyName send_write_p95_ms -NotePropertyValue $null
  $strictInput | Add-Member -Force -NotePropertyName decode_total_p95_ms -NotePropertyValue $null
  $strictInput | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $summaryTmp "summary.json") -Encoding Ascii
  [ordered]@{
    max_first_frame_time_ms = 5000
    min_fps_observed = 120.0
    max_encode_total_p95_ms = 8.0
    max_send_write_p95_ms = 8.0
    max_decode_total_p95_ms = 8.0
    max_render_execute_p95_ms = 0.1
    max_render_prepare_wait_p95_ms = 0.005
    max_render_shared_resource_p95_ms = 0.05
    max_render_draw_present_p95_ms = 0.1
    max_render_present_p95_ms = 7.0
    max_render_queue_replacements = 3
    max_render_stale_frame_drops = 3
    max_render_present_skipped_frames = 1
    max_render_present_skipped_rate = 0.05
    max_warning_count = 20
    max_error_count = 0
  } | ConvertTo-Json -Depth 8 | Set-Content -Path $thresholdPath -Encoding Ascii

  & (Join-Path $scriptDir "summarize_transport_results.ps1") -RunDir $summaryTmp -ThresholdPath $thresholdPath
  $strict = Get-Content (Join-Path $summaryTmp "summary.json") -Raw | ConvertFrom-Json
  if ($strict.run_passed) { throw "strict render threshold should fail the summary" }
  if ($strict.failure_reason -notmatch "render present p95") { throw "failure reason should include render present threshold" }
  if ($strict.failure_reason -notmatch "render draw/present p95") { throw "failure reason should include render draw/present threshold" }
  if ($strict.failure_reason -notmatch "render execute p95") { throw "failure reason should include render execute threshold" }
  if ($strict.failure_reason -notmatch "render shared resource p95") { throw "failure reason should include render shared resource threshold" }
  if ($strict.failure_reason -notmatch "render queue replacements") { throw "failure reason should include render queue replacement threshold" }
  if ($strict.failure_reason -notmatch "render present skipped rate") { throw "failure reason should include render present skipped rate threshold" }
  if ($strict.failure_reason -notmatch "required evidence missing or non-finite") { throw "missing transport evidence should fail closed" }
  $strictCsv = Import-Csv (Join-Path $summaryTmp "summary.csv")
  if ($strictCsv.run_status -ne "FAIL") { throw "strict threshold CSV should expose FAIL run_status" }

  $openglReadbackEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
    opengl_allow_readback_fallback = $true
  })
  if ($openglReadbackEnv["MRD_OPENGL_ALLOW_READBACK_FALLBACK"] -ne "1") {
    throw "OpenGL readback fallback opt-in should set MRD_OPENGL_ALLOW_READBACK_FALLBACK=1"
  }

  $openglReadbackDisabledEnv = Get-TransportMatrixRenderEnvironment -Scenario ([pscustomobject]@{
    opengl_allow_readback_fallback = $false
  })
  if ($openglReadbackDisabledEnv["MRD_OPENGL_ALLOW_READBACK_FALLBACK"] -ne "0") {
    throw "OpenGL readback fallback explicit opt-out should set MRD_OPENGL_ALLOW_READBACK_FALLBACK=0"
  }
} finally {
  Remove-Item $summaryTmp -Recurse -Force -ErrorAction SilentlyContinue
}
