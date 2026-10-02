# Authenticated server connectivity

Device enrollment requires an administrator-issued one-use token. The desktop's local-device card opens the enrollment form; the token is sent in `X-Rdesk-Device-Enrollment` and is never persisted. Later refresh uses `X-Rdesk-Device-Authorization: Bearer <device JWT>`. A refresh failure preserves the assigned remote device ID and reports an error.

For an already-enrolled device with an expired/revoked credential, an administrator uses `POST /api/v1/devices/{id}/credentials/admin-rotate` to issue a replacement device JWT. Enter it in the modal's **更新设备凭据** mode. The desktop validates it with the authenticated serial-refresh endpoint and checks the returned device ID before saving; a new first-enrollment code cannot recover an existing serial.

Set `VITE_RDESK_SERVER_URL` to the HTTPS API base ending in `/api/v1` (including any reverse-proxy prefix), or set native `RDESK_SERVER_URL` to override it. Plain HTTP is accepted only for loopback development.

Native enrollment/refresh requests disable environment proxy forwarding and redirects so credentials intended for a local endpoint cannot be forwarded to a configured HTTP proxy.

## Backend and realtime server

FastAPI uses the existing production `RDESK_JWT_SECRET` and `RDESK_JWT_ISSUER`. Configure a separate purpose audience:

```dotenv
RDESK_SIGNALING_JWT_AUDIENCE=rdesk-signaling
RDESK_SIGNALING_JWT_TTL_SECONDS=3600
```

The Rust executable requires the same signing secret and issuer, injected through its protected service environment:

```dotenv
MRD_REALTIME_JWT_SECRET=<same managed secret as RDESK_JWT_SECRET>
MRD_REALTIME_JWT_ISSUER=<same issuer as RDESK_JWT_ISSUER>
MRD_REALTIME_JWT_AUDIENCE=rdesk-signaling
MRD_REALTIME_BIND=127.0.0.1:9542
MRD_REALTIME_DEPLOYED=true
MRD_REALTIME_TLS_TERMINATED=true
```

Keep the sidecar bound to loopback behind the configured TLS proxy. Its `/health` response advertises protocol3 and supports versions2/3; FastAPI's manager checks both capabilities. An executable without a valid JWT verifier configuration fails startup.

`POST /api/v1/realtime/device-credentials` authenticates the device JWT against current database state and accepts `{ "device_key_id": "<64 lowercase hex SHA256 of Ed25519 public key>", "role": "Controller" }` (or `Agent`). It returns a purpose-specific JWT bound to the public device ID, signing key and role. Ordinary user/device JWTs are rejected by the sidecar. Responses and errors have `Cache-Control: no-store, private`.

Device revocation blocks new credential issuance immediately. Already-issued signaling credentials are verified offline and remain valid until their configured expiry (at most3600 seconds); reconnect obtains a fresh credential. This is distinct from database-checked device API authorization.

## Local service

Configure `MRD_SIGNAL_URL`, `MRD_SIGNAL_DEVICE_TOKEN` (the enrolled device JWT) and `MRD_SIGNAL_AUTH_URL` (the full HTTPS credential endpoint). The credential endpoint can also be derived from `MRD_WAN_SESSION_API_URL`. The service exchanges the token before opening the WebSocket challenge and validates the returned device/key/role/expiry. Requests have bounded timeouts/body sizes and do not follow redirects.

Set `MRD_LAN_DEVICE_ID` to the backend-assigned device ID before starting the service, so its machine identity matches the device JWT. See [service configuration](service-signaling-credentials.md) for all required environment settings.

For an explicit pre-issued signaling credential, omit the authentication URL and WAN API base; `MRD_SIGNAL_DEVICE_TOKEN` then contains the bound signaling JWT. This compatibility mode is primarily useful for integration tests and explicit managed provisioning.

The UI's local registration IPC currently carries device ID/name. The service's WAN credentials still require service configuration; desktop enrollment alone does not automatically enable WAN sessions.

## Acceptance boundary

This repair enables authenticated registration and compatible health checks. The existing product WAN policy selects TURN relay and attended approval. Public direct-first P2P NAT traversal, unattended authorization and multi-region relay failover require their separate implementation/real-network acceptance; they are not certified by these local contract tests or this server deployment.
