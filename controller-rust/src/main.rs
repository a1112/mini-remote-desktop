mod input;
mod render;
mod signaling;
mod stats;
mod video;
mod webrtc;

// 视频帧接收器类型别名
type FrameReceiver = Arc<Mutex<mpsc::Receiver<webrtc::peer::VideoFrame>>>;

use anyhow::{Context, Result};
use uuid::Uuid;
use render::D3D11Renderer;
use signaling::{SignalingClient, SignalingMessagePayload};
use signaling::client::SignalingConfig;
use stats::StatsCollector;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};
use video::{Decoder, H264Decoder, H264DecoderConfig};
use webrtc::peer::{PeerConfig, PeerConnectionManager};

/// 控制器配置
#[derive(Debug, Clone)]
struct ControllerConfig {
    /// 信令服务器配置
    pub signaling: SignalingConfig,
    /// 视频解码器配置
    pub video: H264DecoderConfig,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            signaling: SignalingConfig::default(),
            video: H264DecoderConfig::default(),
        }
    }
}

/// 从配置文件加载
fn load_config(path: &PathBuf) -> Result<ControllerConfig> {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|_| {
            r#"{
                "ws_url": "ws://127.0.0.1:9527",
                "device_name": "Rust Controller"
            }"#.to_string()
        });

    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let ws_url = json["ws_url"]
        .as_str()
        .unwrap_or("ws://127.0.0.1:9527")
        .to_string();
    let device_name = json["device_name"]
        .as_str()
        .unwrap_or("Rust Controller")
        .to_string();

    Ok(ControllerConfig {
        signaling: SignalingConfig { ws_url, device_name },
        video: H264DecoderConfig::default(),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "controller_rust=info,tokio=warn,webrtc=warn".to_string()),
        )
        .init();

    info!("starting remote desktop controller");

    // 加载配置
    let config_path = PathBuf::from("config.json");
    let config = load_config(&config_path)?;
    info!(
        ws_url = %config.signaling.ws_url,
        device_name = %config.signaling.device_name,
        "loaded configuration"
    );

    // 创建信令客户端
    let (signaling, mut signaling_rx) = SignalingClient::new(config.signaling.clone());
    signaling.connect().await?;
    signaling.register().await?;

    // 视频帧统计（在渲染器创建前创建）
    let video_frames_received = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // 创建渲染器（传递视频帧统计）
    let renderer = D3D11Renderer::new_with_stats(
        render::RendererConfig::default(),
        video_frames_received.clone(),
    )?;
    info!("DirectX 11 renderer initialized");

    // 创建解码器
    let _decoder = H264Decoder::new(config.video.clone())?;
    info!("video decoder initialized");

    // 创建统计收集器
    let _stats = StatsCollector::new();

    // 连接状态
    let connected = Arc::new(Mutex::new(false));
    let current_agent_id = Arc::new(Mutex::new(None::<String>));
    let peer_manager = Arc::new(RwLock::new(None::<PeerConnectionManager>));
    let signaling_arc = Arc::new(signaling);

    // 视频帧接收器
    let frame_receiver: Arc<Mutex<Option<FrameReceiver>>> = Arc::new(Mutex::new(None));

    // 统计更新定时器
    let mut stats_interval = tokio::time::interval(Duration::from_secs(1));

    // 标记：是否已经尝试连接
    let connection_attempted = Arc::new(Mutex::new(false));

    // 视频帧统计
    let frame_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let frame_count_clone = frame_count.clone();
    let video_frames_count_clone = video_frames_received.clone();

    // 启动视频帧处理任务
    let frame_receiver_clone = frame_receiver.clone();
    tokio::spawn(async move {
        let local_rx: FrameReceiver = loop {
            // 获取 receiver
            let receiver_guard = frame_receiver_clone.lock().await;
            if let Some(ref rx) = *receiver_guard {
                break rx.clone();
            }
            drop(receiver_guard);
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        // 处理视频帧
        loop {
            let mut rx = local_rx.lock().await;
            if let Some(frame) = rx.recv().await {
                drop(rx);
                let count = frame_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                // 同时更新渲染器的统计计数器
                video_frames_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // 每 100 帧记录一次
                if count % 100 == 0 {
                    info!(
                        bytes = frame.data.len(),
                        timestamp = frame.timestamp,
                        seq = frame.sequence,
                        total_frames = count,
                        "received video frame"
                    );
                }
                // TODO: 解码并渲染视频帧
            } else {
                // 通道关闭
                break;
            }
        }
    });

    // 主事件循环
    loop {
        tokio::select! {
            // 处理信令事件
            Some(event) = signaling_rx.recv() => {
                match event {
                    SignalingMessagePayload::Connected { device_id } => {
                        info!(device_id = %device_id, "connected to signaling server");
                    }
                    SignalingMessagePayload::Registered { device_id, device_list } => {
                        info!(device_id = %device_id, count = device_list.len(), "registered controller");
                        // 显示可用的设备列表
                        for device in &device_list {
                            info!(id = %device.id, name = %device.name, "available agent");
                        }

                        // 自动连接到第一个可用的 agent
                        if let Some(agent) = device_list.first() {
                            let mut attempted = connection_attempted.lock().await;
                            if !*attempted {
                                *attempted = true;
                                drop(attempted);

                                info!("initiating WebRTC connection to agent {}", agent.id);
                                if let Err(e) = initiate_webRTC_connection(
                                    &agent.id,
                                    &signaling_arc,
                                    &peer_manager,
                                    &current_agent_id,
                                    &frame_receiver,
                                ).await {
                                    error!(error = %e, "failed to initiate WebRTC connection");
                                }
                            }
                        }
                    }
                    SignalingMessagePayload::DeviceList { device_list } => {
                        info!(count = device_list.len(), "received device list update");
                        for device in &device_list {
                            info!(id = %device.id, name = %device.name, online = device.online, "agent");
                        }

                        // 如果还没有连接，尝试连接到第一个可用的 agent
                        let agent_id_lock = current_agent_id.lock().await;
                        if agent_id_lock.is_none() {
                            drop(agent_id_lock);
                            if let Some(agent) = device_list.first() {
                                if agent.online {
                                    let mut attempted = connection_attempted.lock().await;
                                    if !*attempted {
                                        *attempted = true;
                                        drop(attempted);

                                        info!("initiating WebRTC connection to agent {}", agent.id);
                                        if let Err(e) = initiate_webRTC_connection(
                                            &agent.id,
                                            &signaling_arc,
                                            &peer_manager,
                                            &current_agent_id,
                                            &frame_receiver,
                                        ).await {
                                            error!(error = %e, "failed to initiate WebRTC connection");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    SignalingMessagePayload::DeviceOffline { device_id } => {
                        info!(device_id = %device_id, "agent went offline");
                        // 清除当前连接
                        let mut current_id = current_agent_id.lock().await;
                        if current_id.as_ref() == Some(&device_id) {
                            *current_id = None;
                            *connected.lock().await = false;
                            *connection_attempted.lock().await = false;
                            warn!("current agent disconnected");
                        }
                    }
                    SignalingMessagePayload::Answer { answer, controller_id } => {
                        info!("received WebRTC Answer from {}", controller_id);
                        // 设置远程描述
                        let manager = peer_manager.read().await;
                        if let Some(ref mgr) = *manager {
                            if let Err(e) = mgr.set_remote_description(answer).await {
                                error!(error = %e, "failed to set remote description");
                            } else {
                                info!("remote description set successfully");
                                *connected.lock().await = true;
                            }
                        }
                    }
                    SignalingMessagePayload::IceCandidate { candidate, .. } => {
                        info!("received ICE candidate");
                        let manager = peer_manager.read().await;
                        if let Some(ref mgr) = *manager {
                            if let Err(e) = mgr.add_ice_candidate(candidate).await {
                                error!(error = %e, "failed to add ICE candidate");
                            }
                        }
                    }
                    _ => {}
                }
            }

            // 定期打印统计信息
            _ = stats_interval.tick() => {
                let count = frame_count.load(std::sync::atomic::Ordering::Relaxed);
                if count > 0 {
                    info!(total_frames = count, "video frames received so far");
                }
            }

            // 检查窗口消息
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                // 继续
            }
        }
    }
}

/// 发起 WebRTC 连接
async fn initiate_webRTC_connection(
    target_device_id: &str,
    signaling: &Arc<SignalingClient>,
    peer_manager: &Arc<RwLock<Option<PeerConnectionManager>>>,
    current_agent_id: &Arc<Mutex<Option<String>>>,
    frame_receiver: &Arc<Mutex<Option<FrameReceiver>>>,
) -> Result<()> {
    // 创建 PeerConnection
    let (manager, frame_rx) = PeerConnectionManager::create(
        target_device_id.to_string(),
        PeerConfig::default(),
        signaling.clone(),
    ).await?;

    // 创建 Offer
    let offer = manager.pc.create_offer(None).await
        .context("failed to create offer")?;

    // 设置本地描述（这会触发 ICE 收集）
    manager.pc.set_local_description(offer.clone()).await
        .context("failed to set local description")?;

    // 等待 ICE 收集完成
    let (ice_complete_tx, mut ice_complete_rx) = tokio::sync::oneshot::channel::<()>();
    let pc_clone = manager.pc.clone();
    tokio::spawn(async move {
        let mut last_state = pc_clone.ice_gathering_state();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let current_state = pc_clone.ice_gathering_state();
            if current_state != last_state {
                info!("ICE gathering state changed: {:?} -> {:?}", last_state, current_state);
                last_state = current_state;
            }
            // Complete 状态的字符串表示是 "complete"
            let state_str = format!("{:?}", current_state).to_lowercase();
            if state_str.contains("complete") {
                info!("ICE gathering complete");
                let _ = ice_complete_tx.send(());
                break;
            }
        }
    });

    // 等待 ICE 收集完成
    let _ = ice_complete_rx.await;

    // 获取更新后的本地描述
    let offer_to_send = manager.pc.local_description().await
        .context("no local description")?;

    // 打印 SDP 用于调试
    match offer_to_send.unmarshal() {
        Ok(sdp) => {
            let sdp_str = sdp.to_string();
            let has_ice = sdp_str.contains("ice-ufrag");
            info!("SDP contains ice-ufrag: {}", has_ice);
            if !has_ice {
                warn!("SDP does NOT contain ice-ufrag! First 500 chars: {}",
                      sdp_str.chars().take(500).collect::<String>());
            }
        }
        Err(e) => {
            warn!("Failed to unmarshal SDP: {}", e);
        }
    }

    // 发送 Offer
    let _controller_id = signaling.device_id().await
        .context("controller not registered")?;
    signaling.send_offer(target_device_id, &offer_to_send, &Uuid::new_v4().to_string()).await
        .context("failed to send offer")?;

    info!(target = %target_device_id, "WebRTC offer sent");

    // 保存 manager 和 frame receiver
    *peer_manager.write().await = Some(manager);
    *current_agent_id.lock().await = Some(target_device_id.to_string());
    *frame_receiver.lock().await = Some(frame_rx);

    Ok(())
}
