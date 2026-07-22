# Encoder Enhancement Roadmap

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enhance the encoding pipeline with AV1/HEVC support, adaptive bitrate, ROI encoding, multi-layer output, and comprehensive telemetry to achieve higher quality at lower bitrates with vendor-agnostic hardware acceleration.

**Architecture:** Extend `agent-rust` encoding pipeline with codec abstraction layer, dynamic bitrate controller, ROI region detector, multi-layer encoder, and GPU telemetry pipeline.

**Tech Stack:** Rust, NVIDIA NVENC, Intel QSV, AMD AMF, DirectX 11/12, Vulkan, FFmpeg.

---

## Phase 1: Codec Capability Expansion

### Task 1.1: NVENC AV1 Support (RTX 40/50 Series)

**Files:**
- Create: `agent-rust/src/codecs/av1/mod.rs`
- Modify: `agent-rust/src/codecs/mod.rs`
- Modify: `agent-rust/src/encoder_policy.rs`

1. Add AV1 codec variant to `CodecType` enum
2. Implement `NVENC_AV1Encoder` wrapper:
   - Profile: Main (0)
   - Tier: Main (0)
   - Block size: 64x64 max, 4x4 min
   - Enable intra block copy
   - Set film grain synthesis (optional)
3. Add capability detection: `cuda::get_device().compute_capability >= 8.9`
4. Configure sequence headers for low-delay streaming
5. Commit: `feat(codec): add NVENC AV1 encoder for RTX 40/50`

### Task 1.2: NVENC HEVC Encoder

**Files:**
- Create: `agent-rust/src/codecs/hevc/mod.rs`
- Modify: `agent-rust/src/codecs/mod.rs`

1. Implement `NVENC_HEVCEncoder`:
   - Profile: Main (high quality)
   - Enable CBR with VBV buffer
   - Enable transform skip
   - Set strong intra smoothing
2. Add quality comparison: H.264 baseline vs HEVC at same bitrate
3. Commit: `feat(codec): add NVENC HEVC encoder`

### Task 1.3: Cross-Vendor Hardware Fallback (QSV/AMF)

**Files:**
- Create: `agent-rust/src/codecs/qsv/mod.rs`
- Create: `agent-rust/src/codecs/amf/mod.rs`
- Modify: `agent-rust/src/capture_policy.rs`

1. Intel QSV H.264/HEVC:
   - Use `mfxImplementation` MFX_IMPL_HARDWARE_ANY
   - Enable low_power mode for Tiger Lake+
2. AMD AMF H.264/HEVC:
   - Initialize via AMF API
   - Set VCE encoding preset
3. Fallback chain: NVENC → QSV → AMF → x264
4. Add cooldown between fallback attempts (30s)
5. Commit: `feat(codec): add Intel QSV and AMD AMF encoders with fallback`

---

## Phase 2: Bitrate Control & Quality Enhancement

### Task 2.1: Dynamic VBV/HRD for Consistent Quality

**Files:**
- Create: `agent-rust/src/rate_control/mod.rs`
- Modify: `agent-rust/src/encoder_runtime.rs`

1. Implement VBV controller:
   - Buffer size: 1-2x target bitrate
   - Initial fullness: 75%
   - Max bitrate: 1.4x target (temporary burst)
2. Add HRD compliance checking:
   - Prevent buffer underflow/overflow
   - Clamp QP within [min_qpreset, max_qpreset]
3. Reactive to RTCP REMB feedback:
   - Decrease bitrate on high loss (>5%)
   - Increase bitrate on low loss (<1%) + low RTT (<50ms)
4. Commit: `feat(rate): add VBV/HRD rate controller with REMB feedback`

### Task 2.2: ROI (Region of Interest) Encoding

**Files:**
- Create: `agent-rust/src/roi_detector.rs`
- Modify: `agent-rust/src/encoder_runtime.rs`

1. Implement ROI detection:
   - Track mouse cursor position (±200px region)
   - Track active window (via Windows Graphics Capture API)
   - Detect motion regions via frame differencing
2. Encode with ROI maps:
   - High priority region: QP -8
   - Medium priority: QP baseline
   - Low priority: QP +4
3. ROI map update rate: 10Hz (every 100ms)
4. Commit: `feat(codec): add ROI encoding for cursor and active window`

### Task 2.3: Content-Adaptive Encoding

**Files:**
- Create: `agent-rust/src/content_analyzer.rs`
- Modify: `agent-rust/src/encoder_policy.rs`

1. Content classification:
   - Text/Document detection (edge density > threshold)
   - Video/Scene detection (temporal variance)
   - Game detection (high frame variance + low spatial complexity)
2. Adaptive parameters:
   - Text mode: higher bitrate, sharper preset, disable B-frames
   - Video mode: standard bitrate, balanced preset
   - Game mode: lowest latency preset (p1), low IDR frequency
