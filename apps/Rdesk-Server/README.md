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
- `GET /api/v1/devices`
- `GET /api/v1/devices/{id}`
- `POST /api/v1/sessions/request`
- Relay enrollment, heartbeat, directory, access, and migration endpoints under
  `/api/v1/relays`

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
