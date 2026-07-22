//! UDP 心跳服务器
//!
//! 参考 RustDesk hbbs 设计：
//! - 使用 UDP 轻量级心跳
//! - 30秒心跳间隔
//! - 60秒超时断开
//! - 维护设备在线状态和 IP 地址信息

pub mod client;
pub mod protocol;

use anyhow::Result;
use protocol::HeartbeatMessage;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket as TokioUdpSocket;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// 默认配置
const DEFAULT_UDP_PORT: u16 = 21114;
const DEFAULT_WEBSOCKET_PORT: u16 = 9527;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const CONNECTION_TIMEOUT_SECS: u64 = 60;

/// 在线设备信息
#[derive(Debug, Clone)]
struct OnlineDevice {
    device_type: String,
    device_name: String,
    last_seen: Instant,
}

/// 服务器状态
struct ServerState {
    devices: HashMap<String, OnlineDevice>,
}

type SharedState = Arc<RwLock<ServerState>>;

/// 服务器配置
#[derive(Debug, Clone, Deserialize)]
struct ServerConfig {
    #[serde(default = "default_udp_port")]
    udp_port: u16,

    #[serde(default = "default_ws_port")]
    websocket_port: u16,

    #[serde(default = "default_host")]
    host: String,

    #[serde(default = "default_heartbeat_interval")]
    heartbeat_interval_secs: u64,

    #[serde(default = "default_timeout")]
    connection_timeout_secs: u64,
}

fn default_udp_port() -> u16 {
    DEFAULT_UDP_PORT
}
fn default_ws_port() -> u16 {
    DEFAULT_WEBSOCKET_PORT
}
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_heartbeat_interval() -> u64 {
    HEARTBEAT_INTERVAL_SECS
}
fn default_timeout() -> u64 {
    CONNECTION_TIMEOUT_SECS
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            udp_port: default_udp_port(),
            websocket_port: default_ws_port(),
            host: default_host(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            connection_timeout_secs: default_timeout(),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "heartbeat_server=info,tokio=warn".to_string()),
        )
        .init();

    // 加载配置
    let cfg = load_config();
    info!(
        udp_port = cfg.udp_port,
        ws_port = cfg.websocket_port,
        host = %cfg.host,
        heartbeat_interval = cfg.heartbeat_interval_secs,
        timeout = cfg.connection_timeout_secs,
        "UDP Heartbeat Server starting"
    );

    // 创建 UDP socket
    let socket = TokioUdpSocket::bind((cfg.host.as_str(), cfg.udp_port)).await?;
    let local_addr = socket.local_addr()?;
    info!(
        addr = %local_addr,
        "UDP heartbeat server listening"
    );

    // 共享状态
    let state = Arc::new(RwLock::new(ServerState {
        devices: HashMap::new(),
    }));

    // 启动超时清理任务
    let state_clone = state.clone();
    let timeout_duration = Duration::from_secs(cfg.connection_timeout_secs);
    tokio::spawn(async move {
        cleanup_stale_devices(state_clone, timeout_duration).await;
    });

    // 启动统计任务
    let state_clone = state.clone();
    tokio::spawn(async move {
        print_statistics(state_clone).await;
    });

    // 消息处理计数器
    let msg_count = Arc::new(AtomicU64::new(0));

    // 主循环：接收并处理心跳
    let mut buf = vec![0u8; 4096];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, addr)) => {
                let state = state.clone();
                let msg_count = msg_count.clone();
                let packet = buf[..len].to_vec();
                tokio::spawn(async move {
                    if let Err(e) = handle_heartbeat(&packet, addr, state).await {
                        warn!(error = %e, peer = %addr, "failed to handle heartbeat");
                    } else {
                        let count = msg_count.fetch_add(1, Ordering::Relaxed) + 1;
                        if count.is_multiple_of(100) {
                            debug!(count, "heartbeats processed");
                        }
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "recv_from error");
            }
        }
    }
}

/// 处理心跳消息
async fn handle_heartbeat(data: &[u8], addr: SocketAddr, state: SharedState) -> Result<()> {
    // 解析 JSON 消息
    let msg: HeartbeatMessage = serde_json::from_slice(data)?;
    let device_id = msg.device_id.clone();
    let device_type = msg.device_type.clone();
    let device_name = msg.device_name.clone();

    let now = Instant::now();
    let device = OnlineDevice {
        device_type: device_type.clone(),
        device_name: device_name.clone(),
        last_seen: now,
    };

    // 更新设备状态
    {
        let mut s = state.write().await;
        let is_new = !s.devices.contains_key(&device_id);
        s.devices.insert(device_id.clone(), device);

        if is_new {
            info!(
                device_id = %device_id,
                device_type = %device_type,
                device_name = %device_name,
                addr = %addr,
                "device came online"
            );
        } else {
            debug!(
                device_id = %device_id,
                addr = %addr,
                "heartbeat from device"
            );
        }
    }

    Ok(())
}

/// 清理超时设备
async fn cleanup_stale_devices(state: SharedState, timeout: Duration) {
    let mut ticker = interval(Duration::from_secs(10));
    loop {
        ticker.tick().await;

        let stale_devices: Vec<(String, String)> = {
            let s = state.read().await;
            s.devices
                .iter()
                .filter(|(_, d)| d.last_seen.elapsed() > timeout)
                .map(|(id, d)| (id.clone(), d.device_name.clone()))
                .collect()
        };

        if !stale_devices.is_empty() {
            let mut s = state.write().await;
            for (id, name) in stale_devices {
                if let Some(device) = s.devices.remove(&id) {
                    info!(
                        device_id = %id,
                        device_name = %name,
                        last_seen_secs = device.last_seen.elapsed().as_secs(),
                        "device timed out"
                    );
                }
            }
        }
    }
}

/// 打印统计信息
async fn print_statistics(state: SharedState) {
    let mut ticker = interval(Duration::from_secs(60));
    loop {
        ticker.tick().await;

        let (total, agents, controllers) = {
            let s = state.read().await;
            let total = s.devices.len();
            let agents = s
                .devices
                .values()
                .filter(|d| d.device_type == "agent")
                .count();
            let controllers = s
                .devices
                .values()
                .filter(|d| d.device_type == "controller")
                .count();
            (total, agents, controllers)
        };

        info!(
            total_devices = total,
            agents = agents,
            controllers = controllers,
            "heartbeat statistics"
        );
    }
}

/// 从配置文件加载配置
fn load_config() -> ServerConfig {
    let config_path = "config.toml";

    if let Ok(content) = std::fs::read_to_string(config_path) {
        if let Ok(mut cfg) = toml::from_str::<ServerConfig>(&content) {
            // 验证配置
            cfg.udp_port = cfg.udp_port.clamp(1024, 65535);
            cfg.websocket_port = cfg.websocket_port.clamp(1024, 65535);
            cfg.heartbeat_interval_secs = cfg.heartbeat_interval_secs.clamp(5, 300);
            cfg.connection_timeout_secs = cfg.connection_timeout_secs.clamp(10, 600);
            info!("loaded config from {}", config_path);
            return cfg;
        }
    }

    info!("using default configuration");
    ServerConfig::default()
}
