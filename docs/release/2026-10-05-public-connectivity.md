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

Login, initialization, enrollment and credential recovery bind only this
resident's current device to the authenticated account. The service derives the
target and submits both user and protected device credentials to its configured
HTTPS origin. Windows requires the installed client in the active console;
the ordinary management pipe cannot borrow machine credentials for binding.
The binding capability is negotiated before sending the ephemeral user token.
The backend rechecks the immutable device authentication snapshot under the
row lock, including credential version, revocation, tenant and machine identity.

Device inventory reads authenticated realtime presence through a private
loopback endpoint after user/tenant visibility checks. Configured query failures
display offline. The query has a shared two-second deadline and bounded batches
and response sizes; the public proxy blocks its internal route.

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

- Frontend: 760 tests, type checking and production bundle passed.
- Backend: 675 tests and 13 subtests, including isolated PostgreSQL enrollment,
  collision/concurrency, refresh/revocation and authorization-race checks.
- Binding boundary: real Windows management and uninstalled-process product
  pipe tests reject binding; own-device dual-credential HTTP tests passed.
- Installed Windows acceptance: the actual protected process-image/kernel
  boundary passed, and privileged Core IPC bound device `1501515774` to its
  authenticated owner. With the UI closed, SCM restart changed the service PID,
  automatically restored authenticated presence and kept the same device code
  and account binding. The server independently confirmed online ownership.
- Windows service/storage/realtime regression: 1,073 tests passed across
  44 suites; 20 existing platform/live tests were ignored separately from the
  explicit real installed-process acceptance.
- Portable lint regression: Linux workspace Clippy excluding the GTK application
  passed with warnings denied; 227 focused Linux tests passed. Windows service,
  Agent and registration regression passed 1,274 tests in 43 suites (28 existing
  ignored), and the native D3D11 renderer passed 25 tests. The Windows production
  UI checked successfully with the unified window-vibrancy dependency.
- Concurrent SQLite birth: a waiting observer reads until the original creator
  commits the sealed schema, with a bounded two-second wait. It never rebuilds
  an existing empty or corrupt file. Linux storage regression passed 27 tests
  after reproducing both the premature rejection and unwanted file mutation.
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
test and an exact-process bundled UI/window smoke before publishing an artifact.
Packages use ad-hoc development signing;
normal macOS application approval, Keychain and privacy prompts remain enabled.

The native Metal presentation layer must pass pointer events to WKWebView and
retain its first responder. An AppKit main-thread regression proves the actual
bare-view interception and the corrected production view policy. The bundle
also uses one window-vibrancy version shared with Tauri to avoid duplicate
Objective-C class definitions. Native package checks must pass on both
architectures before their artifacts are considered accepted.

macOS can be the controller for this Windows device. Actual Mac-to-Windows
desktop acceptance requires the user's Mac and local Windows confirmation.
macOS target-side keyboard/mouse injection is a separate unfinished platform
capability and is not certified by this Windows-host acceptance.

For direct Windows builds, the UI needs the `app/custom-protocol` feature to
embed its production frontend. The canonical installation directory is
`C:\Program Files\MiniRemoteDesktop`; run `install-mrd-service.ps1` with
`-InstallClient` to install the UI, resident service and Agent together.
