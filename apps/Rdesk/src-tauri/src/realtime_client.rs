use futures_util::{SinkExt, StreamExt};
use mrd_proto::{BackendRole, DeviceId};
use mrd_signal_client::{decode_message, encode_message};
use mrd_signal_proto::{RegisterRequest, RegisteredResponse, SignalMessage};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DEFAULT_SIGNALING_URL: &str = "ws://127.0.0.1:9532/ws";

#[derive(Debug, Clone)]
pub struct RealtimeClient {
    signaling_url: String,
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

    pub async fn register(
        &self,
        role: BackendRole,
        device_id: Option<DeviceId>,
        name: String,
    ) -> Result<RegisteredResponse, String> {
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
            SignalMessage::Registered(registered) => Ok(registered),
            other => Err(format!("unexpected register response: {:?}", other)),
        }
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
    use futures_util::{SinkExt, StreamExt};
    use mrd_proto::{BackendRole, DeviceId};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_signal_proto::{RegisteredResponse, SignalMessage};
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

    #[test]
    fn default_signaling_url_points_to_realtime_server_ws() {
        std::env::remove_var("RDESK_SIGNALING_URL");

        let client = RealtimeClient::from_env();

        assert_eq!(client.signaling_url(), "ws://127.0.0.1:9532/ws");
    }
}
