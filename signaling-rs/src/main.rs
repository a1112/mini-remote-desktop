use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};
use uuid::Uuid;

const DEFAULT_PORT: u16 = 9527;
const DEFAULT_CHANNEL_CAPACITY: usize = 32;
const DEFAULT_MAX_MSG_SIZE: usize = 1_048_576; // 1MB
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;
const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 60;
const DEFAULT_DISCOVERY_PORT: u16 = 9528;
const DISCOVERY_MAGIC: &[u8] = b"MRD_DISCOVER_V1";

/// 服务器配置
#[derive(Debug, Clone, Deserialize)]
struct ServerConfig {
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_host")]
    host: String,
    #[serde(default = "default_channel_capacity")]
    channel_capacity: usize,
    #[serde(default = "default_max_msg_size")]
    max_msg_size: usize,
    #[serde(default = "default_heartbeat_interval")]
    heartbeat_interval_secs: u64,
    #[serde(default = "default_connection_timeout")]
    connection_timeout_secs: u64,
    #[serde(default = "default_discovery_enable")]
    discovery_enable: bool,
    #[serde(default = "default_discovery_port")]
    discovery_port: u16,
}

fn default_port() -> u16 { DEFAULT_PORT }
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_channel_capacity() -> usize { DEFAULT_CHANNEL_CAPACITY }
fn default_max_msg_size() -> usize { DEFAULT_MAX_MSG_SIZE }
fn default_heartbeat_interval() -> u64 { DEFAULT_HEARTBEAT_INTERVAL_SECS }
fn default_connection_timeout() -> u64 { DEFAULT_CONNECTION_TIMEOUT_SECS }
fn default_discovery_enable() -> bool { true }
fn default_discovery_port() -> u16 { DEFAULT_DISCOVERY_PORT }

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            channel_capacity: default_channel_capacity(),
            max_msg_size: default_max_msg_size(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            connection_timeout_secs: default_connection_timeout(),
            discovery_enable: default_discovery_enable(),
            discovery_port: default_discovery_port(),
        }
    }
}

/// 从配置文件加载配置，如果文件不存在或解析失败则返回默认配置
fn load_config(path: &Path) -> ServerConfig {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            match toml::from_str::<ServerConfig>(&content) {
                Ok(mut cfg) => {
                    // 验证并限制配置值
                    cfg.port = cfg.port.clamp(1024, 65535);
                    cfg.channel_capacity = cfg.channel_capacity.clamp(1, 1024);
                    cfg.max_msg_size = cfg.max_msg_size.clamp(1024, 100 * 1024 * 1024);
                    cfg.heartbeat_interval_secs = cfg.heartbeat_interval_secs.clamp(5, 300);
                    cfg.connection_timeout_secs = cfg.connection_timeout_secs.clamp(10, 600);
                    cfg.discovery_port = cfg.discovery_port.clamp(1024, 65535);
                    info!(config_path = %path.display(), ?cfg, "loaded configuration from file");
                    cfg
                }
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "failed to parse config file, using defaults");
                    ServerConfig::default()
                }
            }
        }
        Err(_) => {
            info!(path = %path.display(), "config file not found, using defaults");
            ServerConfig::default()
        }
    }
}

#[derive(Clone)]
struct Device {
    id: String,
    kind: String,
    name: String,
    capabilities: Option<Value>,
    transports: Vec<String>,
    tx: mpsc::Sender<Message>,
    last_seen: Instant,
}

#[derive(Default)]
struct State {
    devices: HashMap<String, Device>,
}

