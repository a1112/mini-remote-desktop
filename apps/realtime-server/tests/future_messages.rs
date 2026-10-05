use futures_util::{SinkExt, StreamExt};
use mrd_identity::DeviceIdentity;
use mrd_proto::{BackendRole, DeviceId};
use mrd_signal_client::{decode_authenticated_message, encode_authenticated_message};
use mrd_signal_proto::{
    AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, PresenceHeartbeat,
    PresenceHeartbeatPayload, ProtocolReasonCode, RegisterPayload, ServerChallenge, SignalEnvelope,
};
use realtime_server::{
    ws::{build_router, RealtimeAppState, ServerRuntimeConfig},
    BackendTokenError, BackendTokenVerifier, CoreConfig, RealtimeCore, VerifiedBackendToken,
};
use ring::rand::SystemRandom;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct Tokens(VerifiedBackendToken);
impl BackendTokenVerifier for Tokens {
    fn verify(&self, token: &str, _now: u64) -> Result<VerifiedBackendToken, BackendTokenError> {
        if token == "test-device-token" {
            Ok(self.0.clone())
        } else {
            Err(BackendTokenError::Invalid)
        }
    }
}

struct Fixture {
    task: tokio::task::JoinHandle<()>,
    url: String,
    socket: Socket,
    identity: DeviceIdentity,
    challenge: ServerChallenge,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn start(challenge_ttl_ms: u64, token_lifetime_ms: u64) -> Self {
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let core_config = CoreConfig {
            server_device_id: DeviceId("signal-server".into()),
            challenge_ttl_ms,
            presence_ttl_ms: 30_000,
            route_ttl_ms: 60_000,
            max_connections: 16,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        };
        let tokens = Tokens(VerifiedBackendToken {
            device_id: DeviceId("0123456789".into()),
            device_key_id: identity.key_id().into(),
            role: BackendRole::Peer,
            expires_at_ms: now_ms() + token_lifetime_ms,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = listener.local_addr().unwrap();
        let config = ServerRuntimeConfig {
            bind_addr,
            secure_websocket_required: false,
            max_message_bytes: mrd_signal_client::MAX_SIGNAL_MESSAGE_BYTES,
            outbound_queue_capacity: 64,
            prune_interval: Duration::from_secs(10),
            core: core_config.clone(),
        };
        let state = RealtimeAppState::new(
            RealtimeCore::new(core_config, Arc::new(tokens)).unwrap(),
            config,
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });
        let url = format!("ws://{bind_addr}/ws");
        let (mut socket, _) = connect_async(&url).await.unwrap();
        let challenge = match receive(&mut socket).await.message {
            AuthenticatedSignalMessage::ServerChallenge(challenge) => challenge,
            other => panic!("expected connection challenge: {other:?}"),
        };
        Self {
            task,
            url,
            socket,
            identity,
            challenge,
        }
    }

    fn claims(&self, counter: u64, issued: u64, expires: u64) -> AuthClaims {
        AuthClaims {
            issuer_device_id: DeviceId("0123456789".into()),
            issuer_key_id: self.identity.key_id().into(),
            intended_peer_device_id: DeviceId("signal-server".into()),
            issued_at_ms: issued,
            expires_at_ms: expires,
            counter,
            nonce: [counter as u8; 16],
        }
    }

    fn register(&self, issued: u64, expires: u64) -> SignalEnvelope {
        SignalEnvelope::new(AuthenticatedSignalMessage::Register(
            AuthenticatedRegister::sign(
                &self.identity,
                RegisterPayload {
                    claims: self.claims(1, issued, expires),
                    role: BackendRole::Peer,
                    device_name: "Future device".into(),
                    backend_device_token: "test-device-token".into(),
                    challenge_id: self.challenge.challenge_id,
                    challenge_nonce: self.challenge.challenge_nonce,
                },
            )
            .unwrap(),
        ))
    }
}

async fn send(socket: &mut Socket, envelope: &SignalEnvelope) {
    socket
        .send(Message::Text(
            encode_authenticated_message(envelope).unwrap(),
        ))
        .await
        .unwrap();
}
async fn receive(socket: &mut Socket) -> SignalEnvelope {
    let message = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await
        .expect("bounded server response")
        .unwrap()
        .unwrap();
    decode_authenticated_message(message.to_text().unwrap()).unwrap()
}
fn reason(envelope: SignalEnvelope) -> ProtocolReasonCode {
    match envelope.message {
        AuthenticatedSignalMessage::ProtocolError(error) => error.reason,
        other => panic!("expected strict protocol rejection: {other:?}"),
    }
}

#[tokio::test]
async fn websocket_accepts_register_only_after_its_real_issue_time() {
    let mut fixture = Fixture::start(10_000, 60_000).await;
    let issued = now_ms() + 100;
    let envelope = fixture.register(issued, issued + 10_000);
    send(&mut fixture.socket, &envelope).await;
    let response = receive(&mut fixture.socket).await;
    assert!(
        matches!(response.message, AuthenticatedSignalMessage::Registered(_)),
        "{response:?}"
    );
    assert!(now_ms() >= issued);
}

#[tokio::test]
async fn websocket_accepts_future_heartbeat_but_still_rejects_its_replay() {
    let mut fixture = Fixture::start(10_000, 60_000).await;
    let register = fixture.register(now_ms(), now_ms() + 10_000);
    send(&mut fixture.socket, &register).await;
    let connection = match receive(&mut fixture.socket).await.message {
        AuthenticatedSignalMessage::Registered(registered) => registered.payload.connection_id,
        other => panic!("expected registered: {other:?}"),
    };
    let issued = now_ms() + 100;
    let heartbeat = SignalEnvelope::new(AuthenticatedSignalMessage::PresenceHeartbeat(
        PresenceHeartbeat::sign(
            &fixture.identity,
            PresenceHeartbeatPayload {
                claims: fixture.claims(2, issued, issued + 10_000),
                connection_id: connection,
                observed_at_ms: issued,
            },
        )
        .unwrap(),
    ));
    send(&mut fixture.socket, &heartbeat).await;
    send(&mut fixture.socket, &heartbeat).await;
    assert_eq!(
        reason(receive(&mut fixture.socket).await),
        ProtocolReasonCode::ReplayRejected
    );
    assert!(now_ms() >= issued);
}

#[tokio::test]
async fn websocket_never_accepts_expired_or_excessively_future_register() {
    for future in [false, true] {
        let mut fixture = Fixture::start(10_000, 60_000).await;
        let now = now_ms();
        let envelope = if future {
            fixture.register(now + 60_000, now + 70_000)
        } else {
            fixture.register(now - 100, now - 1)
        };
        send(&mut fixture.socket, &envelope).await;
        let response =
            tokio::time::timeout(Duration::from_millis(500), receive(&mut fixture.socket))
                .await
                .expect("excessively future messages are rejected without sleeping");
        assert_eq!(reason(response), ProtocolReasonCode::Expired);
    }
}

#[tokio::test]
async fn waiting_does_not_extend_challenge_or_backend_token_expiry() {
    for (challenge_ttl, token_ttl) in [(30, 60_000), (10_000, 30)] {
        let mut fixture = Fixture::start(challenge_ttl, token_ttl).await;
        let issued = now_ms() + 120;
        let envelope = fixture.register(issued, issued + 10_000);
        send(&mut fixture.socket, &envelope).await;
        assert_eq!(
            reason(receive(&mut fixture.socket).await),
            ProtocolReasonCode::Expired
        );
    }
}

#[tokio::test]
async fn future_message_wait_does_not_hold_the_global_core_mutex() {
    let mut fixture = Fixture::start(10_000, 60_000).await;
    let issued = now_ms() + 1_000;
    let envelope = fixture.register(issued, issued + 10_000);
    send(&mut fixture.socket, &envelope).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut other = tokio::time::timeout(Duration::from_millis(500), async {
        let (mut socket, _) = connect_async(&fixture.url).await.unwrap();
        assert!(matches!(
            receive(&mut socket).await.message,
            AuthenticatedSignalMessage::ServerChallenge(_)
        ));
        socket
    })
    .await
    .expect("another connection must receive its challenge during the future wait");
    other.close(None).await.unwrap();
    assert!(matches!(
        receive(&mut fixture.socket).await.message,
        AuthenticatedSignalMessage::Registered(_)
    ));
}
