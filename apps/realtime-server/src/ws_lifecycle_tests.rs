use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_identity::DeviceIdentity;
use mrd_proto::{BackendRole, SessionId};
use mrd_signal_proto::{
    AuthClaims, AuthenticatedRegister, PresenceHeartbeat, PresenceHeartbeatPayload,
    RegisterPayload, ServerChallenge, SessionClose, SessionClosePayload, SessionGrantV3,
    SessionIntentV3, WanSessionRequestV3,
};
use ring::hmac;
use serde_json::json;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as ClientMessage, MaybeTlsStream, WebSocketStream,
};

type ClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const TOKEN_SECRET: &[u8] = b"lifecycle-test-backend-key-32-bytes-minimum";

fn token(
    identity: &DeviceIdentity,
    device: &DeviceId,
    browser: bool,
    session: &SessionId,
    lifetime_s: u64,
    guest_authority: bool,
) -> String {
    let browser_principal = browser && device.0.starts_with("browser_");
    let role = if browser_principal {
        BackendRole::Controller
    } else if browser {
        BackendRole::Peer
    } else {
        BackendRole::Agent
    };
    let mut claims = json!({
        "sub": device.0, "device_id": device.0, "device_key_id": identity.key_id(),
        "role": role,
        "token_type": if browser_principal {"browser_signaling"} else {"signaling"},
        "iss": "rdesk-backend", "aud": "rdesk-signaling",
        "iat": now_ms()/1000, "exp": now_ms()/1000+lifetime_s,
    });
    if browser_principal {
        if guest_authority {
            claims["token_type"] = json!("guest_browser_signaling");
            claims["authority_kind"] = json!("temporary_password");
            claims["user_id"] = json!(null);
            claims["temporary_access_generation"] = json!(1);
            claims["target_auth_version"] = json!(2);
        } else {
            claims["user_id"] = json!("lifecycle-user");
        }
        claims["tenant_id"] = json!("lifecycle-tenant");
        claims["session_id"] = json!(session.0);
        claims["target_device_id"] = json!("target-lifecycle");
        claims["allowed_scopes"] = json!(["input.keyboard", "input.pointer", "screen.view"]);
    }
    let encoded = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, TOKEN_SECRET),
        encoded.as_bytes(),
    );
    format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

struct Peer {
    socket: ClientSocket,
    identity: DeviceIdentity,
    device: DeviceId,
    connection: ConnectionId,
    jwt: String,
    counter: u64,
}

impl Peer {
    fn claims(&mut self, intended_peer: DeviceId, future_ms: u64) -> AuthClaims {
        self.counter += 1;
        let issued = now_ms() + future_ms;
        AuthClaims {
            issuer_device_id: self.device.clone(),
            issuer_key_id: self.identity.key_id().into(),
            intended_peer_device_id: intended_peer,
            issued_at_ms: issued,
            expires_at_ms: issued + 10_000,
            counter: self.counter,
            nonce: [self.counter as u8; 16],
        }
    }

    fn heartbeat(&mut self, future_ms: u64) -> SignalEnvelope {
        let claims = self.claims(DeviceId("signal-server".into()), future_ms);
        let observed_at_ms = claims.issued_at_ms;
        SignalEnvelope::new(AuthenticatedSignalMessage::PresenceHeartbeat(
            PresenceHeartbeat::sign(
                &self.identity,
                PresenceHeartbeatPayload {
                    claims,
                    connection_id: *self.connection.as_bytes(),
                    observed_at_ms,
                },
            )
            .unwrap(),
        ))
    }
}

struct Fixture {
    state: RealtimeAppState,
    url: String,
    task: tokio::task::JoinHandle<()>,
    session: SessionId,
    target: Peer,
    browser: Peer,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn start() -> Self {
        Self::start_with_options(2, 300).await
    }

    async fn start_with_options(browser_suffix: u8, browser_lifetime_s: u64) -> Self {
        Self::start_with_authority(browser_suffix, browser_lifetime_s, false).await
    }

