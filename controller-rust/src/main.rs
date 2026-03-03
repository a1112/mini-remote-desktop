mod input;
mod quic_rx;
mod render;
mod signaling;
mod stats;
mod video;
mod webrtc;

// 视频帧接收器类型别名
type FrameReceiver = Arc<Mutex<mpsc::Receiver<webrtc::peer::VideoFrame>>>;

use anyhow::{Context, Result};
use uuid::Uuid;
use quic_rx::{QuicConnectInfo, connect_quic_receiver};
use render::{D3D11Renderer, OverlaySwitchField};
use signaling::{SignalingClient, SignalingMessagePayload};
use signaling::client::SignalingConfig;
use stats::StatsCollector;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};
use video::{Decoder, DecoderBackend, H264Decoder, H264DecoderConfig};
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
    let ws_url = std::env::var("MRD_SIGNALING_URL")
        .ok()
        .or_else(|| {
            json["ws_url"]
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "ws://127.0.0.1:9527".to_string());
    let device_name = json["device_name"]
        .as_str()
        .unwrap_or("Rust Controller")
        .to_string();
    let preferred_transport = std::env::var("MRD_TRANSPORT")
        .ok()
        .or_else(|| {
            json["transport"]
                .as_str()
                .map(|s| s.to_ascii_lowercase())
        })
        .unwrap_or_else(|| "webrtc".to_string());

    let decoder_mode_str = std::env::var("MRD_DECODER")
        .ok()
        .or_else(|| json["video"]["decoder"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "auto".to_string())
        .to_lowercase();
    let decoder_backend = match decoder_mode_str.as_str() {
        "software" | "sw" => DecoderBackend::Software,
        "d3d11va" | "hardware" | "hw" => DecoderBackend::D3d11va,
        "mf" | "mf_d3d11" | "mediafoundation" => DecoderBackend::MfD3d11,
        _ => DecoderBackend::Auto,
    };
    let num_threads = json["video"]["num_decode_threads"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or(2);
    let enable_hardware = json["video"]["enable_hardware_decode"]
        .as_bool()
        .unwrap_or(true);

    Ok(ControllerConfig {
        signaling: SignalingConfig {
            ws_url,
            device_name,
            preferred_transport,
        },
        video: H264DecoderConfig {
            num_threads,
            enable_hardware,
            backend: decoder_backend,
        },
    })
}

fn should_try_discovery(ws_url: &str) -> bool {
    if std::env::var("MRD_DISCOVERY")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    ws_url.contains("127.0.0.1") || ws_url.contains("localhost") || ws_url.contains("0.0.0.0")
}

fn parse_ws_port(ws_url: &str) -> u16 {
    let s = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .unwrap_or(ws_url);
    let host_port = s.split('/').next().unwrap_or(s);
    host_port
        .rsplit(':')
        .next()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(9527)
}

async fn discover_signaling_ws_url(default_ws_port: u16) -> Option<String> {
    let discovery_port = std::env::var("MRD_DISCOVERY_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(9528);
    let timeout_ms = std::env::var("MRD_DISCOVERY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(900);
    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    let _ = sock.set_broadcast(true);
    let probe = b"MRD_DISCOVER_V1";
    let _ = sock.send_to(probe, format!("255.255.255.255:{discovery_port}")).await;
    let _ = sock.send_to(probe, format!("127.0.0.1:{discovery_port}")).await;
    let mut buf = [0_u8; 512];
    let recv = tokio::time::timeout(Duration::from_millis(timeout_ms), sock.recv_from(&mut buf))
        .await
        .ok()?
        .ok()?;
    let (n, addr) = recv;
    if n == 0 {
        return None;
    }
    let mut ws_port = default_ws_port;
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) {
        if let Some(p) = v.get("ws_port").and_then(|x| x.as_u64()) {
            ws_port = p as u16;
        }
    }
    Some(format!("ws://{}:{}", addr.ip(), ws_port))
}

fn select_frame_for_decode(
    _rx: &mut mpsc::Receiver<webrtc::peer::VideoFrame>,
    first: webrtc::peer::VideoFrame,
) -> webrtc::peer::VideoFrame {
    // H.264 inter frames depend on previous references; keep decode order.
    first
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

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
    let mut config = load_config(&config_path)?;
    if should_try_discovery(&config.signaling.ws_url) {
        if let Some(found) = discover_signaling_ws_url(parse_ws_port(&config.signaling.ws_url)).await {
            info!(original = %config.signaling.ws_url, discovered = %found, "signaling discovery succeeded");
            config.signaling.ws_url = found;
        } else {
            warn!(ws_url = %config.signaling.ws_url, "signaling discovery not found, using configured ws_url");
        }
    }
    info!(
        ws_url = %config.signaling.ws_url,
        device_name = %config.signaling.device_name,
        transport = %config.signaling.preferred_transport,
        "loaded configuration"
    );

    // 创建信令客户端
    let (signaling, mut signaling_rx) = SignalingClient::new(config.signaling.clone());
    signaling.connect().await?;
    signaling.register().await?;

    // 视频帧统计（在渲染器创建前创建）
    let video_frames_received = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // 创建渲染器（传递视频帧统计）
    let renderer = Arc::new(D3D11Renderer::new_with_stats(
        render::RendererConfig::default(),
        video_frames_received.clone(),
    )?);
    info!("DirectX 11 renderer initialized");
    let overlay_stats = renderer.overlay_stats_handle();
    if let Ok(mut ov) = overlay_stats.lock() {
        ov.selected_transport = config.signaling.preferred_transport.clone();
        ov.media_path = "webrtc".to_string();
    }

    // 创建解码器
    let decoder = Arc::new(Mutex::new(H264Decoder::new(config.video.clone())?));
    let decoder_backend_name = {
        let decoder_guard = decoder.lock().await;
        decoder_guard.backend_name()
    };
    info!(backend = decoder_backend_name, "video decoder initialized");

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
    let decoded_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let frame_count_clone = frame_count.clone();
    let decoded_count_clone = decoded_count.clone();
    let decoder_clone = decoder.clone();
    let render_sink = renderer.frame_sink();
    let overlay_stats_for_decode = overlay_stats.clone();

    // 启动视频帧处理任务
    let frame_receiver_clone = frame_receiver.clone();
    tokio::spawn(async move {
        let mut decode_samples_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
        let mut frame_interval_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
        let mut e2e_samples_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
        let mut last_decoded_at: Option<std::time::Instant> = None;
        let mut last_stats_at = std::time::Instant::now();
        let decoder_backend_label = {
            let d = decoder_clone.lock().await;
            d.backend_name().to_string()
        };
        if let Ok(mut ov) = overlay_stats_for_decode.lock() {
            ov.decoder_backend = decoder_backend_label.clone();
        }
        // 处理视频帧
        loop {
            let active_rx = {
                let receiver_guard = frame_receiver_clone.lock().await;
                receiver_guard.clone()
            };
            let Some(active_rx) = active_rx else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let mut rx = active_rx.lock().await;
            let recv = tokio::time::timeout(Duration::from_millis(120), rx.recv()).await;
            if let Ok(Some(frame)) = recv {
                let newest = select_frame_for_decode(&mut rx, frame);
                drop(rx);
                let decode_started = std::time::Instant::now();
                let count = frame_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let mut decoder = decoder_clone.lock().await;
                match decoder.decode(&newest) {
                    Ok(Some(decoded)) => {
                        render_sink.submit(decoded);
                        let dec = decoded_count_clone
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;
                        if dec % 100 == 0 {
                            info!(decoded_frames = dec, "decoded video frame");
                        }
                        if let Ok(mut ov) = overlay_stats_for_decode.lock() {
                            ov.decoded_frames = dec;
                        }

                        let now = std::time::Instant::now();
                        let decode_ms = now.duration_since(decode_started).as_secs_f64() * 1000.0;
                        if decode_samples_ms.len() >= 1024 {
                            decode_samples_ms.pop_front();
                        }
                        decode_samples_ms.push_back(decode_ms);
                        if let Some(prev) = last_decoded_at {
                            let delta_ms = now.duration_since(prev).as_secs_f64() * 1000.0;
                            if frame_interval_ms.len() >= 1024 {
                                frame_interval_ms.pop_front();
                            }
                            frame_interval_ms.push_back(delta_ms);
                        }
                        if newest.tx_unix_us != 0 {
                            if let Ok(elapsed) =
                                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                            {
                                let now_us = elapsed.as_micros().min(u64::MAX as u128) as u64;
                                if now_us >= newest.tx_unix_us {
                                    let e2e_ms = (now_us - newest.tx_unix_us) as f64 / 1000.0;
                                    if e2e_samples_ms.len() >= 1024 {
                                        e2e_samples_ms.pop_front();
                                    }
                                    e2e_samples_ms.push_back(e2e_ms);
                                }
                            }
                        }
                        last_decoded_at = Some(now);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, "decode failed");
                        if let Ok(mut ov) = overlay_stats_for_decode.lock() {
                            ov.decode_failures = ov.decode_failures.saturating_add(1);
                            ov.last_decode_error = e.to_string();
                        }
                    }
                }
                drop(decoder);
                // 每 100 帧记录一次
                if count % 100 == 0 {
                    info!(
                        bytes = newest.data.len(),
                        timestamp = newest.timestamp,
                        seq = newest.sequence,
                        total_frames = count,
                        "received video frame"
                    );
                }
                if last_stats_at.elapsed() >= Duration::from_secs(2) && !decode_samples_ms.is_empty() {
                    let mut decode_sorted: Vec<f64> = decode_samples_ms.iter().copied().collect();
                    decode_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let p95_idx = ((decode_sorted.len() as f64) * 0.95).floor() as usize;
                    let p95 = decode_sorted[p95_idx.min(decode_sorted.len().saturating_sub(1))];
                    let avg_decode = decode_sorted.iter().sum::<f64>() / decode_sorted.len() as f64;
                    let fps = if frame_interval_ms.is_empty() {
                        0.0
                    } else {
                        let avg_delta = frame_interval_ms.iter().sum::<f64>() / frame_interval_ms.len() as f64;
                        if avg_delta > 0.0 { 1000.0 / avg_delta } else { 0.0 }
                    };
                    let jitter = if frame_interval_ms.len() < 2 {
                        0.0
                    } else {
                        let mean = frame_interval_ms.iter().sum::<f64>() / frame_interval_ms.len() as f64;
                        let var = frame_interval_ms
                            .iter()
                            .map(|v| {
                                let d = v - mean;
                                d * d
                            })
                            .sum::<f64>()
                            / frame_interval_ms.len() as f64;
                        var.sqrt()
                    };
                    let (e2e_avg, e2e_p50, e2e_p95, e2e_p99, e2e_samples) = if e2e_samples_ms.is_empty() {
                        (-1.0, -1.0, -1.0, -1.0, 0usize)
                    } else {
                        let mut e2e_sorted: Vec<f64> = e2e_samples_ms.iter().copied().collect();
                        e2e_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        let idx = |p: f64| -> usize {
                            ((e2e_sorted.len() as f64) * p)
                                .floor()
                                .min((e2e_sorted.len().saturating_sub(1)) as f64)
                                as usize
                        };
                        let avg = e2e_sorted.iter().sum::<f64>() / e2e_sorted.len() as f64;
                        (
                            avg,
                            e2e_sorted[idx(0.50)],
                            e2e_sorted[idx(0.95)],
                            e2e_sorted[idx(0.99)],
                            e2e_sorted.len(),
                        )
                    };
                    info!(
                        backend = %decoder_backend_label,
                        fps = format!("{:.2}", fps),
                        avg_decode_ms = format!("{:.3}", avg_decode),
                        p95_decode_ms = format!("{:.3}", p95),
                        jitter_ms = format!("{:.3}", jitter),
                        e2e_avg_ms = format!("{:.3}", e2e_avg),
                        e2e_p50_ms = format!("{:.3}", e2e_p50),
                        e2e_p95_ms = format!("{:.3}", e2e_p95),
                        e2e_p99_ms = format!("{:.3}", e2e_p99),
                        e2e_samples,
                        samples = decode_sorted.len(),
                        "[DECODER-STATS]"
                    );
                    if let Ok(mut ov) = overlay_stats_for_decode.lock() {
                        ov.decode_fps = fps;
                        ov.avg_decode_ms = avg_decode;
                        ov.p95_decode_ms = p95;
                        ov.jitter_ms = jitter;
                        ov.e2e_avg_ms = e2e_avg;
                        ov.e2e_p50_ms = e2e_p50;
                        ov.e2e_p95_ms = e2e_p95;
                        ov.e2e_p99_ms = e2e_p99;
                    }
                    last_stats_at = std::time::Instant::now();
                }
                // TODO: 解码并渲染视频帧
            } else {
                drop(rx);
                continue;
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
                    SignalingMessagePayload::Answer {
                        answer,
                        controller_id,
                        selected_transport,
                        quic,
                    } => {
                        if let Ok(mut ov) = overlay_stats.lock() {
                            ov.selected_transport = selected_transport.clone();
                        }
                        info!(
                            controller_id = %controller_id,
                            selected_transport = %selected_transport,
                            "received WebRTC answer"
                        );
                        // Always complete WebRTC SDP handshake first so media can fallback
                        // immediately if QUIC transport setup fails.
                        let mut webrtc_ready = false;
                        let manager = peer_manager.read().await;
                        if let Some(ref mgr) = *manager {
                            if let Err(e) = mgr.set_remote_description(answer).await {
                                error!(error = %e, "failed to set remote description");
                            } else {
                                info!("remote description set successfully");
                                webrtc_ready = true;
                            }
                        }
                        drop(manager);

                        if selected_transport.eq_ignore_ascii_case("quic") {
                            if let Some(q) = quic {
                                let info = QuicConnectInfo {
                                    addr: q.addr,
                                    server_name: q.server_name,
                                    cert_der_base64: q.cert_der_base64,
                                };
                                match connect_quic_receiver(&info).await {
                                    Ok(rx) => {
                                        *frame_receiver.lock().await = Some(rx);
                                        *connected.lock().await = true;
                                        if let Ok(mut ov) = overlay_stats.lock() {
                                            ov.media_path = "quic".to_string();
                                        }
                                        info!("connected to QUIC media transport");
                                    }
                                    Err(e) => {
                                        if webrtc_ready {
                                            *connected.lock().await = true;
                                            if let Ok(mut ov) = overlay_stats.lock() {
                                                ov.media_path = "webrtc".to_string();
                                            }
                                            warn!(
                                                error = %e,
                                                "failed to connect QUIC media transport, fallback to WebRTC media path"
                                            );
                                        } else {
                                            error!(
                                                error = %e,
                                                "failed to connect QUIC media transport and WebRTC fallback is unavailable"
                                            );
                                        }
                                    }
                                }
                            } else if webrtc_ready {
                                *connected.lock().await = true;
                                if let Ok(mut ov) = overlay_stats.lock() {
                                    ov.media_path = "webrtc".to_string();
                                }
                                warn!("selected transport is quic but quic endpoint info is missing, fallback to WebRTC");
                            } else {
                                warn!("selected transport is quic but quic endpoint info is missing");
                            }
                        } else if webrtc_ready {
                            *connected.lock().await = true;
                            if let Ok(mut ov) = overlay_stats.lock() {
                                ov.media_path = "webrtc".to_string();
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
                let decoded = decoded_count.load(std::sync::atomic::Ordering::Relaxed);
                if count > 0 {
                    info!(total_frames = count, decoded_frames = decoded, "video frames received so far");
                }
            }

            // 检查窗口消息
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                let cmds = renderer.drain_overlay_switch_commands();
                if !cmds.is_empty() {
                    let target = current_agent_id.lock().await.clone();
                    if let Some(target_device_id) = target {
                        // 强制切换时先断开本地旧会话，避免旧流状态阻塞新配置生效。
                        let old_manager = {
                            let mut pm = peer_manager.write().await;
                            pm.take()
                        };
                        if let Some(old) = old_manager {
                            if let Err(e) = old.pc.close().await {
                                warn!(error = %e, "failed to close previous controller peer on switch");
                            }
                        }
                        *frame_receiver.lock().await = None;
                        *connected.lock().await = false;

                        for cmd in cmds {
                            let patch = match cmd.field {
                                OverlaySwitchField::Resolution => {
                                    if cmd.value == "0x0" {
                                        json!({"targetWidth": 0, "targetHeight": 0})
                                    } else {
                                        let parts: Vec<&str> = cmd.value.split('x').collect();
                                        if parts.len() == 2 {
                                            let w = parts[0].parse::<u32>().unwrap_or(0);
                                            let h = parts[1].parse::<u32>().unwrap_or(0);
                                            json!({"targetWidth": w, "targetHeight": h})
                                        } else {
                                            json!({})
                                        }
                                    }
                                }
                                OverlaySwitchField::CaptureWindow => json!({"windowMode": cmd.value}),
                                OverlaySwitchField::Bitrate => {
                                    let br = cmd.value.parse::<u32>().unwrap_or(12000);
                                    json!({"bitrateKbps": br})
                                }
                                OverlaySwitchField::CaptureBackend => json!({"backend": cmd.value}),
                                OverlaySwitchField::Encoder => json!({"encoder": cmd.value}),
                            };
                            if patch != json!({}) {
                                if let Err(e) = signaling_arc
                                    .send_capture_update(&target_device_id, patch.clone())
                                    .await
                                {
                                    warn!(error = %e, "send capture update failed");
                                    continue;
                                }
                                info!(target = %target_device_id, patch = %patch, "sent capture update");
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        let _ = initiate_webRTC_connection(
                            &target_device_id,
                            &signaling_arc,
                            &peer_manager,
                            &current_agent_id,
                            &frame_receiver,
                        )
                        .await;
                    } else {
                        warn!("overlay switch requested but no active agent connected");
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn select_frame_for_decode_preserves_decode_order() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(webrtc::peer::VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x61]),
            timestamp: 2,
            is_keyframe: false,
            sequence: 2,
            tx_unix_us: 0,
        })
        .await
        .unwrap();
        tx.send(webrtc::peer::VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x61]),
            timestamp: 3,
            is_keyframe: false,
            sequence: 3,
            tx_unix_us: 0,
        })
        .await
        .unwrap();
        let first = webrtc::peer::VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
            timestamp: 1,
            is_keyframe: true,
            sequence: 1,
            tx_unix_us: 0,
        };
        let picked = select_frame_for_decode(&mut rx, first);
        assert_eq!(picked.sequence, 1);
    }
}
