use super::super::signaling::SignalingClient;
use anyhow::{Context, Result};
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use webrtc::{
    api::APIBuilder,
    api::media_engine::{MediaEngine, MIME_TYPE_H264},
    ice_transport::ice_candidate::RTCIceCandidateInit,
    ice_transport::ice_connection_state::RTCIceConnectionState,
    peer_connection::configuration::RTCConfiguration,
    peer_connection::sdp::session_description::RTCSessionDescription,
    rtp_transceiver::rtp_codec::RTPCodecType,
    track::track_remote::TrackRemote,
};
use webrtc::util::Unmarshal;

/// 视频帧数据
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// H.264 编码数据
    pub data: Bytes,
    /// 时间戳
    pub timestamp: u64,
    /// 是否为关键帧
    pub is_keyframe: bool,
    /// 序列号
    pub sequence: u64,
}

/// PeerConnection 管理器配置
#[derive(Debug, Clone)]
pub struct PeerConfig {
    pub ice_servers: Vec<String>,
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            ice_servers: vec!["stun:stun.l.google.com:19302".to_string()],
        }
    }
}

/// PeerConnection 管理器
pub struct PeerConnectionManager {
    /// PeerConnection (pub for access from main)
    pub pc: Arc<webrtc::peer_connection::RTCPeerConnection>,
    _session_id: String,
    ice_candidate_tx: mpsc::Sender<RTCIceCandidateInit>,
    /// 视频帧接收器（使用 Arc<Mutex<>> 以便共享）
    frame_rx: Arc<Mutex<mpsc::Receiver<VideoFrame>>>,
}

impl PeerConnectionManager {
    /// 创建新的 PeerConnection
    pub async fn create(
        target_device_id: String,
        config: PeerConfig,
        _signaling: Arc<SignalingClient>,
    ) -> Result<(Self, Arc<Mutex<mpsc::Receiver<VideoFrame>>>)> {
        let session_id = Uuid::new_v4().to_string();

        // 设置媒体引擎
        let mut m = MediaEngine::default();
        // 注册 H.264 编解码器
        m.register_default_codecs()?;

        let api = APIBuilder::new().with_media_engine(m).build();

        // 创建 ICE 服务器配置
        let ice_servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer> = config
            .ice_servers
            .iter()
            .map(|url| webrtc::ice_transport::ice_server::RTCIceServer {
                urls: vec![url.clone()],
                ..Default::default()
            })
            .collect();

        // 创建 PeerConnection
        let pc = Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_servers,
                ..Default::default()
            })
            .await
            .context("failed to create peer connection")?,
        );

        // 添加接收视频的 transceiver（recvonly）
        let _transceiver = pc
            .add_transceiver_from_kind(RTPCodecType::Video, None)
            .await
            .context("failed to add video transceiver")?;

        info!(target = %target_device_id, "video transceiver added for receiving");

        // 创建视频帧通道
        let (frame_tx, frame_rx) = mpsc::channel(30);
        let frame_rx = Arc::new(Mutex::new(frame_rx));
        let (ice_candidate_tx, _ice_candidate_rx) = mpsc::channel(10);

        // 设置 track 接收回调
        let frame_tx_clone = frame_tx.clone();
        pc.on_track(Box::new(move |track, _, _| {
            info!(
                "received track: codec={}, kind={}, id={}",
                track.codec().capability.mime_type,
                track.kind(),
                track.id()
            );

            let frame_tx = frame_tx_clone.clone();
            Box::pin(async move {
                if let Err(e) = Self::handle_video_track(track, frame_tx).await {
                    error!(error = %e, "error handling video track");
                }
            })
        }));

        // 设置 ICE 候选回调
        // TODO: 实现 ICE 候选发送逻辑

        // 设置连接状态回调
        pc.on_peer_connection_state_change(Box::new(|s| {
            info!(state = %s, "peer connection state changed");
            Box::pin(async {})
        }));

        pc.on_ice_connection_state_change(Box::new(|s| {
            info!(state = %s, "ICE connection state changed");
            if s == RTCIceConnectionState::Failed || s == RTCIceConnectionState::Disconnected {
                error!("ICE connection failed/disconnected");
            }
            Box::pin(async {})
        }));

        info!(target = %target_device_id, "peer connection created");

        Ok((
            Self {
                pc,
                _session_id: session_id,
                ice_candidate_tx,
                frame_rx: frame_rx.clone(),
            },
            frame_rx,
        ))
    }

    /// 处理视频 track，读取 RTP 包并组装 H.264 帧
    async fn handle_video_track(
        track: Arc<TrackRemote>,
        frame_tx: mpsc::Sender<VideoFrame>,
    ) -> Result<()> {
        let mut packet_count = 0u64;
        let mut h264_buffer = Vec::new();
        let mut last_timestamp = 0u32;

        info!(
            codec = %track.codec().capability.mime_type,
            "starting to read video track"
        );

        // 读取 RTP 包 - read_rtp 返回的 packet 已经可以直接使用
        while let Ok((packet, _)) = track.read_rtp().await {
            packet_count += 1;

            let timestamp = packet.header.timestamp;

            // 检查是否是新的帧（时间戳变化）
            if timestamp != last_timestamp && last_timestamp != 0 {
                // 发送前一帧
                if !h264_buffer.is_empty() {
                    let frame = VideoFrame {
                        data: Bytes::from(h264_buffer.clone()),
                        timestamp: last_timestamp as u64,
                        is_keyframe: false, // TODO: 检测关键帧
                        sequence: packet_count,
                    };

                    if let Err(_e) = frame_tx.try_send(frame) {
                        // 通道已满或已关闭，丢弃帧
                        debug!("frame channel full/closed, dropping frame");
                    }
                    h264_buffer.clear();
                }
            }

            last_timestamp = timestamp;

            // 将 RTP payload 添加到缓冲区
            // 注意：这里需要处理 H.264 RTP payload 格式
            // 简化实现：直接复制 payload
            h264_buffer.extend_from_slice(&packet.payload);

            // 每 1000 个包记录一次
            if packet_count % 1000 == 0 {
                debug!(
                    packets = packet_count,
                    buffer_size = h264_buffer.len(),
                    "received RTP packets"
                );
            }
        }

        info!(total_packets = packet_count, "video track ended");
        Ok(())
    }

    /// 设置远程描述（Offer）
    pub async fn set_remote_description(&self, offer: RTCSessionDescription) -> Result<()> {
        self.pc
            .set_remote_description(offer)
            .await
            .context("failed to set remote description")?;
        Ok(())
    }

    /// 创建并设置 Answer
    pub async fn create_answer(&self) -> Result<RTCSessionDescription> {
        let answer = self
            .pc
            .create_answer(None)
            .await
            .context("failed to create answer")?;

        self.pc
            .set_local_description(answer.clone())
            .await
            .context("failed to set local description")?;

        Ok(answer)
    }

    /// 添加 ICE 候选
    pub async fn add_ice_candidate(&self, candidate: RTCIceCandidateInit) -> Result<()> {
        self.pc
            .add_ice_candidate(candidate)
            .await
            .context("failed to add ICE candidate")?;
        Ok(())
    }

    /// 请求关键帧
    pub async fn request_keyframe(&self) -> Result<()> {
        // TODO: 实现 PLI/FIR 请求
        Ok(())
    }
}