    async fn start_with_authority(
        browser_suffix: u8,
        browser_lifetime_s: u64,
        guest_authority: bool,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = listener.local_addr().unwrap();
        let core_config = CoreConfig {
            server_device_id: DeviceId("signal-server".into()),
            challenge_ttl_ms: 10_000,
            presence_ttl_ms: 30_000,
            route_ttl_ms: 60_000,
            max_connections: 2,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        };
        let state = RealtimeAppState::new(
            RealtimeCore::new(
                core_config.clone(),
                Arc::new(
                    crate::JwtBackendTokenVerifier::new(
                        TOKEN_SECRET,
                        "rdesk-backend".into(),
                        "rdesk-signaling".into(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
            ServerRuntimeConfig {
                bind_addr,
                secure_websocket_required: false,
                max_message_bytes: MAX_SIGNAL_MESSAGE_BYTES,
                outbound_queue_capacity: 4,
                prune_interval: Duration::from_millis(25),
                core: core_config,
            },
        );
        let server_state = state.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, build_router(server_state))
                .await
                .unwrap();
        });
        let url = format!("ws://{bind_addr}/ws");
        let session = SessionId("browser-lifecycle-session".into());
        let target = connect_peer(&url, false, &session, 1, 300, false).await;
        let browser = connect_peer(
            &url,
            true,
            &session,
            browser_suffix,
            browser_lifetime_s,
            guest_authority,
        )
        .await;
        let mut fixture = Self {
            state,
            url,
            task,
            session,
            target,
            browser,
        };
        fixture.open_route().await;
        fixture
    }

    async fn open_route(&mut self) {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/browser_protocol_v3.json"))
                .unwrap();
        let mut request: WanSessionRequestV3 =
            serde_json::from_value(data["request"].clone()).unwrap();
        request.session_id = self.session.clone();
        request.controller_device_id = self.browser.device.clone();
        request.target_device_id = self.target.device.clone();
        let claims = self.browser.claims(self.target.device.clone(), 0);
        let intent = SessionIntentV3::sign(
            &self.browser.identity,
            mrd_signal_proto::SessionIntentV3Payload {
                claims,
                request_commitment: request.commitment().unwrap(),
                request,
            },
        )
        .unwrap();
        send(
            &mut self.browser.socket,
            SignalEnvelope::new(AuthenticatedSignalMessage::SessionIntentV3(intent.clone())),
        )
        .await;
        assert_eq!(
            receive(&mut self.target.socket).await,
            SignalEnvelope::new(AuthenticatedSignalMessage::SessionIntentV3(intent.clone()))
        );
        let mut grant: SessionGrantV3 = serde_json::from_value(data["grant"].clone()).unwrap();
        grant.payload.claims = self.target.claims(self.browser.device.clone(), 0);
        grant.payload.session_id = self.session.clone();
        grant.payload.controller_device_id = self.browser.device.clone();
        grant.payload.target_device_id = self.target.device.clone();
        grant.payload.intent_commitment = intent.commitment().unwrap();
        grant.payload.policy_expires_at_ms = now_ms() + 60_000;
        let grant = SessionGrantV3::sign(&self.target.identity, grant.payload).unwrap();
        let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::SessionGrantV3(grant));
        send(&mut self.target.socket, envelope.clone()).await;
        assert_eq!(receive(&mut self.browser.socket).await, envelope);
    }

    fn close(&mut self, browser_closes: bool) -> SignalEnvelope {
        let intended = if browser_closes {
            self.target.device.clone()
        } else {
            self.browser.device.clone()
        };
        let sender = if browser_closes {
            &mut self.browser
        } else {
            &mut self.target
        };
        let claims = sender.claims(intended, 0);
        SignalEnvelope::new(AuthenticatedSignalMessage::SessionClose(
            SessionClose::sign(
                &sender.identity,
                SessionClosePayload {
                    claims,
                    session_id: self.session.clone(),
                    reason: ProtocolReasonCode::Expired,
                },
            )
            .unwrap(),
        ))
    }
}

async fn receive(socket: &mut ClientSocket) -> SignalEnvelope {
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("bounded websocket response")
        .expect("websocket remains open")
        .unwrap();
    decode_authenticated_message(message.to_text().unwrap()).unwrap()
}

async fn send(socket: &mut ClientSocket, message: SignalEnvelope) {
    socket
        .send(ClientMessage::Text(
            encode_authenticated_message(&message).unwrap(),
        ))
        .await
        .unwrap();
}