3. Mode switching hysteresis: 5 seconds minimum
4. Commit: `feat(codec): add content-adaptive encoding profiles`

---

## Phase 3: Multi-Stream Capability

### Task 3.1: Single-Encoder Multi-Layer Output

**Files:**
- Create: `agent-rust/src/multilayer_encoder.rs`
- Modify: `agent-rust/src/main.rs`

1. Implement temporal scalability:
   - Base layer: 30fps, full resolution
   - Enhancement layer: delta frames, 60fps
2. Simulcast modes:
   - High quality: 1080p@30, 8Mbps
   - Medium quality: 720p@30, 4Mbps
   - Low quality: 480p@30, 2Mbps
3. GPU memory sharing: single encode session, multiple bitstream outputs
4. Commit: `feat(codec): add multi-layer output for adaptive streaming`

### Task 3.2: Multi-Session Fair Scheduler

**Files:**
- Create: `agent-rust/src/session_scheduler.rs`
- Modify: `agent-rust/src/main.rs`

1. Implement weighted fair queueing:
   - Active session: weight 1.0
   - Background session: weight 0.5
2. GPU budget allocation:
   - Per-session fps cap based on active session count
   - Dynamic resolution scaling when >2 sessions
3. Priority levels:
   - Real-time (gaming): highest priority
   - Interactive (office): medium priority
   - Passive (video playback): lowest priority
4. Commit: `feat(sched): add multi-session fair scheduler`

---

## Phase 4: Capture Optimization

### Task 4.1: Dirty-Rect Detection

**Files:**
- Create: `agent-rust/src/dirty_rect.rs`
- Modify: `agent-rust/src/capture_runtime.rs`

1. Implement block-based dirty detection:
   - Block size: 64x64 pixels
   - Compare current vs previous frame (SAD > threshold)
2. Merge adjacent dirty blocks into regions
3. Skip encoding for clean regions:
   - Send region map to decoder
   - Decoder copies from previous frame
4. Savings: 50-90% for static content
5. Commit: `feat(capture): add dirty-rect detection and skip encoding`

### Task 4.2: Hardware Scaling

**Files:**
- Modify: `agent-rust/src/capture_runtime.rs`

1. D3D11 hardware scaler:
   - Use ID3D11VideoContext->VideoProcessor
   - Lanczos4 scaling filter
2. Vendor-specific optimal formats:
   - NVIDIA: NV12/P010
   - Intel: P010
   - AMD: NV12
3. Zero-copy GPU pipeline for resize
4. Commit: `feat(capture): add GPU-accelerated hardware scaling`

### Task 4.3: HDR to SDR Tone Mapping

**Files:**
- Create: `agent-rust/src/hdr_tone_mapper.rs`
- Modify: `agent-rust/src/capture_runtime.rs`

1. Detect HDR10/HDR10+ content:
   - Check DXGI format (DXGI_FORMAT_R10G10B10A2_UNORM)
   - Read HDR metadata
2. Tone mapping options:
   - Reinhard: `L_out = L_in / (1 + L_in)`
   - Hable: cinematic curve
   - Clip: simple clamp (for SDR output)
3. Output: SDR BT.709, 8-bit
4. Commit: `feat(capture): add HDR to SDR tone mapping`

---

## Phase 5: Network Coordination

### Task 5.1: IDR/GOP with Congestion Feedback

**Files:**
- Modify: `agent-rust/src/rtp_send.rs`
- Modify: `agent-rust/src/net_adapt.rs`

1. Dynamic IDR frequency:
   - Low congestion: IDR every 2 seconds
   - High congestion: IDR every 10 seconds
2. GOP structure adaptation:
   - Low RTT: I-B-B-B-P (high compression)
   - High RTT: I-P-P-P (low latency)
3. Key frame request on decoder error:
   - Send PLI on frame loss > threshold
4. Commit: `feat(net): adaptive IDR/GOP based on congestion`

### Task 5.2: FEC/NACK Strategy Refinement

**Files:**
- Modify: `agent-rust/src/rtp_send.rs`
- Create: `agent-rust/src/redundancy.rs`

1. Loss pattern detection:
   - Random loss: use forward error correction (FEC)
   - Burst loss: use NACK retransmission
2. Adaptive FEC:
   - Low loss: 5% FEC overhead
   - Medium loss: 15% FEC overhead
   - High loss: 30% FEC overhead
3. NACK buffer management:
   - 100ms history for retransmission
4. Commit: `feat(net): adaptive FEC/NACK based on loss pattern`

### Task 5.3: QUIC Pacing + Encoder Beat Sync

**Files:**
- Modify: `agent-rust/src/quic_tx.rs`
- Modify: `agent-rust/src/encoder_runtime.rs`

1. Frame pacing synchronization:
   - Align encoder output with QUIC paced sending
   - Batch 2-3 frames per QUIC flush
