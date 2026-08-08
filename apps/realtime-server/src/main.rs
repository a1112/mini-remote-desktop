use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use mrd_proto::{DeviceId, SessionId};
use mrd_signal_client::{decode_message, encode_message};
use mrd_signal_proto::{RegisterRequest, RegisteredResponse, SignalMessage};
use mrd_signal_server::{SessionRoute, SessionRouter};
use serde::Serialize;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::{mpsc, Mutex};
use tracing::info;
use uuid::Uuid;

const DEFAULT_BIND: &str = "127.0.0.1:9542";
const PEER_QUEUE_CAPACITY: usize = 128;
const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    protocol_version: u16,
}

#[derive(Debug, Default)]
struct SignalingState {
    peers: HashMap<DeviceId, mpsc::Sender<String>>,
    routes: SessionRouter,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "realtime-server",
        protocol_version: PROTOCOL_VERSION,
    })
}

type SharedState = Arc<Mutex<SignalingState>>;

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(PEER_QUEUE_CAPACITY);

    let mut current_device: Option<DeviceId> = None;

    let send_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sender.send(Message::Text(message.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        let Message::Text(text) = message else {
            continue;
        };

        let Ok(signal) = decode_message(&text) else {
            continue;
        };

        match signal {
            SignalMessage::Register(register) => {
                let device_id = register
                    .device_id
                    .clone()
                    .unwrap_or_else(|| synthesize_device_id(&register));

                {
                    let mut guard = state.lock().await;
                    guard.peers.insert(device_id.clone(), tx.clone());
                }

                current_device = Some(device_id);

                let ack = SignalMessage::Registered(RegisteredResponse {
                    device_id: current_device.clone().expect("device id after register"),
                });
                let Ok(encoded) = encode_message(&ack) else {
                    continue;
                };
                let _ = tx.send(encoded).await;
            }
            SignalMessage::SessionRequest(request) => {
                let Some(sender_id) = current_device.as_ref() else {
                    continue;
                };
                if &request.source_device_id != sender_id {
                    continue;
                }
                let target = request.target_device_id.clone();
                {
                    let mut guard = state.lock().await;
                    guard.routes.register(
                        request.session_id.clone(),
                        SessionRoute {
                            controller: request.source_device_id.clone(),
                            agent: target.clone(),
                        },
                    );
                }
                forward_to_peer(&state, &target, SignalMessage::SessionRequest(request)).await;
            }
            SignalMessage::SessionAccept(accept) => {
                let Some(sender_id) = current_device.clone() else {
                    continue;
                };
                let Ok(peer) = resolve_peer(&state, &accept.session_id, &sender_id).await else {
                    continue;
                };
                forward_to_peer(&state, &peer, SignalMessage::SessionAccept(accept)).await;
            }
            SignalMessage::WebrtcOffer(offer) => {
                let Some(sender_id) = current_device.clone() else {
                    continue;
                };
                let Ok(peer) = resolve_peer(&state, &offer.session_id, &sender_id).await else {
                    continue;
                };
                forward_to_peer(&state, &peer, SignalMessage::WebrtcOffer(offer)).await;
            }
            SignalMessage::WebrtcAnswer(answer) => {
                let Some(sender_id) = current_device.clone() else {
                    continue;
                };
                let Ok(peer) = resolve_peer(&state, &answer.session_id, &sender_id).await else {
                    continue;
                };
                forward_to_peer(&state, &peer, SignalMessage::WebrtcAnswer(answer)).await;
            }
            SignalMessage::IceCandidate(candidate) => {
                let Some(sender_id) = current_device.clone() else {
                    continue;
                };
                let Ok(peer) = resolve_peer(&state, &candidate.session_id, &sender_id).await else {
                    continue;
                };
                forward_to_peer(&state, &peer, SignalMessage::IceCandidate(candidate)).await;
            }
            SignalMessage::Registered(_) => {}
        }
    }

    if let Some(device_id) = current_device {
        state.lock().await.peers.remove(&device_id);
    }

    send_task.abort();
}

