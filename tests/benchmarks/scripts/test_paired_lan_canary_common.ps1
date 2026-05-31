$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "paired_lan_canary_common.ps1")

function Assert-Equal($Actual, $Expected, [string]$Message) {
  if ($Actual -ne $Expected) {
    throw "$Message. Expected '$Expected', got '$Actual'"
  }
}

function Assert-True($Condition, [string]$Message) {
  if (-not $Condition) {
    throw $Message
  }
}

function Find-Profile([object[]]$Profiles, [string]$Id) {
  $match = @($Profiles | Where-Object { $_.id -eq $Id })
  if ($match.Count -ne 1) {
    throw "Expected exactly one profile '$Id', got $($match.Count)"
  }
  $match[0]
}

$profiles = Get-PairedLanCanaryProfiles -DurationSecs 30 -BitrateMbps 20
Assert-Equal $profiles.Count 14 "Profile count"
Assert-Equal $profiles[0].id "1080p60" "First profile id"
Assert-Equal $profiles[2].id "2k144" "2K144 profile is present"
Assert-Equal $profiles[3].id "2k144_adaptive" "Adaptive 2K144 profile is present"
Assert-Equal $profiles[3].bitrate_mbps 80 "Adaptive 2K144 profile uses the 80 Mbps ceiling"
Assert-True $profiles[3].adaptive "Adaptive 2K144 profile enables adaptive autorun"
Assert-Equal $profiles[4].id "4k120" "4K120 profile is present"
Assert-Equal $profiles[4].width 3840 "4K120 profile width"
Assert-Equal $profiles[4].height 2160 "4K120 profile height"
Assert-Equal $profiles[4].fps 120 "4K120 profile fps"
Assert-Equal $profiles[4].bitrate_mbps 120 "4K120 profile uses high HEVC bitrate"
Assert-Equal $profiles[5].id "2k180" "Native 2K180 profile is present"
Assert-Equal $profiles[5].bitrate_mbps 100 "Native 2K180 profile uses the higher default bitrate"
Assert-Equal $profiles[6].id "2k180_120mbps" "Native 2K180 high-bitrate profile is present"
Assert-Equal $profiles[6].bitrate_mbps 120 "Native 2K180 high-bitrate profile reaches 120 Mbps"
Assert-Equal $profiles[7].id "2k180_120mbps_adaptive" "Native 2K180 adaptive high-bitrate profile is present"
Assert-Equal $profiles[7].bitrate_mbps 120 "Native 2K180 adaptive profile starts from 120 Mbps"
Assert-True $profiles[7].adaptive "Native 2K180 high-bitrate profile enables adaptive autorun"
Assert-Equal $profiles[8].id "1600p165" "Native 1600p165 profile is present"
Assert-Equal $profiles[8].bitrate_mbps 80 "Native 1600p165 profile uses the higher default bitrate"
Assert-Equal $profiles[9].id "1600p165_120mbps" "Native 1600p165 high-bitrate profile is present"
Assert-Equal $profiles[9].bitrate_mbps 120 "Native 1600p165 high-bitrate profile reaches 120 Mbps"
Assert-Equal $profiles[10].id "1600p165_120mbps_adaptive" "Native 1600p165 adaptive high-bitrate profile is present"
Assert-Equal $profiles[10].bitrate_mbps 120 "Native 1600p165 adaptive profile starts from 120 Mbps"
Assert-True $profiles[10].adaptive "Native 1600p165 high-bitrate profile enables adaptive autorun"
Assert-Equal $profiles[12].fps 180 "180 FPS profile is present"
Assert-Equal $profiles[13].fps 249 "249 FPS profile is present"

$h264CrossChain = New-CanaryMediaChain -Mode "cross" -Codec "h264"
Assert-Equal $h264CrossChain "dxgi/nvenc_h264/quic_datagram_media_v3_or_v2/nvdec/d3d11_shared" "H.264 cross chain remains the default"
$hevcCrossChain = New-CanaryMediaChain -Mode "cross" -Codec "hevc"
Assert-Equal $hevcCrossChain "dxgi/nvenc_hevc/quic_datagram_media_v3_or_v2/nvdec_hevc_d3d11_shared/d3d11_shared" "HEVC cross chain uses HEVC encode/decode labels"
$av1LocalChain = New-CanaryMediaChain -Mode "local" -Codec "av1"
Assert-Equal $av1LocalChain "dxgi/nvenc_av1/quic/nvdec_av1/d3d11_shared" "AV1 local chain uses AV1 encode/decode labels"
$av1CrossChain = New-CanaryMediaChain -Mode "cross" -Codec "av1"
Assert-Equal $av1CrossChain "dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared" "AV1 cross chain uses AV1 encode/decode labels"
$av1LocalDualChain = New-CanaryMediaChain -Mode "local-dual-process" -Codec "av1"
Assert-Equal $av1LocalDualChain "local_dual_process/dxgi/nvenc_av1/quic_datagram_media_v3_or_v2/nvdec_av1/d3d11_shared" "AV1 local dual chain uses AV1 encode/decode labels"
Assert-Equal (Normalize-CanaryCodec "av1") "av1" "AV1 codec normalizes to av1"

