# Server Connectivity Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Repair authenticated device enrollment and realtime registration, then deploy the verified backend and signaling server to the existing Tencent Cloud services.

**Architecture:** Keep device enrollment and device JWT authentication in FastAPI. Exchange a device JWT for a purpose-specific HS256 signaling credential bound to the device ID, Ed25519 key ID and Controller/Agent role. The Rust sidecar validates that credential and retains the signed challenge and replay checks. Unify the default port at 9542 and advertise protocol versions 2 and 3.

**Tech Stack:** FastAPI, PyJWT, PostgreSQL, Rust/ring, Axum, reqwest, Tauri, React/Vitest.

## Scope and constraints

- Preserve the main checkout's unrelated mrd-service edits; work in the attached worktree.
- Preserve admin-issued enrollment and authenticated serial refresh; never restore anonymous inventory lookup.
- Keep the current attended WAN TURN route policy. Public NAT traversal and multi-region failover require separate real-network acceptance and must not be described as complete by this deployment.
- Update only serverall-rdesk.service and serverall-rdesk-realtime.service; preserve the shared nginx configuration and other applications.
- Never print JWT secrets, enrollment tokens, credential responses or deployment environment contents.

## Task 1: Device enrollment and client error recovery

Files: apps/Rdesk/src/app/services/deviceService.ts and tests; apps/Rdesk/src/app/components/DeviceRegisterModal.tsx and tests; apps/Rdesk/src/app/adapters/tauri; apps/Rdesk/src-tauri/src/main.rs; crates/mrd-device-registration.

1. Add failing tests for an explicit enrollment token, authenticated credential refresh and preserving server identity when the server is unreachable.
2. Run focused Vitest tests and native request contract tests to observe failures.
3. Add the enrollment UI and header, route through the existing Tauri adapter, and remove obsolete anonymous checks and destructive LAN fallback.
4. Validate configurable HTTPS/loopback HTTP URLs and redact credential-bearing failures.
5. Run frontend type checking and relevant tests.

## Task 2: Backend signaling credentials

Files: apps/Rdesk-Server/app/api/v1/realtime.py; app/schemas/realtime.py; app/services/signaling_credentials.py; app/core/config.py; app/core/response_security.py; tests/test_signaling_credentials.py; .env.example.

1. Add route tests using real device JWT authentication and a deterministic database adapter; require 401 for anonymous/user/revoked credentials, exact device/key/role binding, bounded expiry and no-store on success and errors.
2. Run `python -m pytest tests/test_signaling_credentials.py -q`; expect failures because the credential endpoint is absent.
3. Add `POST /api/v1/realtime/device-credentials` with `X-Rdesk-Device-Authorization`, body `{device_key_id, role}` and result `{token, expires_at_ms, device_id, device_key_id, role}`.
4. Sign HS256 claims `{sub:device_id, device_id, device_key_id, role, token_type:signaling, iss, aud, iat, exp}`; audience defaults to rdesk-signaling, lifetime defaults to 3600 seconds and is bounded to 3600.
5. Run focused and full backend suites, then record any database-dependent skips accurately.

## Task 3: Real production token verifier

Files: apps/realtime-server/src/backend_token.rs; src/main.rs; src/lib.rs; Cargo.toml; token and authenticated routing tests.

1. Add failing tests for valid tokens, tampering, expired/future/overlong credentials, wrong purpose/issuer/audience/role, duplicate claims and signed identity mismatch.
2. Implement strict HS256 verification using ring, base64url and typed serde claims.
3. Configure MRD_REALTIME_JWT_SECRET and MRD_REALTIME_JWT_ISSUER; default MRD_REALTIME_JWT_AUDIENCE to rdesk-signaling. Reject missing configuration at startup without exposing values.
4. Replace the executable's RejectAllBackendTokens with the real verifier.
5. Run `cargo test -p realtime-server --locked` and a real backend-issued-token WebSocket registration smoke test.

## Task 4: Service credential exchange

Files: apps/mrd-service/src/signaling/config.rs; runtime.rs and relevant tests.

1. Add failing tests for device JWT exchange, response binding, timeout/redirect/error handling and reconnect refresh.
2. Add MRD_SIGNAL_AUTH_URL or derive the credential endpoint from MRD_WAN_SESSION_API_URL. Fetch before opening the challenge-bound WebSocket handshake.
3. Preserve explicit pre-issued signaling-token mode when no auth URL is configured; keep secrets in zeroizing storage and redact network errors.
4. Run signaling runtime and WAN session regression suites.

## Task 5: Health compatibility

Files: apps/realtime-server/src/ws.rs; tests/fixtures/health.json; apps/Rdesk-Server/app/services/realtime_manager.py and tests.

1. Add failing actual-router health and default-port tests; share the response fixture across Rust and Python.
2. Emit protocol_version 3 and supported_protocol_versions [2,3], default port9542, preserve explicit override.
3. Backend accepts compatible version advertisements and rejects absent/malformed/incompatible versions.
4. Run Rust ws tests and Python realtime manager tests.

## Task 6: Review and deployment

1. Review the complete diff and obtain an independent code review; fix material findings and rerun affected checks.
2. Confirm local primary edits remain untouched and build the release sidecar on the Tencent host from the validated snapshot.
3. Back up the deployed source files, sidecar binary and relevant environment files; keep an explicit rollback path.
4. Set the sidecar verifier configuration from the existing backend JWT configuration without printing secrets.
5. Install only validated source files/binary; restart only the two Rdesk services.
6. Verify systemd active state, local/proxied health compatibility, anonymous denial and a real backend-issued signaling credential registration.
7. Record deployed revision, validation results and remaining public-network acceptance limitations in docs/release.
