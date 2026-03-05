mod input;
mod quic_rx;
mod recorder;
mod render;
mod signaling;
mod stats;
mod thread_tuning;
mod video;
mod webrtc;

// 视频帧接收器类型别名
type FrameReceiver = Arc<Mutex<mpsc::Receiver<webrtc::peer::VideoFrame>>>;

use anyhow::{Context, Result};
use common_control_proto::ControlEvent;
use quic_rx::{connect_quic_receiver, QuicConnectInfo};
use recorder::{Recorder, RecordingConfig};
use render::{D3D11Renderer, OverlaySwitchField};
use serde_json::json;
use signaling::client::SignalingConfig;
use signaling::{SignalingClient, SignalingMessagePayload};
use stats::StatsCollector;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thread_tuning::{apply_current_thread_tuning, ThreadRole};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;
use video::{Decoder, DecoderBackend, H264Decoder, H264DecoderConfig};
use webrtc::peer::{PeerConfig, PeerConnectionManager};

/// 控制器配置
#[derive(Debug, Clone)]
struct ControllerConfig {
    /// 信令服务器配置
    pub signaling: SignalingConfig,
    /// 视频解码器配置
    pub video: H264DecoderConfig,
    /// 渲染配置
    pub render: render::RendererConfig,
    /// 录制配置
    pub recording: RecordingConfig,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            signaling: SignalingConfig::default(),
            video: H264DecoderConfig::default(),
            render: render::RendererConfig::default(),
            recording: RecordingConfig::default(),
        }
    }
}

