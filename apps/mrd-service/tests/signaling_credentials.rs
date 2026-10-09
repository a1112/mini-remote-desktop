use axum::{http::HeaderMap, routing::post, Json};
use mrd_proto::{BackendRole, DeviceId};
use mrd_service::{
    signaling::{spawn, SignalingConfig, SignalingConnectionState},
    AppState,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn config(endpoint: &str) -> SignalingConfig {
    SignalingConfig::new(
        endpoint,
        DeviceId("local-device".into()),
        "Local workstation",
        BackendRole::Agent,
        "device-backend-jwt-secret",
        DeviceId("signal-server".into()),
        None,
        Duration::from_millis(250),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}

struct ExchangedTokenVerifier {
    key_id: String,
}

impl realtime_server::BackendTokenVerifier for ExchangedTokenVerifier {
    fn verify(
        &self,
        token: &str,
        now_ms: u64,
    ) -> Result<realtime_server::VerifiedBackendToken, realtime_server::BackendTokenError> {
        if token != "bound-signaling-credential" {
            return Err(realtime_server::BackendTokenError::Invalid);
        }
        Ok(realtime_server::VerifiedBackendToken {
            device_id: DeviceId("local-device".into()),
            device_key_id: self.key_id.clone(),
            role: BackendRole::Agent,
            expires_at_ms: now_ms + 60_000,
            browser: None,
        })
    }
}

#[tokio::test]
async fn driver_exchanges_device_jwt_before_real_websocket_registration() {
    let app_state = Arc::new(AppState::new());
    let key_id = app_state
        .device_identities
        .machine_key_id()
        .unwrap()
        .to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_config = realtime_server::ws::ServerRuntimeConfig {
        bind_addr: address,
        secure_websocket_required: false,
        max_message_bytes: mrd_signal_client::MAX_SIGNAL_MESSAGE_BYTES,
        outbound_queue_capacity: 16,
        prune_interval: Duration::from_secs(10),
        core: realtime_server::CoreConfig {
            server_device_id: DeviceId("signal-server".into()),
            challenge_ttl_ms: 1_000,
            presence_ttl_ms: 3_000,
            route_ttl_ms: 30_000,
            max_connections: 8,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        },
    };
    let core = realtime_server::RealtimeCore::new(
        server_config.core.clone(),
        Arc::new(ExchangedTokenVerifier {
            key_id: key_id.clone(),
        }),
    )
    .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::clone(&requests);
    let router = realtime_server::ws::build_router(realtime_server::ws::RealtimeAppState::new(
        core,
        server_config,
    ))
    .route(
        "/api/v1/realtime/device-credentials",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let key_id = key_id.clone();
            let requests = Arc::clone(&request_count);
            async move {
                assert_eq!(
                    headers["X-Rdesk-Device-Authorization"],
                    "Bearer device-backend-jwt-secret"
                );
                assert!(!headers.contains_key("Authorization"));
                assert_eq!(body, json!({"device_key_id": key_id, "role": "Agent"}));
                requests.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "token": "bound-signaling-credential", "expires_at_ms": now_ms() + 60_000,
                    "device_id": "local-device", "device_key_id": key_id, "role": "Agent"
                }))
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let task = spawn(
        config(&format!("ws://{address}/ws"))
            .with_credential_endpoint(&format!(
                "http://{address}/api/v1/realtime/device-credentials"
            ))
            .unwrap(),
        Arc::clone(&app_state),
    )
    .unwrap();
    let authenticated = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if app_state.signaling_status.snapshot().state
                == SignalingConnectionState::Authenticated
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    task.shutdown().await;
    server.abort();
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "driver must exchange its device JWT"
    );
    authenticated.expect("bound credential must authenticate against real WebSocket server");
}

