# Rdesk-Server

FastAPI management server for Rdesk devices and session requests.

Current product path: `apps/Rdesk-Server`

## Quick Start

1. Create PostgreSQL database:

```sql
CREATE DATABASE rdesk_server;
```

2. Install dependencies:

```powershell
cd G:\Project\mini-remote-desktop\apps\Rdesk-Server
python -m venv .venv
.\.venv\Scripts\Activate.ps1
pip install -r requirements.txt
```

3. Configure environment:

```powershell
Copy-Item .env.example .env
```

4. Run server:

```powershell
python -m app.main
```

## API

- `POST /api/v1/auth/login`
- `GET /api/v1/devices`
- `GET /api/v1/devices/{id}`
- `POST /api/v1/sessions/request`
