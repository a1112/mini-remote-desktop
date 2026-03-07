use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use mrd_proto::SessionId;
use mrd_signal_proto::{IceCandidate, SessionDescription};
use crate::webrtc_media::H264AccessUnitAssembler;
use mrd_decode::{DecodedFrame, PixelFormat, VideoDecoder};
use webrtc::{
    api::{media_engine::MediaEngine, setting_engine::SettingEngine, APIBuilder},
    data_channel::data_channel_init::RTCDataChannelInit,
    ice_transport::{ice_candidate::RTCIceCandidateInit, ice_server::RTCIceServer},
    peer_connection::{
        configuration::RTCConfiguration,
        sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
    rtp_transceiver::{rtp_codec::RTPCodecType, RTCRtpTransceiverInit},
    track::track_remote::TrackRemote,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebrtcHostSnapshot {
    pub local_offer: Option<String>,
    pub remote_offer: Option<String>,
    pub local_answer: Option<String>,
    pub remote_answer: Option<String>,
    pub remote_ice_count: usize,
    pub remote_video_track_count: usize,
    pub remote_rtp_packet_count: u64,
    pub last_remote_codec: Option<String>,
    pub remote_h264_access_unit_count: u64,
    pub last_remote_access_unit_bytes: usize,
    pub decoded_frame_count: u64,
    pub last_decoded_width: usize,
    pub last_decoded_height: usize,
    pub last_decoded_pixel_format: Option<String>,
}

#[derive(Debug)]
struct HostedPeer {
    pc: Arc<RTCPeerConnection>,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
}

#[derive(Debug, Default)]
pub struct WebrtcHost {
    sessions: HashMap<SessionId, HostedPeer>,
}

impl WebrtcHost {
    pub async fn create_offer(
        &mut self,
        session_id: SessionId,
    ) -> Result<SessionDescription, String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let offer = pc
            .create_offer(None)
            .await
            .map_err(|e| format!("创建 WebRTC offer 失败: {}", e))?;
        pc.set_local_description(offer.clone())
            .await
            .map_err(|e| format!("设置本地 offer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .local_offer = Some(offer.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: offer.sdp,
        })
    }

    pub async fn apply_remote_offer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let description = RTCSessionDescription::offer(sdp.clone())
            .map_err(|e| format!("构造远端 offer 失败: {}", e))?;
        pc.set_remote_description(description)
            .await
            .map_err(|e| format!("设置远端 offer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .remote_offer = Some(sdp);
        Ok(())
    }

    pub async fn create_answer(
        &mut self,
        session_id: SessionId,
    ) -> Result<SessionDescription, String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let answer = pc
            .create_answer(None)
            .await
            .map_err(|e| format!("创建 WebRTC answer 失败: {}", e))?;
        pc.set_local_description(answer.clone())
            .await
            .map_err(|e| format!("设置本地 answer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .local_answer = Some(answer.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: answer.sdp,
        })
    }

    pub async fn apply_remote_answer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let description = RTCSessionDescription::answer(sdp.clone())
            .map_err(|e| format!("构造远端 answer 失败: {}", e))?;
        pc.set_remote_description(description)
            .await
            .map_err(|e| format!("设置远端 answer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .remote_answer = Some(sdp);
        Ok(())
    }

    pub async fn apply_remote_ice_candidate(
        &mut self,
        session_id: SessionId,
        candidate: IceCandidate,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let init = RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.sdp_mline_index,
            username_fragment: None,
        };
        pc.add_ice_candidate(init)
            .await
            .map_err(|e| format!("添加远端 ICE 候选失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .remote_ice_count += 1;
        Ok(())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Option<WebrtcHostSnapshot> {
        self.sessions.get(session_id).map(|peer| {
            peer.snapshot
                .lock()
                .expect("lock host snapshot")
                .clone()
        })
    }

    async fn get_or_create_peer(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Arc<RTCPeerConnection>, String> {
        if let Some(peer) = self.sessions.get(session_id) {
            return Ok(peer.pc.clone());
        }

        let snapshot = Arc::new(Mutex::new(WebrtcHostSnapshot::default()));
        let pc = build_peer_connection(snapshot.clone()).await?;
        self.sessions.insert(
            session_id.clone(),
            HostedPeer {
                pc: pc.clone(),
                snapshot,
            },
        );
        Ok(pc)
    }
}

async fn build_peer_connection(
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
) -> Result<Arc<RTCPeerConnection>, String> {
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|e| format!("注册默认编解码器失败: {}", e))?;

    let mut setting_engine = SettingEngine::default();
    setting_engine.set_include_loopback_candidate(true);

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_setting_engine(setting_engine)
        .build();

    let pc = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .map_err(|e| format!("创建 PeerConnection 失败: {}", e))?,
    );

    pc.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit {
            direction: webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection::Recvonly,
            send_encodings: vec![],
        }),
    )
    .await
    .map_err(|e| format!("注册视频接收 transceiver 失败: {}", e))?;

    pc.create_data_channel(
        "control",
        Some(RTCDataChannelInit {
            ordered: Some(false),
            max_retransmits: Some(0),
            ..Default::default()
        }),
    )
    .await
    .map_err(|e| format!("创建 control data channel 失败: {}", e))?;

    let packet_counter = Arc::new(AtomicU64::new(0));
    let access_unit_counter = Arc::new(AtomicU64::new(0));
    let on_track_snapshot = snapshot.clone();
    let on_track_counter = packet_counter.clone();
    let on_track_access_unit_counter = access_unit_counter.clone();
    pc.on_track(Box::new(move |track: Arc<TrackRemote>, _, _| {
        let snapshot = on_track_snapshot.clone();
        let counter = on_track_counter.clone();
        let access_unit_counter = on_track_access_unit_counter.clone();
        Box::pin(async move {
            let mime_type = track.codec().capability.mime_type.clone();
            {
                let mut snapshot = snapshot.lock().expect("lock host snapshot");
                snapshot.remote_video_track_count += 1;
                snapshot.last_remote_codec = Some(mime_type.clone());
            }

            let mut h264_assembler = if mime_type.eq_ignore_ascii_case("video/h264") {
                Some(H264AccessUnitAssembler::default())
            } else {
                None
            };
            let mut decoder = if mime_type.eq_ignore_ascii_case("video/h264") {
                match mrd_decode::create_decoder("h264_software") {
                    Ok(decoder) => Some(decoder),
                    Err(_) => None,
                }
            } else {
                None
            };

            while let Ok((_packet, _)) = track.read_rtp().await {
                let packet_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                let next_access_unit = h264_assembler.as_mut().and_then(|assembler| {
                    assembler.push_rtp_payload(&_packet.payload, _packet.header.marker)
                });
                let mut snapshot_guard = snapshot.lock().expect("lock host snapshot");
                snapshot_guard.remote_rtp_packet_count = packet_count;
                if let Some(access_unit) = next_access_unit {
                    let access_unit_count = access_unit_counter.fetch_add(1, Ordering::Relaxed) + 1;
                    snapshot_guard.remote_h264_access_unit_count = access_unit_count;
                    snapshot_guard.last_remote_access_unit_bytes = access_unit.len();
                    drop(snapshot_guard);
                    if let Some(decoder) = decoder.as_mut() {
                        let _ =
                            decode_access_unit_into_snapshot(snapshot.clone(), decoder.as_mut(), &access_unit);
                    }
                    continue;
                }
            }
        })
    }));

    Ok(pc)
}

fn decode_access_unit_into_snapshot(
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    decoder: &mut dyn VideoDecoder,
    access_unit: &[u8],
) -> Result<(), String> {
    decoder
        .push_access_unit(access_unit)
        .map_err(|e| format!("software decoder 解码 access unit 失败: {e}"))?;
    let frames = decoder.drain_decoded_frames();
    apply_decoded_frames_to_snapshot(snapshot, frames);
    Ok(())
}

fn apply_decoded_frames_to_snapshot(
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    frames: Vec<DecodedFrame>,
) {
    if frames.is_empty() {
        return;
    }

    let mut snapshot = snapshot.lock().expect("lock host snapshot");
    for frame in frames {
        snapshot.decoded_frame_count += 1;
        snapshot.last_decoded_width = frame.width;
        snapshot.last_decoded_height = frame.height;
        snapshot.last_decoded_pixel_format = Some(match frame.pixel_format {
            PixelFormat::Rgb24 => "Rgb24".to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mrd_proto::SessionId;
    use openh264::{
        encoder::Encoder,
        formats::{RgbSliceU8, YUVBuffer},
    };

    use super::{decode_access_unit_into_snapshot, WebrtcHost, WebrtcHostSnapshot};
    use crate::webrtc_media::H264AccessUnitAssembler;

    #[tokio::test]
    async fn creating_offer_records_local_offer() {
        let mut host = WebrtcHost::default();

        let offer = host
            .create_offer(SessionId("session-1".into()))
            .await
            .expect("create offer");

        assert!(offer.sdp.contains("m=application"));
        let snapshot = host
            .snapshot(&SessionId("session-1".into()))
            .expect("host snapshot");
        assert!(snapshot.local_offer.as_deref().unwrap_or_default().contains("m=application"));
        assert_eq!(snapshot.remote_video_track_count, 0);
        assert_eq!(snapshot.remote_h264_access_unit_count, 0);
        assert_eq!(snapshot.last_remote_access_unit_bytes, 0);
        assert_eq!(snapshot.decoded_frame_count, 0);
        assert_eq!(snapshot.last_decoded_width, 0);
        assert_eq!(snapshot.last_decoded_height, 0);
        assert_eq!(snapshot.last_decoded_pixel_format, None);
    }

    #[tokio::test]
    async fn offer_answer_roundtrip_between_two_hosts() {
        let mut controller = WebrtcHost::default();
        let mut agent = WebrtcHost::default();
        let session_id = SessionId("session-2".into());

        let offer = controller
            .create_offer(session_id.clone())
            .await
            .expect("controller offer");
        agent
            .apply_remote_offer(session_id.clone(), offer.sdp)
            .await
            .expect("agent apply offer");

        let answer = agent
            .create_answer(session_id.clone())
            .await
            .expect("agent answer");
        controller
            .apply_remote_answer(session_id.clone(), answer.sdp)
            .await
            .expect("controller apply answer");

        let controller_snapshot = controller
            .snapshot(&session_id)
            .expect("controller snapshot");
        let agent_snapshot = agent.snapshot(&session_id).expect("agent snapshot");

        assert!(controller_snapshot.local_offer.is_some());
        assert!(controller_snapshot.remote_answer.is_some());
        assert!(agent_snapshot.remote_offer.is_some());
        assert!(agent_snapshot.local_answer.is_some());
    }

    #[test]
    fn decoding_access_unit_updates_snapshot_statistics() {
        let snapshot = Arc::new(Mutex::new(WebrtcHostSnapshot::default()));
        let mut rgb = Vec::with_capacity(16 * 16 * 3);
        for y in 0..16 {
            for x in 0..16 {
                rgb.push((x * 16) as u8);
                rgb.push((y * 16) as u8);
                rgb.push(96);
            }
        }
        let rgb_source = RgbSliceU8::new(&rgb, (16, 16));
        let yuv = YUVBuffer::from_rgb_source(rgb_source);
        let mut encoder = Encoder::new().expect("openh264 encoder");
        let access_unit = encoder.encode(&yuv).expect("encode access unit").to_vec();

        let mut decoder = mrd_decode::create_decoder("h264_software").expect("decoder instance");
        decode_access_unit_into_snapshot(snapshot.clone(), decoder.as_mut(), access_unit.as_slice())
            .expect("decode access unit into snapshot");

        let snapshot = snapshot.lock().expect("lock host snapshot").clone();
        assert_eq!(snapshot.decoded_frame_count, 1);
        assert_eq!(snapshot.last_decoded_width, 16);
        assert_eq!(snapshot.last_decoded_height, 16);
        assert_eq!(snapshot.last_decoded_pixel_format.as_deref(), Some("Rgb24"));
    }

    #[test]
    fn h264_access_unit_assembler_reconstructs_fua_payloads() {
        let mut assembler = H264AccessUnitAssembler::default();

        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x85, 0xaa, 0xbb], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x45, 0xcc, 0xdd], true),
            Some(vec![0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc, 0xdd])
        );
    }
}
