use super::protocol::{
    create_register_message, AudioQuicTransportInfo, DeviceInfo, QuicTransportInfo,
    SessionDescriptionJson, SignalingMessage, SignalingMessagePayload,
};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

type WsWrite = Arc<
    Mutex<
        futures_util::stream::SplitSink<
            WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
            tokio_tungstenite::tungstenite::Message,
        >,
    >,
>;

/// 信令客户端配置
#[derive(Debug, Clone)]
pub struct SignalingConfig {
    pub ws_url: String,
    pub device_name: String,
    pub preferred_transport: String,
}

impl Default for SignalingConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://127.0.0.1:9527".to_string(),
            device_name: "Rust Controller".to_string(),
            preferred_transport: std::env::var("MRD_TRANSPORT")
                .ok()
                .unwrap_or_else(|| "webrtc".to_string())
                .to_ascii_lowercase(),
        }
    }
}

/// 信令客户端
pub struct SignalingClient {
    config: SignalingConfig,
    device_id: Arc<Mutex<Option<String>>>,
    event_tx: mpsc::Sender<SignalingMessagePayload>,
    write: Arc<Mutex<Option<WsWrite>>>,
}

impl SignalingClient {
    /// 创建新的信令客户端
    pub fn new(config: SignalingConfig) -> (Self, mpsc::Receiver<SignalingMessagePayload>) {
        let (event_tx, event_rx) = mpsc::channel(100);
        let device_id = Arc::new(Mutex::new(None));
        let write = Arc::new(Mutex::new(None));

        let client = Self {
            config,
            device_id,
            event_tx,
            write,
        };

        (client, event_rx)
    }

    /// 连接到信令服务器
    pub async fn connect(&self) -> Result<()> {
        let url = &self.config.ws_url;
        info!(url = %url, "connecting to signaling server");

        let (ws, _) = connect_async(url)
            .await
            .with_context(|| format!("failed to connect to signaling server: {}", url))?;

        let (write, mut read) = ws.split();
        let write = Arc::new(Mutex::new(write));

        // 保存写端
        *self.write.lock().await = Some(write.clone());

        let device_id = self.device_id.clone();
        let event_tx = self.event_tx.clone();

        // 启动消息接收循环
        tokio::spawn(async move {
            tracing::info!("starting WebSocket receive loop");
            let mut msg_count = 0u32;
            loop {
                tracing::debug!("waiting for WebSocket message...");
                match read.next().await {
                    Some(msg) => {
                        msg_count += 1;
                        tracing::debug!("WebSocket loop iteration {}, received message", msg_count);

                        let msg = match msg {
                            Ok(m) => m,
                            Err(e) => {
                                error!(error = %e, "websocket read error");
                                break;
                            }
                        };

                        if !msg.is_text() {
                            tracing::debug!("skipping non-text message");
                            continue;
                        }

                        let text = match msg.into_text() {
                            Ok(t) => t,
                            Err(e) => {
                                warn!(error = %e, "failed to convert message to text");
                                continue;
                            }
                        };

                        if let Err(e) = handle_message(&text, &device_id, &event_tx).await {
                            warn!(error = %e, "failed to handle message");
                        }
                    }
                    None => {
                        tracing::info!("WebSocket stream ended");
                        break;
                    }
                }
            }

            info!("signaling client disconnected");
        });

        info!("connected to signaling server");
        Ok(())
    }

    /// 注册控制器
    pub async fn register(&self) -> Result<()> {
        let msg = create_register_message(&self.config.device_name);
        self.send_text(&msg).await?;
        info!(device_name = %self.config.device_name, "registering controller");
        Ok(())
    }

    /// 发送 Offer
    pub async fn send_offer(
        &self,
        target_device_id: &str,
        offer: &webrtc::peer_connection::sdp::session_description::RTCSessionDescription,
        session_id: &str,
    ) -> Result<()> {
        let device_id = self.device_id.lock().await;
        let controller_id = device_id.as_ref().context("not registered")?;
        let offer_json = SessionDescriptionJson::from(offer.clone());

        let msg = serde_json::json!({
            "type": "webrtc",
            "action": "offer",
            "payload": {
                "targetDeviceId": target_device_id,
                "offer": offer_json,
                "sessionId": session_id,
                "controllerId": controller_id,
                "transport": self.config.preferred_transport,
                "capabilities": {
                    "protocols": ["webrtc", "quic"],
                    "platforms": ["windows", "linux", "macos"],
                    "codecs": ["h264"],
                    "features": ["multi-end-compat", "capability-negotiation"]
                }
            }
        })
        .to_string();

        info!(
            target_device_id = %target_device_id,
            transport = %self.config.preferred_transport,
            "sending offer"
        );
        self.send_text(&msg).await?;
        Ok(())
    }

    /// 发送 ICE 候选
    pub async fn send_ice_candidate(
        &self,
        target_device_id: &str,
        candidate: &webrtc::ice_transport::ice_candidate::RTCIceCandidateInit,
    ) -> Result<()> {
        let device_id = self.device_id.lock().await;
        let controller_id = device_id.as_ref().context("not registered")?;

        let cand_json = super::protocol::IceCandidateJson::from(candidate.clone());

        let msg = serde_json::json!({
            "type": "webrtc",
            "action": "iceCandidate",
            "payload": {
                "targetDeviceId": target_device_id,
                "candidate": cand_json,
                "controllerId": controller_id
            }
        })
        .to_string();

        self.send_text(&msg).await?;
        Ok(())
    }

