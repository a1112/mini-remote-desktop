# Rdesk-Server

FastAPI management server for Rdesk devices, sessions, and multi-region relay control.

## Quick start

1. Create a PostgreSQL database and a dedicated application user.
2. Create a virtual environment and install dependencies.

```bash
cd apps/Rdesk-Server
python -m venv .venv
# Linux/macOS: . .venv/bin/activate
# Windows PowerShell: .\\.venv\\Scripts\\Activate.ps1
python -m pip install -r requirements.txt
```

3. Copy `.env.example` to `.env`, then inject deployment-specific database,
   JWT, enrollment, and relay signing secrets. Production rejects checked-in
   development defaults.
4. Optionally configure all bootstrap variables for the first administrator;
   no built-in administrator credential is created.
5. Start the API.

```bash
python -m app.main
```

Development reload is opt-in through `RDESK_DEVELOPMENT_RELOAD=true`.

## Runtime topology

| Service | Default address | Purpose |
|---|---|---|
| Rdesk-Server | `127.0.0.1:9530` | Management API and relay directory |
| Rdesk web UI | `127.0.0.1:9531` | Local frontend development |
| realtime-server | `127.0.0.1:9542` | Signaling and service health |
| mrd-service Web Bridge | `127.0.0.1:9533` | Optional browser bridge |

Relay node endpoints require a dedicated mTLS-terminating proxy listed in
`RDESK_TRUSTED_MTLS_PROXY`. Keep Uvicorn proxy-header rewriting disabled
(`--no-proxy-headers` when not using `python -m app.main`) and configure the
terminator to strip every client-supplied `Forwarded`, `X-Forwarded-*`, and
relay authentication header before adding its verified metadata. Relay agents
can run on ordinary Linux or Windows hosts; their node credentials and
capacity/region heartbeats are stored in PostgreSQL.

## Tests

Install the development dependency set when running backend tests. It includes
runtime requirements plus asynchronous repository and TestClient support.

```bash
cd apps/Rdesk-Server
python -m pip install -r requirements-dev.txt
python -m pytest tests -q
```

The repository's cross-platform workflow also compiles the backend and runs
these tests on Linux, Windows, and macOS.

## API

- `POST /api/v1/auth/login`
- `GET /api/v1/auth/oidc/capabilities`, `POST /api/v1/auth/oidc/start`,
  `GET /api/v1/auth/oidc/callback`, `POST /api/v1/auth/oidc/complete`, and
  authenticated `POST /api/v1/auth/oidc/link/start` (when enabled)
- `GET /api/v1/devices`
- `GET /api/v1/devices/{id}`
- `POST /api/v1/sessions/request`
- `GET /healthz` for API liveness and `GET /readyz` for database and realtime
  readiness. Use readiness for routing or rollout checks. It returns HTTP 503
  until PostgreSQL and compatible local signaling both respond.
- Relay enrollment, heartbeat, directory, access, and migration endpoints under
  `/api/v1/relays`

## R-Auth / Keycloak sign-in

R-Auth and Rdesk use separate application sessions in the same Keycloak realm.
Register Rdesk as a confidential client in that realm and configure
`RDESK_OIDC_ENABLED=true`, the exact HTTPS realm issuer,
client ID, a client secret of at least 16 characters, an independent Fernet
transaction key, and an exact public HTTPS callback ending in
`/api/v1/auth/oidc/callback`. Inject the two secrets outside source control.
Register the exact callback with Keycloak and use authorization code flow with
PKCE S256, RS256 ID tokens and `client_secret_basic`. The
configured issuer's discovery, authorization, token and JWKS endpoints must
all use HTTPS on the issuer origin. Rdesk checks discovery before each flow.
When discovery advertises authorization-response issuer identification, the
callback must include the exact configured issuer. Browser SSO depends on the
system browser's existing Keycloak session and the realm's client policy.

