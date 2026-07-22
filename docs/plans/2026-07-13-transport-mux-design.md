# TransportMux Design

**Date:** 2026-07-13  
**Status:** Approved as Task 32 of the market remote-capability alignment plan

## Objective

Give the application and media pipeline one transport boundary for LAN QUIC and
Internet WebRTC. Callers work with logical traffic classes and route evidence;
they do not branch on Quinn, WebRTC peer connections, ICE, or data channels.

## Boundary

`mrd-application` owns transport-neutral DTOs and the `TransportMuxPort` trait.
`mrd-service` owns concrete adapters, scheduling, framing, and statistics.
Infrastructure crates continue to own protocol-specific connections.

The port is scoped to one authenticated session. Every envelope still carries
the session ID so an adapter can reject accidental cross-session traffic.

## Logical lanes

| Lane | Semantics | QUIC mapping | WebRTC mapping |
| --- | --- | --- | --- |
| `video` | Ordered encoded access units; bounded queue | media-v3 datagrams, with a dedicated reliable keyframe stream | Existing encoded video track |
| `ctrl_rel` | Ordered, reliable commands and state barriers | Dedicated persistent reliable stream | Reliable ordered data channel |
| `ctrl_rt` | Latest-value realtime input; stale pending value is replaced | QUIC datagrams with mux framing | Unordered, zero-retransmit data channel |
| `bulk` | Reliable transfer independent of interactive control | Independent persistent reliable stream | Separate reliable ordered data channel |

Video remains lossy under sustained overload: an adapter reports the drop and
keeps latency bounded. `ctrl_rel` and `bulk` apply bounded backpressure rather
than silently dropping. `ctrl_rt` has one pending slot and reports stale-value
replacement. Every lane is bounded by both envelope count and retained payload
bytes. Bulk cannot consume the reliable-control queue.

## Port contract

The application contract exposes:

- `send(envelope)` returning `Enqueued`, `ReplacedStale`, `Backpressured`, or
  `Closed`;
- `recv(lane)` returning the next available envelope or end-of-stream;
- a route snapshot containing transport kind, direct/relay evidence, endpoint
  descriptions, per-lane counters, stale replacements, drops, and
  backpressure observations;
- idempotent `close()` that wakes blocked receivers and makes later sends
  return `Closed`.

Payloads are owned byte vectors. Optional video metadata records codec,
timestamp, keyframe status, and dimensions without importing codec or WebRTC
types into `mrd-application`.

Envelope sequence numbers are monotonic at each endpoint. Datagram adapters
preserve submitted video sequences; RTP adapters may reconstruct receive-side
sequences because RTP does not carry the application value. Dimensions are
preserved when carried by the media transport and are explicitly zero when an
RTP adapter cannot observe them. Callers must not use either field as a
cross-transport frame identity.

## Framing and dispatch

QUIC wraps the full video envelope in media-v3 fragmentation/reassembly for
ordinary frames and uses a dedicated persistent reliable stream for keyframes.
Both paths feed one bounded sequence orderer: an initial keyframe establishes
the sequence baseline immediately, while a non-keyframe seen first has a 100 ms
gap window before video reaches the application queue. Incomplete media reassembly has per-frame
and aggregate byte budgets. The reliable-stream dispatcher uses at most eight
concurrent 250 ms header classifiers, so an opened stream that withholds its
lane byte cannot block classification of later streams.
Control and bulk use a versioned mux frame. Reliable video, ordered control,
and bulk have separate persistent streams, bounded lane readers, and no
per-envelope task spawning. Receive dispatch classifies frames before they
reach lane queues, so a stalled bulk consumer cannot head-of-line block
interactive control.

WebRTC converts encoded access units to/from the existing video track; received
timestamps use the selected RTP clock rather than claiming preservation of an
absolute capture clock. Three separate data channels implement the other lanes;
bulk is not multiplexed onto the reliable-control channel. The bulk sender is
paced and observes its data-channel buffered-amount high-water mark so pressure
stays visible at the mux boundary instead of starving interactive control.
Reliable control and bulk envelopes are fragmented into messages below the SCTP
receive limit and reassembled in-order under the lane byte budget; realtime
control must fit one unreliable message. Incoming channel labels, uniqueness,
ordering, and retransmission settings are validated before use. RTP access-unit
assembly, the completed-access-unit queue, and pre-mux data-channel ingress are
all byte-bounded. Failure callbacks retain only weak peer-connection references,
and data-channel callbacks retain only weak channel references, so error and
drop paths cannot keep transport objects alive through callback cycles. Drops
that occur in the completed-access-unit queue are atomically projected into the
video lane's route evidence. Snapshot, failure, explicit close, and drop paths
flush pending adapter counters synchronously so the evidence remains readable
after shutdown.

## Evidence and lifecycle

Adapters publish only verified runtime observations. QUIC reports its local and
peer socket endpoints. WebRTC remains explicitly pending until selected
candidate-pair evidence is observable, then classifies the route as direct or
relay and reports the selected candidate identifiers and kinds.
Fake adapters identify themselves as test-only and never masquerade as a
network route.

Closing an adapter stops dispatch tasks, closes the underlying connection, and
closes every lane queue. Counters remain readable after close for audit and
quality-gate evidence.

## LAN migration

The existing capture/encode and depacketize/decode logic remains in the LAN
media modules. Only their transport boundary changes: sender output becomes a
`video` envelope and receiver input consumes a `video` envelope. QUIC-specific
packetization and endpoint operations move behind the QUIC adapter.

New peers advertise `quic_transport_mux_v1`; when both sides support it, the
production LAN sender and receiver use `TransportMuxPort` and the adapter owns
all connection reads. Legacy QUIC media remains as a negotiated compatibility
fallback for older peers. Sender telemetry and keyframe requests use a bounded
adapter-owned passthrough queue so they cannot compete with the mux for the
same endpoint reader. Mux enqueue acceptance is not counted as a fragment sent;
legacy wire-fragment counters change only where actual legacy endpoint I/O is
observed.

## Test strategy

One conformance suite runs unchanged against paired fake, QUIC-loopback, and
WebRTC-loopback adapters. It proves video delivery, reliable-control ordering,
realtime stale replacement, independent bulk progress, route evidence, close,
and backpressure. Existing transport and service suites remain regression
gates.