    pub async fn send_capture_update(
        &self,
        target_device_id: &str,
        capture_patch: serde_json::Value,
    ) -> Result<()> {
        let device_id = self.device_id.lock().await;
        let controller_id = device_id.as_ref().context("not registered")?;
        let msg = serde_json::json!({
            "type": "control",
            "action": "updateCapture",
            "payload": {
                "targetDeviceId": target_device_id,
                "controllerId": controller_id,
                "capture": capture_patch,
            }
        })
        .to_string();
        self.send_text(&msg).await
    }

    /// 获取设备 ID
    pub async fn device_id(&self) -> Option<String> {
        self.device_id.lock().await.clone()
    }

    /// 发送文本消息
    async fn send_text(&self, text: &str) -> Result<()> {
        let write = self.write.lock().await;
        let write = write.as_ref().context("not connected")?;
        let mut write = write.lock().await;
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                text.to_string(),
            ))
            .await
            .context("failed to send message")?;
        Ok(())
    }
}

/// 处理接收到的消息
async fn handle_message(
    text: &str,
    device_id: &Arc<Mutex<Option<String>>>,
    event_tx: &mpsc::Sender<SignalingMessagePayload>,
) -> Result<()> {
    tracing::debug!("received message: {}", text);

    let v: Value = serde_json::from_str(text)?;

    let msg_type = v["type"].as_str().unwrap_or("");
    let action = v["action"].as_str().unwrap_or("");

    tracing::debug!("parsed message: type={}, action={}", msg_type, action);

    match (msg_type, action) {
        ("system", "connected") => {
            let dev_id = v["payload"]["deviceId"].as_str().unwrap_or("").to_string();
            *device_id.lock().await = Some(dev_id.clone());
            tracing::debug!("sending Connected event to main loop");
            if event_tx
                .send(SignalingMessagePayload::Connected { device_id: dev_id })
                .await
                .is_err()
            {
                tracing::error!("failed to send Connected event - channel closed");
            }
        }
        ("device", "registered") => {
            let dev_id = v["payload"]["deviceId"].as_str().unwrap_or("").to_string();
            let device_list = parse_device_list(&v["payload"]["deviceList"]);
            tracing::debug!(device_id = %dev_id, count = device_list.len(), "sending Registered event to main loop");
            if event_tx
                .send(SignalingMessagePayload::Registered {
                    device_id: dev_id,
                    device_list,
                })
                .await
                .is_err()
            {
                tracing::error!("failed to send Registered event - channel closed");
            }
        }
        ("device", "deviceList") => {
            let device_list = parse_device_list(&v["payload"]["deviceList"]);
            event_tx
                .send(SignalingMessagePayload::DeviceList { device_list })
                .await
                .ok();
        }
        ("device", "offline") => {
            let dev_id = v["payload"]["deviceId"].as_str().unwrap_or("").to_string();
            event_tx
                .send(SignalingMessagePayload::DeviceOffline { device_id: dev_id })
                .await
                .ok();
        }
        ("webrtc", "answer") => {
            let answer_json = v["payload"]["answer"].clone();
            let controller_id = v["payload"]["controllerId"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let selected_transport = v["payload"]["selectedTransport"]
                .as_str()
                .unwrap_or("webrtc")
                .to_string();
            let quic =
                serde_json::from_value::<QuicTransportInfo>(v["payload"]["quic"].clone()).ok();
            let audio_quic =
                serde_json::from_value::<AudioQuicTransportInfo>(v["payload"]["audioQuic"].clone())
                    .ok();

            if let Ok(sd_json) = serde_json::from_value::<SessionDescriptionJson>(answer_json) {
                if let Ok(answer) = sd_json.try_into() {
                    event_tx
                        .send(SignalingMessagePayload::Answer {
                            answer,
                            controller_id,
                            selected_transport,
                            quic,
                            audio_quic,
                        })
                        .await
                        .ok();
                }
            }
        }
        ("webrtc", "iceCandidate") => {
            let cand_json = v["payload"]["candidate"].clone();
            let target_device_id = v["payload"]["targetDeviceId"]
                .as_str()
                .map(|s| s.to_string());
            let controller_id = v["payload"]["controllerId"].as_str().map(|s| s.to_string());

            if let Ok(cand_json) =
                serde_json::from_value::<super::protocol::IceCandidateJson>(cand_json)
            {
                if let Ok(candidate) = cand_json.try_into() {
                    event_tx
                        .send(SignalingMessagePayload::IceCandidate {
                            target_device_id,
                            controller_id,
                            candidate,
                        })
                        .await
                        .ok();
                }
            }
        }
        _ => {
            warn!(msg_type = %msg_type, action = %action, "unknown message type");
        }
    }

    Ok(())
}

fn parse_device_list(value: &Value) -> Vec<DeviceInfo> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let id = v.get("id")?.as_str()?.to_string();
                    let name = v.get("name")?.as_str()?.to_string();
                    let online = v.get("online")?.as_bool().unwrap_or(true);
                    Some(DeviceInfo { id, name, online })
                })
                .collect()
        })
        .unwrap_or_default()
}
