# Authenticated Route Planner Design

**Date:** 2026-07-13
**Status:** Approved as Task 33 of the market remote-capability alignment plan

## Objective

Replace the service's static transport string selection with a grant-bound route
planner that can use authenticated LAN QUIC, public WebRTC direct, and TURN relay
without allowing the UI to weaken identity, trust, capability, or relay-proof
requirements.

The market-facing default is automatic direct-to-relay connectivity. Route
selection must be explainable: every accepted or rejected candidate records the
authenticated evidence, attempt outcome, and decision reason that caused the
planner to select or reject it.

## Scope

Task 33 owns initial route planning and connection. Task 34 remains responsible
for recovery after an active route fails, including ICE restart, reconnect, and
mid-session migration.

This task does not redesign signaling, grants, TransportMux lanes, TURN
credential issuance, or LAN discovery cryptography. It consumes the verified
identity, grant, capability, and runtime route evidence those components already
produce.

## Policy Semantics

The service normalizes the current IPC strings into a closed domain enum:

- `auto`: prefer authenticated LAN QUIC, then WebRTC direct, then TURN relay;
- `lan`: use the same safe candidate set but give authenticated LAN QUIC the
  first preference window;
- `wan`: prefer WebRTC direct, then TURN relay, with authenticated LAN QUIC as a
  final allowed fallback;
- `relay-only`: accept only a WebRTC candidate pair proven to use TURN relay;
- `diagnostic`: gather and evaluate authenticated candidates but do not connect
  a route or start media.

`lan` and `wan` are preferences, not trust exceptions and not hard reachability
constraints. `relay-only` is a hard constraint. The existing
`allow_lan_quic`, `allow_webrtc`, and `allow_relay` fields are hard eligibility
filters. `preferred_transport` adjusts ordering only within the eligible set; it
cannot re-enable a disabled route or override `relay-only`.

Unknown modes or preferred transports fail validation rather than silently
becoming `auto`. A policy that leaves no eligible route produces an explicit
policy failure.

## Domain Model

`mrd-session` owns transport-independent route policy and evidence:

- `RouteIntent` is the closed policy enum above;
- `RoutePolicy` contains intent, eligibility flags, and bounded timing policy;
- `RouteCandidate` identifies `LanQuic`, `WebRtcDirect`, or `WebRtcRelay` and
  carries sanitized authenticated evidence;
- `RouteCandidateOutcome` distinguishes ineligible, pending, connected,
  fallbackable failure, and terminal security failure;
- `RouteDecision` contains the selected route, stable reason code, ordered
  candidate evidence, and policy revision;
- `RouteFailureClass` separates policy/security failures from ordinary
  reachability failures.

Candidate evidence is route-specific:

- LAN QUIC binds the intended `DeviceId` to the signed announcement key ID,
  trusted public key and epoch, advertised QUIC capability, endpoint, and QUIC
  certificate fingerprint;
- WebRTC binds the authenticated signaling peer and grant-committed candidate
  fingerprints to the selected local and remote ICE candidate IDs and kinds;
- TURN additionally records sanitized allocation/server identity so a policy
  boolean can never masquerade as relay evidence.

Raw public keys, TURN credentials, and private endpoints do not cross the IPC
boundary. They may be used during validation, while persisted/audited evidence
uses stable hashes or sanitized identifiers.

## Application Boundaries

`mrd-application` exposes two use cases behind abstract ports.

`plan_route` is pure and deterministic. It validates the policy, current grant,
target identity, capabilities, and gathered candidate evidence. It returns an
ordered plan or a typed terminal rejection. Unit tests can cover ordering and
security rules without opening sockets.

`connect_route` executes that plan. Candidate providers gather LAN and ICE
observations concurrently after signaling identity authentication. They may
perform non-media preflight before consent, but connection racing and all media
or input remain gated by an active grant bound to the exact session, peer,
scopes, and policy revision.

The connector starts the preferred eligible attempt first, then starts the next
attempt after a short configurable preference stagger or an earlier definitive
fallbackable failure. Every attempt and the overall race have deadlines. The
first connected route is accepted only after its `TransportRouteSnapshot`
matches the planned route and evidence requirements; losing attempts are
cancelled and closed. Tests inject timing and fake connectors, so no test relies
on wall-clock races.

The application layer never imports Quinn, WebRTC peer-connection, ICE, or TURN
configuration types.

## Service Integration

`mrd-service` becomes the composition root:

1. Resolve the session's exact remote `DeviceId`, authenticated signaling
   identity, active grant, and policy revision.
2. Start signed LAN-registry lookup and WebRTC/ICE candidate gathering in
   parallel.
3. Project the LAN registry only through its unique controllable peer lookup;
   legacy diagnostic or ambiguous same-device records are never candidates.
4. Project WebRTC selected-pair statistics and authenticated TURN allocation
   metadata into transport-independent evidence.
5. Invoke `plan_route`, then `connect_route` when the intent is not diagnostic.
6. Publish the selected TransportMux only after runtime evidence validates the
   planned route.
7. Update session route state, policy snapshot, secure route evidence, audit,
   and telemetry from the same `RouteDecision`.

The current `handlers/control.rs` string chooser becomes policy parsing and
orchestration rather than claiming a route before a connection exists.
`handlers/session.rs` supplies the grant- and peer-bound session context.
`capabilities.rs` exposes route-planner capability checks using existing
runtime/static capability snapshots.

Existing IPC response shapes remain compatible in Task 33. Their human-readable
fields are projections of stable domain reason codes. `relay_required` describes
policy only; `selected_transport` is `none`/pending until verified runtime
evidence exists. The existing secure route-evidence surface records per-candidate
attempt state and the selected route.

## Security And Failure Rules

The planner fails closed for:

- missing, expired, mismatched, or policy-stale grants;
- target-device, signing-key, public-key, epoch, or certificate mismatch;
- revoked or suspended trust;
- candidate fingerprints not committed by authenticated signaling/grant data;
- a selected route whose runtime evidence does not match its planned kind;
- `relay-only` without a real selected relay candidate pair and matching TURN
  allocation evidence.

These are terminal security failures. They stop the race, cancel other
attempts, and never trigger a less secure fallback.

Unrelated unsigned LAN announcements are ignored before candidate creation. An
unsigned or spoofed announcement that claims the intended peer is recorded as a
security rejection and terminates the plan; it is not treated as a harmless LAN
timeout.

Reachability failures such as timeout, refused QUIC handshake, ICE failure, or
unreachable TURN allocation are fallbackable only to another policy-eligible,
fully authenticated route. Capability absence makes a route ineligible rather
than failed. Cancellation of losing attempts is not reported as a connection
failure.

No candidate may send desktop frames, control input, clipboard data, or bulk
content until the grant gate succeeds. Route selection never broadens approved
permission scopes.

## Timing And Concurrency

Timing is represented by an injected `RouteTimingPolicy` rather than scattered
sleep constants. Production defaults are bounded and conservative; tests use
zero or paused-time schedules. The policy includes candidate-gather timeout,
preference stagger, per-attempt timeout, and overall connection deadline.

Parallel gathering does not mean fastest-unverified-wins. Ranking is
deterministic, while the stagger prevents one slow preferred path from consuming
the entire connection budget. Relay starts immediately for `relay-only`; in
other intents it starts after direct preference or a definitive direct failure.

At most one winner is published. Cancellation is idempotent, and a late success
from a losing attempt cannot replace the winner.

## Observability

Each decision records:

- normalized intent and policy revision;
- candidate kind and sanitized identity/path evidence;
- eligibility or rejection reason;
- attempt start/completion time and bounded latency;
- fallback reason;
- selected route and stable decision reason;
- whether the failure was reachability, capability, policy, or security.

Audit and telemetry derive from this record. Diagnostic mode returns the same
candidate evaluation without activating a transport, making it useful for
support tooling without becoming an authorization bypass.

## Test Strategy

Domain and application tests prove policy normalization, ordering, terminal vs
fallbackable errors, timer behavior, loser cancellation, and late-success
suppression using fake candidates and deterministic time.

The service integration suite proves:

- authenticated LAN QUIC wins when valid;
- unsigned, spoofed, ambiguous, revoked, and mismatched LAN evidence never wins;
- WebRTC direct wins after a fallbackable LAN failure;
- TURN relay wins after direct failure or under `relay-only`;
- `relay-only` rejects host/srflx/prflx evidence even when the WebRTC connection
  reports connected;
- a security failure never causes insecure fallback;
- no media starts before the active grant;
- route reason, candidate evidence, policy revision, and sanitized relay proof
  are recorded;
- diagnostic mode gathers evidence without publishing a route.

Existing `mrd-session`, `mrd-application`, `mrd-service`, WebRTC, QUIC, and
TransportMux suites remain regression gates.