async fn connect_peer(
    url: &str,
    browser: bool,
    session: &SessionId,
    suffix: u8,
    lifetime_s: u64,
    guest_authority: bool,
) -> Peer {
    let (mut socket, _) = connect_async(url).await.unwrap();
    let challenge = match receive(&mut socket).await.message {
        AuthenticatedSignalMessage::ServerChallenge(challenge) => challenge,
        other => panic!("expected challenge: {other:?}"),
    };
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let device = DeviceId(if browser && suffix == 0 {
        "controller-lifecycle".into()
    } else if browser {
        format!("browser_{suffix:032x}")
    } else {
        "target-lifecycle".into()
    });
    let jwt = token(
        &identity,
        &device,
        browser,
        session,
        lifetime_s,
        guest_authority,
    );
    let connection = register(&mut socket, &identity, &device, &jwt, browser, challenge, 1)
        .await
        .unwrap();
    Peer {
        socket,
        identity,
        device,
        connection,
        jwt,
        counter: 1,
    }
}

async fn register(
    socket: &mut ClientSocket,
    identity: &DeviceIdentity,
    device: &DeviceId,
    jwt: &str,
    browser: bool,
    challenge: ServerChallenge,
    counter: u64,
) -> Result<ConnectionId, ProtocolReasonCode> {
    let now = now_ms();
    send(
        socket,
        SignalEnvelope::new(AuthenticatedSignalMessage::Register(
            AuthenticatedRegister::sign(
                identity,
                RegisterPayload {
                    claims: AuthClaims {
                        issuer_device_id: device.clone(),
                        issuer_key_id: identity.key_id().into(),
                        intended_peer_device_id: DeviceId("signal-server".into()),
                        issued_at_ms: now,
                        expires_at_ms: now + 10_000,
                        counter,
                        nonce: [counter as u8; 16],
                    },
                    role: if browser && device.0.starts_with("browser_") {
                        BackendRole::Controller
                    } else if browser {
                        BackendRole::Peer
                    } else {
                        BackendRole::Agent
                    },
                    device_name: "Lifecycle fixture".into(),
                    backend_device_token: jwt.into(),
                    challenge_id: challenge.challenge_id,
                    challenge_nonce: challenge.challenge_nonce,
                },
            )
            .unwrap(),
        )),
    )
    .await;
    match receive(socket).await.message {
        AuthenticatedSignalMessage::Registered(registered) => {
            Ok(ConnectionId::from_bytes(registered.payload.connection_id).unwrap())
        }
        AuthenticatedSignalMessage::ProtocolError(error) => Err(error.reason),
        other => panic!("expected registration result: {other:?}"),
    }
}

async fn assert_closed(socket: &mut ClientSocket) {
    let result = tokio::time::timeout(Duration::from_millis(750), socket.next())
        .await
        .expect("revoked browser transport must close without client traffic or pruning");
    assert!(matches!(result, None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))),
        "revoked browser must close instead of remaining as an unaccounted protocol-error connection");
}