$localSummaryRow = Convert-LocalSummaryToCanaryRow `
  -Profile ([pscustomobject]@{ id = "2k144"; width = 2560; height = 1440; fps = 144; bitrate_mbps = 80; duration_secs = 20 }) `
  -Summary ([pscustomobject]@{
    run_passed = $true
    fps_observed = 143.4
    width = 2560
    height = 1440
    fps_target = 144
    session_established = $true
    first_frame_seen = $true
    first_frame_time_ms = 81.0
    dropped_frames = 0
    encode_total_p95_ms = 0.36
    send_write_p95_ms = 1.72
    decode_total_p95_ms = 1.41
    frame_sink_ingest_p95_ms = 4.0
    render_upload_p95_ms = 0.25
    render_submit_wait_p95_ms = 0.07
    render_execute_p95_ms = 0.18
    render_prepare_wait_p95_ms = 0.02
    render_shared_resource_p95_ms = 0.09
    render_draw_present_p95_ms = 0.11
    render_present_p95_ms = 8.01
  }) `
  -SummaryPath "raw/local-2k144.json"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_present_gap' 8.01 "Local summary exposes render present as canonical present-gap P95"
Assert-Equal $localSummaryRow.render_present_gap_p95_ms 8.01 "Local summary carries render_present_gap_p95_ms for reports"
Assert-Equal $localSummaryRow.stage_p95_ms.present 8.01 "Local summary keeps present P95 compatibility alias"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_submit_wait' 0.07 "Local summary exposes render submit wait P95"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_execute' 0.18 "Local summary exposes renderer execute P95"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_prepare_wait' 0.02 "Local summary exposes render prepare wait P95"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_shared_resource' 0.09 "Local summary exposes render shared resource P95"
Assert-Equal $localSummaryRow.stage_p95_ms.'render_draw_present' 0.11 "Local summary exposes render draw/present P95"

$failedLocalSummaryRow = Convert-LocalSummaryToCanaryRow `
  -Profile ([pscustomobject]@{ id = "2k144"; width = 2560; height = 1440; fps = 144; bitrate_mbps = 80; duration_secs = 20 }) `
  -Summary ([pscustomobject]@{
    run_passed = $false
    failure_reason = "render present collapse: presented 3 of 654 render frames, minimum 65"
    fps_observed = 106.8
    width = 2560
    height = 1440
    fps_target = 144
    session_established = $true
    first_frame_seen = $true
    first_frame_time_ms = 384.0
    dropped_frames = 0
    encode_total_p95_ms = 5.8
    send_write_p95_ms = 0.2
    decode_total_p95_ms = 6.1
    render_present_p95_ms = 537.4
  }) `
  -SummaryPath "raw/local-2k144-failed.json"
Assert-Equal $failedLocalSummaryRow.error_message "render present collapse: presented 3 of 654 render frames, minimum 65" "Failed local canary rows carry benchmark failure reason"

