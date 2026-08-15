# Self-hosted TURN

MRD uses coturn's time-limited REST credentials. The backend signs a username of the form
`expiry:user_id:session_id`; coturn validates it with the same shared HMAC secret. Static TURN
passwords are never shipped to Rdesk or mrd-service.

## Configure

1. Copy `turnserver.conf.example` to a private `turnserver.conf`.
2. Generate a secret, for example `openssl rand -hex 32`, and place the same value in
   `static-auth-secret` and backend environment variable `RDESK_TURN_AUTH_SECRET`.
3. Set `realm`, `server-name`, `external-ip`, and valid TLS certificate paths.
4. Open TCP/UDP 3478, TCP 5349, and the configured UDP relay range (49160-49260 here).
5. Configure the backend URLs, for example:

   ```text
   RDESK_TURN_URLS=turn:relay.example.com:3478?transport=udp,turn:relay.example.com:3478?transport=tcp,turns:relay.example.com:5349?transport=tcp
   RDESK_TURN_CREDENTIAL_TTL_SECONDS=600
   ```

Run coturn directly with `turnserver -c /etc/coturn/turnserver.conf`, or with the official-style
container image:

```powershell
docker run --rm --network host `
  -v ${PWD}/turnserver.conf:/etc/coturn/turnserver.conf:ro `
  -v ${PWD}/tls:/etc/coturn/tls:ro `
  coturn/coturn:latest -c /etc/coturn/turnserver.conf
```

Linux host networking is the simplest container setup. On Docker Desktop, publish 3478 UDP/TCP,
5349 TCP, and the entire relay UDP range explicitly instead.

## Verify forced relay

Obtain a credential from authenticated endpoint `POST /api/v1/turn/credentials`, then run:

```powershell
$env:MRD_TEST_TURN_URL='turn:127.0.0.1:3478?transport=udp'
$env:MRD_TEST_TURN_USERNAME='<temporary username>'
$env:MRD_TEST_TURN_CREDENTIAL='<temporary credential>'
cargo test -p mrd-transport-webrtc --test forced_relay -- --nocapture
```

The integration test forces `iceTransportPolicy=relay` and fails unless both sides of the selected
candidate pair are relay candidates. Without these environment variables it reports an explicit
skip so ordinary developer test runs do not depend on an external TURN server.

Never log the credential or full TURN URL with embedded credentials. mrd-service diagnostics expose
only URL classes (`turn/udp`, `turn/tcp`, `turns/tcp`) and selected candidate kinds.