/// 从配置文件加载
fn load_config(path: &PathBuf) -> Result<ControllerConfig> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| {
        r#"{
                "ws_url": "ws://127.0.0.1:9527",
                "device_name": "Rust Controller"
            }"#
        .to_string()
    });

    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let ws_url = std::env::var("MRD_SIGNALING_URL")
        .ok()
        .or_else(|| json["ws_url"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "ws://127.0.0.1:9527".to_string());
    let device_name = json["device_name"]
        .as_str()
        .unwrap_or("Rust Controller")
        .to_string();
    let preferred_transport = std::env::var("MRD_TRANSPORT")
        .ok()
        .or_else(|| json["transport"].as_str().map(|s| s.to_ascii_lowercase()))
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
    let sr_mode = json["render"]["sr_mode"]
        .as_str()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "off".to_string());
    let record_enabled = std::env::var("MRD_RECORD_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .or_else(|| json["record"]["enabled"].as_bool())
        .unwrap_or(false);
    let record_output_dir = std::env::var("MRD_RECORD_DIR")
        .ok()
        .or_else(|| json["record"]["output_dir"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "recordings".to_string());
    let record_ffmpeg_path = std::env::var("MRD_RECORD_FFMPEG")
        .ok()
        .or_else(|| {
            json["record"]["ffmpeg_path"]
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "ffmpeg".to_string());
    let record_segment_seconds = std::env::var("MRD_RECORD_SEGMENT_SEC")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .or_else(|| json["record"]["segment_seconds"].as_u64().map(|v| v as u32))
        .unwrap_or(60);
    let record_input_fps = std::env::var("MRD_RECORD_FPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .or_else(|| json["record"]["input_fps"].as_u64().map(|v| v as u32))
        .unwrap_or(60);
    let record_container = std::env::var("MRD_RECORD_CONTAINER")
        .ok()
        .or_else(|| json["record"]["container"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "mp4".to_string());
    let record_video_codec = std::env::var("MRD_RECORD_CODEC")
        .ok()
        .or_else(|| {
            json["record"]["video_codec"]
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "copy".to_string());
    let record_video_preset = std::env::var("MRD_RECORD_PRESET")
        .ok()
        .or_else(|| {
            json["record"]["video_preset"]
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "p4".to_string());
    let record_video_crf = std::env::var("MRD_RECORD_CRF")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .or_else(|| json["record"]["video_crf"].as_i64().map(|v| v as i32))
        .unwrap_or(23);
    let record_video_bitrate_kbps = std::env::var("MRD_RECORD_BITRATE_KBPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .or_else(|| {
            json["record"]["video_bitrate_kbps"]
                .as_u64()
                .map(|v| v as u32)
        })
        .unwrap_or(0);
    let record_queue_depth = std::env::var("MRD_RECORD_QUEUE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| json["record"]["queue_depth"].as_u64().map(|v| v as usize))
        .unwrap_or(512);

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
        render: render::RendererConfig {
            sr_mode,
            ..render::RendererConfig::default()
        },
        recording: RecordingConfig {
            enabled: record_enabled,
            output_dir: record_output_dir,
            ffmpeg_path: record_ffmpeg_path,
            segment_seconds: record_segment_seconds,
            input_fps: record_input_fps,
            container: record_container,
            video_codec: record_video_codec,
            video_preset: record_video_preset,
            video_crf: record_video_crf,
            video_bitrate_kbps: record_video_bitrate_kbps,
            queue_depth: record_queue_depth,
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
    let _ = sock
        .send_to(probe, format!("255.255.255.255:{discovery_port}"))
        .await;
    let _ = sock
        .send_to(probe, format!("127.0.0.1:{discovery_port}"))
        .await;
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

#[derive(Debug, Clone, Copy)]
struct ControlBenchConfig {
    enabled: bool,
    rate_hz: u32,
    log_interval_ms: u64,
    amplitude_px: i32,
}

impl ControlBenchConfig {
    fn from_env() -> Self {
        let enabled = std::env::var("MRD_CTRL_BENCH_ENABLE").ok();
        let rate_hz = std::env::var("MRD_CTRL_BENCH_RATE_HZ")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        let log_interval_ms = std::env::var("MRD_CTRL_BENCH_LOG_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        let amplitude_px = std::env::var("MRD_CTRL_BENCH_AMPLITUDE_PX")
            .ok()
            .and_then(|v| v.parse::<i32>().ok());
        Self::from_values(enabled.as_deref(), rate_hz, log_interval_ms, amplitude_px)
    }

    fn from_values(
        enabled: Option<&str>,
        rate_hz: Option<u32>,
        log_interval_ms: Option<u64>,
        amplitude_px: Option<i32>,
    ) -> Self {
        Self {
            enabled: enabled
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            rate_hz: rate_hz.map(|v| v.clamp(1, 2000)).unwrap_or(250),
            log_interval_ms: log_interval_ms
                .map(|v| v.clamp(200, 10_000))
                .unwrap_or(1000),
            amplitude_px: amplitude_px.map(|v| v.clamp(1, 4000)).unwrap_or(4),
        }
    }
}

#[derive(Default)]
struct ControlTxStats {
    attempts: u64,
    sent: u64,
    failed: u64,
    not_ready: u64,
    send_call_us: VecDeque<u64>,
}

impl ControlTxStats {
    fn push_send_us(&mut self, v: u64) {
        const MAX_SAMPLES: usize = 4096;
        self.send_call_us.push_back(v);
        while self.send_call_us.len() > MAX_SAMPLES {
            let _ = self.send_call_us.pop_front();
        }
    }

    fn p95_send_ms(&self) -> f64 {
        percentile_u64(&self.send_call_us, 95).unwrap_or(0.0) / 1000.0
    }
}

fn unix_time_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn percentile_u64(v: &VecDeque<u64>, p: usize) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = v.iter().copied().collect();
    sorted.sort_unstable();
    let idx = ((sorted.len() - 1) * p) / 100;
    Some(sorted[idx] as f64)
}

async fn run_control_bench(
    peer_manager: Arc<RwLock<Option<Arc<PeerConnectionManager>>>>,
    connected: Arc<Mutex<bool>>,
    cfg: ControlBenchConfig,
) {
    if !cfg.enabled {
        return;
    }
    let tick_us = (1_000_000u64 / cfg.rate_hz as u64).max(500);
    let mut ticker = tokio::time::interval(Duration::from_micros(tick_us));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stats = ControlTxStats::default();
    let mut next_log = Instant::now() + Duration::from_millis(cfg.log_interval_ms);
    let mut flip = false;

    info!(
        enabled = cfg.enabled,
        rate_hz = cfg.rate_hz,
        amplitude_px = cfg.amplitude_px,
        log_interval_ms = cfg.log_interval_ms,
        "control latency benchmark enabled"
    );

    loop {
        ticker.tick().await;
        stats.attempts = stats.attempts.saturating_add(1);
        if !*connected.lock().await {
            stats.not_ready = stats.not_ready.saturating_add(1);
            continue;
        }

        let maybe_mgr = { peer_manager.read().await.clone() };
        let Some(mgr) = maybe_mgr else {
            stats.not_ready = stats.not_ready.saturating_add(1);
            continue;
        };

        let event = if flip {
            ControlEvent::MouseMove {
                x: cfg.amplitude_px,
                y: 0,
            }
        } else {
            ControlEvent::MouseMove {
                x: -cfg.amplitude_px,
                y: 0,
            }
        };
        flip = !flip;

        let started = Instant::now();
        match mgr.send_control_event(event, 0, unix_time_us()).await {
            Ok(()) => stats.sent = stats.sent.saturating_add(1),
            Err(_) => stats.failed = stats.failed.saturating_add(1),
        }
        stats.push_send_us(started.elapsed().as_micros().min(u64::MAX as u128) as u64);

        if Instant::now() >= next_log {
            info!(
                target_rate_hz = cfg.rate_hz,
                attempts = stats.attempts,
                sent = stats.sent,
                failed = stats.failed,
                not_ready = stats.not_ready,
                send_call_p95_ms = format!("{:.3}", stats.p95_send_ms()),
                sample_window = stats.send_call_us.len(),
                "[CTRL-TX]"
            );
            next_log = Instant::now() + Duration::from_millis(cfg.log_interval_ms);
        }
    }
}

fn select_frame_for_decode(
    rx: &mut mpsc::Receiver<webrtc::peer::VideoFrame>,
    first: webrtc::peer::VideoFrame,
    policy: DecodeSelectPolicy,
) -> (webrtc::peer::VideoFrame, u64) {
    match policy {
        DecodeSelectPolicy::Ordered => (first, 0),
        DecodeSelectPolicy::LatestKeyframe => {
            // Prefer the newest keyframe in backlog; otherwise keep decode order.
            let mut dropped = 0u64;
            let mut newest_key: Option<webrtc::peer::VideoFrame> = None;
            while let Ok(next) = rx.try_recv() {
                dropped = dropped.saturating_add(1);
                if next.is_keyframe {
                    newest_key = Some(next);
                }
            }
            (newest_key.unwrap_or(first), dropped)
        }
        DecodeSelectPolicy::Latest => {
            // Aggressive low-latency mode: always decode freshest frame available.
            let mut dropped = 0u64;
            let mut newest = first;
            while let Ok(next) = rx.try_recv() {
                dropped = dropped.saturating_add(1);
                newest = next;
            }
            (newest, dropped)
        }
        DecodeSelectPolicy::AdaptiveAge => {
            // WebRTC path may not carry sender-side wall-clock timestamp.
            // If tx_unix_us is unavailable, keep ordered behavior to avoid false stale drops.
            if first.tx_unix_us == 0 {
                return (first, 0);
            }
            // Adaptive mode: drop frames that are too old, but maintain order
            let max_age_ms = policy.max_age_ms();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros() as u64;
            let mut dropped = 0u64;
            let mut selected = first;
            let first_tx_us = selected.tx_unix_us;
            let mut age_dropped = 0u64;

            // Check if first frame is too old
            let first_age_ms = (now.saturating_sub(selected.tx_unix_us)) / 1000;
            if first_age_ms > max_age_ms {
                // First frame is stale, look for a fresher one
                while let Ok(next) = rx.try_recv() {
                    dropped = dropped.saturating_add(1);
                    let next_age_ms = (now.saturating_sub(next.tx_unix_us)) / 1000;
                    if next_age_ms <= max_age_ms {
                        selected = next;
                        break;
                    } else {
                        age_dropped = age_dropped.saturating_add(1);
                    }
                }
                // If all frames were too old, use the last one anyway
                if dropped > 0 && selected.tx_unix_us == first_tx_us {
                    if let Ok(last) = rx.try_recv() {
                        dropped = dropped.saturating_add(1);
                        selected = last;
                    }
                }
            }
            (selected, dropped)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeSelectPolicy {
    Ordered,
    LatestKeyframe,
    Latest,
    AdaptiveAge,
}

impl DecodeSelectPolicy {
    fn from_env() -> Self {
        let requested = std::env::var("MRD_DECODE_SELECT")
            .ok()
            .unwrap_or_else(|| "ordered".to_string())
            .to_ascii_lowercase();
        let allow_unsafe_latest = std::env::var("MRD_ALLOW_UNSAFE_LATEST")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self::from_env_value(&requested, allow_unsafe_latest)
    }

    fn from_env_value(requested: &str, allow_unsafe_latest: bool) -> Self {
        match requested {
            "latest-key" | "latest_key" | "latestkey" | "key" => Self::LatestKeyframe,
            "latest" => {
                if allow_unsafe_latest {
                    Self::Latest
                } else {
                    warn!(
                        "MRD_DECODE_SELECT=latest is unsafe for H.264 reference chain; using latest-keyframe (set MRD_ALLOW_UNSAFE_LATEST=1 to force)"
                    );
                    Self::LatestKeyframe
                }
            }
            "adaptive-age" | "adaptive_age" | "adaptive" => Self::AdaptiveAge,
            _ => Self::Ordered,
        }
    }

    fn max_age_ms(&self) -> u64 {
        std::env::var("MRD_DECODE_ADAPTIVE_MAX_AGE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(50) // Default 50ms
    }
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
        if let Some(found) =
            discover_signaling_ws_url(parse_ws_port(&config.signaling.ws_url)).await
        {
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
        sr_mode = %config.render.sr_mode,
        recording_enabled = config.recording.enabled,
        "loaded configuration"
    );
    let allow_mf_on_webrtc = std::env::var("MRD_ALLOW_MF_ON_WEBRTC")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if config
        .signaling
        .preferred_transport
        .eq_ignore_ascii_case("webrtc")
        && config.video.backend == DecoderBackend::MfD3d11
        && !allow_mf_on_webrtc
    {
        warn!(
            "MRD_DECODER=mf on WebRTC is unstable in current build; falling back to d3d11va (set MRD_ALLOW_MF_ON_WEBRTC=1 to force mf)"
        );
        config.video.backend = DecoderBackend::D3d11va;
    }

    // 创建信令客户端
    let (signaling, mut signaling_rx) = SignalingClient::new(config.signaling.clone());
    signaling.connect().await?;
    signaling.register().await?;

    // 视频帧统计（在渲染器创建前创建）
    let video_frames_received = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // 创建渲染器（传递视频帧统计）
    let renderer = Arc::new(D3D11Renderer::new_with_stats(
        config.render.clone(),
        video_frames_received.clone(),
    )?);
    info!("DirectX 11 renderer initialized");
    let overlay_stats = renderer.overlay_stats_handle();
    if let Ok(mut ov) = overlay_stats.lock() {
        ov.selected_transport = config.signaling.preferred_transport.clone();
        ov.media_path = "webrtc".to_string();
    }
    let recorder = Arc::new(Recorder::new(config.recording.clone())?);

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
    let peer_manager = Arc::new(RwLock::new(None::<Arc<PeerConnectionManager>>));
    let signaling_arc = Arc::new(signaling);

    // 视频帧接收器
    let frame_receiver: Arc<Mutex<Option<FrameReceiver>>> = Arc::new(Mutex::new(None));

    let control_bench_cfg = ControlBenchConfig::from_env();
    if control_bench_cfg.enabled {
        let peer_manager_for_bench = peer_manager.clone();
        let connected_for_bench = connected.clone();
        tokio::spawn(async move {
            run_control_bench(
                peer_manager_for_bench,
                connected_for_bench,
                control_bench_cfg,
            )
            .await;
        });
    }

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
    let peer_manager_for_decode = peer_manager.clone();
    let video_cfg_for_recover = config.video.clone();
    let render_sink = renderer.frame_sink();
    let overlay_stats_for_decode = overlay_stats.clone();
    let recorder_for_decode = recorder.clone();

    // 启动视频帧处理任务
    let frame_receiver_clone = frame_receiver.clone();
    thread::Builder::new()
        .name("mrd-decode".to_string())
        .spawn(move || {
        let (_decode_thread_tuning, _decode_thread_guard) =
            apply_current_thread_tuning(ThreadRole::Decode);
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                error!(error = %e, "failed to build decode runtime");
                return;
            }
        };
        rt.block_on(async move {
            let decode_select_policy = DecodeSelectPolicy::from_env();
            let disable_decode_recover = std::env::var("MRD_DISABLE_DECODE_RECOVER")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let no_output_recover_streak = std::env::var("MRD_NO_OUTPUT_RECOVER_STREAK")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(180)
                .clamp(30, 2000);
            let no_output_recover_cooldown = Duration::from_millis(
                std::env::var("MRD_NO_OUTPUT_RECOVER_COOLDOWN_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1200)
                    .clamp(100, 10_000),
            );
            let rx_stall_recover_after = Duration::from_millis(
                std::env::var("MRD_RX_STALL_RECOVER_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(800)
                    .clamp(100, 10_000),
            );
            let rx_stall_pli_interval = Duration::from_millis(
                std::env::var("MRD_RX_STALL_PLI_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(300)
                    .clamp(100, 5000),
            );
            let mut decode_samples_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
            let mut frame_interval_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
            let mut e2e_samples_ms: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(1024);
            let mut dropped_old_frames = 0u64;
            let mut decode_recover_stage = 0u8;
            let mut no_output_streak = 0u32;
            let mut waiting_recover_keyframe = false;
            let mut waiting_probe_budget = 0u32;
            let mut waiting_recover_since: Option<std::time::Instant> = None;
            let mut waiting_recover_pli_requests: u32 = 0;
            let mut last_wait_diag_at = std::time::Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(std::time::Instant::now);
            let mut last_recover_keyframe_req = std::time::Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now);
            let mut last_recover_enter = std::time::Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(std::time::Instant::now);
            let mut last_frame_rx_at = std::time::Instant::now();
            let mut has_received_frame = false;
            let mut last_rx_stall_pli_req = std::time::Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now);
            let mut rx_stall_active = false;
            let mut rx_stall_flush_issued = false;
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
                    if rx_stall_active {
                        info!(
                            stall_ms = last_frame_rx_at.elapsed().as_millis() as u64,
                            "video frame intake resumed after stall"
                        );
                        rx_stall_active = false;
                        rx_stall_flush_issued = false;
                    }
                    has_received_frame = true;
                    last_frame_rx_at = std::time::Instant::now();
                    let (newest, dropped_now) =
                        select_frame_for_decode(&mut rx, frame, decode_select_policy);
                    dropped_old_frames = dropped_old_frames.saturating_add(dropped_now);
                    recorder_for_decode.record_frame(&newest);
                    drop(rx);
                    let decode_started = std::time::Instant::now();
                    let count = frame_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    if waiting_recover_keyframe && !newest.is_keyframe {
                        if waiting_recover_since.is_none() {
                            waiting_recover_since = Some(std::time::Instant::now());
                            waiting_recover_pli_requests = 0;
                        }
                        waiting_probe_budget = waiting_probe_budget.saturating_add(1);
                        if last_recover_keyframe_req.elapsed() >= Duration::from_millis(500) {
                            let manager_guard = peer_manager_for_decode.read().await;
                            if let Some(ref mgr) = *manager_guard {
                                let _ = mgr.request_keyframe().await;
                                waiting_recover_pli_requests =
                                    waiting_recover_pli_requests.saturating_add(1);
                            }
                            last_recover_keyframe_req = std::time::Instant::now();
                        }
                        if let Some(since) = waiting_recover_since {
                            if last_wait_diag_at.elapsed() >= Duration::from_secs(1) {
                                warn!(
                                    wait_ms = since.elapsed().as_millis() as u64,
                                    pli_requests = waiting_recover_pli_requests,
                                    probe_budget = waiting_probe_budget,
                                    seq = newest.sequence,
                                    "waiting for recover keyframe: still receiving non-keyframes"
                                );
                                last_wait_diag_at = std::time::Instant::now();
                            }
                        }
                        // Some senders/paths may not expose reliable keyframe markers.
                        // Periodically probe decode to avoid getting stuck in permanent wait.
                        if waiting_probe_budget % 120 != 0 {
                            continue;
                        }
                    }
                    if waiting_recover_keyframe && newest.is_keyframe {
                        waiting_recover_keyframe = false;
                        no_output_streak = 0;
                        waiting_probe_budget = 0;
                        waiting_recover_since = None;
                        waiting_recover_pli_requests = 0;
                        info!(seq = newest.sequence, "decoder recovery synchronized on keyframe");
                    }
                    let mut decoder = decoder_clone.lock().await;
                    match decoder.decode(&newest) {
                    Ok(Some(decoded)) => {
                        no_output_streak = 0;
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
                        if waiting_recover_keyframe {
                            waiting_recover_keyframe = false;
                            no_output_streak = 0;
                            waiting_probe_budget = 0;
                            waiting_recover_since = None;
                            waiting_recover_pli_requests = 0;
                            info!(seq = newest.sequence, "decoder recovery probe succeeded");
                        }
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
                    Ok(None) => {
                        no_output_streak = no_output_streak.saturating_add(1);
                        if !disable_decode_recover
                            && !waiting_recover_keyframe
                            && no_output_streak >= no_output_recover_streak
                            && last_recover_enter.elapsed() >= no_output_recover_cooldown
                        {
                            let streak_before_recover = no_output_streak;
                            waiting_recover_keyframe = true;
                            no_output_streak = 0;
                            last_recover_enter = std::time::Instant::now();
                            waiting_probe_budget = 0;
                            last_recover_keyframe_req = std::time::Instant::now()
                                .checked_sub(Duration::from_secs(1))
                                .unwrap_or_else(std::time::Instant::now);
                            warn!(
                                no_output_streak = streak_before_recover,
                                "decoder no-output streak; entering keyframe resync before backend fallback"
                            );
                        }
                        if !disable_decode_recover && no_output_streak >= 300 && decode_recover_stage < 2 {
                            let mut next_cfg = video_cfg_for_recover.clone();
                            let current_backend = decoder.backend_name().to_ascii_lowercase();
                            if decode_recover_stage == 0 && !current_backend.contains("mf") {
                                next_cfg.backend = DecoderBackend::MfD3d11;
                                warn!(
                                    no_output_streak,
                                    current_backend = %current_backend,
                                    "decoder produced no output for too long; trying MF d3d11 fallback"
                                );
                            } else {
                                next_cfg.backend = DecoderBackend::Software;
                                warn!(
                                    no_output_streak,
                                    current_backend = %current_backend,
                                    "decoder still no output; trying software decoder"
                                );
                            }
                            match H264Decoder::new(next_cfg) {
                                Ok(new_decoder) => {
                                    *decoder = new_decoder;
                                    decode_recover_stage = decode_recover_stage.saturating_add(1);
                                    no_output_streak = 0;
                                    waiting_recover_keyframe = true;
                                    last_recover_keyframe_req = std::time::Instant::now()
                                        .checked_sub(Duration::from_secs(1))
                                        .unwrap_or_else(std::time::Instant::now);
                                    if let Ok(mut ov) = overlay_stats_for_decode.lock() {
                                        ov.decoder_backend = decoder.backend_name().to_string();
                                    }
                                    continue;
                                }
                                Err(recover_err) => {
                                    warn!(error = %recover_err, "decoder no-output recovery attempt failed");
                                    decode_recover_stage = decode_recover_stage.saturating_add(1);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let err_text = e.to_string().to_ascii_lowercase();
                        let non_hw_output_err =
                            err_text.contains("hardware output required but decoder output is");
                        if non_hw_output_err {
                            // First-frame strict HW probe may produce software output.
                            // Keep current decoder alive so following frames can continue on CPU path.
                            warn!(
                                "decoder reported non-D3D11 output in strict mode; keeping current decoder and continuing"
                            );
                            continue;
                        }
                        let invalid_bitstream_err =
                            err_text.contains("invalid data found when processing input")
                                || err_text.contains("send packet to decoder failed")
                                || err_text.contains("corrupt");
                        if !disable_decode_recover && invalid_bitstream_err {
                            // Bitstream desync: wait for next keyframe and keep requesting PLI.
                            waiting_recover_keyframe = true;
                            no_output_streak = 0;
                            last_recover_enter = std::time::Instant::now();
                            last_recover_keyframe_req = std::time::Instant::now()
                                .checked_sub(Duration::from_secs(1))
                                .unwrap_or_else(std::time::Instant::now);
                            let _ = decoder.flush();
                            warn!("decode failed with invalid bitstream; waiting for keyframe resync");
                            continue;
                        }
                        let recoverable_hw_err = err_text.contains("hardware output required")
                            || err_text.contains("d3d11")
                            || err_text.contains("strict mode");
                        if !disable_decode_recover && recoverable_hw_err && decode_recover_stage < 2 {
                            let mut next_cfg = video_cfg_for_recover.clone();
                            let current_backend = decoder.backend_name().to_ascii_lowercase();
                            if decode_recover_stage == 0 && !current_backend.contains("mf") {
                                next_cfg.backend = DecoderBackend::MfD3d11;
                                warn!(
                                    current_backend = %current_backend,
                                    "decode failed on hw strict path; trying MF d3d11 fallback"
                                );
                            } else {
                                next_cfg.backend = DecoderBackend::Software;
                                warn!(
                                    current_backend = %current_backend,
                                    "decode failed on current backend; trying software decoder"
                                );
                            }
                            match H264Decoder::new(next_cfg) {
                                Ok(new_decoder) => {
                                    *decoder = new_decoder;
                                    decode_recover_stage = decode_recover_stage.saturating_add(1);
                                    waiting_recover_keyframe = true;
                                    no_output_streak = 0;
                                    last_recover_enter = std::time::Instant::now();
                                    last_recover_keyframe_req = std::time::Instant::now()
                                        .checked_sub(Duration::from_secs(1))
                                        .unwrap_or_else(std::time::Instant::now);
                                    if let Ok(mut ov) = overlay_stats_for_decode.lock() {
                                        ov.decoder_backend = decoder.backend_name().to_string();
                                    }
                                    continue;
                                }
                                Err(recover_err) => {
                                    warn!(error = %recover_err, "decoder recovery attempt failed");
                                    decode_recover_stage = decode_recover_stage.saturating_add(1);
                                }
                            }
                        }
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
                    let current_backend = {
                        let d = decoder_clone.lock().await;
                        d.backend_name().to_string()
                    };
                    info!(
                        backend = %current_backend,
                        decode_select = ?decode_select_policy,
                        fps = format!("{:.2}", fps),
                        avg_decode_ms = format!("{:.3}", avg_decode),
                        p95_decode_ms = format!("{:.3}", p95),
                        jitter_ms = format!("{:.3}", jitter),
                        e2e_avg_ms = format!("{:.3}", e2e_avg),
                        e2e_p50_ms = format!("{:.3}", e2e_p50),
                        e2e_p95_ms = format!("{:.3}", e2e_p95),
                        e2e_p99_ms = format!("{:.3}", e2e_p99),
                        decode_drop_old_total = dropped_old_frames,
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
                    let rec = recorder_for_decode.snapshot();
                    info!(
                        in_frames = rec.in_frames,
                        written_frames = rec.written_frames,
                        dropped_frames = rec.dropped_frames,
                        write_failures = rec.write_failures,
                        bytes_written = rec.bytes_written,
                        avg_write_us = format!("{:.1}", rec.avg_write_us),
                        "[RECORD-STATS]"
                    );
                    last_stats_at = std::time::Instant::now();
                }
                // TODO: 解码并渲染视频帧
            } else {
                drop(rx);
                if !disable_decode_recover
                    && has_received_frame
                    && last_frame_rx_at.elapsed() >= rx_stall_recover_after
                {
                    if !rx_stall_active {
                        rx_stall_active = true;
                        waiting_recover_keyframe = true;
                        waiting_probe_budget = 0;
                        no_output_streak = 0;
                        waiting_recover_since = Some(std::time::Instant::now());
                        waiting_recover_pli_requests = 0;
                        warn!(
                            stall_ms = last_frame_rx_at.elapsed().as_millis() as u64,
                            "video frame intake stalled; entering keyframe resync"
                        );
                    }
                    if !rx_stall_flush_issued {
                        let mut decoder = decoder_clone.lock().await;
                        let _ = decoder.flush();
                        rx_stall_flush_issued = true;
                    }
                    if last_rx_stall_pli_req.elapsed() >= rx_stall_pli_interval {
                        let manager_guard = peer_manager_for_decode.read().await;
                        if let Some(ref mgr) = *manager_guard {
                            let _ = mgr.request_keyframe().await;
                            waiting_recover_pli_requests =
                                waiting_recover_pli_requests.saturating_add(1);
                        }
                        last_rx_stall_pli_req = std::time::Instant::now();
                    }
                }
                continue;
            }
            }
        });
    })
    .context("failed to spawn decode thread")?;

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
    peer_manager: &Arc<RwLock<Option<Arc<PeerConnectionManager>>>>,
    current_agent_id: &Arc<Mutex<Option<String>>>,
    frame_receiver: &Arc<Mutex<Option<FrameReceiver>>>,
) -> Result<()> {
    // 创建 PeerConnection
    let (manager, frame_rx) = PeerConnectionManager::create(
        target_device_id.to_string(),
        PeerConfig::default(),
        signaling.clone(),
    )
    .await?;

    // 创建 Offer
    let offer = manager
        .pc
        .create_offer(None)
        .await
        .context("failed to create offer")?;

    // 设置本地描述（这会触发 ICE 收集）
    manager
        .pc
        .set_local_description(offer.clone())
        .await
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
                info!(
                    "ICE gathering state changed: {:?} -> {:?}",
                    last_state, current_state
                );
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
    let offer_to_send = manager
        .pc
        .local_description()
        .await
        .context("no local description")?;

    // 打印 SDP 用于调试
    match offer_to_send.unmarshal() {
        Ok(sdp) => {
            let sdp_str = sdp.to_string();
            let has_ice = sdp_str.contains("ice-ufrag");
            info!("SDP contains ice-ufrag: {}", has_ice);
            if !has_ice {
                warn!(
                    "SDP does NOT contain ice-ufrag! First 500 chars: {}",
                    sdp_str.chars().take(500).collect::<String>()
                );
            }
        }
        Err(e) => {
            warn!("Failed to unmarshal SDP: {}", e);
        }
    }

    // 发送 Offer
    let _controller_id = signaling
        .device_id()
        .await
        .context("controller not registered")?;
    signaling
        .send_offer(
            target_device_id,
            &offer_to_send,
            &Uuid::new_v4().to_string(),
        )
        .await
        .context("failed to send offer")?;

    info!(target = %target_device_id, "WebRTC offer sent");

    // 保存 manager 和 frame receiver
    *peer_manager.write().await = Some(Arc::new(manager));
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
        let (picked, dropped) =
            select_frame_for_decode(&mut rx, first, DecodeSelectPolicy::Ordered);
        assert_eq!(picked.sequence, 1);
        assert_eq!(dropped, 0);
    }

    #[tokio::test]
    async fn select_frame_for_decode_latest_key_prefers_new_keyframe() {
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
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
            timestamp: 3,
            is_keyframe: true,
            sequence: 3,
            tx_unix_us: 0,
        })
        .await
        .unwrap();
        let first = webrtc::peer::VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x61]),
            timestamp: 1,
            is_keyframe: false,
            sequence: 1,
            tx_unix_us: 0,
        };
        let (picked, dropped) =
            select_frame_for_decode(&mut rx, first, DecodeSelectPolicy::LatestKeyframe);
        assert_eq!(picked.sequence, 3);
        assert_eq!(dropped, 2);
    }

    #[tokio::test]
    async fn select_frame_for_decode_adaptive_age_without_timestamp_keeps_first() {
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
        let first = webrtc::peer::VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
            timestamp: 1,
            is_keyframe: true,
            sequence: 1,
            tx_unix_us: 0,
        };
        let (picked, dropped) =
            select_frame_for_decode(&mut rx, first, DecodeSelectPolicy::AdaptiveAge);
        assert_eq!(picked.sequence, 1);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn decode_select_latest_is_safened_by_default() {
        let policy = DecodeSelectPolicy::from_env_value("latest", false);
        assert_eq!(policy, DecodeSelectPolicy::LatestKeyframe);
    }

    #[test]
    fn decode_select_latest_can_be_enabled_explicitly() {
        let policy = DecodeSelectPolicy::from_env_value("latest", true);
        assert_eq!(policy, DecodeSelectPolicy::Latest);
    }

    #[test]
    fn control_bench_config_clamps_values() {
        let cfg = ControlBenchConfig::from_values(Some("true"), Some(9_999), Some(10), Some(0));
        assert!(cfg.enabled);
        assert_eq!(cfg.rate_hz, 2_000);
        assert_eq!(cfg.log_interval_ms, 200);
        assert_eq!(cfg.amplitude_px, 1);
    }

    #[test]
    fn percentile_u64_returns_expected_bucket() {
        let mut v = VecDeque::new();
        v.push_back(1);
        v.push_back(2);
        v.push_back(10);
        v.push_back(20);
        v.push_back(30);
        let p95 = percentile_u64(&v, 95).unwrap();
        assert_eq!(p95, 20.0);
    }
}
