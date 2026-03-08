param(
  [int]$DurationSec = 60,
  [string]$Tag = ("rust.pipeline.best." + (Get-Date -Format "yyyyMMdd_HHmmss")),
  [string]$Transport = "quic",
  [string]$Decoder = "d3d11va",
  [int]$AgentCaptureFps = 60,
  [int]$AgentCaptureBitrateKbps = 22000,
  [int]$AgentCaptureMaxBitrateKbps = 36000
)

$ErrorActionPreference = "Stop"

$base = "J:/ProjectTest/remote-desktop/mini-remote-desktop"
$runner = Join-Path $base "controller-rust/scripts/run_rust_pipeline_quic_60s.ps1"

powershell -ExecutionPolicy Bypass -File $runner `
  -DurationSec $DurationSec `
  -Tag $Tag `
  -Transport $Transport `
  -Decoder $Decoder `
  -AgentCaptureFps $AgentCaptureFps `
  -AgentCaptureMinFps $AgentCaptureFps `
  -AgentCaptureMaxFps $AgentCaptureFps `
  -AgentCaptureBitrateKbps $AgentCaptureBitrateKbps `
  -AgentCaptureMaxBitrateKbps $AgentCaptureMaxBitrateKbps `
  -AgentCaptureNetworkFloorKbps 12000 `
  -AgentCaptureNetworkCeilingKbps 60000 `
  -AgentCaptureEncoderPreset p6 `
  -AgentCaptureEncoderTune hq `
  -AgentCapturePerfProfile quality `
  -AgentCaptureProfileTemplate quality_first `
  -AgentCaptureFpsMode balanced `
  -RenderMode low_latency `
  -RenderDropOld 1 `
  -RenderMaxAgeMs 20 `
  -PresentAdaptive 1 `
  -PresentAdaptiveMinFps 60 `
  -PresentAdaptiveMaxFps 144 `
  -DecodeSelect adaptive-age `
  -DecodeAdaptiveMaxAgeMs 20 `
  -TargetFps 60 `
  -QuicRxQueue 6

