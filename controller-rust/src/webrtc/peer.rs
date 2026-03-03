use super::super::signaling::SignalingClient;
use anyhow::{Context, Result};
use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use rtp::packetizer::Depacketizer;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info};
use uuid::Uuid;
use webrtc::{
    api::APIBuilder,
    api::media_engine::MediaEngine,
    ice_transport::ice_candidate::RTCIceCandidateInit,
    ice_transport::ice_connection_state::RTCIceConnectionState,
    peer_connection::configuration::RTCConfiguration,
    peer_connection::sdp::session_description::RTCSessionDescription,
    rtp_transceiver::rtp_codec::RTPCodecType,
    track::track_remote::TrackRemote,
};

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
    /// 发送端 Unix 微秒时间戳（仅 QUIC 路径有效，WebRTC 为 0）
    pub tx_unix_us: u64,
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
        let (frame_tx, frame_rx) = mpsc::channel(32);
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
        let mut access_unit = Vec::<u8>::new();
        let mut depacketizer = H264Packet::default();
        depacketizer.is_avc = false; // output AnnexB for decoder input
        let mut current_timestamp: Option<u32> = None;

        info!(
            codec = %track.codec().capability.mime_type,
            "starting to read video track"
        );

        // 读取 RTP 包并进行 H264 depacketize -> AnnexB AU 重组。
        while let Ok((packet, _)) = track.read_rtp().await {
            packet_count += 1;
            let timestamp = packet.header.timestamp;
            if current_timestamp.is_none() {
                current_timestamp = Some(timestamp);
            }

            // 若时间戳跳变但上一帧未靠 marker 结束，兜底刷新一次。
            if Some(timestamp) != current_timestamp && !access_unit.is_empty() {
                let ts = current_timestamp.unwrap_or(timestamp);
                let is_key = contains_idr_annexb(&access_unit);
                let frame = VideoFrame {
                    data: Bytes::from(std::mem::take(&mut access_unit)),
                    timestamp: ts as u64,
                    is_keyframe: is_key,
                    sequence: packet.header.sequence_number as u64,
                    tx_unix_us: 0,
                };
                if let Err(_e) = frame_tx.try_send(frame) {
                    debug!("frame channel full/closed, dropping fallback AU");
                }
                current_timestamp = Some(timestamp);
            }

            let dep = depacketizer.depacketize(&packet.payload);
            let nalu = match dep {
                Ok(v) => v,
                Err(e) => {
                    debug!(error = %e, "depacketize failed, dropping RTP payload");
                    continue;
                }
            };
            if !nalu.is_empty() {
                access_unit.extend_from_slice(&nalu);
            }

            if packet.header.marker && !access_unit.is_empty() {
                let ts = current_timestamp.unwrap_or(timestamp);
                let is_key = contains_idr_annexb(&access_unit);
                let frame = VideoFrame {
                    data: Bytes::from(std::mem::take(&mut access_unit)),
                    timestamp: ts as u64,
                    is_keyframe: is_key,
                    sequence: packet.header.sequence_number as u64,
                    tx_unix_us: 0,
                };
                if let Err(_e) = frame_tx.try_send(frame) {
                    debug!("frame channel full/closed, dropping decoded AU");
                }
                current_timestamp = None;
            }

            // 每 1000 个包记录一次
            if packet_count % 1000 == 0 {
                debug!(
                    packets = packet_count,
                    au_buffer_size = access_unit.len(),
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

fn contains_idr_annexb(buf: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 4 < buf.len() {
        let sc_len = if i + 3 < buf.len()
            && buf[i] == 0
            && buf[i + 1] == 0
            && buf[i + 2] == 1
        {
            3
        } else if i + 4 < buf.len()
            && buf[i] == 0
            && buf[i + 1] == 0
            && buf[i + 2] == 0
            && buf[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };
        let hdr = i + sc_len;
        if hdr < buf.len() {
            let nal_type = buf[hdr] & 0x1F;
            if nal_type == 5 {
                return true;
            }
        }
        i = hdr.saturating_add(1);
    }
    false
}
