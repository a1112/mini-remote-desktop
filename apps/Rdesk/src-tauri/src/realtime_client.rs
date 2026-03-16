use futures_util::{SinkExt, StreamExt};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::{decode_message, encode_message};
use mrd_signal_proto::{
    IceCandidate, RegisterRequest, RegisteredResponse, SessionAccept, SessionDescription,
    SessionRequest, SignalMessage,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DEFAULT_SIGNALING_URL: &str = "ws://127.0.0.1:9532/ws";

#[derive(Debug, Clone)]
pub struct RealtimeClient {
    signaling_url: String,
}

#[derive(Debug)]
pub struct RealtimeConnection {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    pub registered: RegisteredResponse,
    inbound_events: Vec<SignalMessage>,
}

impl RealtimeClient {
    pub fn new(signaling_url: impl Into<String>) -> Self {
        Self {
            signaling_url: signaling_url.into(),
        }
    }

    pub fn from_env() -> Self {
        let signaling_url =
            std::env::var("RDESK_SIGNALING_URL").unwrap_or_else(|_| DEFAULT_SIGNALING_URL.into());
        Self::new(signaling_url)
    }

    pub fn signaling_url(&self) -> &str {
        &self.signaling_url
    }

    pub async fn connect_and_register(
        &self,
        role: BackendRole,
        device_id: Option<DeviceId>,
        name: String,
    ) -> Result<RealtimeConnection, String> {
        let (mut ws, _) = connect_async(&self.signaling_url)
            .await
            .map_err(|e| format!("连接 realtime signaling 失败: {}", e))?;

        let register_message = SignalMessage::Register(RegisterRequest {
            role,
            device_id,
            name,
        });

        let payload = encode_message(&register_message)
            .map_err(|e| format!("编码 register 消息失败: {}", e))?;

        ws.send(Message::Text(payload))
            .await
            .map_err(|e| format!("发送 register 消息失败: {}", e))?;

        let Some(Ok(Message::Text(raw))) = ws.next().await else {
            return Err("未收到 register 响应".into());
        };

        let response =
            decode_message(&raw).map_err(|e| format!("解析 register 响应失败: {}", e))?;

        match response {
            SignalMessage::Registered(registered) => Ok(RealtimeConnection {
                socket: ws,
                registered,
                inbound_events: Vec::new(),
            }),
            other => Err(format!("unexpected register response: {:?}", other)),
        }
    }

    pub async fn register(
        &self,
        role: BackendRole,
        device_id: Option<DeviceId>,
        name: String,
    ) -> Result<RegisteredResponse, String> {
        let connection = self.connect_and_register(role, device_id, name).await?;
        Ok(connection.registered)
    }
}

impl RealtimeConnection {
    pub async fn request_session(
        &mut self,
        session_id: SessionId,
        target_device_id: DeviceId,
    ) -> Result<(), String> {
        self.request_session_with_transport(
            session_id,
            target_device_id,
            "webrtc".into(),
            None,
            None,
            None,
        )
        .await
    }

    pub async fn request_session_with_transport(
        &mut self,
        session_id: SessionId,
        target_device_id: DeviceId,
        transport: String,
        quic_listen_addr: Option<String>,
        quic_server_name: Option<String>,
        quic_cert_der_b64: Option<String>,
    ) -> Result<(), String> {
        let message = SignalMessage::SessionRequest(SessionRequest {
            session_id,
            source_device_id: self.registered.device_id.clone(),
            target_device_id,
            transport,
            quic_listen_addr,
            quic_server_name,
            quic_cert_der_b64,
        });
        self.send_message(message).await
    }

    pub async fn accept_session(&mut self, session_id: SessionId) -> Result<(), String> {
        self.accept_session_with_transport(session_id, "webrtc".into(), None, None, None)
            .await
    }

    pub async fn accept_session_with_transport(
        &mut self,
        session_id: SessionId,
        transport: String,
        quic_listen_addr: Option<String>,
        quic_server_name: Option<String>,
        quic_cert_der_b64: Option<String>,
    ) -> Result<(), String> {
        self.send_message(SignalMessage::SessionAccept(SessionAccept {
            session_id,
            transport,
            quic_listen_addr,
            quic_server_name,
            quic_cert_der_b64,
        }))
        .await
    }

    pub async fn send_offer(&mut self, description: SessionDescription) -> Result<(), String> {
        self.send_message(SignalMessage::WebrtcOffer(description))
            .await
    }

    pub async fn send_answer(&mut self, description: SessionDescription) -> Result<(), String> {
        self.send_message(SignalMessage::WebrtcAnswer(description))
            .await
    }

    pub async fn send_ice_candidate(&mut self, candidate: IceCandidate) -> Result<(), String> {
        self.send_message(SignalMessage::IceCandidate(candidate))
            .await
    }

    pub async fn recv_event(&mut self) -> Result<SignalMessage, String> {
        let Some(Ok(Message::Text(raw))) = self.socket.next().await else {
            return Err("未收到 signaling 事件".into());
        };

        let message =
            decode_message(&raw).map_err(|e| format!("解析 signaling 事件失败: {}", e))?;
        self.inbound_events.push(message.clone());
        Ok(message)
    }

    pub fn drain_inbound_events(&mut self) -> Vec<SignalMessage> {
        std::mem::take(&mut self.inbound_events)
    }

    async fn send_message(&mut self, message: SignalMessage) -> Result<(), String> {
        let payload =
            encode_message(&message).map_err(|e| format!("编码 signaling 消息失败: {}", e))?;

        self.socket
            .send(Message::Text(payload))
            .await
            .map_err(|e| format!("发送 signaling 消息失败: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::RealtimeClient;
    use axum::{
        extract::ws::{Message, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::StreamExt;
    use mrd_proto::{BackendRole, DeviceId, SessionId};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_signal_proto::{IceCandidate, RegisteredResponse, SessionDescription, SignalMessage};
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
            .expect("bind realtime client test server");
        let addr = listener.local_addr().expect("test server addr");

        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test ws");
        });

        format!("ws://{}/ws", addr)
    }

    #[tokio::test]
    async fn register_receives_registered_ack() {
        let ws_url = spawn_server().await;
        let client = RealtimeClient::new(ws_url);

        let registered = client
            .register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("register over websocket");

        assert_eq!(registered.device_id, DeviceId("controller-1".into()));
    }

    #[tokio::test]
    async fn request_and_accept_session_roundtrip_into_event_queue() {
        let ws_url = spawn_server().await;
        let client = RealtimeClient::new(ws_url);

        let mut connection = client
            .connect_and_register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("connect and register");

        connection
            .request_session(SessionId("session-1".into()), DeviceId("agent-1".into()))
            .await
            .expect("request session");

        let request = connection
            .recv_event()
            .await
            .expect("receive request event");
        assert!(matches!(request, SignalMessage::SessionRequest(_)));

        connection
            .accept_session(SessionId("session-1".into()))
            .await
            .expect("accept session");

        let accept = connection.recv_event().await.expect("receive accept event");
        assert!(matches!(accept, SignalMessage::SessionAccept(_)));

        let drained = connection.drain_inbound_events();
        assert_eq!(drained.len(), 2);
    }

    #[tokio::test]
    async fn offer_answer_and_ice_roundtrip_into_event_queue() {
        let ws_url = spawn_server().await;
        let client = RealtimeClient::new(ws_url);

        let mut connection = client
            .connect_and_register(
                BackendRole::Controller,
                Some(DeviceId("controller-1".into())),
                "Rdesk".into(),
            )
            .await
            .expect("connect and register");

        connection
            .send_offer(SessionDescription {
                session_id: SessionId("session-1".into()),
                sdp: "offer-sdp".into(),
            })
            .await
            .expect("send offer");
        let offer = connection.recv_event().await.expect("receive offer event");
        assert!(matches!(offer, SignalMessage::WebrtcOffer(_)));

        connection
            .send_answer(SessionDescription {
                session_id: SessionId("session-1".into()),
                sdp: "answer-sdp".into(),
            })
            .await
            .expect("send answer");
        let answer = connection.recv_event().await.expect("receive answer event");
        assert!(matches!(answer, SignalMessage::WebrtcAnswer(_)));

        connection
            .send_ice_candidate(IceCandidate {
                session_id: SessionId("session-1".into()),
                candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host".into(),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            })
            .await
            .expect("send ice candidate");
        let ice = connection.recv_event().await.expect("receive ice event");
        assert!(matches!(ice, SignalMessage::IceCandidate(_)));

        let drained = connection.drain_inbound_events();
        assert_eq!(drained.len(), 3);
    }

    #[test]
    fn default_signaling_url_points_to_realtime_server_ws() {
        std::env::remove_var("RDESK_SIGNALING_URL");

        let client = RealtimeClient::from_env();

        assert_eq!(client.signaling_url(), "ws://127.0.0.1:9532/ws");
    }
}
