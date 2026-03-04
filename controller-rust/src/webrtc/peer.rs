use super::super::signaling::SignalingClient;
use anyhow::{Context, Result};
use bytes::Bytes;
use common_control_proto::{ChannelClass, ControlEvent, Frame};
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtp::codecs::h264::H264Packet;
use rtp::packetizer::Depacketizer;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use webrtc::{
    api::APIBuilder,
    api::media_engine::MediaEngine,
    api::setting_engine::SettingEngine,
    data_channel::data_channel_init::RTCDataChannelInit,
    data_channel::RTCDataChannel,
    ice_transport::ice_candidate::RTCIceCandidateInit,
    ice_transport::ice_connection_state::RTCIceConnectionState,
    peer_connection::configuration::RTCConfiguration,
    peer_connection::sdp::session_description::RTCSessionDescription,
    rtp_transceiver::rtp_codec::{RTCRtpHeaderExtensionCapability, RTPCodecType},
    track::track_remote::TrackRemote,
};

const TX_UNIX_US_EXT_URI: &str = "urn:mini-remote-desktop:tx-unix-us";

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
    ctrl_rt_dc: Arc<RTCDataChannel>,
    ctrl_rel_dc: Arc<RTCDataChannel>,
    seq_rt: AtomicU32,
    seq_rel: AtomicU32,
    video_ssrc: Arc<AtomicU32>,
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
        m.register_header_extension(
            RTCRtpHeaderExtensionCapability {
                uri: TX_UNIX_US_EXT_URI.to_string(),
            },
            RTPCodecType::Video,
            None,
        )?;

        let mut se = SettingEngine::default();
        se.set_include_loopback_candidate(true);
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_setting_engine(se)
            .build();

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
        let (frame_tx, frame_rx) = mpsc::channel(128);
        let frame_rx = Arc::new(Mutex::new(frame_rx));
        let (ice_candidate_tx, _ice_candidate_rx) = mpsc::channel(10);
        let video_ssrc = Arc::new(AtomicU32::new(0));

        let ctrl_rt_dc = pc
            .create_data_channel(
                "ctrl_rt",
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(0),
                    ..Default::default()
                }),
            )
            .await
            .context("failed to create ctrl_rt data channel")?;
        ctrl_rt_dc.on_open(Box::new(move || {
            Box::pin(async move {
                info!("control data channel ctrl_rt open");
            })
        }));

        let ctrl_rel_dc = pc
            .create_data_channel(
                "ctrl_rel",
                Some(RTCDataChannelInit {
                    ordered: Some(true),
                    ..Default::default()
                }),
            )
            .await
            .context("failed to create ctrl_rel data channel")?;
        ctrl_rel_dc.on_open(Box::new(move || {
            Box::pin(async move {
                info!("control data channel ctrl_rel open");
            })
        }));

        // 设置 track 接收回调
        let frame_tx_clone = frame_tx.clone();
        let video_ssrc_for_track = video_ssrc.clone();
        pc.on_track(Box::new(move |track, _, _| {
            info!(
                "received track: codec={}, kind={}, id={}",
                track.codec().capability.mime_type,
                track.kind(),
                track.id()
            );

            let frame_tx = frame_tx_clone.clone();
            let video_ssrc = video_ssrc_for_track.clone();
            Box::pin(async move {
                if let Err(e) = Self::handle_video_track(track, frame_tx, video_ssrc).await {
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
                ctrl_rt_dc,
                ctrl_rel_dc,
                seq_rt: AtomicU32::new(0),
                seq_rel: AtomicU32::new(0),
                video_ssrc,
            },
            frame_rx,
        ))
    }

    pub async fn send_control_event(
        &self,
        event: ControlEvent,
        flags: u8,
        ts_us: u64,
    ) -> Result<()> {
        let (seq, dc) = match event.channel_class() {
            ChannelClass::Realtime => (
                self.seq_rt.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
                &self.ctrl_rt_dc,
            ),
            ChannelClass::Reliable => (
                self.seq_rel.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
                &self.ctrl_rel_dc,
            ),
        };
        let frame = Frame {
            flags,
            seq,
            ts_us,
            event,
        };
        let bytes = Bytes::from(frame.encode());
        dc.send(&bytes)
            .await
            .context("failed to send control frame")?;
        Ok(())
    }

    /// 处理视频 track，读取 RTP 包并组装 H.264 帧
    async fn handle_video_track(
        track: Arc<TrackRemote>,
        frame_tx: mpsc::Sender<VideoFrame>,
        video_ssrc: Arc<AtomicU32>,
    ) -> Result<()> {
        let mut packet_count = 0u64;
        let mut access_unit = Vec::<u8>::new();
        let mut depacketizer = H264Packet::default();
        depacketizer.is_avc = false; // output AnnexB for decoder input
        let mut current_timestamp: Option<u32> = None;
        let mut au_debug_count = 0u32;

        info!(
            codec = %track.codec().capability.mime_type,
            "starting to read video track"
        );
        let tx_ext_id = track
            .params()
            .header_extensions
            .iter()
            .find(|e| e.uri == TX_UNIX_US_EXT_URI)
            .map(|e| e.id as u8);
        info!(tx_ext_id = ?tx_ext_id, "webrtc tx-unix-us header extension mapping");

        // 读取 RTP 包并进行 H264 depacketize -> AnnexB AU 重组。
        loop {
            let (packet, _) = match track.read_rtp().await {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, total_packets = packet_count, "video track read_rtp ended");
                    break;
                }
            };
            packet_count += 1;
            let current_ssrc = packet.header.ssrc;
            if video_ssrc.load(Ordering::Relaxed) != current_ssrc {
                video_ssrc.store(current_ssrc, Ordering::Relaxed);
            }
            let timestamp = packet.header.timestamp;
            let tx_unix_us = tx_ext_id
                .and_then(|id| packet.header.get_extension(id))
                .and_then(|v| parse_tx_unix_us_extension(&v))
                .unwrap_or(0);
            if current_timestamp.is_none() {
                current_timestamp = Some(timestamp);
            }

            // Finalize AU on RTP timestamp change.
            // Some senders may set marker on each NAL (e.g. SPS/PPS/IDR), which would split
            // a single access unit if we flushed on marker.
            if Some(timestamp) != current_timestamp && !access_unit.is_empty() {
                let ts = current_timestamp.unwrap_or(timestamp);
                let is_key = contains_idr_annexb(&access_unit);
                if au_debug_count < 8 {
                    let has_sc3 = access_unit.starts_with(&[0, 0, 1]);
                    let has_sc4 = access_unit.starts_with(&[0, 0, 0, 1]);
                    info!(
                        au_idx = au_debug_count,
                        au_len = access_unit.len(),
                        seq = packet.header.sequence_number,
                        ts = ts,
                        key = is_key,
                        startcode3 = has_sc3,
                        startcode4 = has_sc4,
                        head = %hex_head(&access_unit, 20),
                        "webrtc h264 access unit"
                    );
                    au_debug_count = au_debug_count.saturating_add(1);
                }
                let frame = VideoFrame {
                    data: Bytes::from(std::mem::take(&mut access_unit)),
                    timestamp: ts as u64,
                    is_keyframe: is_key,
                    sequence: packet.header.sequence_number as u64,
                    tx_unix_us,
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

            if packet.header.marker {
                debug!("marker observed; waiting for timestamp change to finalize AU");
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

        if !access_unit.is_empty() {
            let ts = current_timestamp.unwrap_or(0);
            let is_key = contains_idr_annexb(&access_unit);
            let frame = VideoFrame {
                data: Bytes::from(std::mem::take(&mut access_unit)),
                timestamp: ts as u64,
                is_keyframe: is_key,
                sequence: packet_count,
                tx_unix_us: 0,
            };
            if let Err(_e) = frame_tx.try_send(frame) {
                debug!("frame channel full/closed, dropping trailing AU");
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
        let media_ssrc = self.video_ssrc.load(Ordering::Relaxed);
        if media_ssrc == 0 {
            return Ok(());
        }
        self.pc
            .write_rtcp(&[Box::new(PictureLossIndication {
                sender_ssrc: 0,
                media_ssrc,
            })])
            .await
            .context("failed to send RTCP PLI")?;
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

fn parse_tx_unix_us_extension(payload: &[u8]) -> Option<u64> {
    if payload.len() != 8 {
        return None;
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(payload);
    Some(u64::from_be_bytes(b))
}

fn hex_head(buf: &[u8], max_bytes: usize) -> String {
    let n = buf.len().min(max_bytes);
    let mut out = String::with_capacity(n.saturating_mul(3));
    for (i, b) in buf.iter().take(n).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}
