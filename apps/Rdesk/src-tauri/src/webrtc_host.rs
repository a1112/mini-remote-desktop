use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use crate::frame_sink::DecodedFrameSink;
use crate::webrtc_media::H264AccessUnitAssembler;
use mrd_capture_dxgi::DxgiDesktopCapture;
use mrd_decode::{DecodedFrame, PixelFormat, VideoDecoder};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_observability::{
    MediaProbeEvent, PipelineProbeSnapshot, ProbeRegistry, ProbeSessionHandle, StageId,
};
use mrd_pipeline_core::{FrameCapture, VideoEncoder};
use mrd_proto::SessionId;
use mrd_signal_proto::{IceCandidate, SessionDescription};
use mrd_transport_webrtc::{H264Profile, H264RtpSender};
use tokio::task::JoinHandle;
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::MediaEngine,
        setting_engine::SettingEngine,
        APIBuilder,
    },
    data_channel::data_channel_init::RTCDataChannelInit,
    ice_transport::{ice_candidate::RTCIceCandidateInit, ice_server::RTCIceServer},
    ice_transport::ice_connection_state::RTCIceConnectionState,
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
    rtp_transceiver::{rtp_codec::RTPCodecType, RTCRtpTransceiverInit},
    track::track_local::TrackLocal,
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
    pub decode_error_count: u64,
    pub last_decode_error: Option<String>,
    pub available_video_source_ids: Vec<String>,
    pub local_video_track_count: usize,
    pub captured_frame_count: u64,
    pub sent_access_unit_count: u64,
    pub sent_rtp_bytes: u64,
    pub zero_write_access_unit_count: u64,
    pub sender_running: bool,
    pub peer_connection_state: Option<String>,
    pub ice_connection_state: Option<String>,
}

struct HostedPeer {
    pc: Arc<RTCPeerConnection>,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    sample_sender: Option<Arc<tokio::sync::Mutex<H264RtpSender>>>,
    sender_running: Arc<AtomicBool>,
    sender_task: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct WebrtcHost {
    sessions: HashMap<SessionId, HostedPeer>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
    probe_registry: ProbeRegistry,
}

impl WebrtcHost {
    pub fn with_frame_sink(frame_sink: Arc<Mutex<DecodedFrameSink>>) -> Self {
        Self::with_frame_sink_and_probes(frame_sink, ProbeRegistry::default())
    }

    pub fn with_frame_sink_and_probes(
        frame_sink: Arc<Mutex<DecodedFrameSink>>,
        probe_registry: ProbeRegistry,
    ) -> Self {
        Self {
            sessions: HashMap::new(),
            frame_sink: Some(frame_sink),
            probe_registry,
        }
    }