$localThresholdMissRow = Convert-LocalSummaryToCanaryRow `
  -Profile ([pscustomobject]@{ id = "2k144"; width = 2560; height = 1440; fps = 144; bitrate_mbps = 80; duration_secs = 20 }) `
  -Summary ([pscustomobject]@{
    run_passed = $false
    failure_reason = "render execute p95 16.5ms exceeded 8.0ms"
    fps_observed = 143.0
    width = 2560
    height = 1440
    fps_target = 144
    session_established = $true
    first_frame_seen = $true
    first_frame_time_ms = 120.0
    dropped_frames = 0
  }) `
  -SummaryPath "raw/local-2k144-threshold.json"
Assert-Equal $localThresholdMissRow.classification "threshold_miss" "Failed local threshold breaches are classified as threshold misses"

$localRow = [pscustomobject]@{
  id = "1080p144"
  width = 1920
  height = 1080
  fps = 144
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 140.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 144; bitrate_mbps = 20 }
}
$crossRow = [pscustomobject]@{
  id = "1080p144"
  width = 1920
  height = 1080
  fps = 144
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 120.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 144; bitrate_mbps = 20 }
}

$comparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($crossRow) -RatioThreshold 0.8
Assert-Equal $comparison[0].status "completed" "Cross row above 80 percent passes"
Assert-Equal ([Math]::Round($comparison[0].fps_ratio, 3)) 0.857 "FPS ratio is calculated"

$slowCrossRow = $crossRow.PSObject.Copy()
$slowCrossRow.fps_observed = 100.0
$slowComparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($slowCrossRow) -RatioThreshold 0.8
Assert-Equal $slowComparison[0].status "threshold_miss" "Cross row below 80 percent is threshold_miss"

$local60Row = [pscustomobject]@{
  id = "1080p60"
  width = 1920
  height = 1080
  fps = 60
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 144.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 60; bitrate_mbps = 20 }
}
$cross60Row = [pscustomobject]@{
  id = "1080p60"
  width = 1920
  height = 1080
  fps = 60
  bitrate_mbps = 20
  status = "completed"
  classification = "completed"
  fps_observed = 56.0
  selected_profile = [pscustomobject]@{ width = 1920; height = 1080; fps = 60; bitrate_mbps = 20 }
}
$cappedComparison = Compare-PairedLanCanaryRows -LocalRows @($local60Row) -CrossRows @($cross60Row) -RatioThreshold 0.8
Assert-Equal $cappedComparison[0].status "completed" "Comparison caps local baseline to requested FPS"
Assert-Equal ([Math]::Round($cappedComparison[0].fps_ratio, 3)) 0.933 "Capped FPS ratio uses requested FPS"

$downgradedCrossRow = $crossRow.PSObject.Copy()
$downgradedCrossRow.selected_profile = [pscustomobject]@{ width = 1728; height = 1080; fps = 144; bitrate_mbps = 20 }
$downgradeComparison = Compare-PairedLanCanaryRows -LocalRows @($localRow) -CrossRows @($downgradedCrossRow) -RatioThreshold 0.8
Assert-Equal $downgradeComparison[0].status "profile_downgraded" "Profile mismatch is classified as downgrade"
Assert-True (-not $downgradeComparison[0].comparable) "Profile downgraded rows are not comparable"

$peerMissingReport = [pscustomobject]@{
  status = "failed"
  failureReason = "peer_not_found"
  errorMessage = "No LAN peer available"
  probeSnapshot = $null
  mediaPipelineSnapshot = $null
  sessionSnapshot = $null
}
$peerMissingRow = Convert-CrossReportToCanaryRow -Profile $profiles[0] -Report $peerMissingReport -ReportPath "raw/cross-1080p60.json"
Assert-Equal $peerMissingRow.status "skipped" "Missing LAN peer is an environment skip"
Assert-Equal $peerMissingRow.classification "unsupported" "Missing LAN peer is classified as unsupported"

$displayLimitedReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 144.0
  displayModeChange = [pscustomobject]@{
    active = [pscustomobject]@{ width = 1920; height = 1080; refresh_hz = 144 }
  }
  probeSnapshot = [pscustomobject]@{
    current_fps = 144.0
    frames_decoded = 1440
    frames_dropped = 0
    media_probe_width = 1920
    media_probe_height = 1080
    media_probe_target_fps = 249
    media_probe_target_bitrate_mbps = 20
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @()
    test_impairment = $null
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$displayLimitedRow = Convert-CrossReportToCanaryRow -Profile (Find-Profile $profiles "1080p249") -Report $displayLimitedReport -ReportPath "raw/cross-1080p249.json"
Assert-Equal $displayLimitedRow.status "skipped" "Display refresh capped profiles are environment skips"
Assert-Equal $displayLimitedRow.classification "display_refresh_limited" "Display refresh cap is classified explicitly"
Assert-True ($displayLimitedRow.error_message -match "144Hz") "Display refresh cap carries an actionable reason"

$displayLimitedComparison = Compare-PairedLanCanaryRows -LocalRows @($displayLimitedRow) -CrossRows @($crossRow) -RatioThreshold 0.8
Assert-Equal $displayLimitedComparison[0].status "display_refresh_limited" "Display-limited local rows are not compared"
Assert-True (-not $displayLimitedComparison[0].comparable) "Display-limited local rows are non-comparable"

$sampleFpsReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 57.0
  sampleFramesDecoded = 1710
  sampleFramesDropped = 1
  sampleSequenceGapDrops = 2
  sampleDecodeErrorDrops = 3
  sampleTransientDrops = 4
  probeSnapshot = [pscustomobject]@{
    current_fps = 44.0
    frames_decoded = 484
    frames_dropped = 0
    sequence_gap_drops = 5
    decode_error_drops = 6
    transient_drops = 7
    media_probe_width = 1920
    media_probe_height = 1080
    media_probe_target_fps = 60
    media_probe_target_bitrate_mbps = 20
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @()
    test_impairment = [pscustomobject]@{
      loss_pct = 1.0
      base_delay_ms = 2
      jitter_ms = 3
      mtu_bytes = 1200
      seed = 42
      datagrams_sent = 100
      datagrams_dropped = 1
      datagrams_delayed = 80
      datagrams_fragmented_by_mtu = 10
    }
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$sampleFpsRow = Convert-CrossReportToCanaryRow -Profile $profiles[0] -Report $sampleFpsReport -ReportPath "raw/cross-1080p60.json"
Assert-Equal $sampleFpsRow.fps_observed 57.0 "Cross report prefers sample-window FPS over cumulative probe FPS"
Assert-Equal $sampleFpsRow.sample_probe_dropped_frames 1 "Cross row carries sample-window probe drops separately from cumulative drops"
Assert-Equal $sampleFpsRow.sample_sequence_gap_drops 2 "Cross row carries sample-window sequence gap drops separately"
Assert-Equal $sampleFpsRow.sample_decode_error_drops 3 "Cross row carries sample-window decode/probe error drops separately"
Assert-Equal $sampleFpsRow.sample_transient_drops 4 "Cross row carries sample-window transient drops separately"
Assert-Equal $sampleFpsRow.sequence_gap_drops 5 "Cross row keeps cumulative sequence gap drops"
Assert-Equal $sampleFpsRow.decode_error_drops 6 "Cross row keeps cumulative decode/probe error drops"
Assert-Equal $sampleFpsRow.transient_drops 7 "Cross row keeps cumulative transient drops"
Assert-Equal $sampleFpsRow.test_impairment.datagrams_dropped 1 "Cross row carries media impairment counters"

$adaptiveDowngradeReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 94.0
  sampleObservedRenderFps = 91.0
  sampleRenderFramesPresented = 2730
  probeSnapshot = [pscustomobject]@{
    current_fps = 94.0
    frames_decoded = 2835
    frames_dropped = 308
    media_probe_width = 1920
    media_probe_height = 1200
    media_probe_target_fps = 90
    media_probe_target_bitrate_mbps = 28
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    render_queue_replacements = 17
    render_stale_frame_drops = 0
    render_lock_drops = 0
    render_presented_frames = 2734
    stage_metrics = @(
      [pscustomobject]@{ stage = "render_present_gap"; p50_ms = 10.9; p95_ms = 15.0 }
    )
    test_impairment = $null
    sender_transport = [pscustomobject]@{
      datagram_fragments_attempted = 40
      datagram_fragments_sent = 38
      datagram_fragments_delayed = 0
      datagram_fragments_dropped_by_impairment = 0
      datagram_fragments_dropped_for_capacity = 2
      datagram_fragments_dropped_for_budget = 0
      datagram_frames_cut_short_for_capacity = 1
      datagram_frames_cut_short_for_budget = 0
      reliable_fragments_sent = 0
      reliable_frames_sent = 0
    }
    active_codec = "hevc"
    adaptation = [pscustomobject]@{
      current_profile = [pscustomobject]@{ width = 1920; height = 1200; fps = 90; bitrate_mbps = 28; codec = "hevc" }
      target_profile = [pscustomobject]@{ width = 1920; height = 1200; fps = 90; bitrate_mbps = 28; codec = "hevc" }
      state = "stable"
      ladder_index = 5
      last_reason = "present gap p95 exceeds 10.42ms perceptual budget"
    }
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$native1600p165AdaptiveProfile = Find-Profile $profiles "1600p165_120mbps_adaptive"
$adaptiveDowngradeRow = Convert-CrossReportToCanaryRow -Profile $native1600p165AdaptiveProfile -Report $adaptiveDowngradeReport -ReportPath "raw/cross-1600p165-adaptive.json" -RequestedCodec "hevc"
Assert-Equal $adaptiveDowngradeRow.status "completed" "Adaptive profile changes are accepted as completed rows"
Assert-Equal $adaptiveDowngradeRow.classification "completed" "Adaptive profile changes are not marked as profile downgrade"
Assert-Equal $adaptiveDowngradeRow.selected_profile.width 1920 "Adaptive row keeps the final selected width"
Assert-Equal $adaptiveDowngradeRow.active_codec "hevc" "Adaptive row keeps HEVC after downgrade"
Assert-Equal $adaptiveDowngradeRow.adaptation_state "stable" "Adaptive row exposes the final adaptation state"
Assert-Equal $adaptiveDowngradeRow.adaptation_ladder_index 5 "Adaptive row exposes the final ladder index"
Assert-Equal $adaptiveDowngradeRow.adaptation_last_reason "present gap p95 exceeds 10.42ms perceptual budget" "Adaptive row exposes the last adaptation reason"

$startupDropOnlyIssue = Get-CanaryVisualIntegrityIssue `
  -Probe ([pscustomobject]@{ frames_decoded = 4800; frames_dropped = 250 }) `
  -Pipeline ([pscustomobject]@{ render_queue_replacements = 0; render_stale_frame_drops = 0; render_lock_drops = 0; stage_metrics = @() }) `
  -Report ([pscustomobject]@{ sampleFramesDecoded = 4700; sampleFramesDropped = 1 }) `
  -Profile $native1600p165AdaptiveProfile
Assert-True ($null -eq $startupDropOnlyIssue) "Visual integrity check uses sample-window drops when available"

$hevcReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 122.0
  probeSnapshot = [pscustomobject]@{
    current_fps = 122.0
    frames_decoded = 3660
    frames_dropped = 12
    media_probe_width = 2560
    media_probe_height = 1440
    media_probe_target_fps = 144
    media_probe_target_bitrate_mbps = 80
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @(
      [pscustomobject]@{ stage = "sender.encode"; p50_ms = 0.6; p95_ms = 4.8 },
      [pscustomobject]@{ stage = "sender.send_datagram"; p50_ms = 1.1; p95_ms = 3.2 },
      [pscustomobject]@{ stage = "receiver.decode"; p50_ms = 1.3; p95_ms = 1.9 },
      [pscustomobject]@{ stage = "render_present"; p50_ms = 0.2; p95_ms = 0.4 }
    )
    test_impairment = $null
    sender_transport = [pscustomobject]@{
      datagram_fragments_attempted = 40
      datagram_fragments_sent = 38
      datagram_fragments_delayed = 0
      datagram_fragments_dropped_by_impairment = 0
      datagram_fragments_dropped_for_capacity = 2
      datagram_fragments_dropped_for_budget = 0
      datagram_frames_cut_short_for_capacity = 1
      datagram_frames_cut_short_for_budget = 0
      reliable_fragments_sent = 0
      reliable_frames_sent = 0
    }
    active_codec = "hevc"
    active_codec_profile = "main"
    active_chroma_subsampling = "4:2:0"
    active_pixel_format = "d3d11_shared_nv12"
    active_bitrate_mbps = 80
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$profile2k14480 = [pscustomobject]@{ id = "2k144"; width = 2560; height = 1440; fps = 144; bitrate_mbps = 80; duration_secs = 30 }
$hevcRow = Convert-CrossReportToCanaryRow -Profile $profile2k14480 -Report $hevcReport -ReportPath "raw/cross-2k144-hevc.json"
Assert-Equal $hevcRow.chain "dxgi/nvenc_hevc/quic_datagram_media_v3_or_v2/nvdec_hevc_d3d11_shared/d3d11_shared" "HEVC cross row reports the active HEVC chain"
Assert-Equal $hevcRow.active_codec "hevc" "HEVC cross row carries active codec"
Assert-Equal $hevcRow.active_codec_profile "main" "HEVC cross row carries active profile"
Assert-Equal $hevcRow.active_chroma_subsampling "4:2:0" "HEVC cross row carries chroma sampling"
Assert-Equal $hevcRow.active_pixel_format "d3d11_shared_nv12" "HEVC cross row carries pixel format"
Assert-Equal $hevcRow.visual_integrity_status "ok" "Healthy HEVC rows report visual integrity as ok"
Assert-Equal $hevcRow.stage_p50_ms.'sender.encode' 0.6 "HEVC row exposes sender encode P50"
Assert-Equal $hevcRow.stage_p50_ms.'sender.send_datagram' 1.1 "HEVC row exposes sender datagram send P50"
Assert-Equal $hevcRow.stage_p95_ms.'sender.send_datagram' 3.2 "HEVC row still exposes sender datagram send P95"
Assert-Equal $hevcRow.sender_transport.datagram_fragments_dropped_for_capacity 2 "HEVC row exposes sender capacity drop counters"
Assert-Equal $hevcRow.sender_transport.datagram_fragments_attempted 40 "HEVC row exposes sender datagram attempted counters"

$displaySourceReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 175.0
  captureSource = [pscustomobject]@{
    id = "windows:display-shared:1"
    source_kind = "display_shared"
    title = "Display 2 (D3D11 shared copy)"
    class_name = "DXGIShared:\\\\.\\DISPLAY2"
    width = 2560
    height = 1440
  }
  displayModeChange = [pscustomobject]@{
    active = [pscustomobject]@{
      source_id = "windows:display-shared:1"
      width = 2560
      height = 1440
      refresh_hz = 180
    }
  }
  probeSnapshot = [pscustomobject]@{
    current_fps = 175.0
    frames_decoded = 7875
    frames_dropped = 4
    media_probe_width = 2560
    media_probe_height = 1440
    media_probe_target_fps = 180
    media_probe_target_bitrate_mbps = 120
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @(
      [pscustomobject]@{ stage = "sender.capture"; p50_ms = 0.18; p95_ms = 0.28 },
      [pscustomobject]@{ stage = "sender.encode"; p50_ms = 0.42; p95_ms = 0.63 },
      [pscustomobject]@{ stage = "sender.send_datagram"; p50_ms = 0.05; p95_ms = 1.65 },
      [pscustomobject]@{ stage = "receiver.decode"; p50_ms = 1.48; p95_ms = 2.11 },
      [pscustomobject]@{ stage = "render_present_gap"; p50_ms = 5.63; p95_ms = 6.68 }
    )
    test_impairment = $null
    active_codec = "hevc"
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$displaySourceRow = Convert-CrossReportToCanaryRow -Profile (Find-Profile $profiles "2k180_120mbps") -Report $displaySourceReport -ReportPath "raw/cross-2k180.json" -RequestedCodec "hevc"
Assert-Equal $displaySourceRow.actual_capture_source_id "windows:display-shared:1" "Cross row records the actual remote display source id"
Assert-Equal $displaySourceRow.actual_capture_source_class_name "DXGIShared:\\\\.\\DISPLAY2" "Cross row records the DXGI/GDI display mapping"
Assert-Equal $displaySourceRow.active_display_mode_summary "2560x1440@180" "Cross row records active remote display mode"
Assert-Equal $displaySourceRow.capture_source_summary "windows:display-shared:1 / display_shared / 2560x1440 / Display 2 (D3D11 shared copy)" "Cross row exposes a readable remote display summary"

$localStageRow = [pscustomobject]@{
  id = "2k180_120mbps"
  width = 2560
  height = 1440
  fps = 180
  bitrate_mbps = 120
  status = "completed"
  classification = "completed"
  fps_observed = 176.0
  selected_profile = [pscustomobject]@{ width = 2560; height = 1440; fps = 180; bitrate_mbps = 120 }
  stage_p95_ms = [pscustomobject]@{
    'sender.capture' = 0.25
    'sender.encode' = 0.65
    'sender.send_datagram' = 0.8
    'receiver.decode' = 2.0
    'render_present_gap' = 6.7
  }
}
$slowSendCrossRow = $displaySourceRow.PSObject.Copy()
$slowSendCrossRow.fps_observed = 110.0
$slowSendCrossRow.stage_p95_ms = [pscustomobject]@{
  'sender.capture' = 0.3
  'sender.encode' = 0.75
  'sender.send_datagram' = 6.2
  'receiver.decode' = 2.2
  'render_present_gap' = 7.0
}
$stageComparison = Compare-PairedLanCanaryRows -LocalRows @($localStageRow) -CrossRows @($slowSendCrossRow) -RatioThreshold 0.8
Assert-Equal $stageComparison[0].status "threshold_miss" "Slow cross row remains a threshold miss"
Assert-Equal $stageComparison[0].root_cause "transport_send_p95_regression" "Comparison identifies high sender send P95 as the likely gap source"
Assert-Equal ([Math]::Round($stageComparison[0].send_delta_p95_ms, 1)) 5.4 "Comparison reports sender send P95 delta"
Assert-Equal $stageComparison[0].cross_capture_source "windows:display-shared:1 / display_shared / 2560x1440 / Display 2 (D3D11 shared copy)" "Comparison carries cross display source context"

$reliableSendReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 150.0
  probeSnapshot = [pscustomobject]@{
    current_fps = 150.0
    frames_decoded = 9000
    frames_dropped = 100
    media_probe_width = 2560
    media_probe_height = 1600
    media_probe_target_fps = 165
    media_probe_target_bitrate_mbps = 120
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    queue_depth = 0
    stage_metrics = @(
      [pscustomobject]@{ stage = "sender.encode"; p50_ms = 0.4; p95_ms = 0.8 },
      [pscustomobject]@{ stage = "sender.send_datagram"; p50_ms = 0.04; p95_ms = 0.06 },
      [pscustomobject]@{ stage = "sender.send_reliable"; p50_ms = 2.1; p95_ms = 5.4 }
    )
    test_impairment = $null
    active_codec = "hevc"
    active_bitrate_mbps = 120
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$profile1600p165120 = [pscustomobject]@{ id = "1600p165_120mbps"; width = 2560; height = 1600; fps = 165; bitrate_mbps = 120; duration_secs = 30 }
$reliableSendRow = Convert-CrossReportToCanaryRow -Profile $profile1600p165120 -Report $reliableSendReport -ReportPath "raw/cross-1600p165-120.json"
Assert-Equal $reliableSendRow.stage_p50_ms.'sender.send_reliable' 2.1 "Reliable HEVC row exposes sender reliable-send P50"
Assert-Equal (Select-CanarySenderSendStageValue -StageMap $reliableSendRow.stage_p50_ms -Fallback 0) 2.1 "Report send P50 prefers reliable send when present"
Assert-Equal (Select-CanarySenderSendStageValue -StageMap $reliableSendRow.stage_p95_ms -Fallback 0) 5.4 "Report send P95 prefers reliable send when present"

$renderCappedReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 130.0
  sampleRenderFramesPresented = 5670
  sampleObservedRenderFps = 126.0
  sampleDurationMs = 45000
  probeSnapshot = [pscustomobject]@{
    current_fps = 130.0
    frames_decoded = 5850
    frames_dropped = 0
    media_probe_width = 2560
    media_probe_height = 1600
    media_probe_target_fps = 144
    media_probe_target_bitrate_mbps = 120
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 0
    render_presented_frames = 5670
    render_queue_replacements = 0
    render_stale_frame_drops = 0
    render_lock_drops = 0
    render_pacing_target_fps = 144
    queue_depth = 1
    stage_metrics = @(
      [pscustomobject]@{ stage = "sender.encode"; p50_ms = 0.4; p95_ms = 0.8 },
      [pscustomobject]@{ stage = "sender.send_reliable"; p50_ms = 0.9; p95_ms = 1.6 },
      [pscustomobject]@{ stage = "render_present_gap"; p50_ms = 6.94; p95_ms = 7.4 }
    )
    test_impairment = $null
    active_codec = "hevc"
    active_fps = 144
    active_bitrate_mbps = 120
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$renderCappedRow = Convert-CrossReportToCanaryRow -Profile $profile1600p165120 -Report $renderCappedReport -ReportPath "raw/cross-1600p165-render-capped.json"
Assert-Equal $renderCappedRow.status "completed" "Receiver render-paced FPS cap remains a completed canary row"
Assert-Equal $renderCappedRow.classification "completed" "Receiver render-paced FPS cap is not profile_downgraded"
Assert-Equal $renderCappedRow.selected_profile.fps 144 "Receiver render-paced row reports selected media FPS"
Assert-Equal $renderCappedRow.render_pacing_target_fps 144 "Receiver render-paced row exposes pacing target"
Assert-Equal $renderCappedRow.sample_observed_render_fps 126.0 "Receiver render-paced row carries sample-window render FPS"

$local1600Row = [pscustomobject]@{
  id = "1600p165_120mbps"
  width = 2560
  height = 1600
  fps = 165
  bitrate_mbps = 120
  status = "completed"
  classification = "completed"
  fps_observed = 160.0
  selected_profile = [pscustomobject]@{ width = 2560; height = 1600; fps = 165; bitrate_mbps = 120 }
  stage_p95_ms = [pscustomobject]@{ decode = 1.5; present = 0.4 }
}
$renderCappedComparison = Compare-PairedLanCanaryRows -LocalRows @($local1600Row) -CrossRows @($renderCappedRow) -RatioThreshold 0.8
Assert-Equal $renderCappedComparison[0].status "completed" "Receiver render-paced cap compares against the capped baseline"
Assert-True $renderCappedComparison[0].comparable "Receiver render-paced cap remains comparable"
Assert-Equal $renderCappedComparison[0].local_baseline_fps 144 "Receiver render-paced comparison baseline uses the local render target"
Assert-Equal ([Math]::Round($renderCappedComparison[0].fps_ratio, 3)) 0.903 "Receiver render-paced FPS ratio uses capped baseline"

$renderDropReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 136.0
  probeSnapshot = [pscustomobject]@{
    current_fps = 136.0
    frames_decoded = 8282
    frames_dropped = 37
    media_probe_width = 2560
    media_probe_height = 1440
    media_probe_target_fps = 144
    media_probe_target_bitrate_mbps = 80
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 464
    render_queue_replacements = 455
    render_stale_frame_drops = 0
    render_lock_drops = 2
    render_present_skips = 7
    queue_depth = 0
    stage_metrics = @()
    test_impairment = $null
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$renderDropRow = Convert-CrossReportToCanaryRow -Profile $profile2k14480 -Report $renderDropReport -ReportPath "raw/cross-2k144.json"
Assert-Equal $renderDropRow.dropped_frames 37 "Cross row dropped_frames tracks probe/transport drops"
Assert-Equal $renderDropRow.probe_dropped_frames 37 "Cross row exposes probe drops separately"
Assert-Equal $renderDropRow.pipeline_dropped_frames 464 "Cross row preserves legacy pipeline dropped frames"
Assert-Equal $renderDropRow.render_queue_replacements 455 "Cross row exposes render queue replacements"
Assert-Equal $renderDropRow.render_stale_frame_drops 0 "Cross row exposes render stale frame drops"
Assert-Equal $renderDropRow.render_lock_drops 2 "Cross row exposes render lock drops"
Assert-Equal $renderDropRow.render_present_skips 7 "Cross row exposes non-blocking D3D11 present skips"
Assert-Equal $renderDropRow.status "failed" "Severe visual integrity risk fails the canary row"
Assert-Equal $renderDropRow.classification "visual_integrity_risk" "Severe visual integrity risk is classified explicitly"
Assert-True ($renderDropRow.error_message -match "render drop/coalesce ratio") "Visual integrity risk carries an actionable render-drop reason"
Assert-Equal $renderDropRow.visual_integrity_status "risk" "Visual integrity status is exposed for reports"

$pacedRenderReport = [pscustomobject]@{
  status = "completed"
  failureReason = $null
  errorMessage = $null
  sampleObservedFps = 164.34
  sampleObservedRenderFps = 141.0
  sampleRenderFramesPresented = 6370
  sampleDurationMs = 45178
  probeSnapshot = [pscustomobject]@{
    current_fps = 164.34
    frames_decoded = 7425
    frames_dropped = 5
    media_probe_width = 2560
    media_probe_height = 1600
    media_probe_target_fps = 165
    media_probe_target_bitrate_mbps = 120
  }
  mediaPipelineSnapshot = [pscustomobject]@{
    dropped_frames = 721
    render_presented_frames = 6370
    render_queue_replacements = 0
    render_stale_frame_drops = 721
    render_lock_drops = 0
    render_present_skips = 0
    render_pacing_target_fps = 144
    queue_depth = 8
    stage_metrics = @(
      [pscustomobject]@{ stage = "sender.encode"; p50_ms = 0.34; p95_ms = 0.42 },
      [pscustomobject]@{ stage = "sender.send_reliable"; p50_ms = 0.06; p95_ms = 0.08 },
      [pscustomobject]@{ stage = "render_present_gap"; p50_ms = 6.28; p95_ms = 7.07 }
    )
    test_impairment = $null
    active_codec = "hevc"
    active_bitrate_mbps = 120
  }
  sessionSnapshot = [pscustomobject]@{ state = "streaming" }
}
$pacedRenderRow = Convert-CrossReportToCanaryRow -Profile $profile1600p165120 -Report $pacedRenderReport -ReportPath "raw/cross-1600p165-paced.json"
Assert-Equal $pacedRenderRow.status "completed" "Stable paced render coalescing does not fail the canary row"
Assert-Equal $pacedRenderRow.classification "completed" "Stable paced render coalescing keeps the completed classification"
Assert-Equal $pacedRenderRow.visual_integrity_status "paced" "Stable render coalescing is exposed as paced"
Assert-Equal ([Math]::Round($pacedRenderRow.estimated_render_fps, 1)) 141.0 "Estimated render FPS prefers sample-window presented frames"
Assert-Equal $pacedRenderRow.render_pacing_target_fps 144 "Paced row exposes the local render pacing target"
Assert-Equal ([Math]::Round($pacedRenderRow.render_present_gap_p95_ms, 2)) 7.07 "Paced row exposes render present gap P95"
Assert-Equal $pacedRenderRow.render_stale_frame_drops 721 "Paced row exposes render stale frame drops"
Assert-True ($pacedRenderRow.render_coalesce_ratio -gt 0.09) "Paced row exposes render coalesce ratio"

$singlePeerDiagnostics = [pscustomobject]@{
  udp_responses = @(
    [pscustomobject]@{
      payload = '{"type":"announce","device_id":"lan-peer-a","media_protocol_version":3,"transports":["quic","quic_datagram","quic_datagram_2k144","quic_datagram_media_v3","media_profile_control_v1"],"media_capabilities":["dxgi_capture","nvenc_h264","nvdec","d3d11_native_render"]}'
    }
  )
}
Assert-Equal (Resolve-PairedLanCanaryTargetDeviceId -Diagnostics $singlePeerDiagnostics -RequestedTargetDeviceId "") "lan-peer-a" "Single discovered peer is selected automatically"

$multiPeerDiagnostics = [pscustomobject]@{
  udp_responses = @(
    [pscustomobject]@{ payload = '{"type":"announce","device_id":"lan-peer-a"}' },
    [pscustomobject]@{ payload = '{"type":"announce","device_id":"lan-peer-b"}' }
  )
}
Assert-Equal (Resolve-PairedLanCanaryTargetDeviceId -Diagnostics $multiPeerDiagnostics -RequestedTargetDeviceId "") "" "Ambiguous discovered peers are not auto-selected"
Assert-Equal (Resolve-PairedLanCanaryTargetDeviceId -Diagnostics $multiPeerDiagnostics -RequestedTargetDeviceId "explicit-peer") "explicit-peer" "Explicit target device id wins over discovery"

$tauriNoBuildEnv = New-LocalDualProcessTauriEnvPlan `
  -OutputRoot ([System.IO.Path]::Combine("tmp", "canary")) `
  -ServiceExe ([System.IO.Path]::Combine("tmp", "canary", "run", "mrd-service.exe")) `
  -WorkspaceTargetDir ([System.IO.Path]::Combine("tmp", "repo", "target")) `
  -NoBuild:$true
Assert-Equal $tauriNoBuildEnv.MRD_SERVICE_PREBUILT_EXE ([System.IO.Path]::Combine("tmp", "canary", "run", "mrd-service.exe")) "NoBuild local dual canary uses the copied service executable"
Assert-Equal $tauriNoBuildEnv.MRD_SERVICE_EXE ([System.IO.Path]::Combine("tmp", "canary", "run", "mrd-service.exe")) "NoBuild local dual canary exposes the copied service executable to Tauri"
Assert-Equal $tauriNoBuildEnv.CARGO_TARGET_DIR ([System.IO.Path]::Combine("tmp", "repo", "target")) "NoBuild local dual canary pins Tauri to the workspace cargo target"

$tauriBuildEnv = New-LocalDualProcessTauriEnvPlan `
  -OutputRoot ([System.IO.Path]::Combine("tmp", "canary")) `
  -ServiceExe ([System.IO.Path]::Combine("tmp", "canary", "run", "mrd-service.exe")) `
  -WorkspaceTargetDir ([System.IO.Path]::Combine("tmp", "repo", "target")) `
  -NoBuild:$false
Assert-Equal $tauriBuildEnv.CARGO_TARGET_DIR ([System.IO.Path]::Combine("tmp", "repo", "target")) "Build local dual canary also pins Tauri to the workspace cargo target"

$localDualScript = Get-Content -Path (Join-Path $scriptDir "run_local_dual_process_lan_canary.ps1") -Raw
$pairedLanScript = Get-Content -Path (Join-Path $scriptDir "run_paired_lan_canary.ps1") -Raw
Assert-True ($pairedLanScript -match 'ValidateSet\("h264", "hevc", "av1"\)') "Paired LAN canary accepts AV1 codec selection"
Assert-True ($localDualScript -match 'ValidateSet\("h264", "hevc", "av1"\)') "Local dual canary accepts AV1 codec selection"
Assert-True ($pairedLanScript -match 'ValidateSet\("low_latency", "ultra_low_latency", "high_refresh"\)') "Paired LAN canary accepts AV1 mode selection"
Assert-True ($localDualScript -match 'ValidateSet\("low_latency", "ultra_low_latency", "high_refresh"\)') "Local dual canary accepts AV1 mode selection"
Assert-True ($pairedLanScript -match '\[string\]\$Av1Mode = "high_refresh"') "Paired LAN canary defaults AV1 to the stable high-refresh mode"
Assert-True ($localDualScript -match '\[string\]\$Av1Mode = "high_refresh"') "Local dual canary defaults AV1 to the stable high-refresh mode"
Assert-True ($pairedLanScript -match 'MRD_BENCH_NVENC_AV1_MODE') "Paired LAN local benchmark forwards AV1 mode to the harness"
Assert-True ($pairedLanScript -match 'transport\.2k144\.json') "Paired LAN local 2K144 benchmark uses the 2K144 threshold profile"
Assert-True ($pairedLanScript -match '-ThresholdPath') "Paired LAN local benchmark passes threshold files to the summarizer"
Assert-True ($pairedLanScript -match '\$localReport \| Add-Member -Force -NotePropertyName "codec_request"') "Paired LAN local report records codec request metadata"
Assert-True ($localDualScript -match "cargo build -p mrd-service") "Local dual canary prebuilds the service executable"
Assert-True ($localDualScript -match "cargo build -p app --no-default-features") "Local dual canary prebuilds the same Tauri shell target used by tauri dev"
Assert-True ($localDualScript -match "CARGO_TARGET_DIR") "Local dual canary keeps prebuild and Tauri dev on the same cargo target"
Assert-True ($localDualScript -match "MRD_LAN_E2E_RENDER_DISPLAY_SOURCE_ID") "Local dual canary can route the receiver window to an explicit display source"
Assert-True ($localDualScript -match "RenderPresentMode") "Local dual canary exposes render present mode selection"
Assert-True ($localDualScript -match "MRD_D3D11_RENDER_WAITABLE_OBJECT") "Local dual canary can enable waitable D3D11 present"

Write-Host "paired LAN canary common tests passed"
