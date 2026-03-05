//! UDP 心跳客户端
//!
//! 参考 RustDesk 客户端实现：
//! - 定期发送 UDP 心跳到服务器
//! - 轻量级，低开销
//! - 支持动态服务器发现

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::{interval, Instant};
use tracing::{debug, error, info, warn};

/// 心跳消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatMessage {
    pub device_id: String,
    pub device_type: String,
    pub device_name: String,
    pub protocol_version: u32,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub transports: Vec<String>,
}

/// 心跳响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub server_timestamp_ms: u64,
    pub online_count: usize,
    #[serde(default)]
    pub reregister: bool,
}

/// 心跳客户端配置
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// 心跳服务器地址
    pub server_addr: SocketAddr,

    /// 设备 ID
    pub device_id: String,

    /// 设备类型
    pub device_type: String,

    /// 设备名称
    pub device_name: String,

    /// 心跳间隔（秒）
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// 协议版本
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u32,

    /// 支持的传输协议
    #[serde(default)]
    pub transports: Vec<String>,
}

fn default_heartbeat_interval() -> u64 { 30 }
fn default_protocol_version() -> u32 { 2 }

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:21114".parse().unwrap(),
            device_id: String::new(),
            device_type: "agent".to_string(),
            device_name: "Unknown Device".to_string(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            protocol_version: default_protocol_version(),
            transports: vec!["webrtc".to_string(), "quic".to_string()],
        }
    }
}

/// 心跳客户端
pub struct HeartbeatClient {
    config: HeartbeatConfig,
    socket: Arc<UdpSocket>,
    running: Arc<AtomicBool>,
}

impl HeartbeatClient {
    /// 创建新的心跳客户端
    pub fn new(config: HeartbeatConfig) -> Result<Self> {
        // 绑定到随机端口
        let socket = UdpSocket::bind("0.0.0.0:0")
            .context("failed to bind UDP socket for heartbeat")?;

        info!(
            server_addr = %config.server_addr,
            device_id = %config.device_id,
            interval_secs = config.heartbeat_interval_secs,
            "heartbeat client created"
        );

        Ok(Self {
            config,
            socket: Arc::new(socket),
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 启动心跳任务
    pub fn start(&self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            warn!("heartbeat client already running");
            return Ok(());
        }

        self.running.store(true, Ordering::Relaxed);
        let socket = self.socket.clone();
        let config = self.config.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(config.heartbeat_interval_secs));
            ticker.tick().await; // 跳过第一次立即触发

            info!(
                device_id = %config.device_id,
                interval_secs = config.heartbeat_interval_secs,
                "heartbeat task started"
            );

            // 立即发送第一次心跳
            if let Err(e) = send_heartbeat(&socket, &config).await {
                error!(error = %e, "failed to send initial heartbeat");
            }

            while running.load(Ordering::Relaxed) {
                ticker.tick().await;

                if !running.load(Ordering::Relaxed) {
                    break;
                }

                if let Err(e) = send_heartbeat(&socket, &config).await {
                    warn!(error = %e, "heartbeat send failed, will retry");
                } else {
                    debug!(
                        device_id = %config.device_id,
                        "heartbeat sent"
                    );
                }
            }

            info!("heartbeat task stopped");
        });

        Ok(())
    }

    /// 停止心跳
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// 手动发送一次心跳
    pub async fn send_once(&self) -> Result<()> {
        send_heartbeat(&self.socket, &self.config).await
    }

    /// 检查是否正在运行
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for HeartbeatClient {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 发送心跳消息
async fn send_heartbeat(socket: &UdpSocket, config: &HeartbeatConfig) -> Result<()> {
    let msg = HeartbeatMessage {
        device_id: config.device_id.clone(),
        device_type: config.device_type.clone(),
        device_name: config.device_name.clone(),
        protocol_version: config.protocol_version,
        timestamp_ms: now_ms(),
        transports: config.transports.clone(),
    };

    let data = serde_json::to_vec(&msg)
        .context("failed to serialize heartbeat message")?;

    socket.send_to(&data, config.server_addr)
        .await
        .context("failed to send heartbeat UDP packet")?;

    Ok(())
}

/// 获取当前时间戳（毫秒）
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 心跳发现客户端
///
/// 用于发现本地网络中的心跳服务器
pub struct HeartbeatDiscovery {
    socket: Arc<UdpSocket>,
    discovery_port: u16,
}

impl HeartbeatDiscovery {
    /// 创建发现客户端
    pub fn new(discovery_port: u16) -> Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))
            .context("failed to bind UDP socket for discovery")?;

        socket.set_broadcast(true)
            .context("failed to enable broadcast")?;

        Ok(Self {
            socket: Arc::new(socket),
            discovery_port,
        })
    }

    /// 发现本地服务器
    pub async fn discover(&self, timeout_ms: u64) -> Result<Vec<SocketAddr>> {
        const DISCOVERY_MAGIC: &[u8] = b"MRD_DISCOVER_V1";
        const BROADCAST_ADDR: &str = "255.255.255.255";

        // 发送广播发现消息
        self.socket.send_to(DISCOVERY_MAGIC, (BROADCAST_ADDR, self.discovery_port))
            .await
            .context("failed to send discovery broadcast")?;

        debug!("sent discovery broadcast");

        // 等待响应
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut addrs = Vec::new();

        while Instant::now() < deadline {
            let remaining = deadline.duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining, self.socket.recv_from(&mut buf)).await {
                Ok(Ok((len, addr))) => {
                    if let Ok(response) = serde_json::from_slice::<serde_json::Value>(&buf[..len]) {
                        if response["proto"] == "mrd-discovery-v1" {
                            if let Some(ws_port) = response["ws_port"].as_u64() {
                                // 构造 WebSocket 地址
                                let ws_addr = SocketAddr::new(addr.ip(), ws_port as u16);
                                addrs.push(ws_addr);
                                info!(ws_addr = %ws_addr, "discovered signaling server");
                            }
                        }
                    }
                }
                _ => break,
            }
        }

        Ok(addrs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_message_serialization() {
        let msg = HeartbeatMessage {
            device_id: "test-device-001".to_string(),
            device_type: "agent".to_string(),
            device_name: "Test Device".to_string(),
            protocol_version: 2,
            timestamp_ms: 1234567890,
            transports: vec!["webrtc".to_string()],
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("test-device-001"));
        assert!(json.contains("agent"));

        let decoded: HeartbeatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.device_id, msg.device_id);
        assert_eq!(decoded.device_type, msg.device_type);
    }

    #[test]
    fn test_heartbeat_config_defaults() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(cfg.heartbeat_interval_secs, 30);
        assert_eq!(cfg.protocol_version, 2);
        assert!(cfg.transports.contains(&"webrtc".to_string()));
    }
}