fn synthesize_device_id(register: &RegisterRequest) -> DeviceId {
    let role = match register.role {
        mrd_proto::BackendRole::Controller => "controller",
        mrd_proto::BackendRole::Agent => "agent",
    };
    DeviceId(format!("{role}-{}", Uuid::new_v4()))
}

async fn resolve_peer(
    state: &SharedState,
    session_id: &SessionId,
    sender: &DeviceId,
) -> Result<DeviceId, ()> {
    state
        .lock()
        .await
        .routes
        .resolve_peer(session_id, sender)
        .map_err(|_| ())
}

async fn forward_to_peer(state: &SharedState, target: &DeviceId, message: SignalMessage) {
    let Ok(encoded) = encode_message(&message) else {
        return;
    };

    let peer = state.lock().await.peers.get(target).cloned();
    if let Some(peer) = peer {
        let _ = peer.send(encoded).await;
    }
}

fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let state = Arc::new(Mutex::new(SignalingState::default()));
    let app = build_router(state);
    let addr = std::env::var("MRD_REALTIME_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND.to_string())
        .parse::<SocketAddr>()
        .expect("parse MRD_REALTIME_BIND_ADDR");

    info!("realtime-server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind realtime-server listener");

    axum::serve(listener, app)
        .await
        .expect("run realtime-server");
}

#[cfg(test)]
mod tests {
    use super::{build_router, SignalingState};
    use futures_util::{SinkExt, StreamExt};
    use mrd_proto::{BackendRole, DeviceId, SessionId};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_signal_proto::{
        IceCandidate, RegisterRequest, RegisteredResponse, SessionAccept, SessionDescription,
        SessionRequest, SignalMessage,
    };
    use std::sync::Arc;
    use tokio::{
        net::TcpListener,
        sync::Mutex,
        time::{timeout, Duration},
    };
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    async fn spawn_server() -> String {
        let state = Arc::new(Mutex::new(SignalingState::default()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind realtime test server");
        let addr = listener.local_addr().expect("test server addr");
        let app = build_router(state);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve test signaling");
        });

        format!("ws://{}/ws", addr)
    }