2. Pacing rate calculation:
   - BBR min RTT as target
   - Pacing rate = min(cwnd, bbr_bw) / min_rtt
3. Avoid queue buildup:
   - Send queue depth < 10ms worth of data
4. Commit: `feat(net): sync encoder output with QUIC pacing`

---

## Phase 6: Observability & Automation

### Task 6.1: GPU Timestamp Pipeline

**Files:**
- Create: `agent-rust/src/gpu_telemetry.rs`
- Modify: `agent-rust/src/runtime_stats.rs`

1. Instrumentation points:
   - `t0`: capture start
   - `t1`: capture complete (GPU timestamp)
   - `t2`: encode start
   - `t3`: encode complete (GPU timestamp)
   - `t4`: packet send
2. Report per-frame:
   - Capture latency: t1 - t0
   - Encode latency: t3 - t2
   - E2E latency: t4 - t0
   - GPU utilization: encode_time / frame_time
3. Export to Prometheus/Prometheus Pushgateway
4. Commit: `feat(obs): add GPU timestamp telemetry pipeline`

### Task 6.2: Auto-Fallback Matrix

**Files:**
- Modify: `agent-rust/src/capture_policy.rs`
- Modify: `agent-rust/src/encoder_policy.rs`

1. Capture fallback chain:
   ```
   DXGI desktop dup → WGC → PowerShell ScreenCapture → CPU fallback
   ```
2. Encoder fallback chain:
   ```
   NVENC AV1 → NVENC HEVC → NVENC H264 → QSV → AMF → x264
   ```
3. Cooldown and retry:
   - 30s cooldown after fallback
   - Retry preferred path every 5 minutes
4. State persistence across restarts
5. Commit: `feat(policy): add automatic fallback matrix with cooldown`

### Task 6.3: Baseline Gates & Long-Running Tests

**Files:**
- Create: `agent-rust/tests/soak_tests.rs`
- Create: `mini-remote-desktop/tests/baselines.json`

1. Baseline metrics:
   ```json
   {
     "p95_encode_latency_ms": 8,
     "p99_encode_latency_ms": 12,
     "p95_frame_drop_rate": 0.01,
     "max_gpu_utilization": 0.85
   }
   ```
2. 4-hour soak test scenarios:
   - Static document
   - Video playback
   - Gaming (fast motion)
3. Regression detection: fail if >10% deviation from baseline
4. Commit: `feat(test): add baseline gates and 4-hour soak tests`

---

## Priority Order

```
┌─────────────────────────────────────────────────────────────────┐
│                      Implementation Priority                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  P0 - Foundation (Week 1-2)                                     │
│  ├─ Task 1.1: NVENC AV1                                         │
│  ├─ Task 1.3: Cross-vendor fallback                             │
│  └─ Task 6.1: GPU telemetry                                     │
│                                                                  │
│  P1 - Quality (Week 3-4)                                        │
│  ├─ Task 2.1: VBV/HRD control                                   │
│  ├─ Task 2.2: ROI encoding                                      │
│  └─ Task 4.1: Dirty-rect detection                              │
│                                                                  │
│  P2 - Advanced (Week 5-8)                                       │
│  ├─ Task 2.3: Content-adaptive encoding                         │
│  ├─ Task 3.1: Multi-layer output                                │
│  └─ Task 5.1: Congestion-aware GOP                              │
│                                                                  │
│  P3 - Optimization (Week 9+)                                    │
│  ├─ Task 4.2: Hardware scaling                                  │
│  ├─ Task 4.3: HDR tone mapping                                  │
│  ├─ Task 5.3: QUIC pacing sync                                  │
│  └─ Task 6.3: Baseline gates                                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Success Criteria

| Metric | Target | Measurement |
|--------|--------|-------------|
| AV1 quality gain | +30% PSNR at same bitrate | vs H.264 baseline |
| HEVC quality gain | +20% PSNR at same bitrate | vs H.264 baseline |
| Encode latency P95 | <8ms @ 1080p60 | GPU telemetry |
| GPU utilization | <85% | sustained load |
| Dirty-rect savings | >50% bitrate @ static | content detection |
| Multi-session | 3 concurrent @ 30fps | scheduler tests |

---

## Open Questions

1. Should AV1 be default for RTX 40+ or opt-in?
2. What's the target bitrate for 4K60 streaming?
3. Should we support simultaneous codec outputs (H.264 + AV1)?
4. ROI map transmission overhead vs encoding savings tradeoff?

---

## References

- NVENC API: https://docs.nvidia.com/video/nvenc/
- Intel QSV: https://github.com/Intel-Media-SDK/MediaSDK
- AMD AMF: https://github.com/GPUOpen-LibrariesAndSDKs/AMF
- WebRTC Codec: https://webrtc.googlesource.com/src/+/refs/heads/main/modules/video_coding/
