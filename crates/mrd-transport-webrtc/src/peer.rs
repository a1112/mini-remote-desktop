use std::{
    fmt,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use bytes::Bytes;
use mrd_pipeline_core::EncodedAccessUnit;
use tokio::{
    sync::{mpsc, watch, Mutex},
    task::JoinHandle,
};
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
        setting_engine::SettingEngine, APIBuilder,
    },
    data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel},
    ice_transport::{ice_candidate::RTCIceCandidateInit, ice_server::RTCIceServer},
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        policy::ice_transport_policy::RTCIceTransportPolicy,
        sdp::session_description::RTCSessionDescription, RTCPeerConnection,
    },
    track::track_local::TrackLocal,
};

use crate::{
    config::{IceTransportPolicy, PeerConnectionConfig, PeerConnectionRole},
    control::{
        channel_info, realtime_channel_init, reliable_channel_init, ControlChannels, ControlLane,
        ControlState, CTRL_REL_LABEL, CTRL_RT_LABEL,
    },
    stats::selected_candidate_pair,
    H264RtpIngress, H264RtpSender, SelectedCandidatePairStats, TransportError,
};

const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDescriptionType {
    Offer,
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescription {
    pub kind: SessionDescriptionType,
    pub sdp: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
    pub username_fragment: Option<String>,
}

impl From<RTCIceCandidateInit> for IceCandidate {
    fn from(value: RTCIceCandidateInit) -> Self {
        Self {
            candidate: value.candidate,
            sdp_mid: value.sdp_mid,
            sdp_mline_index: value.sdp_mline_index,
            username_fragment: value.username_fragment,
        }
    }
}

impl From<IceCandidate> for RTCIceCandidateInit {
    fn from(value: IceCandidate) -> Self {
        Self {
            candidate: value.candidate,
            sdp_mid: value.sdp_mid,
            sdp_mline_index: value.sdp_mline_index,
            username_fragment: value.username_fragment,
        }
    }
}

pub struct WebRtcPeerConnection {
    pc: Arc<RTCPeerConnection>,
    config: PeerConnectionConfig,
    h264_sender: Mutex<H264RtpSender>,
    local_candidates: Mutex<mpsc::Receiver<IceCandidate>>,
    h264_rx: Mutex<mpsc::Receiver<EncodedAccessUnit>>,
    reliable_rx: Mutex<mpsc::Receiver<Bytes>>,
    realtime_rx: Mutex<mpsc::Receiver<Bytes>>,
    control: Arc<ControlState>,
    connection_state_rx: watch::Receiver<RTCPeerConnectionState>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    active_tasks: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl fmt::Debug for WebRtcPeerConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebRtcPeerConnection")
            .field("role", &self.config.role)
            .field("active_tasks", &self.active_task_count())
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl WebRtcPeerConnection {
    pub async fn new(config: PeerConnectionConfig) -> Result<Self, TransportError> {
        let codec = config.preflight()?.clone();
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .map_err(|error| TransportError::Message(format!("register codecs failed: {error}")))?;
        let registry =
            register_default_interceptors(Registry::new(), &mut media_engine).map_err(|error| {
                TransportError::Message(format!("register interceptors failed: {error}"))
            })?;
        let mut settings = SettingEngine::default();
        settings.set_include_loopback_candidate(config.include_loopback_candidates);
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(settings)
            .build();
        let ice_servers = config
            .ice_servers
            .iter()
            .map(|server| RTCIceServer {
                urls: server.urls.clone(),
                username: server.username.clone(),
                credential: server.credential.clone(),
            })
            .collect();
        let ice_transport_policy = match config.ice_transport_policy {
            IceTransportPolicy::All => RTCIceTransportPolicy::All,
            IceTransportPolicy::Relay => RTCIceTransportPolicy::Relay,
        };
        let pc = Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_servers,
                ice_transport_policy,
                ..Default::default()
            })
            .await
            .map_err(|error| {
                TransportError::Message(format!("create peer connection failed: {error}"))
            })?,
        );

        let capacity = config.event_queue_capacity;
        let (candidate_tx, candidate_rx) = mpsc::channel(capacity);
        pc.on_ice_candidate(Box::new(move |candidate| {
            let candidate_tx = candidate_tx.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    if let Ok(candidate) = candidate.to_json() {
                        let _ = candidate_tx.try_send(candidate.into());
                    }
                }
            })
        }));

        let (connection_state_tx, connection_state_rx) =
            watch::channel(RTCPeerConnectionState::New);
        pc.on_peer_connection_state_change(Box::new(move |state| {
            let connection_state_tx = connection_state_tx.clone();
            Box::pin(async move {
                let _ = connection_state_tx.send(state);
            })
        }));

        let (control, reliable_rx, realtime_rx) = ControlState::new(capacity);
        let remote_control = Arc::clone(&control);
        pc.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
            let remote_control = Arc::clone(&remote_control);
            Box::pin(async move {
                remote_control.install(channel).await;
            })
        }));

        let tasks = Arc::new(Mutex::new(Vec::new()));
        let active_tasks = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let (h264_tx, h264_rx) = mpsc::channel(capacity);
        let remote_tasks = Arc::clone(&tasks);
        let remote_active_tasks = Arc::clone(&active_tasks);
        let remote_closed = Arc::clone(&closed);
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let h264_tx = h264_tx.clone();
            let tasks = Arc::clone(&remote_tasks);
            let active_tasks = Arc::clone(&remote_active_tasks);
            let closed = Arc::clone(&remote_closed);
            Box::pin(async move {
                let mut tasks = tasks.lock().await;
                if closed.load(Ordering::Acquire) {
                    return;
                }
                let task_counter = Arc::clone(&active_tasks);
                let handle = spawn_tracked(&task_counter, async move {
                    let mut ingress = H264RtpIngress::default();
                    while let Ok((packet, _attributes)) = track.read_rtp().await {
                        let timestamp_us = u64::from(packet.header.timestamp) * 1_000_000 / 90_000;
                        if let Some(access_unit) = ingress.push_packet(
                            &packet.payload,
                            packet.header.marker,
                            packet.header.sequence_number,
                            timestamp_us,
                        ) {
                            if h264_tx.send(access_unit).await.is_err() {
                                break;
                            }
                        }
                    }
                });
                tasks.push(handle);
            })
        }));

        if config.role == PeerConnectionRole::Offerer {
            let reliable = pc
                .create_data_channel(CTRL_REL_LABEL, Some(reliable_channel_init()))
                .await
                .map_err(|error| {
                    TransportError::Message(format!("create ctrl_rel failed: {error}"))
                })?;
            control.install(reliable).await;
            let realtime = pc
                .create_data_channel(CTRL_RT_LABEL, Some(realtime_channel_init()))
                .await
                .map_err(|error| {
                    TransportError::Message(format!("create ctrl_rt failed: {error}"))
                })?;
            control.install(realtime).await;
        }

        let h264_sender = H264RtpSender::new_with_profile_level_id(
            "screen",
            "desktop",
            config.fps,
            config.mtu,
            codec.profile.into(),
            codec.profile_level_id,
        );
        let rtp_sender = pc
            .add_track(h264_sender.track() as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .map_err(|error| TransportError::Message(format!("add H.264 track failed: {error}")))?;
        let rtcp_task = spawn_tracked(&active_tasks, async move {
            while rtp_sender.read_rtcp().await.is_ok() {}
        });
        tasks.lock().await.push(rtcp_task);

        Ok(Self {
            pc,
            config,
            h264_sender: Mutex::new(h264_sender),
            local_candidates: Mutex::new(candidate_rx),
            h264_rx: Mutex::new(h264_rx),
            reliable_rx: Mutex::new(reliable_rx),
            realtime_rx: Mutex::new(realtime_rx),
            control,
            connection_state_rx,
            tasks,
            active_tasks,
            closed,
        })
    }

    pub async fn create_offer(&self) -> Result<SessionDescription, TransportError> {
        self.require_role(PeerConnectionRole::Offerer)?;
        let offer = self
            .pc
            .create_offer(None)
            .await
            .map_err(peer_error("create offer"))?;
        let description = SessionDescription {
            kind: SessionDescriptionType::Offer,
            sdp: offer.sdp.clone(),
        };
        self.pc
            .set_local_description(offer)
            .await
            .map_err(peer_error("set local offer"))?;
        Ok(description)
    }

    pub async fn accept_offer(
        &self,
        offer: SessionDescription,
    ) -> Result<SessionDescription, TransportError> {
        self.require_role(PeerConnectionRole::Answerer)?;
        if offer.kind != SessionDescriptionType::Offer {
            return Err(TransportError::Message("expected an SDP offer".into()));
        }
        let offer = RTCSessionDescription::offer(offer.sdp).map_err(peer_error("parse offer"))?;
        self.pc
            .set_remote_description(offer)
            .await
            .map_err(peer_error("set remote offer"))?;
        let answer = self
            .pc
            .create_answer(None)
            .await
            .map_err(peer_error("create answer"))?;
        let description = SessionDescription {
            kind: SessionDescriptionType::Answer,
            sdp: answer.sdp.clone(),
        };
        self.pc
            .set_local_description(answer)
            .await
            .map_err(peer_error("set local answer"))?;
        Ok(description)
    }

    pub async fn accept_answer(&self, answer: SessionDescription) -> Result<(), TransportError> {
        self.require_role(PeerConnectionRole::Offerer)?;
        if answer.kind != SessionDescriptionType::Answer {
            return Err(TransportError::Message("expected an SDP answer".into()));
        }
        let answer =
            RTCSessionDescription::answer(answer.sdp).map_err(peer_error("parse answer"))?;
        self.pc
            .set_remote_description(answer)
            .await
            .map_err(peer_error("set remote answer"))
    }

    pub async fn next_local_candidate(&self) -> Option<IceCandidate> {
        self.local_candidates.lock().await.recv().await
    }

    pub async fn add_ice_candidate(&self, candidate: IceCandidate) -> Result<(), TransportError> {
        self.pc
            .add_ice_candidate(candidate.into())
            .await
            .map_err(peer_error("add ICE candidate"))
    }

    pub async fn wait_connected(&self) -> Result<(), TransportError> {
        let mut states = self.connection_state_rx.clone();
        loop {
            let state = *states.borrow_and_update();
            match state {
                RTCPeerConnectionState::Connected => return Ok(()),
                RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Disconnected
                | RTCPeerConnectionState::Closed => {
                    return Err(TransportError::Message(format!(
                        "peer connection entered {state}"
                    )));
                }
                _ => {}
            }
            states.changed().await.map_err(|_| {
                TransportError::Message("peer connection state stream closed".into())
            })?;
        }
    }

    pub async fn send_h264_access_unit(
        &self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<usize, TransportError> {
        self.h264_sender
            .lock()
            .await
            .send_access_unit(access_unit)
            .await
    }

    pub async fn next_h264_access_unit(&self) -> Option<EncodedAccessUnit> {
        self.h264_rx.lock().await.recv().await
    }

    pub async fn send_control(
        &self,
        lane: ControlLane,
        payload: &[u8],
    ) -> Result<usize, TransportError> {
        let channel = self.wait_for_channel(lane).await?;
        channel
            .send(&Bytes::copy_from_slice(payload))
            .await
            .map_err(peer_error("send control message"))
    }

    pub async fn next_control(&self, lane: ControlLane) -> Option<Bytes> {
        match lane {
            ControlLane::Reliable => self.reliable_rx.lock().await.recv().await,
            ControlLane::Realtime => self.realtime_rx.lock().await.recv().await,
        }
    }

    pub async fn control_channels(&self) -> ControlChannels {
        let reliable = self.control.channel(ControlLane::Reliable).await;
        let realtime = self.control.channel(ControlLane::Realtime).await;
        ControlChannels {
            reliable: reliable.as_deref().map(channel_info).unwrap_or_else(|| {
                crate::ControlChannelInfo {
                    label: CTRL_REL_LABEL.to_owned(),
                    ordered: true,
                    max_retransmits: None,
                }
            }),
            realtime: realtime.as_deref().map(channel_info).unwrap_or_else(|| {
                crate::ControlChannelInfo {
                    label: CTRL_RT_LABEL.to_owned(),
                    ordered: false,
                    max_retransmits: Some(0),
                }
            }),
        }
    }

    pub async fn selected_candidate_pair_stats(&self) -> Option<SelectedCandidatePairStats> {
        selected_candidate_pair(self.pc.get_stats().await)
    }

    pub fn active_task_count(&self) -> usize {
        self.active_tasks.load(Ordering::Acquire)
    }

    pub async fn close(&self) -> Result<(), TransportError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let mut tasks = self.tasks.lock().await;
        for task in tasks.iter() {
            task.abort();
        }
        for task in tasks.drain(..) {
            let _ = task.await;
        }
        drop(tasks);
        if let Some(channel) = self.control.channel(ControlLane::Reliable).await {
            let _ = channel.close().await;
        }
        if let Some(channel) = self.control.channel(ControlLane::Realtime).await {
            let _ = channel.close().await;
        }
        self.pc
            .close()
            .await
            .map_err(peer_error("close peer connection"))
    }

    fn require_role(&self, expected: PeerConnectionRole) -> Result<(), TransportError> {
        if self.config.role == expected {
            Ok(())
        } else {
            Err(TransportError::Message(format!(
                "operation requires {expected:?} role"
            )))
        }
    }

    async fn wait_for_channel(
        &self,
        lane: ControlLane,
    ) -> Result<Arc<RTCDataChannel>, TransportError> {
        let deadline = tokio::time::Instant::now() + CHANNEL_OPEN_TIMEOUT;
        loop {
            if let Some(channel) = self.control.channel(lane).await {
                if channel.ready_state() == RTCDataChannelState::Open {
                    return Ok(channel);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(TransportError::Message(format!(
                    "control channel {lane:?} did not open"
                )));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn spawn_tracked<F>(counter: &Arc<AtomicUsize>, future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    counter.fetch_add(1, Ordering::AcqRel);
    let counter = Arc::clone(counter);
    tokio::spawn(async move {
        struct TaskGuard(Arc<AtomicUsize>);
        impl Drop for TaskGuard {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _guard = TaskGuard(counter);
        future.await;
    })
}

fn peer_error(context: &'static str) -> impl FnOnce(webrtc::Error) -> TransportError {
    move |error| TransportError::Message(format!("{context} failed: {error}"))
}