At startup with OIDC enabled, an advisory-locked, versioned PostgreSQL migration
creates `oidc_identities` and `oidc_login_transactions`, leaving existing
accounts and devices untouched. Back up the database and review runtime schema
permissions before deployment. Rotating the transaction key invalidates any
in-progress login (maximum five minutes). All workers must share the same key,
database, issuer and client configuration. Admission limits are process-local
and use the socket peer by default. Behind a reverse proxy, configure
`RDESK_OIDC_TRUSTED_PROXIES` with its actual socket addresses or dedicated CIDRs.
On each trusted proxy replace all inbound `X-Real-IP` headers with one verified
client IP. Keep Uvicorn proxy-header rewriting disabled and configure an
independent ingress rate limit. Test with two clients behind the proxy and
confirm the clients receive separate IP budgets.
Configure HTTPS and prevent reverse-proxy, platform and application logs from
recording callback queries, request bodies, authorization headers or provider
responses. The built-in `python -m app.main` redacts callback queries in its
Uvicorn access log. Apply equivalent redaction to separately configured Uvicorn
instances and external proxies. Set the exact redirect URI in server configuration.

The client posts to `/start`, opens the returned `authorization_url`, and keeps
the returned `completion_token` private in the initiating app. It posts that
token to `/complete`. HTTP 202 means pending and HTTP 200 returns the Rdesk
login response exactly once. An expired, failed or consumed flow returns HTTP
400. The server stores the completion token as a hash and keeps provider tokens
server-side. The callback uses single-use state and PKCE and verifies the
signature, issuer, audience, nonce and time claims.

The first verified `(issuer, subject)` gets an independent Rdesk user with a
random internal username. Account association uses the issuer and subject. To attach an existing
Rdesk account, first authenticate that account and call `/link/start` with its
Rdesk bearer token, then perform the same OIDC callback and send that bearer to
`/complete` as well. The callback stores the verified subject in encrypted form
until the original authenticated account redeems the flow. Rdesk checks the
local session version at callback and redemption. Each Rdesk user can link one
OIDC identity. Use the realm's subject policy to verify equivalence across
clients. Identity reassignment, revocation and issuer migration require an
administrative process. Rdesk and Keycloak maintain separate session lifecycles,
so revoke issued credentials in Rdesk when disabling a user at the identity provider.

When both participants belong to the same verified R-Auth-linked account,
the target may approve the session without a separate prompt. The approval is
limited to the requested scopes and requires the signed controller and target
keys to match registered machine keys, or the session's authenticated browser
key. The server persists the original owner, issuer, key and device credential
versions and rechecks them on session inspection and credential issuance.
Unbinding either physical device revokes its automatically approved sessions.
Other accounts, unlinked local accounts, unpinned devices and guest sessions
continue to require the existing explicit approval.

An independent versioned PostgreSQL migration adds the nullable session
authorization snapshot on every startup, including when OIDC is disabled, so
existing databases remain compatible with the session model. Setting
`RDESK_OIDC_ENABLED=false` disables automatic approvals and ends existing
same-account leases at their next authority revalidation. Credentials already
issued to a session remain bounded by their original expiry.

## Automatic first-start device codes

Set `RDESK_DEVICE_SELF_ENROLLMENT_ENABLED=true` to permit first-start allocation.
The resident proves its persistent Ed25519 machine key through the version 1
`POST /api/v1/devices/self-enrollment-challenge` and
`POST /api/v1/devices/self-register` protocol. Challenges expire after 60 seconds,
are consumed once, and are limited across workers by the shared PostgreSQL
transaction lock and global/IP/key budgets. Only explicitly trusted socket
proxies may supply a single `X-Real-IP`; configure the proxy to replace inbound
values and bound request bodies.

New codes have no account owner and grant no screen permission. Fresh proof of
the same key returns the stored code after a restart or loss of the registration
file. A revoked device stays revoked, and a motherboard serial cannot recover
or replace an existing device. Administrator OTP registration and legacy
authenticated refresh remain available. The independent versioned migration
creates and verifies the two new tables without altering existing Device columns
or the public connection bootstrap. Back up PostgreSQL before deploying.

Self-enrolled devices can receive signaling credentials only for their pinned
key; the existing signed WebSocket challenge then proves its private key.
Ordinary HTTP device tokens remain bearer credentials protected by resident
storage. This feature does not claim proof-of-possession for every HTTP route.

## Guest temporary password access

The resident can publish a short lived, machine signed temporary password verifier and remain remotely accessible without account login. The browser controller also works without user login, but local attended approval is still required. See [the API and crypto contract](docs/guest-browser-temporary-access.md) for configuration, rotation/revocation, separate guest credentials and schema migration.
