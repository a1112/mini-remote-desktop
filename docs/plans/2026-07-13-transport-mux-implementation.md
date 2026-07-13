# TransportMux Implementation Plan

> Execute with test-driven development and verify every listed regression suite
> before claiming Task 32 complete.

**Goal:** Introduce one application-facing transport mux and make LAN QUIC and
service-owned WebRTC conform to the same four-lane contract.

**Architecture:** `mrd-application` defines transport-neutral lane, envelope,
outcome, evidence, and port types. `mrd-service` provides shared bounded lane
queues plus QUIC and WebRTC adapters. Existing transport crates remain the
protocol engines and gain only capabilities required to preserve lane
independence.

## Task 1: Create the failing conformance contract

**Files:**

- Create `apps/mrd-service/tests/transport_mux.rs`

1. Define a pair factory abstraction used unchanged by fake, QUIC loopback, and
   WebRTC loopback fixtures.
2. Add cases for video metadata/payload round-trip, ordered reliable control,
   latest-value realtime control, bulk progress while interactive traffic is
   pending, route counters/evidence, bounded backpressure, and idempotent close.
3. Run `cargo test -p mrd-service --test transport_mux` and record the expected
   failure because the port and adapters do not exist.

## Task 2: Add the application port

**Files:**

- Create `crates/mrd-application/src/ports/transport_mux.rs`
- Modify `crates/mrd-application/src/lib.rs`

1. Add the four lane values and transport-neutral envelope/video metadata.
2. Add send outcomes, route kind, per-lane counters, and route snapshot.
3. Add the async session-scoped `TransportMuxPort` trait.
4. Add focused DTO invariants and run `cargo test -p mrd-application`.

## Task 3: Implement shared service scheduling

**Files:**

- Modify `apps/mrd-service/src/transports/mod.rs`

1. Add FIFO queues for video, reliable control, and bulk, bounded by both
   envelope count and retained payload bytes.
2. Add a single-slot latest-value queue for realtime control.
3. Add shared atomic counters, close notification, and route snapshot helpers.
4. Implement the fake pair inside the conformance test and make its suite pass
   before adding network adapters.

## Task 4: Implement the QUIC adapter

**Files:**

- Create `apps/mrd-service/src/transports/quic.rs`
- Modify `crates/mrd-transport-quic-quinn` only if its public primitives cannot
  keep reliable control and bulk independent.

1. Add versioned lane framing and strict session validation.
2. Dispatch video/realtime datagrams and independent reliable-control/bulk
   stream messages into shared queues.
3. Preserve reliable-control ordering on a dedicated persistent stream; keep
   bulk and reliable video on separate streams. Merge reliable keyframes and
   datagram video through one bounded sequence orderer.
4. Classify new persistent streams with bounded concurrent header readers and
   enforce per-frame plus aggregate media-reassembly byte limits.
5. Populate endpoint evidence and close the Quinn connection cleanly.
6. Make the unchanged conformance suite pass for QUIC loopback.

## Task 5: Implement the WebRTC adapter

**Files:**

- Modify `apps/mrd-service/src/transports/webrtc.rs`
- Modify `apps/mrd-service/Cargo.toml`
- Modify `crates/mrd-transport-webrtc` to add a distinct bulk data channel.

1. Expose service-owned WebRTC transport in the default service build; keep the
   browser preview's direct WebRTC dependency feature-gated.
2. Convert mux video envelopes to/from encoded access units.
3. Map reliable control, realtime control, and bulk to three distinct data
   channels.
4. Pace bulk sends against a data-channel buffered-amount high-water mark so
   stalled bulk cannot starve reliable interactive control.
5. Fragment reliable-control/bulk envelopes below the SCTP message limit;
   byte-bound RTP assembly, completed-video queues, and pre-mux channel ingress;
   project pre-mux video drops into route evidence; retain only weak peer/channel
   references in callbacks; and validate the exact label/reliability/uniqueness
   contract of incoming data channels.
6. Project selected ICE candidate evidence and direct/relay classification,
   with a short recovery window for transient disconnected states.
7. Make the unchanged conformance suite pass for WebRTC loopback.

## Task 6: Migrate the LAN media boundary

**Files:**

- Modify `apps/mrd-service/src/lan_discovery/media_sender.rs`
- Modify `apps/mrd-service/src/lan_discovery/media_receiver.rs`
- Modify `apps/mrd-service/src/lan_discovery.rs`
- Modify LAN capability/protocol declarations for negotiated legacy fallback.

1. Replace direct endpoint-facing sender output with video mux envelopes while
   preserving existing access-unit packetization behavior behind the adapter.
2. Replace direct endpoint-facing receiver input with video mux envelopes while
   preserving decode/render behavior.
3. Remove feature decisions based on concrete Quinn/WebRTC types from the
   migrated boundary.

## Task 7: Verify and integrate

1. Run `cargo fmt --all -- --check`.
2. Run `cargo test -p mrd-application`.
3. Run `cargo test -p mrd-transport-quic-quinn`.
4. Run `cargo test -p mrd-transport-webrtc`.
5. Run `cargo test -p mrd-service`.
6. Review the diff for concrete transport leakage, unbounded queues, silent
   drops, test-only shortcuts, and unrelated changes.
7. Commit implementation as `refactor: unify remote transport lanes` and
   fast-forward `codex/market-remote-capability-alignment` after verification.
