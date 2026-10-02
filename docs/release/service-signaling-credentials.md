# Service signaling credential configuration

The local service connects to authenticated WAN signaling when `MRD_SIGNAL_URL` is set. Configure the following values in the service's protected environment:

| Variable | Meaning |
| --- | --- |
| `MRD_SIGNAL_URL` | Secure WebSocket endpoint, for example `wss://signal.example/ws`. |
| `MRD_SIGNAL_DEVICE_TOKEN` | Device JWT returned by authenticated device registration. Never use an administrator token. |
| `MRD_LAN_DEVICE_ID` | Set the service's startup device ID to the ID returned by backend registration. A default `lan-*` ID will not match a backend device JWT. |
| `MRD_LAN_DEVICE_NAME` | Optional startup display name. |
| `MRD_SIGNAL_AUTH_URL` | Optional full credential endpoint, for example `https://api.example/api/v1/realtime/device-credentials`. |
| `MRD_WAN_SESSION_API_URL` | Existing WAN API root, for example `https://api.example/api/v1`. When the explicit authentication URL is absent, signaling derives `realtime/device-credentials` from this root. |
| `MRD_SIGNAL_ROLE` | `agent` (default) or `controller`. The credential binds this exact role. |
| `MRD_SIGNAL_SERVER_KEY_ID` | Optional pinned signaling server key ID. Existing pinning across reconnects still applies. |

The service exchanges its device JWT before opening the WebSocket. Each reconnect obtains a fresh credential bound to the registered device ID, the service's machine Ed25519 key ID, and the configured role. It verifies all response bindings and expiry before signing the server challenge. Rejection bodies and credential data never appear in runtime health errors.

Credential endpoints require HTTPS, with HTTP permitted only on loopback for local development. Redirects are disabled, responses are limited to 16 KiB, and the complete exchange uses `MRD_SIGNAL_CONNECT_TIMEOUT_MS` (default 10 seconds) as its deadline.

If both authentication URLs are absent, `MRD_SIGNAL_DEVICE_TOKEN` must instead contain an already issued signaling credential bound to the service's current machine key and role. This mode supports tests and explicit managed provisioning; an ordinary device JWT cannot authenticate directly against the realtime server.

The current local IPC device-registration command supplies a device ID and name. It does not transfer the desktop application's device JWT into the service or rebuild an already running signaling configuration after its ID changes. Deployments must provision the matching device ID and JWT in the existing protected service environment before starting the service. This change does not provide automatic GUI-to-service credential provisioning or public P2P direct-first routing.