    #[tokio::test]
    async fn routes_session_offer_answer_and_ice_by_session_id() {
        let ws_url = spawn_server().await;

        let (mut controller, _) = connect_async(&ws_url).await.expect("connect controller");
        let (mut agent, _) = connect_async(&ws_url).await.expect("connect agent");

        controller
            .send(Message::Text(
                encode_message(&SignalMessage::Register(RegisterRequest {
                    role: BackendRole::Controller,
                    device_id: Some(DeviceId("controller-1".into())),
                    name: "controller".into(),
                }))
                .expect("encode controller register"),
            ))
            .await
            .expect("send controller register");

        let registered = match timeout(Duration::from_secs(3), controller.next())
            .await
            .expect("timeout waiting for controller registered")
            .expect("controller registered frame")
            .expect("controller registered")
        {
            Message::Text(text) => decode_message(&text).expect("decode controller registered"),
            other => panic!("unexpected controller registered message: {:?}", other),
        };
        assert!(matches!(
            registered,
            SignalMessage::Registered(RegisteredResponse { .. })
        ));

        agent
            .send(Message::Text(
                encode_message(&SignalMessage::Register(RegisterRequest {
                    role: BackendRole::Agent,
                    device_id: Some(DeviceId("agent-1".into())),
                    name: "agent".into(),
                }))
                .expect("encode agent register"),
            ))
            .await
            .expect("send agent register");

        let registered = match timeout(Duration::from_secs(3), agent.next())
            .await
            .expect("timeout waiting for agent registered")
            .expect("agent registered frame")
            .expect("agent registered")
        {
            Message::Text(text) => decode_message(&text).expect("decode agent registered"),
            other => panic!("unexpected agent registered message: {:?}", other),
        };
        assert!(matches!(
            registered,
            SignalMessage::Registered(RegisteredResponse { .. })
        ));

        controller
            .send(Message::Text(
                encode_message(&SignalMessage::SessionRequest(SessionRequest {
                    session_id: SessionId("session-1".into()),
                    source_device_id: DeviceId("controller-1".into()),
                    target_device_id: DeviceId("agent-1".into()),
                    transport: "webrtc".into(),
                    quic_listen_addr: None,
                    quic_server_name: None,
                    quic_cert_der_b64: None,
                }))
                .expect("encode session request"),
            ))
            .await
            .expect("send session request");

        let session_request = match timeout(Duration::from_secs(3), agent.next())
            .await
            .expect("timeout waiting for session request")
            .expect("agent ws frame")
            .expect("agent frame")
        {
            Message::Text(text) => decode_message(&text).expect("decode session request"),
            other => panic!("unexpected agent message: {:?}", other),
        };
        assert!(matches!(session_request, SignalMessage::SessionRequest(_)));

        agent
            .send(Message::Text(
                encode_message(&SignalMessage::SessionAccept(SessionAccept {
                    session_id: SessionId("session-1".into()),
                    transport: "webrtc".into(),
                    quic_listen_addr: None,
                    quic_server_name: None,
                    quic_cert_der_b64: None,
                }))
                .expect("encode session accept"),
            ))
            .await
            .expect("send session accept");

        let session_accept = match timeout(Duration::from_secs(3), controller.next())
            .await
            .expect("timeout waiting for session accept")
            .expect("controller ws frame")
            .expect("controller frame")
        {
            Message::Text(text) => decode_message(&text).expect("decode session accept"),
            other => panic!("unexpected controller message: {:?}", other),
        };
        assert!(matches!(session_accept, SignalMessage::SessionAccept(_)));

        controller
            .send(Message::Text(
                encode_message(&SignalMessage::WebrtcOffer(SessionDescription {
                    session_id: SessionId("session-1".into()),
                    sdp: "offer-sdp".into(),
                }))
                .expect("encode offer"),
            ))
            .await
            .expect("send offer");

        let offer = match timeout(Duration::from_secs(3), agent.next())
            .await
            .expect("timeout waiting for offer")
            .expect("agent offer frame")
            .expect("agent offer")
        {
            Message::Text(text) => decode_message(&text).expect("decode offer"),
            other => panic!("unexpected offer message: {:?}", other),
        };
        assert!(matches!(offer, SignalMessage::WebrtcOffer(_)));

        agent
            .send(Message::Text(
                encode_message(&SignalMessage::WebrtcAnswer(SessionDescription {
                    session_id: SessionId("session-1".into()),
                    sdp: "answer-sdp".into(),
                }))
                .expect("encode answer"),
            ))
            .await
            .expect("send answer");

        let answer = match timeout(Duration::from_secs(3), controller.next())
            .await
            .expect("timeout waiting for answer")
            .expect("controller answer frame")
            .expect("controller answer")
        {
            Message::Text(text) => decode_message(&text).expect("decode answer"),
            other => panic!("unexpected answer message: {:?}", other),
        };
        assert!(matches!(answer, SignalMessage::WebrtcAnswer(_)));

        agent
            .send(Message::Text(
                encode_message(&SignalMessage::IceCandidate(IceCandidate {
                    session_id: SessionId("session-1".into()),
                    candidate: "candidate:1".into(),
                    sdp_mid: Some("0".into()),
                    sdp_mline_index: Some(0),
                }))
                .expect("encode ice"),
            ))
            .await
            .expect("send ice");

        let ice = match timeout(Duration::from_secs(3), controller.next())
            .await
            .expect("timeout waiting for ice")
            .expect("controller ice frame")
            .expect("controller ice")
        {
            Message::Text(text) => decode_message(&text).expect("decode ice"),
            other => panic!("unexpected ice message: {:?}", other),
        };
        assert!(matches!(ice, SignalMessage::IceCandidate(_)));
    }
}
