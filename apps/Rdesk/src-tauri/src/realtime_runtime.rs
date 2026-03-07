use std::{collections::HashMap, sync::Arc};

use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_proto::{IceCandidate, SessionDescription, SignalMessage};
use tokio::sync::Mutex;

use crate::realtime_client::{RealtimeClient, RealtimeConnection};

#[derive(Clone, Debug)]
pub struct RealtimeRuntime {
    client: RealtimeClient,
    inner: Arc<Mutex<RealtimeRuntimeState>>,
}

#[derive(Debug, Default)]
struct RealtimeRuntimeState {
    next_handle: u64,
    connections: HashMap<u64, RealtimeConnection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeRegistration {
    pub handle: u64,
    pub device_id: DeviceId,
}

impl RealtimeRuntime {
    pub fn new(signaling_url: impl Into<String>) -> Self {
        Self {
            client: RealtimeClient::new(signaling_url),
            inner: Arc::new(Mutex::new(RealtimeRuntimeState::default())),
        }
    }

    pub fn from_env() -> Self {
        Self {
            client: RealtimeClient::from_env(),
            inner: Arc::new(Mutex::new(RealtimeRuntimeState::default())),
        }
    }

    pub async fn register(
        &self,
        role: BackendRole,
        device_id: Option<DeviceId>,
        name: String,
    ) -> Result<RealtimeRegistration, String> {
        let connection = self
            .client
            .connect_and_register(role, device_id, name)
            .await?;

        let registration = RealtimeRegistration {
            handle: self.next_handle().await,
            device_id: connection.registered.device_id.clone(),
        };

        let mut state = self.inner.lock().await;
        state
            .connections
            .insert(registration.handle, connection);

        Ok(registration)
    }

    pub async fn request_session(
        &self,
        handle: u64,
        session_id: SessionId,
        target_device_id: DeviceId,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        connection
            .request_session(session_id, target_device_id)
            .await?;
        connection.recv_event().await.map(|_| ())
    }

    pub async fn accept_session(&self, handle: u64, session_id: SessionId) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        connection.accept_session(session_id).await?;
        connection.recv_event().await.map(|_| ())
    }

    pub async fn drain_events(&self, handle: u64) -> Result<Vec<SignalMessage>, String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        Ok(connection.drain_inbound_events())
    }

    pub async fn send_offer(
        &self,
        handle: u64,
        description: SessionDescription,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        connection.send_offer(description).await?;
        connection.recv_event().await.map(|_| ())
    }

    pub async fn send_answer(
        &self,
        handle: u64,
        description: SessionDescription,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        connection.send_answer(description).await?;
        connection.recv_event().await.map(|_| ())
    }

    pub async fn send_ice_candidate(
        &self,
        handle: u64,
        candidate: IceCandidate,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        let connection = state
            .connections
            .get_mut(&handle)
            .ok_or_else(|| format!("未找到 realtime 连接句柄: {}", handle))?;

        connection.send_ice_candidate(candidate).await?;
        connection.recv_event().await.map(|_| ())
    }

    async fn next_handle(&self) -> u64 {
        let mut state = self.inner.lock().await;
        state.next_handle += 1;
        state.next_handle
    }
}

#[cfg(test)]
mod tests {
    use super::RealtimeRuntime;
    use axum::{
        extract::ws::{Message, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::StreamExt;
    use mrd_proto::{BackendRole, DeviceId, SessionId};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_signal_proto::{
        IceCandidate, RegisteredResponse, SessionDescription, SignalMessage,
    };
    use tokio::net::TcpListener;

    async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(handle_socket)
    }

    async fn handle_socket(mut socket: WebSocket) {
        let Some(Ok(Message::Text(raw))) = socket.next().await else {
            return;
        };

        let message = decode_message(&raw).expect("decode register message");
        assert!(matches!(message, SignalMessage::Register(_)));

        let ack = encode_message(&SignalMessage::Registered(RegisteredResponse {
            device_id: DeviceId("controller-1".into()),
        }))
        .expect("encode registered response");

        socket
            .send(Message::Text(ack.into()))
            .await
            .expect("send registered response");

        while let Some(Ok(Message::Text(raw))) = socket.next().await {
            let signal = decode_message(&raw).expect("decode session signal");
            let outbound = encode_message(&signal).expect("encode echoed session signal");
            socket
                .send(Message::Text(outbound.into()))
                .await
                .expect("echo session signal");
        }
    }

    async fn spawn_server() -> String {
        let app = Router::new().route("/ws", get(ws_handler));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind realtime runtime test server");
        let addr = listener.local_addr().expect("test server addr");

        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve runtime test ws");
        });

        format!("ws://{}/ws", addr)
    }

    #[tokio::test]
    async fn registering_connection_returns_handle_and_device_id() {
        let runtime = RealtimeRuntime::new(spawn_server().await);

        let registration = runtime
            .register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("register runtime connection");

        assert_eq!(registration.handle, 1);
        assert_eq!(registration.device_id, DeviceId("controller-1".into()));
    }

    #[tokio::test]
    async fn request_and_accept_session_roundtrip_through_runtime_queue() {
        let runtime = RealtimeRuntime::new(spawn_server().await);

        let registration = runtime
            .register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("register runtime connection");

        runtime
            .request_session(
                registration.handle,
                SessionId("session-1".into()),
                DeviceId("agent-1".into()),
            )
            .await
            .expect("request session");

        runtime
            .accept_session(registration.handle, SessionId("session-1".into()))
            .await
            .expect("accept session");

        let events = runtime
            .drain_events(registration.handle)
            .await
            .expect("drain realtime events");

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], SignalMessage::SessionRequest(_)));
        assert!(matches!(events[1], SignalMessage::SessionAccept(_)));
    }

    #[tokio::test]
    async fn offer_answer_and_ice_roundtrip_through_runtime_queue() {
        let runtime = RealtimeRuntime::new(spawn_server().await);

        let registration = runtime
            .register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("register runtime connection");

        runtime
            .send_offer(
                registration.handle,
                SessionDescription {
                    session_id: SessionId("session-1".into()),
                    sdp: "offer-sdp".into(),
                },
            )
            .await
            .expect("send offer");

        runtime
            .send_answer(
                registration.handle,
                SessionDescription {
                    session_id: SessionId("session-1".into()),
                    sdp: "answer-sdp".into(),
                },
            )
            .await
            .expect("send answer");

        runtime
            .send_ice_candidate(
                registration.handle,
                IceCandidate {
                    session_id: SessionId("session-1".into()),
                    candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host".into(),
                    sdp_mid: Some("0".into()),
                    sdp_mline_index: Some(0),
                },
            )
            .await
            .expect("send ice candidate");

        let events = runtime
            .drain_events(registration.handle)
            .await
            .expect("drain realtime events");

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], SignalMessage::WebrtcOffer(_)));
        assert!(matches!(events[1], SignalMessage::WebrtcAnswer(_)));
        assert!(matches!(events[2], SignalMessage::IceCandidate(_)));
    }
}
