use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_identity::DeviceIdentity;
use mrd_proto::{BackendRole, DeviceId};
use mrd_signal_proto::{
    AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, RegisterPayload, SignalEnvelope,
};
use realtime_server::{
    ws::{build_router, RealtimeAppState, ServerRuntimeConfig},
    BackendTokenError, BackendTokenVerifier, ConnectionId, CoreConfig, RealtimeCore,
    VerifiedBackendToken,
};
use ring::{hmac, rand::SystemRandom};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const SECRET: [u8; 32] = [7; 32];
const CONTEXT: &[u8] = b"MRD_REALTIME_PRESENCE_QUERY_V1\0";
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn bearer(context: &[u8]) -> String {
    format!(
        "Bearer {}",
        URL_SAFE_NO_PAD
            .encode(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &SECRET), context).as_ref())
    )
}
struct Tokens(HashMap<String, VerifiedBackendToken>);
impl BackendTokenVerifier for Tokens {
    fn verify(&self, token: &str, _: u64) -> Result<VerifiedBackendToken, BackendTokenError> {
        self.0.get(token).cloned().ok_or(BackendTokenError::Invalid)
    }
}
struct Fixture {
    address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn start(secret: Option<&str>, entries: &[(&str, u64, u64)]) -> Self {
        let config = CoreConfig {
            server_device_id: DeviceId("server".into()),
            challenge_ttl_ms: 10_000,
            presence_ttl_ms: 30_000,
            route_ttl_ms: 60_000,
            max_connections: 256,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        };
        let mut tokens = HashMap::new();
        let mut identities = Vec::new();
        for (index, (device_id, _last_seen, expiry)) in entries.iter().enumerate() {
            let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
            tokens.insert(
                index.to_string(),
                VerifiedBackendToken {
                    device_id: DeviceId((*device_id).into()),
                    device_key_id: identity.key_id().into(),
                    role: BackendRole::Peer,
                    expires_at_ms: *expiry,
                },
            );
            identities.push(identity);
        }
        let mut core = RealtimeCore::new(config.clone(), Arc::new(Tokens(tokens))).unwrap();
        for (index, (device_id, last_seen, _expiry)) in entries.iter().enumerate() {
            let connection = ConnectionId::from_bytes([(index + 1) as u8; 16]).unwrap();
            let challenge = core.open_connection(connection, *last_seen).unwrap();
            let identity = &identities[index];
            let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::Register(
                AuthenticatedRegister::sign(
                    identity,
                    RegisterPayload {
                        claims: AuthClaims {
                            issuer_device_id: DeviceId((*device_id).into()),
                            issuer_key_id: identity.key_id().into(),
                            intended_peer_device_id: DeviceId("server".into()),
                            issued_at_ms: *last_seen,
                            expires_at_ms: last_seen + 60_000,
                            counter: 1,
                            nonce: [1; 16],
                        },
                        role: BackendRole::Peer,
                        device_name: "Device".into(),
                        backend_device_token: index.to_string(),
                        challenge_id: challenge.challenge_id,
                        challenge_nonce: challenge.challenge_nonce,
                    },
                )
                .unwrap(),
            ));
            core.handle(connection, envelope, *last_seen).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let runtime = ServerRuntimeConfig {
            bind_addr: address,
            secure_websocket_required: false,
            max_message_bytes: 512 * 1024,
            outbound_queue_capacity: 64,
            prune_interval: Duration::from_secs(10),
            core: config,
        };
        let mut state = RealtimeAppState::new(core, runtime);
        if let Some(secret) = secret {
            state = state.with_presence_secret(secret).unwrap();
        }
        let task = tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });
        Self { address, task }
    }
    async fn query(&self, authorization: &[&str], body: &str) -> (u16, String) {
        let headers = authorization
            .iter()
            .map(|value| format!("Authorization: {value}\r\n"))
            .collect::<String>();
        let request = format!("POST /internal/presence HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}", self.address, body.len());
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut stream = tokio::net::TcpStream::connect(self.address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            read_http_response(&mut stream).await
        })
        .await
        .unwrap()
    }
}