#[tokio::test]
async fn driver_rejects_credential_binding_mismatch_before_opening_websocket() {
    let app_state = Arc::new(AppState::new());
    let key_id = app_state
        .device_identities
        .machine_key_id()
        .unwrap()
        .to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/credentials",
        post(move || {
            let key_id = key_id.clone();
            async move {
                Json(json!({
                    "token": "SECRET_ERROR_BODY_SENTINEL", "expires_at_ms": now_ms() + 60_000,
                    "device_id": "wrong-device", "device_key_id": key_id, "role": "Agent"
                }))
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let task = spawn(
        config(&format!("ws://{address}/ws"))
            .with_credential_endpoint(&format!("http://{address}/credentials"))
            .unwrap(),
        Arc::clone(&app_state),
    )
    .unwrap();
    let snapshot = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = app_state.signaling_status.snapshot();
            if snapshot.last_error.is_some() {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.shutdown().await;
    server.abort();
    assert_eq!(
        snapshot.last_error.as_deref(),
        Some("signaling_credentials_invalid")
    );
    assert!(!format!("{snapshot:?}").contains("SECRET_ERROR_BODY_SENTINEL"));
}

#[tokio::test]
async fn reconnect_fetches_a_fresh_bound_credential() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let app_state = Arc::new(AppState::new());
    let key_id = app_state
        .device_identities
        .machine_key_id()
        .unwrap()
        .to_owned();
    let requests = Arc::new(AtomicUsize::new(0));
    let issued_count = Arc::clone(&requests);
    let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_address = api_listener.local_addr().unwrap();
    let router = axum::Router::new().route("/credentials", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let key_id = key_id.clone();
        let requests = Arc::clone(&issued_count);
        async move {
            assert_eq!(headers["X-Rdesk-Device-Authorization"], "Bearer device-backend-jwt-secret");
            assert_eq!(body["device_key_id"], key_id);
            let generation = requests.fetch_add(1, Ordering::SeqCst) + 1;
            Json(json!({
                "token": format!("credential-generation-{generation}"), "expires_at_ms": now_ms() + 60_000,
                "device_id": "local-device", "device_key_id": key_id, "role": "Agent"
            }))
        }
    }));
    let api = tokio::spawn(async move {
        axum::serve(api_listener, router).await.unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_identity =
        mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
    let verified = Arc::new(AtomicUsize::new(0));
    let verified_count = Arc::clone(&verified);
    let server = tokio::spawn(async move {
        for generation in 1..=2_u8 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let current_time = now_ms();
            let challenge = mrd_signal_proto::ServerChallenge {
                challenge_id: [generation; 16],
                challenge_nonce: [generation; 32],
                issued_at_ms: current_time,
                expires_at_ms: current_time + 5_000,
            };
            socket
                .send(Message::Text(
                    mrd_signal_client::encode_authenticated_message(
                        &mrd_signal_proto::SignalEnvelope::new(
                            mrd_signal_proto::AuthenticatedSignalMessage::ServerChallenge(
                                challenge.clone(),
                            ),
                        ),
                    )
                    .unwrap(),
                ))
                .await
                .unwrap();
            let Message::Text(raw) = socket.next().await.unwrap().unwrap() else {
                panic!("registration required")
            };
            let envelope = mrd_signal_client::decode_authenticated_message(&raw).unwrap();
            let mrd_signal_proto::AuthenticatedSignalMessage::Register(register) = envelope.message
            else {
                panic!("registration required")
            };
            assert_eq!(
                register.payload.backend_device_token,
                format!("credential-generation-{generation}")
            );
            assert_eq!(register.payload.challenge_id, challenge.challenge_id);
            verified_count.fetch_add(1, Ordering::SeqCst);
            let registered = mrd_signal_proto::Registered::sign(
                &server_identity,
                mrd_signal_proto::RegisteredPayload {
                    claims: mrd_signal_proto::AuthClaims {
                        issuer_device_id: DeviceId("signal-server".into()),
                        issuer_key_id: server_identity.key_id().into(),
                        intended_peer_device_id: DeviceId("local-device".into()),
                        issued_at_ms: now_ms(),
                        expires_at_ms: now_ms() + 60_000,
                        counter: u64::from(generation),
                        nonce: [generation; 16],
                    },
                    registered_device_id: DeviceId("local-device".into()),
                    connection_id: [generation; 16],
                    heartbeat_interval_ms: 500,
                },
            )
            .unwrap();
            socket
                .send(Message::Text(
                    mrd_signal_client::encode_authenticated_message(
                        &mrd_signal_proto::SignalEnvelope::new(
                            mrd_signal_proto::AuthenticatedSignalMessage::Registered(registered),
                        ),
                    )
                    .unwrap(),
                ))
                .await
                .unwrap();
            if generation == 1 {
                socket.close(None).await.unwrap();
            } else {
                while socket.next().await.is_some() {}
            }
        }
    });
    let task = spawn(
        config(&format!("ws://{address}/ws"))
            .with_credential_endpoint(&format!("http://{api_address}/credentials"))
            .unwrap(),
        Arc::clone(&app_state),
    )
    .unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if verified.load(Ordering::SeqCst) == 2
                && app_state.signaling_status.snapshot().state
                    == SignalingConnectionState::Authenticated
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    task.shutdown().await;
    api.abort();
    server.abort();
    completed.expect("fresh credential must be used by the reconnected socket");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn credential_http_failures_remain_bounded_and_redacted() {
    use axum::{body::Body, http::StatusCode, response::Response, routing::get};

    for case in [
        "key",
        "role",
        "expired",
        "oversize",
        "chunked",
        "malformed",
        "redirect",
        "rejected",
        "timeout",
    ] {
        let app_state = Arc::new(AppState::new());
        let key_id = app_state
            .device_identities
            .machine_key_id()
            .unwrap()
            .to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let redirected = Arc::new(AtomicUsize::new(0));
        let redirected_count = Arc::clone(&redirected);
        let websocket_requests = Arc::new(AtomicUsize::new(0));
        let websocket_count = Arc::clone(&websocket_requests);
        let router = axum::Router::new()
            .route("/credentials", post(move || {
                let key_id = key_id.clone();
                async move {
                    let mut response = json!({
                        "token": "SECRET_ERROR_BODY_SENTINEL", "expires_at_ms": now_ms() + 60_000,
                        "device_id": "local-device", "device_key_id": key_id, "role": "Agent"
                    });
                    match case {
                        "key" => response["device_key_id"] = json!("f".repeat(64)),
                        "role" => response["role"] = json!("Controller"),
                        "expired" => response["expires_at_ms"] = json!(now_ms()),
                        "oversize" => response["token"] = json!("x".repeat(20 * 1024)),
                        "chunked" => return Response::builder().body(Body::from_stream(futures_util::stream::iter([
                            Ok::<_, std::io::Error>(vec![b' '; 16 * 1024]), Ok(vec![b' '; 16])
                        ]))).unwrap(),
                        "malformed" => return Response::builder().body(Body::from("{SECRET_ERROR_BODY_SENTINEL")).unwrap(),
                        "redirect" => return Response::builder().status(StatusCode::TEMPORARY_REDIRECT)
                            .header("Location", "/redirect-target").body(Body::empty()).unwrap(),
                        "rejected" => return Response::builder().status(StatusCode::UNAUTHORIZED)
                            .body(Body::from("SECRET_ERROR_BODY_SENTINEL")).unwrap(),
                        "timeout" => tokio::time::sleep(Duration::from_secs(1)).await,
                        _ => unreachable!(),
                    }
                    Response::builder().header("Content-Type", "application/json")
                        .body(Body::from(serde_json::to_vec(&response).unwrap())).unwrap()
                }
            }))
            .route("/redirect-target", post(move || {
                redirected_count.fetch_add(1, Ordering::SeqCst);
                async { StatusCode::OK }
            }))
            .route("/ws", get(move || {
                websocket_count.fetch_add(1, Ordering::SeqCst);
                async { StatusCode::BAD_REQUEST }
            }));
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let task = spawn(
            config(&format!("ws://{address}/ws"))
                .with_credential_endpoint(&format!("http://{address}/credentials"))
                .unwrap(),
            Arc::clone(&app_state),
        )
        .unwrap();
        let snapshot = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = app_state.signaling_status.snapshot();
                if snapshot.last_error.is_some() {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.shutdown().await;
        server.abort();
        let expected = match case {
            "redirect" | "rejected" => "signaling_credentials_unavailable",
            "timeout" => "signaling_credentials_timeout",
            _ => "signaling_credentials_invalid",
        };
        assert_eq!(
            snapshot.last_error.as_deref(),
            Some(expected),
            "case {case}"
        );
        assert_eq!(
            redirected.load(Ordering::SeqCst),
            0,
            "credential headers must not follow redirects"
        );
        assert_eq!(
            websocket_requests.load(Ordering::SeqCst),
            0,
            "invalid credential must fail before websocket"
        );
        assert!(!format!("{snapshot:?}").contains("SECRET_ERROR_BODY_SENTINEL"));
    }
}
