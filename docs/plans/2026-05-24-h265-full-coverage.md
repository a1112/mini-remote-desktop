# H.265 Full Coverage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete H.265/HEVC support across the existing capture, encode, transport, decode, browser preview, and test UI surfaces that already participate in the desktop streaming pipeline.

**Architecture:** Treat HEVC as a first-class encoded access-unit codec beside H.264 and AV1. Reuse existing NVENC/NVDEC/QUIC HEVC support, add RFC 7798 RTP packetization/assembly for WebRTC, and remove UI/test harness blocks only where the runtime path is backed by tests. Keep unrelated gaps explicit: mrd-service WebRTC media is currently absent for all codecs, and cross-platform software HEVC decode requires a separate dependency decision.

**Tech Stack:** Rust, Tauri, React/TypeScript, Vitest, WebRTC `webrtc-rs`, RTP `rtp`, NVENC/NVDEC, WebCodecs.

---

### Task 1: Add HEVC RTP Transport Primitives

**Files:**
- Modify: `crates/mrd-transport-webrtc/src/lib.rs`
- Test: `crates/mrd-transport-webrtc/src/lib.rs`

**Step 1: Write failing HEVC RTP tests**

Add tests that exercise the desired public API before implementation:

```rust
#[test]
fn hevc_assembler_emits_single_nal_access_unit_on_marker() {
    let mut assembler = HevcAccessUnitAssembler::default();
    let au = assembler
        .push_rtp_payload(&[0x40, 0x01, 0xaa], true)
        .expect("complete AU");
    assert_eq!(au, vec![0, 0, 0, 1, 0x40, 0x01, 0xaa]);
}

#[test]
fn hevc_assembler_reassembles_fragmentation_unit() {
    let mut assembler = HevcAccessUnitAssembler::default();
    assert!(assembler
        .push_rtp_payload(&[0x62, 0x01, 0x93, 0xaa], false)
        .is_none());
    let au = assembler
        .push_rtp_payload(&[0x62, 0x01, 0x53, 0xbb], true)
        .expect("complete FU");
    assert_eq!(au, vec![0, 0, 0, 1, 0x26, 0x01, 0xaa, 0xbb]);
}

#[test]
fn hevc_ingress_marks_irap_access_units_as_keyframes() {
    let mut ingress = HevcRtpIngress::default();
    let au = ingress
        .push_packet(&[0x26, 0x01, 0xaa], true, 7, 123)
        .expect("access unit");
    assert_eq!(au.codec, VideoCodec::Hevc);
    assert!(au.is_keyframe);
}

#[tokio::test]
async fn hevc_sender_and_ingress_roundtrip_annex_b() {
    let mut sender = HevcRtpSender::new("video", "stream", 30, 1200);
    let mut ingress = HevcRtpIngress::default();
    let packets = sender.packetize_annex_b_for_test(&hevc_vps_sps_irap_access_unit())?;
    let mut decoded = None;
    for packet in packets {
        decoded = ingress.push_packet(&packet.payload, packet.header.marker, packet.header.sequence_number, 42);
    }
    assert_eq!(decoded.expect("decoded").codec, VideoCodec::Hevc);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-transport-webrtc hevc -- --nocapture`

Expected: FAIL because `HevcAccessUnitAssembler`, `HevcRtpIngress`, and `HevcRtpSender` do not exist.

**Step 3: Write minimal implementation**

Add:
- `HevcRtpSender`, `HevcRtpSendReport`, `HevcAccessUnitAssembler`, and `HevcRtpIngress`.
- `hevc_codec_capability()` returning a `video/HEVC` codec capability.
- `hevc_annex_b_contains_keyframe()` that detects VPS/SPS/PPS and IRAP NAL types.
- RFC 7798 payload handling for Single NAL, Aggregation Packet type 48, and Fragmentation Unit type 49.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-transport-webrtc hevc -- --nocapture`

Expected: PASS.

### Task 2: Enable HEVC in the Tauri Test Harness WebRTC RTP Path

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/test_orchestrator.rs`
- Test: existing `#[cfg(test)]` modules in those files

**Step 1: Write failing harness tests**

Add tests proving that `EncoderType::NvencHevc` and `TransportKind::WebrtcRtp` no longer return the current "not implemented" error and that WebRTC RTP loopback uses `VideoCodec::Hevc` for HEVC access units.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app test_harness_hevc_webrtc -- --nocapture`

Expected: FAIL with the existing HEVC WebRTC packetizer error.

**Step 3: Write minimal implementation**

Replace H.264-only sender/ingress enums with codec-aware enums:
- H.264 -> existing `H264RtpSender` / `H264RtpIngress`
- HEVC -> new `HevcRtpSender` / `HevcRtpIngress`
- AV1 -> existing AV1 path

Remove only the HEVC-specific WebRTC RTP bail-out. Keep HEVC + software decoder blocked until a real software HEVC decoder is added.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app test_harness_hevc_webrtc -- --nocapture`

