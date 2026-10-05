# Public connectivity completion

The user has authorized completing implementation, replacing the installed local service, deploying to Tencent, and committing/pushing. Work is isolated on `codex/public-connectivity-complete`; unrelated primary-checkout deletions remain untouched.

## Intended behavior

- New device registrations receive a persistent, unique, ten-character ASCII numeric code, including leading zeroes. Database constraints arbitrate concurrent allocation; enrollment replay and ownership controls remain intact. Existing device codes continue to work.
- The client shows local service availability separately from authenticated public signaling. API reachability never implies that a device is online. Desktop and mobile use the same secret-free status projection.
- The resident service owns enrollment, encrypted credentials, refresh, reconnect, authenticated signaling, signed relay-directory verification, and attended WAN sessions. Closing the UI does not disconnect presence. Management IPC does not expose device JWTs or grant arbitrary SYSTEM capabilities.
- Tencent runs a publicly trusted HTTPS endpoint, authenticated signaling, STUN/TURN UDP/TCP/TLS, and a leased, authenticated relay agent. One real server uses an explicitly signed zero-backup policy; multiple-node policy retains topology constraints. No fictitious failure domains.
- WAN attempts support authenticated direct candidates and TURN fallback without weakening consent, key binding, or permission scopes. macOS starts a protected user service and can use its existing platform transport/media implementations.

## Work ownership and sequence

1. Backend allocation: add failing tests for width, leading zeroes, collision retry, non-code integrity failures and concurrency; implement bounded SAVEPOINT allocation in `device.py` and `device_enrollment.py`; run ownership, session, migration and isolated PostgreSQL concurrency checks.
2. Public infrastructure and policy: back up project configurations/database, add signed persisted `max_backups` policy, issue IP certificate with ACME staging first, configure renewal and a dedicated mTLS control proxy, build supported coturn and relay agent, enroll/approve the real node, then verify actual public allocations and traffic.
3. Shared service connectivity: add secret-free IPC status and constrained enrollment commands, protected credential persistence, validated public bootstrap settings, and a service-owned supervisor shared by Windows/macOS. Test redaction, endpoint restrictions, restart persistence, invalid registration, authenticated status and cancellation.
4. Client: replace ambiguous status panel, add mobile status, format ten-digit codes as 3/3/4 while copying raw digits, normalize spaces without dropping invalid characters, preserve historical IDs, and route public enrollment to the resident service. Verify meaningful UI tests, type checking and production build.
5. Portable runtime: implement macOS Keychain-backed secret protection and owner-only Unix state/socket paths, stable hardware/installation identity, real service startup and bounded cleanup. Run platform-independent checks and target compilation where available; require a real Mac for final media acceptance.
6. Direct-first WAN and ordinary-user product IPC: retain signed route policy and authorization, verify route evidence, keep privileged filesystem/process commands inaccessible. Run transport, consent, signaling and product IPC regression checks.
7. Integration: review changes, build release binaries, replace installed Windows service with rollback, verify ordinary-user IPC and authenticated public presence across UI closure/service restart. Commit and push after checks; synchronize Tencent source and verify deployment health and live TURN data. Record exact live acceptance and outstanding external-device evidence.

## Verification and rollback

The live acceptance additionally exposed first-WSS Rustls provider ambiguity and
an 80 ms clock difference. WSS now selects the configured crypto provider before
first use and monitors its process owner. Transport receivers defer a future
message for at most two seconds until its original issuance time, then invoke
the unchanged strict verifier with a fresh clock reading. They do not change
claims, extend expiry, accept a not-yet-valid message or update the OS clock.

Use focused tests before implementation and then relevant regression suites. Do not replace live acceptance with fixtures. Protect all credentials and backups; print only non-secret public keys, URLs, counts, and status. Preserve other NGINX applications and ports. Existing source, environment, certificate and service backups must allow scoped rollback. A single host cannot certify multi-region failover, and no implementation guarantees connections through every possible firewall.

IP certificate capability is supported by the official [Let's Encrypt IP certificate announcement](https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability) and [Certbot webroot instructions](https://letsencrypt.org/2026/03/11/shorter-certs-certbot); short lifetime requires automatic renewal.
