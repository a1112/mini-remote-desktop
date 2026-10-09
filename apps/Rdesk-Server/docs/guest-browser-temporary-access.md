# Guest browser temporary access

A resident physical target and its browser controller need no user login. The target still requires its resident service, a registered durable machine key, valid device credentials, and local attended consent. The browser uses its own nonextractable Ed25519 key and WebRTC; it does not connect to a local mrd-service.

## Configuration

Enable explicitly with RDESK_GUEST_BROWSER_ENABLED=true. RDESK_PUBLIC_API_URL must be the exact public HTTPS API base. Existing trusted WSS server pins, authenticated sidecar identity lookup, relay directory signing keys and protected RDESK_DEVICE_SERIAL_PEPPER must be configured. The pepper is reused only with new, distinct HMAC domains. When behind a trusted reverse proxy, configure RDESK_DEVICE_SELF_ENROLLMENT_TRUSTED_PROXIES and ensure the proxy replaces X-Real-IP.

Defaults are 300 global, 10 source IP and 5 target device attempts per minute. RDESK_GUEST_BROWSER_GLOBAL_PER_MINUTE, RDESK_GUEST_BROWSER_IP_PER_MINUTE and RDESK_GUEST_BROWSER_DEVICE_PER_MINUTE change these bounded limits. Attempts are committed before verification, shared across API workers and retained on failures; PostgreSQL uses a transaction advisory lock for counting and insertion.

## Resident publication

POST /api/v1/devices/temporary-access uses the existing physical X-Rdesk-Device-Authorization header plus a contextual machine Ed25519 signature. Browser or user credentials cannot publish. The body is {key_id,public_key,access_json,signature}; keys and signature are lowercase hex, access_json is the exact signed UTF-8 JSON string.

The document is {device_id,auth_version,generation,enabled,expires_at_ms,salt,verifier,allowed_scopes}. Enabled publications expire within 600 seconds. The password is eight characters from ABCDEFGHJKLMNPQRSTUVWXYZ23456789, case sensitive. Native code derives verifier=PBKDF2-HMAC-SHA256(password ASCII,salt raw16,600000,dkLen32); salt and verifier are hex. Allowed scopes are sorted, unique screen.view plus optional input.keyboard/input.pointer. Disabled publications have null expiry/salt/verifier and empty scopes.

Canonical bytes are UTF-8 POST, the exact public API base plus /devices/temporary-access, key_id, and SHA256(access_json UTF-8) lowercase hex, separated by LF, without a trailing LF. Signed bytes concatenate MRD_CONTEXT_SIGNATURE_V1, big-endian u16 domain length, MRD_DEVICE_TEMPORARY_ACCESS_V1, big-endian u64 canonical byte length and canonical bytes. A shared synthetic vector is tests/fixtures/temporary-access-publication-v1.json.

Generation is strictly monotonic; an exact retry of the same generation and signed document is idempotent. GET /api/v1/devices/temporary-access returns only {enabled,ready,expires_at_ms,generation,reason}; neither endpoint returns the password. The server stores a domain-separated HMAC of the verifier and a separate HMAC publication digest. It never persists a plaintext password, bare verifier or unkeyed verifier-bearing document digest. Guest password derivation runs outside the event loop.

## Browser contract

POST /api/v1/guest-browser-sessions has the same fields as BrowserSessionCreateIn plus temporary_password. It requires no Authorization user token. Wrong code, missing/disabled/expired publication and wrong password use the same guest_access_invalid response. A successful response is the existing BrowserSessionOut plus http_credential:{token,expires_at_ms}. The original request commitment and canonical WAN request remain unchanged.

GET /api/v1/guest-browser-sessions/{session_id}, POST .../{session_id}/close and POST .../{session_id}/relay-access require Bearer http_credential.token. Its type is guest_browser_http and audience is rdesk-guest-browser-http; it cannot authorize account APIs, physical device APIs or another session. The signaling token has distinct type guest_browser_signaling. Both bind the session, controller public-key ID, target, target auth version, temporary access generation, tenant, scopes and short expiry. User ID is explicitly null; no fake User is created.

Native target endpoints stay /device-sessions/{id} and /relays/access with physical credentials. Guest session responses add authority_kind=temporary_password, temporary_access_generation and target_auth_version. Account responses retain their exact prior shape. Approval requires local consent, includes screen.view and cannot widen requested scopes or media profile. Grant and policy expiries are capped by the original temporary authority expiry.

Rotation revokes pending requests and makes the previous password unusable. Approved sessions remain valid only to their original short expiry. Disabling revokes all guest principals, grants and relay reservations. Target credential revocation/version changes invalidate guest authority. Credential issuance re-locks and revalidates authoritative rows after awaited sidecar identity lookup, and its target key must match the durable registered machine key.

## Schema upgrade and verification

migrate_add_guest_temporary_access upgrades the known account-only browser/session schemas, adds explicit mutually exclusive account/temporary_password CHECK domains, and creates verifier/attempt tables. Nullable user IDs are accepted only in the validated guest authority domain. Existing account ownership checks stay in effect. PostgreSQL alters in place in the startup transaction; SQLite legacy rebuilding requires offline foreign-key migration, while fresh schemas need no rebuild. Repeated startup verifies the owned schema and rejects drift.

Run pytest tests/test_guest_browser_session_api.py tests/test_guest_authority_migration.py, then the complete backend suite. Deployment acceptance must also use real PostgreSQL, target service, browser decoder and native Mac screen/input observations.
