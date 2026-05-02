param(
    [int]$Duration = 5,
    [string]$ResultsDir = "target\pipeline-compare",
    [string]$Capture = "dxgi",
    [string]$Codec = "av1",
    [string]$Transport = "quic",
    [switch]$Software,
    [switch]$SkipDecode
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$resultsPath = Join-Path $repoRoot $ResultsDir
New-Item -ItemType Directory -Force -Path $resultsPath | Out-Null

function Test-SoftwareCodecRequest {
    $codecName = $Codec.ToLowerInvariant()
    return $Software `
        -or $codecName -eq "openh264" `
        -or $codecName -eq "software_h264" `
        -or $codecName -eq "software-h264" `
        -or $codecName -eq "h264_software" `
        -or $codecName -eq "h264-software"
}

function Set-HarnessEnv {
    param(
        [string]$Pipeline,
        [string]$Encoder,
        [string]$Decoder,
        [string]$ResultPath,
        [bool]$NativeRender = $true,
        [bool]$RequireDecode = $false,
        [bool]$ZeroCopy = $true
    )

    $env:MRD_HARNESS_CHAIN = "custom"
    $env:MRD_HARNESS_CAPTURE = $Capture
    $env:MRD_HARNESS_ENCODER = $Encoder
    $env:MRD_HARNESS_DECODER = $Decoder
    $env:MRD_HARNESS_RENDERER = if ($NativeRender) { "d3d11" } else { "none" }
    $env:MRD_HARNESS_ZERO_COPY = if ($ZeroCopy) { "1" } else { "0" }
    $env:MRD_HARNESS_TRANSPORT = $Transport
    $env:MRD_HARNESS_PROBE_SECONDS = "$Duration"
    $env:MRD_HARNESS_RESULT_PATH = $ResultPath
    $env:MRD_HARNESS_PIPELINE = $Pipeline
    $env:MRD_HARNESS_REQUIRE_DECODE = if ($RequireDecode) { "1" } else { "0" }
}

function Clear-HarnessEnv {
    "MRD_HARNESS_CHAIN",
    "MRD_HARNESS_CAPTURE",
    "MRD_HARNESS_ENCODER",
    "MRD_HARNESS_DECODER",
    "MRD_HARNESS_RENDERER",
    "MRD_HARNESS_ZERO_COPY",
    "MRD_HARNESS_TRANSPORT",
    "MRD_HARNESS_PROBE_SECONDS",
    "MRD_HARNESS_RESULT_PATH",
    "MRD_HARNESS_PIPELINE",
    "MRD_HARNESS_REQUIRE_DECODE" | ForEach-Object {
        Remove-Item "Env:\$_" -ErrorAction SilentlyContinue
    }
}

function Invoke-RdeskPipeline {
    param(
        [string]$Pipeline,
        [string]$Encoder,
        [string]$Decoder,
        [bool]$RequireDecode = $false
    )

    $stamp = Get-Date -Format "yyyyMMdd_HHmmss"
    $resultPath = Join-Path $resultsPath ("rdesk_{0}_{1}.json" -f $Pipeline, $stamp)
    $stdoutPath = Join-Path $resultsPath ("rdesk_{0}_{1}.stdout.log" -f $Pipeline, $stamp)
    $stderrPath = Join-Path $resultsPath ("rdesk_{0}_{1}.stderr.log" -f $Pipeline, $stamp)
    $zeroCopy = -not (Test-SoftwareCodecRequest)
    Set-HarnessEnv `
        -Pipeline $Pipeline `
        -Encoder $Encoder `
        -Decoder $Decoder `
        -ResultPath $resultPath `
        -NativeRender ($Pipeline -ne "capture-encode") `
        -RequireDecode $RequireDecode `
        -ZeroCopy $zeroCopy

    Write-Host "Running $Pipeline..."
    $process = Start-Process `
        -FilePath "cargo" `
        -ArgumentList @("test", "-p", "app", "nvenc_nvdec_harness_prints_stage_metrics", "--", "--ignored", "--nocapture") `
        -Wait `
        -PassThru `
        -NoNewWindow `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath
    if ($process.ExitCode -ne 0) {
        Write-Host "stdout: $stdoutPath"
        Write-Host "stderr: $stderrPath"
        throw "$Pipeline failed with exit code $($process.ExitCode)"
    }
    if (-not (Test-Path $resultPath)) {
        throw "$Pipeline completed but did not produce $resultPath"
    }

    Write-Host "  metrics: $resultPath"
    return $resultPath
}

function Show-PipelineComparison {
    param([string[]]$ResultPaths)

    $rows = foreach ($path in $ResultPaths) {
        $json = Get-Content $path -Raw | ConvertFrom-Json
        [pscustomobject]@{
            pipeline = $json.pipeline
            codec = $json.codec
            memory_path = $json.memory_path
            transport = $json.transport
            frames = $json.frames
            encoded_units = $json.encoded_units
            decoded_frames = $json.decoded_frames
            encode_failures = $json.encode_failures
            decode_failures = $json.decode_failures
            avg_capture_ms = "{0:N3}" -f $json.avg_capture_time_ms
            avg_encode_ms = "{0:N3}" -f $json.avg_encode_time_ms
            avg_transport_ms = "{0:N3}" -f $json.avg_transport_time_ms
            avg_decode_ms = "{0:N3}" -f $json.avg_decode_time_ms
            avg_render_ms = "{0:N3}" -f $json.avg_render_time_ms
            avg_present_ms = "{0:N3}" -f $json.avg_present_time_ms
            avg_total_ms = "{0:N3}" -f $json.avg_total_time_ms
            fps = "{0:N2}" -f $json.avg_fps
            bytes = $json.total_bitstream_bytes
        }
    }

    $format = "{0,-31} {1,-5} {2,-13} {3,-13} {4,6} {5,6} {6,6} {7,8} {8,8} {9,7} {10,8} {11,8} {12,8} {13,8} {14,8} {15,10}"
    Write-Host ($format -f "pipeline", "codec", "memory", "transport", "frames", "enc", "dec", "cap", "encode", "tx", "decode", "render", "present", "total", "fps", "bytes")
    Write-Host ($format -f ("-" * 31), ("-" * 5), ("-" * 13), ("-" * 13), ("-" * 6), ("-" * 6), ("-" * 6), ("-" * 8), ("-" * 8), ("-" * 7), ("-" * 8), ("-" * 8), ("-" * 8), ("-" * 8), ("-" * 8), ("-" * 10))
    foreach ($row in $rows) {
        Write-Host ($format -f $row.pipeline, $row.codec, $row.memory_path, $row.transport, $row.frames, $row.encoded_units, $row.decoded_frames, $row.avg_capture_ms, $row.avg_encode_ms, $row.avg_transport_ms, $row.avg_decode_ms, $row.avg_render_ms, $row.avg_present_ms, $row.avg_total_ms, $row.fps, $row.bytes)
    }
}

try {
    $compareInputs = @()
    $compareInputs += Invoke-RdeskPipeline -Pipeline "capture-render" -Encoder "none" -Decoder "none"

    $codecName = $Codec.ToLowerInvariant()
    if ($Software -or $codecName -eq "openh264" -or $codecName -eq "software_h264" -or $codecName -eq "software-h264" -or $codecName -eq "h264_software" -or $codecName -eq "h264-software") {
        $encoder = "openh264"
        $decoder = "software"
    } elseif ($codecName -eq "av1") {
        $encoder = "nvenc_av1"
        $decoder = "nvdec"
    } elseif ($codecName -eq "h264") {
        $encoder = "nvenc_h264"
        $decoder = "nvdec"
    } elseif ($codecName -eq "hevc") {
        $encoder = "nvenc_hevc"
        $decoder = "nvdec"
    } elseif ($codecName -eq "hevc-main10" -or $codecName -eq "hevc_main10" -or $codecName -eq "main10") {
        $encoder = "nvenc_hevc_main10"
        $decoder = "nvdec"
    } else {
        throw "Unsupported codec '$Codec'. Supported values: av1, h264, hevc, hevc-main10, openh264, software-h264"
    }

    $compareInputs += Invoke-RdeskPipeline -Pipeline "capture-encode" -Encoder $encoder -Decoder "none"

    if (-not $SkipDecode) {
        $compareInputs += Invoke-RdeskPipeline `
            -Pipeline "capture-encode-decode-render" `
            -Encoder $encoder `
            -Decoder $decoder `
            -RequireDecode $true
    }

    Write-Host ""
    Show-PipelineComparison -ResultPaths $compareInputs
} finally {
    Clear-HarnessEnv
}