type SharedState = Arc<RwLock<State>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "signaling_rs=info,tokio=warn".to_string()),
        )
        .init();

    // 加载配置文件
    let cfg = load_config(Path::new("config.toml"));

    let listener = TcpListener::bind((cfg.host.as_str(), cfg.port))
        .await
        .expect("bind signaling-rs failed");

    info!(
        host = %cfg.host,
        port = cfg.port,
        channel_capacity = cfg.channel_capacity,
        max_msg_size = cfg.max_msg_size,
        "signaling server listening"
    );

    let state = Arc::new(RwLock::new(State::default()));
    spawn_timeout_sweeper(
        state.clone(),
        Duration::from_secs(cfg.heartbeat_interval_secs),
        Duration::from_secs(cfg.connection_timeout_secs),
    );
    if cfg.discovery_enable {
        let discovery_host = cfg.host.clone();
        let discovery_port = cfg.discovery_port;
        let ws_port = cfg.port;
        tokio::spawn(async move {
            if let Err(e) = run_discovery_responder(&discovery_host, discovery_port, ws_port).await {
                error!(error = %e, "discovery responder stopped");
            }
        });
    } else {
        info!("UDP discovery responder disabled by config");
    }

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "accept error");
                continue;
            }
        };

        let state = state.clone();
        let channel_capacity = cfg.channel_capacity;
        let max_msg_size = cfg.max_msg_size;
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, state, channel_capacity, max_msg_size).await {
                error!(address = %addr, error = %e, "connection error");
            }
        });
    }
}

async fn run_discovery_responder(host: &str, discovery_port: u16, ws_port: u16) -> Result<(), String> {
    let sock = UdpSocket::bind((host, discovery_port))
        .await
        .map_err(|e| format!("discovery udp bind failed: {e}"))?;
    info!(
        host = %host,
        discovery_port,
        ws_port,
        "UDP discovery responder listening"
    );
    let mut buf = [0_u8; 512];
    loop {
        let (n, peer) = sock
            .recv_from(&mut buf)
            .await
            .map_err(|e| format!("discovery recv_from failed: {e}"))?;
        if &buf[..n] != DISCOVERY_MAGIC {
            continue;
        }
        let reply = json!({
            "proto": "mrd-discovery-v1",
            "ws_port": ws_port
        })
        .to_string();
        if let Err(e) = sock.send_to(reply.as_bytes(), peer).await {
            warn!(error = %e, peer = %peer, "failed to send discovery response");
            continue;
        }
        info!(peer = %peer, "sent discovery response");
    }
}

async fn handle_conn(stream: TcpStream, state: SharedState, channel_capacity: usize, max_msg_size: usize) -> Result<(), String> {
    let ws = accept_async(stream)
        .await
        .map_err(|e| format!("ws handshake failed: {e}"))?;

    let conn_id = Uuid::new_v4().to_string();
    let (mut ws_tx, mut ws_rx) = ws.split();

    let (tx, mut rx) = mpsc::channel::<Message>(channel_capacity);

    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tx.send(Message::Text(
        json!({"type":"system","action":"connected","payload":{"deviceId":conn_id}})
            .to_string(),
    ))
    .await
    .map_err(|e| format!("send connected failed: {e}"))?;

    while let Some(msg) = ws_rx.next().await {
        let msg = match msg {
            Ok(v) => v,
            Err(e) => return Err(format!("ws read failed: {e}")),
        };

        if msg.is_close() {
            break;
        }

        if !msg.is_text() {
            continue;
        }

        let text = msg.into_text().map_err(|e| format!("text decode failed: {e}"))?;

        // 验证消息大小
        if text.len() > max_msg_size {
            warn!(
                conn_id = %conn_id,
                size = text.len(),
                max_size = max_msg_size,
                "message too large, rejecting"
            );
            continue;
        }

        handle_message(&conn_id, &tx, &text, &state).await;
    }

    on_disconnect(&conn_id, &state).await;
    write_task.abort();
    Ok(())
}

