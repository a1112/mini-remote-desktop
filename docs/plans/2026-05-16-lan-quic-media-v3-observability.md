# LAN QUIC Media V3 Observability Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make LAN QUIC media datagrams self-describing and observable before deeper adaptive transport work.

**Architecture:** Keep the existing v2 access-unit fragment path compatible, but add a v3 media envelope with payload type, codec, profile id, flags, fragment metadata, and payload length. Advertise v3 capability through LAN discovery while allowing v2 peers during the rollout. Use paired canary reports to surface v3 readiness and transport counters before enabling FEC/NACK policy changes.

**Tech Stack:** Rust, Quinn QUIC datagrams, `mrd-transport-quic-quinn`, `mrd-service` LAN discovery, Rdesk TypeScript capability checks, Vitest.

---

### Task 1: Add QUIC media envelope v3 primitives

**Files:**
- Modify: `crates/mrd-transport-quic-quinn/src/lib.rs`
- Test: `crates/mrd-transport-quic-quinn/tests/loopback.rs`

**Steps:**
1. Write tests for v3 H.264 media fragmentation/reassembly preserving payload type, codec, profile id, frame id, timestamp, keyframe flag, and payload bytes.
2. Write tests for invalid magic/version/payload length rejection.
3. Implement `QuicMediaPayloadType`, `QuicMediaCodec`, `QuicMediaFragment`, `QuicMediaFrame`, `QuicMediaReassembler`, and `fragment_media_payload_v3`.
4. Keep existing `fragment_access_unit` and `QuicAuReassembler` unchanged for v2 compatibility.
5. Run `cargo test -p mrd-transport-quic-quinn --test loopback -- --nocapture`.

### Task 2: Advertise v3 without breaking v2 peers

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Test: `cargo test -p mrd-service lan_discovery -- --nocapture`

**Steps:**
1. Add `quic_datagram_media_v3` to LAN transports and media capabilities.
2. Bump advertised `LAN_MEDIA_PROTOCOL_VERSION` to `3`.
3. Keep `quic_datagram_media_v2` advertised during rollout.
4. Update LAN discovery tests to assert v2 and v3 are both present.

### Task 3: Update UI and automation capability gates

**Files:**
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.ts`
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx`
- Test: `apps/Rdesk/src/app/services/lanE2eAutomationService.test.ts`
- Test: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

**Steps:**
1. Accept peers with media protocol version `>= 3` and `quic_datagram_media_v3`.
2. Accept v2 peers only as compatibility fallback when v3 is absent.
3. Improve not-ready messages so they distinguish v2 fallback from missing media controls.
4. Run targeted Vitest tests.

### Task 4: Verification

**Commands:**
- `cargo fmt --all -- --check`
- `cargo test -p mrd-transport-quic-quinn --test loopback -- --nocapture`
- `cargo test -p mrd-service lan_discovery -- --nocapture`
- `pnpm test -- --run src/app/services/lanE2eAutomationService.test.ts src/app/components/TestWorkbench/MatrixTestPage.test.tsx`
- `cargo build -p app -p mrd-service -p mrd-transport-quic-quinn`