    pub async fn create_offer(
        &mut self,
        session_id: SessionId,
    ) -> Result<SessionDescription, String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let should_add_recvonly = self
            .sessions
            .get(&session_id)
            .map(|session| session.sample_sender.is_none())
            .unwrap_or(true);
        if should_add_recvonly {
            ensure_recvonly_video_transceiver(&pc).await?;
        }
        let offer = pc
            .create_offer(None)
            .await
            .map_err(|e| format!("创建 WebRTC offer 失败: {}", e))?;
        let mut gather_complete = pc.gathering_complete_promise().await;
        pc.set_local_description(offer.clone())
            .await
            .map_err(|e| format!("设置本地 offer 失败: {}", e))?;
        let _ = gather_complete.recv().await;
        let local = pc
            .local_description()
            .await
            .ok_or_else(|| format!("缺少本地 offer 描述: {}", session_id.0))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .local_offer = Some(local.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: local.sdp,
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
        let mut gather_complete = pc.gathering_complete_promise().await;
        pc.set_local_description(answer.clone())
            .await
            .map_err(|e| format!("设置本地 answer 失败: {}", e))?;
        let _ = gather_complete.recv().await;
        let local = pc
            .local_description()
            .await
            .ok_or_else(|| format!("缺少本地 answer 描述: {}", session_id.0))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .local_answer = Some(local.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: local.sdp,
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

    pub fn probe_snapshot(&self, session_id: &SessionId) -> Option<PipelineProbeSnapshot> {
        self.probe_registry
            .snapshot(session_id, crate::frame_sink::DEFAULT_SOURCE_ID)
    }

    pub fn probe_recent_events(&self, session_id: &SessionId, limit: usize) -> Vec<MediaProbeEvent> {
        self.probe_registry
            .recent_events(session_id, crate::frame_sink::DEFAULT_SOURCE_ID, limit)
    }

    pub async fn start_embedded_desktop_sender(
        &mut self,
        session_id: SessionId,
        fps: u32,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;

        if session.sender_running.load(Ordering::Relaxed) {
            return Ok(());
        }

        let sample_sender =
            ensure_sample_sender(&pc, &session_id, session, H264Profile::Baseline).await?;
        let probe = self
            .probe_registry
            .session_handle(session_id.clone(), crate::frame_sink::DEFAULT_SOURCE_ID);
        probe.set_backend("dxgi");
        probe.set_codec("h264");
        probe.set_transport("webrtc");
        session.sender_running.store(true, Ordering::Relaxed);
        {
            let mut snapshot = session.snapshot.lock().expect("lock host snapshot");
            snapshot.sender_running = true;
        }
        let running = session.sender_running.clone();
        let snapshot = session.snapshot.clone();
        let frame_interval = Duration::from_millis((1000 / fps.max(1)) as u64);
        let task = tokio::task::spawn_blocking(move || {
            let mut capture = match DxgiDesktopCapture::new_primary() {
                Ok(capture) => capture,
                Err(_) => {
                    snapshot.lock().expect("lock host snapshot").sender_running = false;
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let mut encoder = match OpenH264Encoder::new(capture.width(), capture.height(), fps) {
                Ok(encoder) => encoder,
                Err(_) => {
                    snapshot.lock().expect("lock host snapshot").sender_running = false;
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let handle = tokio::runtime::Handle::current();
            run_blocking_desktop_sender_loop(
                &mut capture,
                &mut encoder,
                frame_interval,
                sample_sender,
                snapshot,
                probe,
                running,
                handle,
            );
        });
        session.sender_task = Some(task);
        Ok(())
    }

    pub async fn stop_embedded_video_sender(&mut self, session_id: &SessionId) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.sender_running.store(false, Ordering::Relaxed);
        if let Some(task) = session.sender_task.take() {
            task.abort();
        }
        session
            .snapshot
            .lock()
            .expect("lock host snapshot")
            .sender_running = false;
        Ok(())
    }

    async fn start_sender_with_components<C, E>(
        &mut self,
        session_id: SessionId,
        capture: C,
        encoder: E,
        frame_interval: Duration,
    ) -> Result<(), String>
    where
        C: FrameCapture + Send + 'static,
        E: VideoEncoder + Send + 'static,
    {
        let pc = self.get_or_create_peer(&session_id).await?;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;

        if session.sender_running.load(Ordering::Relaxed) {
            return Ok(());
        }

        let sample_sender = if let Some(sender) = session.sample_sender.as_ref() {
            sender.clone()
        } else {
            ensure_sample_sender(&pc, &session_id, session, H264Profile::Baseline).await?
        };
        let probe = self
            .probe_registry
            .session_handle(session_id.clone(), crate::frame_sink::DEFAULT_SOURCE_ID);
        probe.set_codec("h264");
        probe.set_transport("webrtc");

        session.sender_running.store(true, Ordering::Relaxed);
        {
            let mut snapshot = session.snapshot.lock().expect("lock host snapshot");
            snapshot.sender_running = true;
        }
        let running = session.sender_running.clone();
        let snapshot = session.snapshot.clone();
        let task = tokio::spawn(run_embedded_sender_loop(
            capture,
            encoder,
            frame_interval,
            sample_sender,
            snapshot,
            probe,
            running,
        ));
        session.sender_task = Some(task);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn prepare_test_video_sender_with_backend(
        &mut self,
        session_id: SessionId,
        backend: &str,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        let profile = match backend {
            "nvenc" => H264Profile::High,
            _ => H264Profile::Baseline,
        };
        let _ = ensure_sample_sender(&pc, &session_id, session, profile).await?;
        Ok(())
    }

    async fn get_or_create_peer(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Arc<RTCPeerConnection>, String> {
        if let Some(peer) = self.sessions.get(session_id) {
            return Ok(peer.pc.clone());
        }

        let snapshot = Arc::new(Mutex::new(WebrtcHostSnapshot::default()));
        let probe = self
            .probe_registry
            .session_handle(session_id.clone(), crate::frame_sink::DEFAULT_SOURCE_ID);
        probe.set_codec("h264");
        probe.set_transport("webrtc");
        let pc = build_peer_connection(
            session_id.clone(),
            snapshot.clone(),
            self.frame_sink.clone(),
            probe,
        )
        .await?;
        self.sessions.insert(
            session_id.clone(),
            HostedPeer {
                pc: pc.clone(),
                snapshot,
                sample_sender: None,
                sender_running: Arc::new(AtomicBool::new(false)),
                sender_task: None,
            },
        );
        Ok(pc)
    }

    #[cfg(test)]
    pub(crate) async fn start_test_video_sender<C, E>(
        &mut self,
        session_id: SessionId,
        capture: C,
        encoder: E,
        frame_interval: Duration,
    ) -> Result<(), String>
    where
        C: FrameCapture + Send + 'static,
        E: VideoEncoder + Send + 'static,
    {
        self.start_sender_with_components(session_id, capture, encoder, frame_interval)
            .await
    }
}

async fn ensure_sample_sender(
    pc: &Arc<RTCPeerConnection>,
    session_id: &SessionId,
    session: &mut HostedPeer,
    profile: H264Profile,
) -> Result<Arc<tokio::sync::Mutex<H264RtpSender>>, String> {
    if let Some(sender) = session.sample_sender.as_ref() {
        return Ok(sender.clone());
    }

    let sender = Arc::new(tokio::sync::Mutex::new(H264RtpSender::new_with_profile(
        "video",
        format!("{}-embedded", session_id.0),
        30,
        1200,
        profile,
    )));
    let track: Arc<dyn TrackLocal + Send + Sync> = sender.lock().await.track();
    let rtp_sender = pc
        .add_track(track)
        .await
        .map_err(|error| format!("add local video track failed: {error}"))?;
    tokio::spawn(async move {
        while rtp_sender.read_rtcp().await.is_ok() {}
    });
    session.sample_sender = Some(sender.clone());
    session
        .snapshot
        .lock()
        .expect("lock host snapshot")
        .local_video_track_count = 1;
    Ok(sender)
}

async fn ensure_recvonly_video_transceiver(pc: &Arc<RTCPeerConnection>) -> Result<(), String> {
    pc.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit {
            direction: webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection::Recvonly,
            send_encodings: vec![],
        }),
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("注册视频接收 transceiver 失败: {}", e))
}

async fn build_peer_connection(
    session_id: SessionId,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
    probe: ProbeSessionHandle,
) -> Result<Arc<RTCPeerConnection>, String> {
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|e| format!("注册默认编解码器失败: {}", e))?;
    let mut interceptor_registry = Registry::new();
    interceptor_registry = register_default_interceptors(interceptor_registry, &mut media_engine)
        .map_err(|e| format!("注册默认 interceptor 失败: {}", e))?;

    let mut setting_engine = SettingEngine::default();
    setting_engine.set_include_loopback_candidate(true);

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(interceptor_registry)
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
    let connection_snapshot = snapshot.clone();
    pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
        let snapshot = connection_snapshot.clone();
        Box::pin(async move {
            snapshot
                .lock()
                .expect("lock host snapshot")
                .peer_connection_state = Some(state.to_string());
        })
    }));
    let ice_snapshot = snapshot.clone();
    pc.on_ice_connection_state_change(Box::new(move |state: RTCIceConnectionState| {
        let snapshot = ice_snapshot.clone();
        Box::pin(async move {
            snapshot
                .lock()
                .expect("lock host snapshot")
                .ice_connection_state = Some(state.to_string());
        })
    }));
    let on_track_snapshot = snapshot.clone();
    let on_track_session_id = session_id.clone();
    let on_track_frame_sink = frame_sink.clone();
    let on_track_probe = probe.clone();
    let on_track_counter = packet_counter.clone();
    let on_track_access_unit_counter = access_unit_counter.clone();
    pc.on_track(Box::new(move |track: Arc<TrackRemote>, _, _| {
        let snapshot = on_track_snapshot.clone();
        let session_id = on_track_session_id.clone();
        let frame_sink = on_track_frame_sink.clone();
        let probe = on_track_probe.clone();
        let counter = on_track_counter.clone();
        let access_unit_counter = on_track_access_unit_counter.clone();
        Box::pin(async move {
            let mime_type = track.codec().capability.mime_type.clone();
            let source_id = {
                let mut snapshot = snapshot.lock().expect("lock host snapshot");
                snapshot.remote_video_track_count += 1;
                snapshot.last_remote_codec = Some(mime_type.clone());
                let source_id = format!("video-track-{}", snapshot.remote_video_track_count);
                if !snapshot.available_video_source_ids.contains(&source_id) {
                    snapshot.available_video_source_ids.push(source_id.clone());
                }
                source_id
            };

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
                let ingress_started_at = std::time::Instant::now();
                let packet_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                let assemble_started_at = std::time::Instant::now();
                let next_access_unit = h264_assembler.as_mut().and_then(|assembler| {
                    assembler.push_rtp_payload(&_packet.payload, _packet.header.marker)
                });
                probe.record_stage(
                    StageId::NetworkIngress,
                    ingress_started_at.elapsed(),
                    _packet.payload.len(),
                    false,
                );
                let mut snapshot_guard = snapshot.lock().expect("lock host snapshot");
                snapshot_guard.remote_rtp_packet_count = packet_count;
                if let Some(access_unit) = next_access_unit {
                    let access_unit_count = access_unit_counter.fetch_add(1, Ordering::Relaxed) + 1;
                    snapshot_guard.remote_h264_access_unit_count = access_unit_count;
                    snapshot_guard.last_remote_access_unit_bytes = access_unit.len();
                    drop(snapshot_guard);
                    probe.record_stage(
                        StageId::H264Assemble,
                        assemble_started_at.elapsed(),
                        access_unit.len(),
                        access_unit.windows(5).any(|window| window == [0, 0, 0, 1, 0x65]),
                    );
                    if let Some(decoder) = decoder.as_mut() {
                        let _ = decode_access_unit_into_snapshot(
                            session_id.clone(),
                            source_id.clone(),
                            snapshot.clone(),
                            frame_sink.clone(),
                            probe.clone(),
                            decoder.as_mut(),
                            &access_unit,
                        );
                    }
                    continue;
                }
            }
        })
    }));

