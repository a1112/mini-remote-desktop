# Public connectivity — 2026-10-05

## Product behavior

Newly enrolled devices receive a stable, unique 10-digit decimal code. Leading
zeroes are preserved; the UI displays `123 456 7890` and copies `1234567890`.
Existing device codes remain usable. Allocation uses a server-side keyed digest,
database uniqueness and bounded collision retries. A device refresh cannot
replace an existing code or bypass concurrent credential revocation.

The resident service owns protected credentials, refresh, reconnect and
authenticated presence. The client separately shows resident-service health,
HTTPS reachability, device enrollment and authenticated signaling. Device JWTs
are not returned to the WebView. Public sessions retain device/key binding,
signed route policy and attended permission checks. Windows capture and input
run in the authenticated interactive Agent; Session 0 has no capture/input
fallback. Streaming requires an actual accepted frame, and input acknowledgments
require an actual Agent result. Resource cleanup survives lost responses and
native permission expiry without renewing the original authorization.

## Tencent endpoints and operation

- API: `https://175.178.16.90/rdesk/api/v1`
- Authenticated signaling: `wss://175.178.16.90/rdesk-realtime/ws`
- Public bootstrap: `/rdesk/api/v1/public/connection-config`
- TURN UDP/TCP: `175.178.16.90:3478`; TURN TLS: `175.178.16.90:5349`
- Relay UDP ports: `49160–49260`

The API, realtime server, coturn, authenticated relay agent, private mTLS control
proxy and bounded control socket are supervised services. Certificate renewal
is enabled and its deployment hook verifies the credential snapshot used by
coturn. Existing unrelated NGINX applications are preserved.

There is one real relay node. Its signed policy has zero backup nodes,
16 allocations and 200 Mbps total capacity; no multi-region failover is claimed.
Health requires a live nominated relay pair, DTLS, real control/media round trips
and nonzero counter increments bound to the same connection. Upstream ICE pair
payload counters are not fabricated when the upstream implementation returns
zero. Candidate identifiers accept the actual RFC foundation encoding while
remaining bounded.

## Verification evidence

- Frontend: 753 tests, type checking and production bundle passed.
- Backend: 598 tests and 13 subtests, including isolated PostgreSQL enrollment,
  collision/concurrency, refresh/revocation and authorization-race checks.
- Agent: 245 tests, including real child-process bootstrap.
- Windows native fixture: signed Agent IPC, actual DXGI/OpenH264 frames, service
  media delivery and decoding, input result handling, lost-response cleanup,
  native expiry and desktop changes. Consent/injection test adapters do not
  bypass the product's real local confirmation.
- Public WSS first-connect regression: real server challenge failed before the
  explicit Rustls provider selection and passed after it. An exited signaling
  task now terminates its resident owner so SCM recovery can restart it.
- Transport receivers wait up to two seconds for a slightly early message's
  original issue time, then authenticate it against a fresh real clock. Signed
  claims, challenge/token expiry and replay rules remain unchanged. Waiting is
  cancellable and does not hold the realtime server's global core lock.
- Relay: strict installer/preflight, stable backend lease, real relay traffic and
  signed-directory request/approval/access workflow passed. Deployment backups
  remain under `/root/mrd-public-connectivity-20261005/backup`.

Machine-specific installation hashes and the final source revision are recorded
in the deployment acceptance manifest, separate from credentials.

## macOS controller package

The manual `macOS client package` workflow builds Apple Silicon, Intel or both.
It verifies native Mach-O architecture, nested service resources, signing and
privacy metadata, and executes a real two-start IPC/Keychain persistence smoke
test before publishing an artifact. Packages use ad-hoc development signing;
normal macOS application approval, Keychain and privacy prompts remain enabled.

macOS can be the controller for this Windows device. Actual Mac-to-Windows
desktop acceptance requires the user's Mac and local Windows confirmation.
macOS target-side keyboard/mouse injection is a separate unfinished platform
capability and is not certified by this Windows-host acceptance.

For direct Windows builds, the UI needs the `app/custom-protocol` feature to
embed its production frontend. The canonical installation directory is
`C:\Program Files\MiniRemoteDesktop`; run `install-mrd-service.ps1` with
`-InstallClient` to install the UI, resident service and Agent together.