async fn handle_message(conn_id: &str, tx: &mpsc::Sender<Message>, text: &str, state: &SharedState) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        warn!(conn_id = %conn_id, "invalid json message");
        return;
    };

    let action = v["action"].as_str().unwrap_or("");

    // Treat any valid message as activity. This prevents active clients from
    // being swept only because they don't send explicit ping.
    if action != "register" {
        touch(conn_id, state).await;
    }

    match action {
        "register" => {
            let kind = v["payload"]["type"].as_str().unwrap_or("unknown").to_string();
            let name = v["payload"]["name"]
                .as_str()
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{kind}-{conn_id}"));
            let capabilities = v["payload"]
                .get("capabilities")
                .filter(|val| val.is_object())
                .cloned();
            let transports = v["payload"]["transports"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            {
                let mut s = state.write().await;
                s.devices.insert(
                    conn_id.to_string(),
                    Device {
                        id: conn_id.to_string(),
                        kind: kind.clone(),
                        name: name.clone(),
                        capabilities,
                        transports,
                        tx: tx.clone(),
                        last_seen: Instant::now(),
                    },
                );
            }

            info!(kind = %kind, name = %name, conn_id = %conn_id, "device registered");
            send_registered(conn_id, tx, state).await;
            broadcast_device_list(state, Some(conn_id)).await;
        }
        "getDeviceList" => {
            send_device_list(tx, state).await;
        }
        "ping" => {
            touch(conn_id, state).await;
            let msg = json!({"type":"device","action":"pong"}).to_string();
            let _ = tx.try_send(Message::Text(msg));
        }
        "offer" => {
            let target = v["payload"]["targetDeviceId"].as_str().unwrap_or("");
            if target.is_empty() {
                warn!(conn_id = %conn_id, "offer missing targetDeviceId");
                return;
            }
            let offer = v["payload"]["offer"].clone();
            let transport = v["payload"]
                .get("transport")
                .and_then(|vv| vv.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| "webrtc".to_string());
            let capabilities = v["payload"]
                .get("capabilities")
                .filter(|val| val.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let msg = json!({
                "type":"webrtc",
                "action":"offer",
                "payload": {
                  "targetDeviceId": target,
                  "offer": offer,
                  "sessionId": Uuid::new_v4().to_string(),
                  "controllerId": conn_id,
                  "transport": transport,
                  "capabilities": capabilities,
                }
            })
            .to_string();
            send_to_device(target, &msg, state).await;
        }
        "answer" => {
            let controller_id = v["payload"]["controllerId"].as_str().unwrap_or("");
            if controller_id.is_empty() {
                warn!(conn_id = %conn_id, "answer missing controllerId");
                return;
            }
            let payload = v["payload"].clone();
            let msg = json!({"type":"webrtc","action":"answer","payload":payload}).to_string();
            send_to_device(controller_id, &msg, state).await;
        }
        "iceCandidate" => {
            let target = v["payload"]["targetDeviceId"].as_str().unwrap_or("");
            if target.is_empty() {
                warn!(conn_id = %conn_id, "iceCandidate missing targetDeviceId");
                return;
            }
            let candidate = v["payload"]["candidate"].clone();
            let msg = json!({
                "type":"webrtc",
                "action":"iceCandidate",
                "payload":{"candidate": candidate, "controllerId": conn_id}
            })
            .to_string();
            send_to_device(target, &msg, state).await;
        }
        "updateCapture" => {
            let target = v["payload"]["targetDeviceId"].as_str().unwrap_or("");
            if target.is_empty() {
                warn!(conn_id = %conn_id, "updateCapture missing targetDeviceId");
                return;
            }
            let capture = v["payload"]["capture"].clone();
            let msg = json!({
                "type":"control",
                "action":"updateCapture",
                "payload":{"controllerId": conn_id, "capture": capture}
            })
            .to_string();
            send_to_device(target, &msg, state).await;
        }
        _ => {
            warn!(conn_id = %conn_id, action = %action, "unknown action");
        }
    }
}

async fn send_registered(conn_id: &str, tx: &mpsc::Sender<Message>, state: &SharedState) {
    let list = build_device_list(state).await;
    let msg = json!({
        "type":"device",
        "action":"registered",
        "payload":{"deviceId": conn_id, "deviceList": list}
    })
    .to_string();
    info!(conn_id = %conn_id, device_count = list.len(), "sending registered message");
    if let Err(e) = tx.try_send(Message::Text(msg.clone())) {
        error!(error = %e, "failed to send registered message");
    } else {
        info!(msg = %msg, "registered message sent");
    }
}

async fn send_device_list(tx: &mpsc::Sender<Message>, state: &SharedState) {
    let list = build_device_list(state).await;
    let msg = json!({"type":"device","action":"deviceList","payload":{"deviceList":list}}).to_string();
    let _ = tx.try_send(Message::Text(msg));
}

async fn build_device_list(state: &SharedState) -> Vec<Value> {
    let s = state.read().await;
    s.devices
        .values()
        .filter(|d| d.kind == "agent" || d.kind == "agent-rust")
        .map(device_to_list_item)
        .collect()
}

async fn broadcast_device_list(state: &SharedState, exclude_id: Option<&str>) {
    // 一次加锁完成所有操作
    let (list, targets) = {
        let s = state.read().await;
        let list: Vec<Value> = s
            .devices
            .values()
            .filter(|d| d.kind == "agent" || d.kind == "agent-rust")
            .map(device_to_list_item)
            .collect();
        let targets: Vec<mpsc::Sender<Message>> = s
            .devices
            .values()
            .filter(|d| d.kind == "controller")
            .filter(|d| exclude_id.map(|x| x != d.id).unwrap_or(true))
            .map(|d| d.tx.clone())
            .collect();
        (list, targets)
    };

    let msg = Message::Text(
        json!({"type":"device","action":"deviceList","payload":{"deviceList":list}})
            .to_string(),
    );

    for tx in targets {
        let _ = tx.try_send(msg.clone());
    }
}

fn device_to_list_item(d: &Device) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), Value::String(d.id.clone()));
    obj.insert("name".to_string(), Value::String(d.name.clone()));
    obj.insert("online".to_string(), Value::Bool(true));
    if let Some(cap) = &d.capabilities {
        obj.insert("capabilities".to_string(), cap.clone());
    }
    if !d.transports.is_empty() {
        obj.insert(
            "transports".to_string(),
            Value::Array(
                d.transports
                    .iter()
                    .map(|v| Value::String(v.clone()))
                    .collect(),
            ),
        );
    }
    Value::Object(obj)
}