    Ok(pc)
}

fn decode_access_unit_into_snapshot(
    session_id: SessionId,
    source_id: String,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
    probe: ProbeSessionHandle,
    decoder: &mut dyn VideoDecoder,
    access_unit: &[u8],
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    if let Err(error) = decoder.push_access_unit(access_unit) {
        let message = format!("software decoder 解码 access unit 失败: {error}");
        let mut snapshot_guard = snapshot.lock().expect("lock host snapshot after decode error");
        snapshot_guard.decode_error_count += 1;
        snapshot_guard.last_decode_error = Some(message.clone());
        return Err(message);
    }
    let frames = decoder.drain_decoded_frames();
    probe.record_stage(
        StageId::DecodeTotal,
        started_at.elapsed(),
        access_unit.len(),
        access_unit.windows(5).any(|window| window == [0, 0, 0, 1, 0x65]),
    );
    apply_decoded_frames_to_snapshot(session_id, source_id, snapshot, frame_sink, probe, frames);
    Ok(())
}

fn apply_decoded_frames_to_snapshot(
    session_id: SessionId,
    source_id: String,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
    probe: ProbeSessionHandle,
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
        if let Some(frame_sink) = frame_sink.as_ref() {
            let bytes = frame.data.len();
            let started_at = std::time::Instant::now();
            frame_sink
                .lock()
                .expect("lock decoded frame sink")
                .ingest_frame_for_source(session_id.clone(), source_id.clone(), frame);
            probe.record_stage(StageId::FrameSinkIngest, started_at.elapsed(), bytes, false);
        }
    }
}