Expected: PASS.

### Task 3: Enable HEVC in the Runtime WebRTC Host

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/webrtc_media.rs`
- Modify: `apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Test: existing `#[cfg(test)]` module in `apps/Rdesk/src-tauri/src/webrtc_host.rs`

**Step 1: Write failing runtime WebRTC tests**

Add tests that prove:
- A HEVC `EncodedAccessUnit` is sent instead of skipped.
- A remote `video/HEVC` RTP track is assembled into `VideoCodec::Hevc`.
- Decode backend selection chooses `nvdec_hevc` on Windows or Linux HEVC on Linux, and returns an explicit unsupported error where no HEVC decode backend exists.

**Step 2: Run test to verify it fails**

Run: `cargo test -p app webrtc_host_hevc -- --nocapture`

Expected: FAIL because runtime WebRTC host currently imports only H.264 RTP helpers and skips non-H.264 access units.

**Step 3: Write minimal implementation**

Add codec-aware sender and assembler wrappers in `webrtc_host.rs`. Preserve existing snapshot fields for compatibility, and add HEVC-aware metadata fields only where tests need visibility. Register HEVC in the media engine so SDP negotiation can advertise the new codec.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app webrtc_host_hevc -- --nocapture`

Expected: PASS.

### Task 4: Add HEVC WebCodecs Browser Preview Branch

**Files:**
- Modify: `apps/mrd-service/src/browser_webcodecs_preview.rs`
- Modify: `apps/Rdesk/src/app/workers/webCodecsPreview.worker.ts`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Test: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Step 1: Write failing WebCodecs tests**

Add tests proving that:
- HEVC-capable encoders are not filtered out of the browser preview when WebCodecs reports HEVC support.
- The worker sends a HEVC `VideoDecoderConfig` without H.264 `avc` metadata.
- The service request can carry `codec: "hevc"` while preserving legacy H.264 fields.

**Step 2: Run test to verify it fails**

Run: `cd apps/Rdesk; pnpm test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/workers/webCodecsPreview.worker.test.ts`

Expected: FAIL because UI text and worker configuration are currently H.264-only.

**Step 3: Write minimal implementation**

Add a codec field to the WebCodecs preview request. Branch encoder creation:
- H.264 -> existing `NvencH264Encoder` and `avc1.*` codec string
- HEVC -> `NvencHevcEncoder` and a WebCodecs HEVC codec string after `VideoDecoder.isConfigSupported`

Keep AV1 disabled for browser preview unless a tested AV1 WebCodecs branch already exists.

**Step 4: Run test to verify it passes**

Run: `cd apps/Rdesk; pnpm test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/workers/webCodecsPreview.worker.test.ts`

Expected: PASS.

### Task 5: Remove HEVC + WebRTC UI Matrix Blocks

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx`
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/CustomTestPage.tsx`
- Test: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

**Step 1: Write failing UI tests**

Add tests showing that HEVC + WebRTC is allowed when decoder selection is compatible, while HEVC + software decoder and HEVC + H.264-only decoder remain blocked.

**Step 2: Run test to verify it fails**

Run: `cd apps/Rdesk; pnpm test -- --run src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

Expected: FAIL with the current "HEVC WebRTC RTP packetizer is not implemented" blocker.

**Step 3: Write minimal implementation**

Remove HEVC-specific WebRTC blocker text and badge logic. Leave decoder compatibility and capability checks intact.

**Step 4: Run test to verify it passes**

Run: `cd apps/Rdesk; pnpm test -- --run src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

Expected: PASS.

### Task 6: Regression Verification

**Files:**
- No source changes unless verification exposes a regression.

**Step 1: Run targeted Rust checks**

Run:

```powershell
cargo test -p mrd-transport-webrtc hevc -- --nocapture
cargo test -p app webrtc -- --nocapture
cargo test -p app test_harness -- --nocapture
cargo test -p mrd-service browser_webcodecs -- --nocapture
```

Expected: all targeted tests pass or report environment-gated skips for hardware-only paths.

**Step 2: Run targeted frontend checks**

Run:

```powershell
Set-Location apps/Rdesk
pnpm test -- --run src/app/components/TestWorkbench/MatrixTestPage.test.tsx src/app/components/RemoteDisplayWindowPage.test.tsx
pnpm type-check
```

Expected: all targeted tests and type checks pass.

**Step 3: Run package builds**

Run:

```powershell
cargo build -p mrd-transport-webrtc -p app -p mrd-service
```

Expected: build completes successfully.