async fn wait_for_cleanup(state: &RealtimeAppState, browser: ConnectionId) {
    tokio::time::timeout(Duration::from_millis(750), async {
        loop {
            let peers = state.peers.lock().await;
            if !peers.contains_key(&browser) && peers.len() == 1 {
                drop(peers);
                let core = state.core.lock().await;
                if core.rates.len() == 1 && core.presence_count() == 1 && core.route_count() == 0 {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closed socket must release peer, presence, route and capacity");
}

async fn close_lifecycle(browser_closes: bool, future_wait: bool) {
    close_lifecycle_authority(browser_closes, future_wait, false).await;
}

async fn close_lifecycle_authority(browser_closes: bool, future_wait: bool, guest_authority: bool) {
    let mut fixture = Fixture::start_with_authority(2, 300, guest_authority).await;
    let old_outbound = fixture
        .state
        .peers
        .lock()
        .await
        .get(&fixture.browser.connection)
        .unwrap()
        .outbound
        .clone();
    if future_wait {
        let waiting = fixture.browser.heartbeat(1_500);
        send(&mut fixture.browser.socket, waiting).await;
    }
    let close = fixture.close(browser_closes);
    if browser_closes {
        send(&mut fixture.browser.socket, close.clone()).await;
        assert_eq!(receive(&mut fixture.target.socket).await, close);
    } else {
        send(&mut fixture.target.socket, close.clone()).await;
        assert_eq!(
            receive(&mut fixture.browser.socket).await,
            close,
            "target-signed close must arrive before the browser transport closes"
        );
    }
    assert_closed(&mut fixture.browser.socket).await;
    wait_for_cleanup(&fixture.state, fixture.browser.connection).await;
    tokio::time::timeout(Duration::from_millis(750), old_outbound.closed())
        .await
        .expect("revoked browser writer and its outbound receiver must be dropped");

    let heartbeat = fixture.target.heartbeat(0);
    send(&mut fixture.target.socket, heartbeat.clone()).await;
    send(&mut fixture.target.socket, heartbeat).await;
    match receive(&mut fixture.target.socket).await.message {
        AuthenticatedSignalMessage::ProtocolError(error) => {
            assert_eq!(error.reason, ProtocolReasonCode::ReplayRejected)
        }
        other => panic!("physical target socket must remain active: {other:?}"),
    }

    let (mut replay, _) = connect_async(&fixture.url).await.unwrap();
    let challenge = match receive(&mut replay).await.message {
        AuthenticatedSignalMessage::ServerChallenge(challenge) => challenge,
        other => panic!("expected capacity to be available: {other:?}"),
    };
    assert_eq!(
        register(
            &mut replay,
            &fixture.browser.identity,
            &fixture.browser.device,
            &fixture.browser.jwt,
            true,
            challenge,
            30
        )
        .await,
        Err(ProtocolReasonCode::UnauthorizedRoute)
    );
    replay.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_millis(750), async {
        while fixture.state.peers.lock().await.len() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rejected registration transport is released after the client close");
    let mut replacement = connect_peer(
        &fixture.url,
        true,
        &fixture.session,
        3,
        300,
        guest_authority,
    )
    .await;
    let (mut overflow, _) = connect_async(&fixture.url).await.unwrap();
    assert_closed(&mut overflow).await;
    assert_eq!(fixture.state.peers.lock().await.len(), 2);
    replacement.socket.close(None).await.unwrap();
}

#[tokio::test]
async fn browser_signed_close_releases_real_transport_and_retains_target_socket() {
    close_lifecycle(true, false).await;
}

#[tokio::test]
async fn target_signed_close_is_delivered_then_releases_browser_transport() {
    close_lifecycle(false, false).await;
}

#[tokio::test]
async fn target_close_cancels_browser_waiting_on_future_message() {
    close_lifecycle(false, true).await;
}

async fn wait_for_token_expiry(peer: &Peer) {
    let payload: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(peer.jwt.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    let deadline = payload["exp"].as_u64().unwrap() * 1000;
    tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now_ms()) + 1)).await;
}

#[tokio::test]
async fn idle_browser_token_expiry_pruner_closes_real_socket_and_queue() {
    let mut fixture = Fixture::start_with_options(2, 1).await;
    let old_outbound = fixture
        .state
        .peers
        .lock()
        .await
        .get(&fixture.browser.connection)
        .unwrap()
        .outbound
        .clone();
    let pruner = fixture.state.spawn_pruner();
    wait_for_token_expiry(&fixture.browser).await;
    assert_closed(&mut fixture.browser.socket).await;
    wait_for_cleanup(&fixture.state, fixture.browser.connection).await;
    tokio::time::timeout(Duration::from_millis(750), old_outbound.closed())
        .await
        .unwrap();
    pruner.abort();
    let heartbeat = fixture.target.heartbeat(0);
    send(&mut fixture.target.socket, heartbeat.clone()).await;
    send(&mut fixture.target.socket, heartbeat).await;
    match receive(&mut fixture.target.socket).await.message {
        AuthenticatedSignalMessage::ProtocolError(error) => {
            assert_eq!(error.reason, ProtocolReasonCode::ReplayRejected)
        }
        other => panic!("pruning must retain the physical target: {other:?}"),
    }
}

#[tokio::test]
async fn browser_expired_message_error_also_terminates_real_transport() {
    let mut fixture = Fixture::start_with_options(2, 1).await;
    let old_outbound = fixture
        .state
        .peers
        .lock()
        .await
        .get(&fixture.browser.connection)
        .unwrap()
        .outbound
        .clone();
    wait_for_token_expiry(&fixture.browser).await;
    let heartbeat = fixture.browser.heartbeat(0);
    send(&mut fixture.browser.socket, heartbeat).await;
    // Terminal rejection must not leave an unregistered socket outside the
    // capacity/rate accounting, whether the error or Close is sent first.
    let response = tokio::time::timeout(Duration::from_millis(750), fixture.browser.socket.next())
        .await
        .unwrap();
    if let Some(Ok(ClientMessage::Text(text))) = response {
        match decode_authenticated_message(&text).unwrap().message {
            AuthenticatedSignalMessage::ProtocolError(error) => {
                assert_eq!(error.reason, ProtocolReasonCode::Expired)
            }
            other => panic!("expired traffic must never be accepted: {other:?}"),
        }
        assert_closed(&mut fixture.browser.socket).await;
    } else {
        assert!(matches!(
            response,
            None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
        ));
    }
    wait_for_cleanup(&fixture.state, fixture.browser.connection).await;
    tokio::time::timeout(Duration::from_millis(750), old_outbound.closed())
        .await
        .unwrap();
}

#[tokio::test]
async fn native_physical_session_close_keeps_both_real_websockets_alive() {
    for controller_closes in [true, false] {
        let mut fixture = Fixture::start_with_options(0, 300).await;
        let close = fixture.close(controller_closes);
        if controller_closes {
            send(&mut fixture.browser.socket, close.clone()).await;
            assert_eq!(receive(&mut fixture.target.socket).await, close);
        } else {
            send(&mut fixture.target.socket, close.clone()).await;
            assert_eq!(receive(&mut fixture.browser.socket).await, close);
        }
        assert_eq!(fixture.state.peers.lock().await.len(), 2);
        let core = fixture.state.core.lock().await;
        assert_eq!(core.rates.len(), 2);
        assert_eq!(core.presence_count(), 2);
        assert_eq!(core.route_count(), 0);
        drop(core);
        for peer in [&mut fixture.browser, &mut fixture.target] {
            let heartbeat = peer.heartbeat(0);
            send(&mut peer.socket, heartbeat.clone()).await;
            send(&mut peer.socket, heartbeat).await;
            match receive(&mut peer.socket).await.message {
                AuthenticatedSignalMessage::ProtocolError(error) => {
                    assert_eq!(error.reason, ProtocolReasonCode::ReplayRejected)
                }
                other => panic!("physical peers must retain their original sockets: {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn terminated_browser_retains_capacity_until_transport_disconnect() {
    let mut fixture = Fixture::start().await;
    let close = fixture.close(true);
    let mut core = fixture.state.core.lock().await;
    let deliveries = core
        .handle(fixture.browser.connection, close, now_ms())
        .unwrap();
    assert_eq!(core.presence_count(), 1);
    assert_eq!(
        core.rates.len(),
        2,
        "authority removal alone must not release a transport slot"
    );
    let extra = ConnectionId::from_bytes([9; 16]).unwrap();
    assert!(matches!(
        core.open_connection(extra, now_ms()),
        Err(RealtimeError::ConnectionCapacity)
    ));
    let heartbeat = fixture.browser.heartbeat(0);
    assert!(matches!(
        core.handle(fixture.browser.connection, heartbeat, now_ms()),
        Err(RealtimeError::InvalidConnection)
    ));
    let terminal = core.take_transport_closures();
    assert_eq!(terminal, vec![fixture.browser.connection]);
    deliver_all(&fixture.state, deliveries, terminal).await;
    drop(core);
    receive(&mut fixture.target.socket).await;
    assert_closed(&mut fixture.browser.socket).await;
    wait_for_cleanup(&fixture.state, fixture.browser.connection).await;
    assert!(fixture
        .state
        .core
        .lock()
        .await
        .open_connection(extra, now_ms())
        .is_ok());
    fixture.state.core.lock().await.disconnect(extra);
}

#[tokio::test]
async fn guest_temporary_signed_close_preserves_real_transport_order_capacity_and_target_socket() {
    for browser_closes in [true, false] {
        close_lifecycle_authority(browser_closes, false, true).await;
    }
}