async fn send_to_device(target_id: &str, msg: &str, state: &SharedState) {
    let tx = {
        let s = state.read().await;
        s.devices.get(target_id).map(|d| d.tx.clone())
    };
    if let Some(tx) = tx {
        if let Err(e) = tx.try_send(Message::Text(msg.to_string())) {
            warn!(error = %e, target_id = %target_id, "failed to send message to device");
        }
    } else {
        warn!(target_id = %target_id, "device not found");
    }
}

async fn touch(conn_id: &str, state: &SharedState) {
    let mut s = state.write().await;
    if let Some(d) = s.devices.get_mut(conn_id) {
        d.last_seen = Instant::now();
    }
}

async fn on_disconnect(conn_id: &str, state: &SharedState) {
    // 一次加锁完成所有操作
    let (device_info, targets) = {
        let mut s = state.write().await;
        let removed = s.devices.remove(conn_id);
        if let Some(ref d) = removed {
            let targets: Vec<mpsc::Sender<Message>> = s
                .devices
                .values()
                .filter(|v| v.kind == "controller")
                .map(|v| v.tx.clone())
                .collect();
            (Some((d.name.clone(), d.id.clone())), targets)
        } else {
            (None, Vec::new())
        }
    };

    if let Some((name, id)) = device_info {
        info!(name = %name, conn_id = %id, "device offline");

        let msg = Message::Text(
            json!({"type":"device","action":"offline","payload":{"deviceId":id}})
                .to_string(),
        );

        for tx in targets {
            let _ = tx.try_send(msg.clone());
        }
    }
}

fn spawn_timeout_sweeper(state: SharedState, heartbeat_interval: Duration, connection_timeout: Duration) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(heartbeat_interval).await;
            let stale = {
                let s = state.read().await;
                s.devices
                    .values()
                    .filter(|d| d.last_seen.elapsed() > connection_timeout)
                    .map(|d| d.id.clone())
                    .collect::<Vec<_>>()
            };

            for id in stale {
                on_disconnect(&id, &state).await;
            }
        }
    });
}