async fn read_http_response(stream: &mut (impl AsyncRead + Unpin)) -> (u16, String) {
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    const MAX_RESPONSE_BYTES: usize = 64 * 1024;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        assert!(
            bytes.len() < MAX_HEADER_BYTES,
            "HTTP headers exceed fixture bound"
        );
        let mut chunk = [0; 1024];
        let available = chunk.len().min(MAX_HEADER_BYTES - bytes.len());
        let received = stream.read(&mut chunk[..available]).await.unwrap();
        assert!(
            received > 0,
            "connection ended before complete HTTP headers"
        );
        bytes.extend_from_slice(&chunk[..received]);
    };
    let headers = std::str::from_utf8(&bytes[..header_end - 4]).unwrap();
    let mut lines = headers.split("\r\n");
    let mut status_line = lines.next().unwrap().splitn(3, ' ');
    assert_eq!(status_line.next(), Some("HTTP/1.1"));
    let code = status_line.next().unwrap();
    assert!(code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()));
    let status: u16 = code.parse().unwrap();
    assert!((100..=599).contains(&status));
    let mut content_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        assert!(!name.eq_ignore_ascii_case("transfer-encoding"));
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            assert!(!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()));
            assert!(content_length
                .replace(value.parse::<usize>().unwrap())
                .is_none());
        }
    }
    let response_len = header_end.checked_add(content_length.unwrap()).unwrap();
    assert!(
        response_len <= MAX_RESPONSE_BYTES,
        "HTTP response exceeds fixture bound"
    );
    assert!(
        bytes.len() <= response_len,
        "unexpected bytes after HTTP response"
    );
    let received = bytes.len();
    bytes.resize(response_len, 0);
    // A rejected unread request can reset the connection after the response.
    // Read exactly its frame, so a complete response never depends on TCP EOF.
    stream.read_exact(&mut bytes[received..]).await.unwrap();
    (
        status,
        String::from_utf8(bytes[header_end..].to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn fixture_reads_complete_http_response_without_waiting_for_connection_close() {
    let (mut reader, mut writer) = tokio::io::duplex(256);
    let task = tokio::spawn(async move {
        writer
            .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 4\r\n\r\nbody")
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let response = tokio::time::timeout(Duration::from_secs(2), read_http_response(&mut reader))
        .await
        .unwrap();
    task.abort();
    assert_eq!(response, (413, "body".into()));
}

#[tokio::test]
async fn private_presence_is_unavailable_without_explicit_secret() {
    let fixture = Fixture::start(None, &[]).await;
    assert_eq!(
        fixture
            .query(&[], r#"{"device_ids":["0123456789"]}"#)
            .await
            .0,
        503
    );
}

#[tokio::test]
async fn private_presence_rejects_missing_duplicate_malformed_and_wrong_domain_authorization() {
    let secret = URL_SAFE_NO_PAD.encode(SECRET);
    let fixture = Fixture::start(Some(&secret), &[]).await;
    let valid = bearer(CONTEXT);
    let foreign = bearer(b"MRD_RELAY_REQUEST_V1\0");
    let malformed = format!("{valid}=");
    for headers in [
        vec![],
        vec![valid.as_str(), valid.as_str()],
        vec![foreign.as_str()],
        vec![malformed.as_str()],
        vec!["Bearer short"],
        vec!["Basic Zm9v"],
        vec!["Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
    ] {
        let (status, body) = fixture
            .query(&headers, r#"{"device_ids":["0123456789"]}"#)
            .await;
        assert_eq!(status, 403);
        assert!(!body.contains(&secret) && !body.contains(&valid));
    }
}

#[tokio::test]
async fn private_presence_returns_only_the_requested_unique_devices_and_exact_contract() {
    let now = now_ms();
    let secret = URL_SAFE_NO_PAD.encode(SECRET);
    let fixture = Fixture::start(
        Some(&secret),
        &[
            ("0123456789", now, now + 60_000),
            ("hidden-device", now, now + 60_000),
        ],
    )
    .await;
    let (status, body) = fixture
        .query(
            &[&bearer(CONTEXT)],
            r#"{"device_ids":["unknown","0123456789","unknown"]}"#,
        )
        .await;
    assert_eq!(status, 200);
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result.as_object().unwrap().len(), 3);
    assert_eq!(result["version"], 1);
    assert!(result["sampled_at_ms"].as_u64().unwrap() >= now);
    assert_eq!(
        result["devices"],
        json!([
            {"device_id":"unknown","online":false,"last_seen_ms":null},
            {"device_id":"0123456789","online":true,"last_seen_ms":now},
        ])
    );
    assert!(!body.contains("hidden-device") && !body.contains("key") && !body.contains("token"));
}

#[tokio::test]
async fn private_presence_does_not_report_stale_heartbeats_or_expired_tokens_online() {
    let now = now_ms();
    let secret = URL_SAFE_NO_PAD.encode(SECRET);
    let fixture = Fixture::start(
        Some(&secret),
        &[
            ("live", now - 1_000, now + 60_000),
            ("stale", now - 30_000, now + 60_000),
            ("expired-token", now - 10, now - 1),
        ],
    )
    .await;
    let (status, body) = fixture
        .query(
            &[&bearer(CONTEXT)],
            r#"{"device_ids":["live","stale","expired-token"]}"#,
        )
        .await;
    assert_eq!(status, 200);
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["devices"][0]["online"], true);
    assert_eq!(result["devices"][1]["online"], false);
    assert_eq!(result["devices"][2]["online"], false);
    assert_eq!(result["devices"][1]["last_seen_ms"], now - 30_000);
}

#[tokio::test]
async fn private_presence_bounds_body_ids_and_payload_shape() {
    let secret = URL_SAFE_NO_PAD.encode(SECRET);
    let fixture = Fixture::start(Some(&secret), &[]).await;
    let auth = bearer(CONTEXT);
    for body in [
        json!({"device_ids":vec!["same";129]}).to_string(),
        json!({"device_ids":["a".repeat(129)]}).to_string(),
        json!({"device_ids":[""]}).to_string(),
        json!({"device_ids":["设备"]}).to_string(),
        json!({"device_ids":["bad/id"]}).to_string(),
        json!({"device_ids":["bad id"]}).to_string(),
    ] {
        assert_eq!(fixture.query(&[&auth], &body).await.0, 400);
    }
    assert_eq!(
        fixture
            .query(&[&auth], r#"{"device_ids":[],"enumerate":true}"#)
            .await
            .0,
        422
    );
    assert_eq!(
        fixture.query(&[&auth], r#"{"device_ids":[true]}"#).await.0,
        422
    );
    let large = format!("{}{}", r#"{"device_ids":[]}"#, " ".repeat(32 * 1024));
    assert_eq!(fixture.query(&[&auth], &large).await.0, 413);
    assert_eq!(fixture.query(&[], &large).await.0, 403);
    let maximum = json!({"device_ids":(0..128).map(|id| format!("{id:0128}")).collect::<Vec<_>>()})
        .to_string();
    let (status, body) = fixture.query(&[&auth], &maximum).await;
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["devices"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
}
