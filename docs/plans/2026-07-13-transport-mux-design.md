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
| `video` | Ordered encoded access units; bounded queue | QUIC datagrams with mux framing | Existing encoded video track |
| `ctrl_rel` | Ordered, reliable commands and state barriers | Dedicated reliable stream messages | Reliable ordered data channel |
| `ctrl_rt` | Latest-value realtime input; stale pending value is replaced | QUIC datagrams with mux framing | Unordered, zero-retransmit data channel |
| `bulk` | Reliable transfer independent of interactive control | Independent reliable streams | Separate reliable ordered data channel |

Video remains lossy under sustained overload: an adapter reports the drop and
keeps latency bounded. `ctrl_rel` and `bulk` apply bounded backpressure rather
than silently dropping. `ctrl_rt` has one pending slot and reports stale-value
replacement. Bulk cannot consume the reliable-control queue.

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

## Framing and dispatch

QUIC uses one versioned mux frame header for datagrams and reliable stream
messages. A single receive dispatcher classifies frames before they reach lane
queues. Reliable-control sequence numbers are reordered before delivery, while
bulk messages remain independently deliverable.

WebRTC converts encoded access units to/from the existing video track. Three
separate data channels implement the other lanes; bulk is not multiplexed onto
the reliable-control channel.

## Evidence and lifecycle

Adapters publish only verified runtime observations. QUIC reports its local and
peer socket endpoints. WebRTC reports selected candidate-pair evidence and
classifies the route as direct or relay only after ICE establishes the pair.
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

## Test strategy

One conformance suite runs unchanged against paired fake, QUIC-loopback, and
WebRTC-loopback adapters. It proves video delivery, reliable-control ordering,
realtime stale replacement, independent bulk progress, route evidence, close,
and backpressure. Existing transport and service suites remain regression
gates.

