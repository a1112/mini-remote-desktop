# Tencent Cloud server connectivity deployment — 2026-10-03

Code revision: `e6423ac9194aea305a846fa5d773df9a5386d623`.

The changes were developed in an isolated worktree and fast-forwarded into local `main`. The three preexisting edits in mrd-service `main.rs`, `windows_pipe.rs` and `windows_sessions.rs` were preserved byte-for-byte and were excluded from deployment.

## Delivered

- Explicit one-use enrollment, authenticated device refresh, preserved server identity on network failures, initialization race prevention, and a reachable desktop credential-recovery flow.
- Backend device authentication exchanges a device JWT for a signaling JWT bound to the device ID, Ed25519 key ID and Controller/Agent role.
- Production realtime-server now verifies HS256 purpose credentials instead of rejecting every token. Invalid configuration prevents startup; challenge, replay, key and role checks remain enforced.
- Default signal port9542; live health advertises protocol3 and compatible versions2/3. Backend health validation uses this contract.
- Local service exchanges credentials before the WebSocket challenge and obtains a fresh credential on reconnect.

## Verification

| Check | Result |
| --- | --- |
| Full backend suite on Tencent host, dedicated PostgreSQL database, Linux release executable | 517 passed, 13 subtests passed, no skipped tests |
| Full frontend Vitest suite | 717 passed across55 files |
| Final targeted enrollment/recovery frontend regressions | 95 passed |
| TypeScript type check and Vite production build | Passed; existing large-chunk notice |
| Native app cargo check | Passed |
| Native registration adapter tests | 10 passed |
| Realtime tests on Windows and Linux release profile | 19 passed on each platform |
| Service credential exchange tests | 4 passed, including9 HTTP failure/binding cases |
| Existing service signaling integration / unit selection | 30 /12 passed |
| WAN and relay regression suites | 111 passed,12 explicitly ignored lab tests |
| Independent review | All concrete P2 findings fixed; no remaining P0/P1/P2 blockers in repaired scope |

The existing backend test suite emits dependency/deprecation warnings. PostgreSQL integration tests used a dedicated temporary database and role, separate from production data.

## Deployment and live checks

Host: `tencent-lighthouse` (`175.178.16.90`). Updated only `serverall-rdesk.service` and `serverall-rdesk-realtime.service`.

- Both systemd units active after restart.
- [Backend HTTPS health](https://175.178.16.90/rdesk/healthz): `status=ok`.
- [Realtime HTTPS health](https://175.178.16.90/rdesk-realtime/health): `status=ok`, protocol3, supported[2,3]. External checks from the local Windows host passed with the existing certificate explicitly trusted.
- Actual production HTTPS device-authenticated credential issuance followed by WSS signed registration succeeded. A mismatched signed role was rejected; anonymous issuance returned401 with no-store headers. Verification used a temporary device row, removed in a finally block.
- Backend realtime manager reported reachable/ok. Final signal presence and route counts returned to zero.
- Existing shared nginx routes and other applications were preserved. The existing IP TLS certificate remains in use; it is self-signed and requires trust provisioning for native clients.

Installed signal executable SHA256: `1078c7e637aae1358170b18cd10f3e600220c2e1ea4432eb666a4061001b2585`.

Server-side backup and rollback script:

```text
/home/ubuntu/projects/.deployment/audits/connectivity-20261003/backup-20261002T184935Z/
python3 /home/ubuntu/projects/.deployment/audits/connectivity-20261003/backup-20261002T184935Z/rollback.py
```

The rollback restores the previous source revision, signal binary and protected sidecar environment, then restarts the same two services. Backend credentials were copied internally into the sidecar environment without being printed.

## Remaining setup and acceptance

- At inspection, production had no administrators or registered devices. Administrator initialization was offered separately; no account was created without user selection. Device enrollment requires an administrator to issue the first one-use code.
- Desktop and mrd-service source fixes are local; existing installed native clients were not replaced by this backend deployment. WAN service credentials and matching `MRD_LAN_DEVICE_ID` must be configured as described in [service configuration](service-signaling-credentials.md). Existing IPC does not automatically provision JWTs or rebuild live signaling configuration.
- Public direct-first P2P NAT traversal remains unimplemented in the active product route policy. TURN public-network/two-region failover acceptance and unattended operation remain outside this deployment. No local test result certifies those capabilities.
- Signaling credentials are verified offline until expiry, bounded to3600 seconds. Device revocation prevents new credential issuance immediately; already-issued signaling credentials retain this documented expiry window.

See [authenticated connectivity configuration](server-connectivity-configuration.md) for enrollment, admin credential recovery and protected environment settings.