async fn run_embedded_sender_loop<C, E>(
    mut capture: C,
    mut encoder: E,
    frame_interval: Duration,
    sample_sender: Arc<tokio::sync::Mutex<H264RtpSender>>,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    probe: ProbeSessionHandle,
    running: Arc<AtomicBool>,
) where
    C: FrameCapture + Send + 'static,
    E: VideoEncoder + Send + 'static,
{
    let mut last_tick = std::time::Instant::now();
    while running.load(Ordering::Relaxed) {
        probe.record_stage(StageId::CaptureWait, last_tick.elapsed(), 0, false);
        let capture_started_at = std::time::Instant::now();
        match capture.capture_frame() {
            Ok(frame) => {
                probe.record_stage(
                    StageId::CaptureCopy,
                    capture_started_at.elapsed(),
                    frame.data.len(),
                    false,
                );
                {
                    let mut guard = snapshot.lock().expect("lock host snapshot");
                    guard.captured_frame_count += 1;
                    guard.sender_running = true;
                }
                let encode_started_at = std::time::Instant::now();
                match encoder.encode(&frame) {
                    Ok(access_units) => {
                        let total_encoded_bytes =
                            access_units.iter().map(|access_unit| access_unit.bytes.len()).sum();
                        let keyframe = access_units.iter().any(|access_unit| access_unit.is_keyframe);
                        probe.record_stage(
                            StageId::EncodeTotal,
                            encode_started_at.elapsed(),
                            total_encoded_bytes,
                            keyframe,
                        );
                        for access_unit in access_units {
                            let send_started_at = std::time::Instant::now();
                            if let Ok(written) =
                                sample_sender.lock().await.send_access_unit(&access_unit).await
                            {
                                probe.record_stage(
                                    StageId::SendPacketize,
                                    Duration::from_micros(1),
                                    access_unit.bytes.len(),
                                    access_unit.is_keyframe,
                                );
                                probe.record_stage(
                                    StageId::SendWrite,
                                    send_started_at.elapsed(),
                                    written,
                                    access_unit.is_keyframe,
                                );
                                let mut guard = snapshot.lock().expect("lock host snapshot");
                                guard.sent_access_unit_count += 1;
                                guard.sent_rtp_bytes += written as u64;
                                if written == 0 {
                                    guard.zero_write_access_unit_count += 1;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        running.store(false, Ordering::Relaxed);
                    }
                }
            }
            Err(_) => {
                running.store(false, Ordering::Relaxed);
            }
        }

        last_tick = std::time::Instant::now();
        tokio::time::sleep(frame_interval).await;
    }

    snapshot.lock().expect("lock host snapshot").sender_running = false;
}

fn run_blocking_desktop_sender_loop<C, E>(
    capture: &mut C,
    encoder: &mut E,
    frame_interval: Duration,
    sample_sender: Arc<tokio::sync::Mutex<H264RtpSender>>,
    snapshot: Arc<Mutex<WebrtcHostSnapshot>>,
    probe: ProbeSessionHandle,
    running: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
) where
    C: FrameCapture,
    E: VideoEncoder,
{
    let mut last_tick = std::time::Instant::now();
    while running.load(Ordering::Relaxed) {
        probe.record_stage(StageId::CaptureWait, last_tick.elapsed(), 0, false);
        let capture_started_at = std::time::Instant::now();
        match capture.capture_frame() {
            Ok(frame) => {
                probe.record_stage(
                    StageId::CaptureCopy,
                    capture_started_at.elapsed(),
                    frame.data.len(),
                    false,
                );
                {
                    let mut guard = snapshot.lock().expect("lock host snapshot");
                    guard.captured_frame_count += 1;
                    guard.sender_running = true;
                }
                let encode_started_at = std::time::Instant::now();
                match encoder.encode(&frame) {
                    Ok(access_units) => {
                        let total_encoded_bytes =
                            access_units.iter().map(|access_unit| access_unit.bytes.len()).sum();
                        let keyframe = access_units.iter().any(|access_unit| access_unit.is_keyframe);
                        probe.record_stage(
                            StageId::EncodeTotal,
                            encode_started_at.elapsed(),
                            total_encoded_bytes,
                            keyframe,
                        );
                        for access_unit in access_units {
                            let send_started_at = std::time::Instant::now();
                            if let Ok(written) = handle.block_on(async {
                                sample_sender.lock().await.send_access_unit(&access_unit).await
                            }) {
                                probe.record_stage(
                                    StageId::SendPacketize,
                                    Duration::from_micros(1),
                                    access_unit.bytes.len(),
                                    access_unit.is_keyframe,
                                );
                                probe.record_stage(
                                    StageId::SendWrite,
                                    send_started_at.elapsed(),
                                    written,
                                    access_unit.is_keyframe,
                                );
                                let mut guard = snapshot.lock().expect("lock host snapshot");
                                guard.sent_access_unit_count += 1;
                                guard.sent_rtp_bytes += written as u64;
                                if written == 0 {
                                    guard.zero_write_access_unit_count += 1;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        running.store(false, Ordering::Relaxed);
                    }
                }
            }
            Err(_) => {
                running.store(false, Ordering::Relaxed);
            }
        }

        last_tick = std::time::Instant::now();
        std::thread::sleep(frame_interval);
    }

    snapshot.lock().expect("lock host snapshot").sender_running = false;
}

#[cfg(test)]
mod tests {
    use std::{sync::{Arc, Mutex, Once}, time::Duration};

    use mrd_encode_nvenc::NvencH264Encoder;
    use mrd_encode_openh264::OpenH264Encoder;
    use mrd_observability::{PipelineProbeSnapshot, ProbeRegistry, StageId};
    use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
    use mrd_proto::SessionId;
    use openh264::{
        encoder::Encoder,
        formats::{RgbSliceU8, YUVBuffer},
    };

    use super::{decode_access_unit_into_snapshot, WebrtcHost, WebrtcHostSnapshot};
    use crate::frame_sink::{DecodedFrameSink, DecodedFrameSnapshot};
    use crate::webrtc_media::H264AccessUnitAssembler;

    fn ensure_rustls_crypto_provider() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    #[tokio::test]
    async fn creating_offer_records_local_offer() {
        ensure_rustls_crypto_provider();
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
        assert!(snapshot.available_video_source_ids.is_empty());
    }

    #[tokio::test]
    async fn offer_answer_roundtrip_between_two_hosts() {
        ensure_rustls_crypto_provider();
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
        decode_access_unit_into_snapshot(
            SessionId("session-3".into()),
            "video-track-1".to_string(),
            snapshot.clone(),
            None,
            ProbeRegistry::default()
                .session_handle(SessionId("session-3".into()), crate::frame_sink::DEFAULT_SOURCE_ID),
            decoder.as_mut(),
            access_unit.as_slice(),
        )
        .expect("decode access unit into snapshot");

        let snapshot = snapshot.lock().expect("lock host snapshot").clone();
        assert_eq!(snapshot.decoded_frame_count, 1);
        assert_eq!(snapshot.last_decoded_width, 16);
        assert_eq!(snapshot.last_decoded_height, 16);
        assert_eq!(snapshot.last_decoded_pixel_format.as_deref(), Some("Rgb24"));
        assert_eq!(snapshot.decode_error_count, 0);
        assert_eq!(snapshot.last_decode_error, None);
    }

    #[test]
    fn decode_failure_updates_snapshot_diagnostics() {
        let snapshot = Arc::new(Mutex::new(WebrtcHostSnapshot::default()));
        let mut decoder = mrd_decode::create_decoder("h264_software").expect("decoder instance");

        let error = decode_access_unit_into_snapshot(
            SessionId("session-decode-error".into()),
            "video-track-1".to_string(),
            snapshot.clone(),
            None,
            ProbeRegistry::default().session_handle(
                SessionId("session-decode-error".into()),
                crate::frame_sink::DEFAULT_SOURCE_ID,
            ),
            decoder.as_mut(),
            &[0, 1, 2, 3],
        )
        .expect_err("invalid access unit should fail");

        let snapshot = snapshot.lock().expect("lock host snapshot").clone();
        assert!(
            error.contains("decoder"),
            "decode error should mention decoder path: {error}"
        );
        assert_eq!(snapshot.decode_error_count, 1);
        assert!(
            snapshot
                .last_decode_error
                .as_deref()
                .unwrap_or_default()
                .contains("decoder"),
            "snapshot should retain last decode error: {:?}",
            snapshot.last_decode_error
        );
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

    struct FakeCapture {
        tick: u8,
    }

    impl FrameCapture for FakeCapture {
        fn capture_frame(&mut self) -> Result<CapturedFrame, mrd_pipeline_core::PipelineError> {
            self.tick = self.tick.wrapping_add(1);
            let mut data = vec![0_u8; 16 * 16 * 4];
            for chunk in data.chunks_exact_mut(4) {
                chunk[0] = self.tick;
                chunk[1] = 64;
                chunk[2] = 192;
                chunk[3] = 255;
            }

            Ok(CapturedFrame {
                width: 16,
                height: 16,
                pixel_format: FramePixelFormat::Bgra32,
                timestamp_us: self.tick as u64 * 33_000,
                data,
            })
        }
    }

    struct HostedPairHarness {
        controller: WebrtcHost,
        agent: WebrtcHost,
        sink: Arc<Mutex<DecodedFrameSink>>,
        session_id: SessionId,
    }

    struct FrameProgressSample {
        start_frame_count: u64,
        end_frame_count: u64,
        observed_samples: usize,
    }

    impl HostedPairHarness {
        fn new(session_id: &str) -> Self {
            let sink = Arc::new(Mutex::new(DecodedFrameSink::default()));
            Self {
                controller: WebrtcHost::with_frame_sink(sink.clone()),
                agent: WebrtcHost::default(),
                sink,
                session_id: SessionId(session_id.into()),
            }
        }

        async fn start(&mut self) -> Result<(), String> {
            self.agent
                .start_test_video_sender(
                    self.session_id.clone(),
                    FakeCapture { tick: 0 },
                    OpenH264Encoder::new(16, 16, 30).expect("openh264 encoder"),
                    Duration::from_millis(33),
                )
                .await?;

            self.finish_signaling().await
        }

        async fn start_with_encoder<E>(&mut self, encoder: E) -> Result<(), String>
        where
            E: VideoEncoder + Send + 'static,
        {
            self.agent
                .start_test_video_sender(
                    self.session_id.clone(),
                    FakeCapture { tick: 0 },
                    encoder,
                    Duration::from_millis(33),
                )
                .await?;

            self.finish_signaling().await
        }

        async fn finish_signaling(&mut self) -> Result<(), String> {
            let offer = self.agent.create_offer(self.session_id.clone()).await?;
            self.controller
                .apply_remote_offer(self.session_id.clone(), offer.sdp)
                .await?;
            let answer = self.controller.create_answer(self.session_id.clone()).await?;
            self.agent
                .apply_remote_answer(self.session_id.clone(), answer.sdp)
                .await?;
            Ok(())
        }

        async fn wait_for_first_frame(&self, timeout: Duration) -> Result<(), String> {
            tokio::time::timeout(timeout, async {
                loop {
                    let snapshot = self
                        .controller
                        .snapshot(&self.session_id)
                        .expect("controller snapshot");
                    if snapshot.decoded_frame_count > 0 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .map_err(|_| format!("timed out waiting for first frame for {}", self.session_id.0))
        }

        fn controller_snapshot(&self) -> WebrtcHostSnapshot {
            self.controller
                .snapshot(&self.session_id)
                .expect("controller snapshot")
        }

        fn agent_snapshot(&self) -> WebrtcHostSnapshot {
            self.agent.snapshot(&self.session_id).expect("agent snapshot")
        }

        fn controller_probe(&self) -> PipelineProbeSnapshot {
            self.controller
                .probe_snapshot(&self.session_id)
                .expect("controller probe snapshot")
        }

        fn agent_probe(&self) -> PipelineProbeSnapshot {
            self.agent
                .probe_snapshot(&self.session_id)
                .expect("agent probe snapshot")
        }

        fn sink_snapshot(&self) -> Option<DecodedFrameSnapshot> {
            self.sink
                .lock()
                .expect("lock sink")
                .snapshot(&self.session_id)
                .cloned()
        }

        async fn sample_frame_progress(
            &self,
            duration: Duration,
            step: Duration,
        ) -> FrameProgressSample {
            let start_frame_count = self
                .sink_snapshot()
                .map(|snapshot| snapshot.frame_count)
                .unwrap_or(0);
            let started_at = tokio::time::Instant::now();
            let mut observed_samples = 0usize;
            while started_at.elapsed() < duration {
                tokio::time::sleep(step).await;
                observed_samples += 1;
            }
            let end_frame_count = self
                .sink_snapshot()
                .map(|snapshot| snapshot.frame_count)
                .unwrap_or(0);

            FrameProgressSample {
                start_frame_count,
                end_frame_count,
                observed_samples,
            }
        }
    }

    #[tokio::test]
    async fn embedded_sender_delivers_decoded_frames_to_remote_host() {
        ensure_rustls_crypto_provider();
        let sink = Arc::new(Mutex::new(crate::frame_sink::DecodedFrameSink::default()));
        let mut controller = WebrtcHost::with_frame_sink(sink.clone());
        let mut agent = WebrtcHost::default();
        let session_id = SessionId("session-e2e".into());

        agent
            .start_test_video_sender(
                session_id.clone(),
                FakeCapture { tick: 0 },
                OpenH264Encoder::new(16, 16, 30).expect("openh264 encoder"),
                Duration::from_millis(33),
            )
            .await
            .expect("start embedded sender");

        let offer = agent
            .create_offer(session_id.clone())
            .await
            .expect("agent offer");

        controller
            .apply_remote_offer(session_id.clone(), offer.sdp)
            .await
            .expect("controller apply offer");

        let answer = controller
            .create_answer(session_id.clone())
            .await
            .expect("controller answer");
        assert!(
            answer.sdp.contains("m=video"),
            "controller answer should negotiate a video m-line: {}",
            answer.sdp
        );
        assert!(
            answer.sdp.to_ascii_lowercase().contains("a=recvonly")
                || answer.sdp.to_ascii_lowercase().contains("a=sendrecv"),
            "controller answer should expose a receiving video direction: {}",
            answer.sdp
        );
        assert!(
            answer.sdp.to_ascii_lowercase().contains("h264"),
            "controller answer should include H264 codec negotiation: {}",
            answer.sdp
        );

        agent
            .apply_remote_answer(session_id.clone(), answer.sdp)
            .await
            .expect("agent apply answer");

        let wait_result = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let snapshot = controller.snapshot(&session_id).expect("controller snapshot");
                if snapshot.decoded_frame_count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        let controller_snapshot = controller.snapshot(&session_id).expect("controller snapshot");
        let agent_snapshot = agent.snapshot(&session_id).expect("agent snapshot");
        let controller_probe = controller
            .probe_snapshot(&session_id)
            .expect("controller probe snapshot");
        let agent_probe = agent.probe_snapshot(&session_id).expect("agent probe snapshot");
        let controller_stats = controller
            .sessions
            .get(&session_id)
            .expect("controller peer")
            .pc
            .get_stats()
            .await;
        let agent_stats = agent
            .sessions
            .get(&session_id)
            .expect("agent peer")
            .pc
            .get_stats()
            .await;
        let controller_report_count = controller_stats.reports.len();
        let agent_report_count = agent_stats.reports.len();
        assert!(
            wait_result.is_ok(),
            "remote host receives decoded frames: controller={controller_snapshot:?} agent={agent_snapshot:?} controller_reports={controller_report_count} agent_reports={agent_report_count} controller_stats={controller_stats:?} agent_stats={agent_stats:?}"
        );

        assert!(controller_snapshot.decoded_frame_count > 0);
        assert!(controller_snapshot.remote_h264_access_unit_count > 0);
        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::DecodeTotal && stats.count > 0));
        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::FrameSinkIngest && stats.count > 0));
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::EncodeTotal && stats.count > 0));
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::SendWrite && stats.count > 0));
    }

    #[tokio::test]
    async fn single_process_pipeline_exposes_probe_stages() {
        ensure_rustls_crypto_provider();
        let mut harness = HostedPairHarness::new("session-composed-probe");

        harness.start().await.expect("start composed pipeline");
        harness
            .wait_for_first_frame(Duration::from_secs(8))
            .await
            .expect("remote decoded frame");

        let controller_probe = harness.controller_probe();
        let agent_probe = harness.agent_probe();

        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::NetworkIngress && stats.count > 0));
        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::H264Assemble && stats.count > 0));
        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::DecodeTotal && stats.count > 0));
        assert!(controller_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::FrameSinkIngest && stats.count > 0));
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::CaptureCopy && stats.count > 0));
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::EncodeTotal && stats.count > 0));
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::SendWrite && stats.count > 0));
    }

    #[tokio::test]
    async fn single_process_pipeline_delivers_remote_frames() {
        ensure_rustls_crypto_provider();
        let mut harness = HostedPairHarness::new("session-composed-frames");

        harness.start().await.expect("start composed pipeline");
        harness
            .wait_for_first_frame(Duration::from_secs(8))
            .await
            .expect("remote decoded frame");

        let controller_snapshot = harness.controller_snapshot();
        let agent_snapshot = harness.agent_snapshot();
        let sink_snapshot = harness.sink_snapshot().expect("sink snapshot");

        assert!(controller_snapshot.decoded_frame_count > 0);
        assert!(controller_snapshot.remote_rtp_packet_count > 0);
        assert!(controller_snapshot.remote_h264_access_unit_count > 0);
        assert!(agent_snapshot.sender_running);
        assert!(sink_snapshot.frame_count > 0);
    }

    #[tokio::test]
    async fn single_process_pipeline_runs_for_fixed_duration_without_stalling() {
        ensure_rustls_crypto_provider();
        let mut harness = HostedPairHarness::new("session-composed-stable");

        harness.start().await.expect("start composed pipeline");
        harness
            .wait_for_first_frame(Duration::from_secs(8))
            .await
            .expect("remote decoded frame");

        let progress = harness
            .sample_frame_progress(Duration::from_secs(2), Duration::from_millis(250))
            .await;

        assert!(progress.start_frame_count > 0);
        assert!(progress.end_frame_count > progress.start_frame_count);
        assert!(progress.observed_samples > 0);
        assert!(harness.agent_snapshot().sender_running);
    }

    #[tokio::test]
    async fn nvenc_single_process_pipeline_delivers_remote_frames() {
        ensure_rustls_crypto_provider();
        let Ok(encoder) = NvencH264Encoder::new(16, 16, 30) else {
            return;
        };
        let mut harness = HostedPairHarness::new("session-composed-nvenc");

        harness
            .start_with_encoder(encoder)
            .await
            .expect("start nvenc composed pipeline");
        harness
            .wait_for_first_frame(Duration::from_secs(8))
            .await
            .expect("remote decoded frame");

        let controller_snapshot = harness.controller_snapshot();
        let agent_probe = harness.agent_probe();
        assert!(controller_snapshot.decoded_frame_count > 0);
        assert!(controller_snapshot.remote_h264_access_unit_count > 0);
        assert!(agent_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::EncodeTotal && stats.count > 0));
    }

    #[test]
    fn nvenc_720p_access_unit_survives_rtp_ingress_and_software_decode() {
        let Ok(mut encoder) = NvencH264Encoder::new(1280, 720, 30) else {
            return;
        };
        let mut frame = vec![0_u8; 1280 * 720 * 4];
        for (index, chunk) in frame.chunks_exact_mut(4).enumerate() {
            let x = (index % 1280) as u8;
            let y = ((index / 1280) % 256) as u8;
            chunk[0] = x;
            chunk[1] = y;
            chunk[2] = x.wrapping_add(y);
            chunk[3] = 255;
        }

        let access_unit = encoder
            .encode(&CapturedFrame {
                width: 1280,
                height: 720,
                pixel_format: FramePixelFormat::Bgra32,
                timestamp_us: 33_000,
                data: frame,
            })
            .expect("encode nvenc frame")
            .into_iter()
            .next()
            .expect("single access unit");
        let mut sender = mrd_transport_webrtc::H264RtpSender::new("video", "stream", 30, 1200);
        let packets = sender
            .packetize_access_unit(&access_unit)
            .expect("packetize access unit");
        let mut ingress = mrd_transport_webrtc::H264RtpIngress::default();
        let reassembled = packets
            .into_iter()
            .filter_map(|packet| {
                ingress.push_payload(&packet.payload, packet.header.marker, access_unit.timestamp_us)
            })
            .last()
            .expect("reassembled access unit");
        let mut decoder = mrd_decode::create_decoder("h264_software").expect("decoder instance");

        decoder
            .push_access_unit(&reassembled.bytes)
            .expect("decode reassembled access unit");
        let frames = decoder.drain_decoded_frames();
        assert!(
            !frames.is_empty(),
            "reassembled access unit should decode into at least one frame"
        );
    }
}
