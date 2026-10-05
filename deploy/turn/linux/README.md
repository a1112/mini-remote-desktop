# Public single-node deployment

The production endpoint is `175.178.16.90`. This deployment uses one real node
in `ap-guangzhou`, with signed `max_backups=0`. It provides public connections
without claiming multi-node failover. The default backend configuration still
requires a backup; an operator must explicitly select the single-node policy.
An approved grant stores its redundancy policy, and the signed directory uses
that stored value. Changing the global policy does not silently change a grant.

Public addresses:

- API: `https://175.178.16.90/rdesk/api/v1`
- Client configuration: `/public/connection-config` under that API
- Signaling: `wss://175.178.16.90/rdesk-realtime/ws`
- TURN: UDP/TCP `3478`, TLS `5349`, relay ports `49160–49260`
- Node control: private-CA HTTPS/mTLS `9443`

The control listener has its own `mrd-relay-control.service` and HAProxy
configuration. It preserves the shared nginx applications on ports 80 and 443.
Enrollment and certificate pickup use a node enrollment token. Heartbeat,
renewal, and rotation require a verified client certificate. HAProxy removes
incoming forwarding and client-certificate headers before creating its own
verified certificate fingerprint; uvicorn disables proxy-header rewriting.
The backend accepts that identity only from its configured loopback proxy.

## Capacity

`max_egress_bps` is a configured admission limit in bits/s. Coturn uses bytes/s.
Both installers derive `bps-capacity = max_egress_bps / 8` and
`max-bps = floor(bps-capacity / max_allocations)`. The broker rejects mismatches,
noncanonical numbers, and zero per-allocation capacity. This prevents a fixed
per-allocation limit from consuming the whole node capacity on its first use.
The production node declares 16 allocations and 200 Mbit/s, giving each
allocation a 12.5 Mbit/s ceiling. These limits do not certify cloud bandwidth.

The local health probe uses one UDP relay path and still requires real
allocation, permission, nominated relay/relay candidates, and bidirectional
control/media packets. Public UDP, TCP, and TLS are tested independently from
an external device; a local proof is not public reachability evidence.
With webrtc-ice 0.12, its candidate-pair payload counters are unimplemented.
The probe measures native SCTP application packet/payload counter changes and
requires simultaneous ICE transport byte growth. Both checkpoints bind the
same physical peer, nominated relay pair, channel, and transport; changed
identities, reset counters, and zero traffic are rejected. Candidate identifiers
retain the exact RFC 5245 foundation alphabet, including `+` and `/`.

## Public certificate renewal

Use Certbot 5.4 or newer with Let's Encrypt's `shortlived` profile and IP HTTP-01.
The dedicated webroot is `/var/www/mrd-acme`; nginx serves only the challenge
path there. Issue and verify a staging certificate first, then issue the
production `mrd-public-ip` lineage. All existing shared routes remain intact.

`mrd-public-tls-renew.timer` checks every six hours and persists missed runs.
Install the deploy hook as `/usr/local/libexec/mrd-public-tls-deploy`. It checks
the IP chain, remaining validity, and matching key, reloads nginx, updates the
protected coturn certificate sources, and requests an authenticated secret
rotation. The broker owns draining and restarting coturn to refresh systemd
credential snapshots. Failed deployment is retried on the next timer run;
the protected certificate fingerprint avoids repeated rotations.

The hook's deployment-specific lineage, node ID, and administrator credential
path are constants. Adjust them together when deploying another installation.
The administrator JSON contains `username`, `email`, and `password`; keep it
root-owned mode `0600` under a root-only directory. Bootstrap is disabled after
the account is created. Never put passwords, tokens, or private keys in command
arguments or repository files.

## Stable signaling identity

The realtime service loads its independently generated Ed25519 PKCS8 key via
systemd `LoadCredential`. Its counter is a service-owned mode `0600` file in
`/var/lib/mrd-realtime`. The public client metadata contains the SHA-256 key ID
of the raw public key. Restart keeps that identity and skips previously reserved
signing counters. Never replace the counter with an older snapshot during a
rollback or reuse the directory signing key for signaling.

## Backup and recovery

The initial production backup is root-only at
`/root/mrd-public-connectivity-20261005/backup`. It contains scoped nginx/unit/
environment/certificate files, source manifests and originals, the pre-change
database dump, and the original node capacity. Revalidate nginx before reload
and restore only paths named in those manifests. Database rollback requires a
maintenance recovery decision after new device registrations or grants exist;
blindly restoring the initial dump would lose those records.

Preserve agent identity/request sequences, runtime fences, secret versions, and
signaling counters. A failed fresh installer can remove broker state while
leaving a higher runtime generation fence. Recover the same secret through the
authenticated broker and advance its generation through controlled restarts to
the protected minimum; never erase the fence to make validation pass.

Run `verify-relay-node.sh`, the deployment contract checks, an external STUN
Binding request, and external forced relay/relay control/media tests for each
advertised transport. Verify an available node and advancing authenticated
heartbeats afterward. Multi-region recovery remains a separate acceptance
matrix in `docs/release/multi-region-turn-relay-acceptance.md`.
