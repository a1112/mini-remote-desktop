# Rdesk-Server

FastAPI management server for Rdesk devices and session requests.

## Quick start

1. Create the PostgreSQL database and a dedicated application user.
2. Create a virtual environment and install dependencies.

```bash
cd apps/Rdesk-Server
python -m venv .venv
# Linux/macOS: . .venv/bin/activate
# Windows PowerShell: .\.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
```

3. Copy `.env.example` to `.env`, then replace the database password and JWT
   secret. Production mode rejects the checked-in development defaults.
4. Optionally configure all three `RDESK_INITIAL_ADMIN_*` variables for the
   first administrator. The server never creates an `admin/admin123` account.
5. Start the API.

```bash
python -m app.main
```

Development reload is opt-in through `RDESK_DEVELOPMENT_RELOAD=true`.

## Runtime topology

| Service | Default address | Purpose |
|---|---|---|
| Rdesk-Server | `127.0.0.1:9530` | Management API |
| Rdesk web UI | `127.0.0.1:9531` | Local frontend development |
| realtime-server | `127.0.0.1:9542` | Signaling and service health |
| mrd-service Web Bridge | `127.0.0.1:9533` | Optional browser bridge |

The Web Bridge is disabled by default. Enabling it requires
`MRD_WEB_BRIDGE_ENABLED=true` and a non-empty `MRD_WEB_BRIDGE_TOKEN`, even
when it only listens on loopback.

## Tests

```bash
python -m unittest discover -s tests -v
```

The repository's cross-platform workflow also compiles the backend and runs
these tests on Linux, Windows, and macOS.

## API

- `POST /api/v1/auth/login`
- `GET /api/v1/devices`
- `GET /api/v1/devices/{id}`
- `POST /api/v1/sessions/request`
