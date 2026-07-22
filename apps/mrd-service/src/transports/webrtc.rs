use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{bail, Result};
use bytes::Bytes;
use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportMuxPort, TransportRouteKind, TransportRouteSnapshot,
    TransportSendOutcome, VideoEnvelopeMetadata,
};
use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
use mrd_proto::SessionId;
use mrd_transport_webrtc::{
    ControlLane, IceCandidate, IceServerConfig, IceTransportPolicy, PeerConnectionConfig,
    SelectedCandidatePairStats, SessionDescription, WebRtcPeerConnection,
};
use thiserror::Error;
use tokio::sync::RwLock;

use super::{quic, SessionMuxCore, TransportMuxConfig};

const DATA_FRAGMENT_MAGIC: &[u8; 4] = b"MRDF";
const DATA_FRAGMENT_VERSION: u8 = 1;
const DATA_FRAGMENT_HEADER_LEN: usize = 4 + 1 + 8 + 2 + 2 + 4;
const DATA_FRAGMENT_PAYLOAD_LEN: usize = 60 * 1024;
const MAX_ENVELOPE_WIRE_OVERHEAD: usize = 38 + u16::MAX as usize + u8::MAX as usize;
const DATA_CHANNEL_WIRE_BUDGET_OVERHEAD: usize = 128 * 1024;

#[derive(Debug, Error)]
pub enum ServiceWebRtcTransportError {
    #[error("WebRTC session {0:?} already exists")]
    DuplicateSession(SessionId),
    #[error("WebRTC session {0:?} was not found")]
    SessionNotFound(SessionId),
    #[error("WebRTC transport failed: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayUrlClass {
    TurnUdp,
    TurnTcp,
    TurnsTcp,
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServiceTurnRelayCredentials {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
    pub expires_at_unix_seconds: u64,
}

impl fmt::Debug for ServiceTurnRelayCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceTurnRelayCredentials")
            .field("url_classes", &self.url_classes())
            .field("username", &"[REDACTED]")
            .field("credential", &"[REDACTED]")
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

impl ServiceTurnRelayCredentials {
    pub fn apply_relay_only(&self, mut config: PeerConnectionConfig) -> PeerConnectionConfig {
        config.ice_servers = vec![IceServerConfig::new(
            self.urls.clone(),
            self.username.clone(),
            self.credential.clone(),
        )];
        config.ice_transport_policy = IceTransportPolicy::Relay;
        config
    }

    pub fn url_classes(&self) -> Vec<RelayUrlClass> {
        self.urls
            .iter()
            .map(|url| {
                if url.starts_with("turns:") {
                    RelayUrlClass::TurnsTcp
                } else if url.starts_with("turn:") && url.contains("transport=tcp") {
                    RelayUrlClass::TurnTcp
                } else if url.starts_with("turn:") && url.contains("transport=udp") {
                    RelayUrlClass::TurnUdp
                } else {
                    RelayUrlClass::Unknown
                }
            })
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct ServiceWebRtcTransportHost {
    sessions: RwLock<HashMap<SessionId, ServiceWebRtcSession>>,
}

#[derive(Debug)]
struct ServiceWebRtcSession {
    peer: Arc<WebRtcPeerConnection>,
    mux: Arc<WebRtcTransportMux>,
}

impl ServiceWebRtcTransportHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn open_session(
        &self,
        session_id: SessionId,
        mut config: PeerConnectionConfig,
    ) -> Result<(), ServiceWebRtcTransportError> {
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Err(ServiceWebRtcTransportError::DuplicateSession(session_id));
            }
        }
        let mux_config = TransportMuxConfig::default();
        config.max_h264_access_unit_bytes = config
            .max_h264_access_unit_bytes
            .min(mux_config.video_byte_capacity);
        config.video_queue_bytes = config.video_queue_bytes.min(mux_config.video_byte_capacity);
        config.reliable_queue_bytes = config.reliable_queue_bytes.min(
            mux_config
                .control_reliable_byte_capacity
                .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
        );
        config.realtime_queue_bytes = config
            .realtime_queue_bytes
            .min(mux_config.control_realtime_byte_capacity);
        config.bulk_queue_bytes = config.bulk_queue_bytes.min(
            mux_config
                .bulk_byte_capacity
                .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
        );
        let peer = Arc::new(
            WebRtcPeerConnection::new(config)
                .await
                .map_err(transport_error)?,
        );
        let mux = Arc::new(
            WebRtcTransportMux::new(session_id.clone(), mux_config, Arc::clone(&peer))
                .await
                .map_err(|error| ServiceWebRtcTransportError::Transport(error.to_string()))?,
        );
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            drop(sessions);
            let _ = mux.close().await;
            return Err(ServiceWebRtcTransportError::DuplicateSession(session_id));
        }
        sessions.insert(session_id, ServiceWebRtcSession { peer, mux });
        Ok(())
    }

    pub async fn create_offer(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionDescription, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .create_offer()
            .await
            .map_err(transport_error)
    }

    pub async fn accept_offer(
        &self,
        session_id: &SessionId,
        offer: SessionDescription,
    ) -> Result<SessionDescription, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .accept_offer(offer)
            .await
            .map_err(transport_error)
    }

    pub async fn accept_answer(
        &self,
        session_id: &SessionId,
        answer: SessionDescription,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .accept_answer(answer)
            .await
            .map_err(transport_error)
    }

    pub async fn next_local_candidate(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<IceCandidate>, ServiceWebRtcTransportError> {
        Ok(self.session(session_id).await?.next_local_candidate().await)
    }

    pub async fn add_ice_candidate(
        &self,
        session_id: &SessionId,
        candidate: IceCandidate,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .add_ice_candidate(candidate)
            .await
            .map_err(transport_error)
    }

    pub async fn wait_connected(
        &self,
        session_id: &SessionId,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .wait_connected()
            .await
            .map_err(transport_error)
    }

    pub async fn selected_candidate_pair_stats(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SelectedCandidatePairStats>, ServiceWebRtcTransportError> {
        Ok(self
            .session(session_id)
            .await?
            .selected_candidate_pair_stats()
            .await)
    }

    pub async fn close_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), ServiceWebRtcTransportError> {
        let session = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or_else(|| ServiceWebRtcTransportError::SessionNotFound(session_id.clone()))?;
        session
            .mux
            .close()
            .await
            .map_err(|error| ServiceWebRtcTransportError::Transport(error.to_string()))
    }

    pub async fn shutdown(&self) -> Result<(), ServiceWebRtcTransportError> {
        let sessions = std::mem::take(&mut *self.sessions.write().await);
        let mut first_error = None;
        for session in sessions.into_values() {
            if let Err(error) = session.mux.close().await {
                first_error.get_or_insert_with(|| {
                    ServiceWebRtcTransportError::Transport(error.to_string())
                });
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub async fn transport_mux(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<WebRtcTransportMux>, ServiceWebRtcTransportError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .map(|session| Arc::clone(&session.mux))
            .ok_or_else(|| ServiceWebRtcTransportError::SessionNotFound(session_id.clone()))
    }

    async fn session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<WebRtcPeerConnection>, ServiceWebRtcTransportError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .map(|session| Arc::clone(&session.peer))
            .ok_or_else(|| ServiceWebRtcTransportError::SessionNotFound(session_id.clone()))
    }
}

fn transport_error(error: mrd_transport_webrtc::TransportError) -> ServiceWebRtcTransportError {
    ServiceWebRtcTransportError::Transport(error.to_string())
}

/// Session transport mux backed by a service-owned WebRTC peer connection.
#[derive(Debug)]
pub struct WebRtcTransportMux {
    core: Arc<SessionMuxCore>,
    peer: Arc<WebRtcPeerConnection>,
}

impl WebRtcTransportMux {
    pub async fn loopback(
        session_id: SessionId,
        config: TransportMuxConfig,
    ) -> Result<(Self, Self)> {
        use mrd_transport_webrtc::PeerConnectionRole;

        let offerer = Arc::new(
            WebRtcPeerConnection::new(PeerConnectionConfig {
                role: PeerConnectionRole::Offerer,
                include_loopback_candidates: true,
                max_h264_access_unit_bytes: config.video_byte_capacity,
                video_queue_bytes: config.video_byte_capacity,
                reliable_queue_bytes: config
                    .control_reliable_byte_capacity
                    .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
                realtime_queue_bytes: config.control_realtime_byte_capacity,
                bulk_queue_bytes: config
                    .bulk_byte_capacity
                    .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
                ..PeerConnectionConfig::default()
            })
            .await?,
        );
        let answerer = Arc::new(
            WebRtcPeerConnection::new(PeerConnectionConfig {
                role: PeerConnectionRole::Answerer,
                include_loopback_candidates: true,
                max_h264_access_unit_bytes: config.video_byte_capacity,
                video_queue_bytes: config.video_byte_capacity,
                reliable_queue_bytes: config
                    .control_reliable_byte_capacity
                    .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
                realtime_queue_bytes: config.control_realtime_byte_capacity,
                bulk_queue_bytes: config
                    .bulk_byte_capacity
                    .saturating_add(DATA_CHANNEL_WIRE_BUDGET_OVERHEAD),
                ..PeerConnectionConfig::default()
            })
            .await?,
        );
        let offer = offerer.create_offer().await?;
        let answer = answerer.accept_offer(offer).await?;
        offerer.accept_answer(answer).await?;
        let offer_candidate = offerer
            .next_local_candidate()
            .await
            .ok_or_else(|| anyhow::anyhow!("offerer produced no loopback ICE candidate"))?;
        let answer_candidate = answerer
            .next_local_candidate()
            .await
            .ok_or_else(|| anyhow::anyhow!("answerer produced no loopback ICE candidate"))?;
        answerer.add_ice_candidate(offer_candidate).await?;
        offerer.add_ice_candidate(answer_candidate).await?;
        tokio::try_join!(offerer.wait_connected(), answerer.wait_connected())?;

        let left = Self::new(session_id.clone(), config, offerer).await?;
        let right = Self::new(session_id, config, answerer).await?;
        Ok((left, right))
    }

    pub async fn new(
        session_id: SessionId,
        config: TransportMuxConfig,
        peer: Arc<WebRtcPeerConnection>,
    ) -> Result<Self> {
        let core = SessionMuxCore::new(
            session_id,
            config,
            TransportRouteKind::WebRtcPending,
            "webrtc:pending-local-candidate",
            "webrtc:pending-remote-candidate",
        );
        refresh_webrtc_route(&core, &peer).await;
        spawn_webrtc_senders(Arc::clone(&core), Arc::clone(&peer));
        spawn_webrtc_receivers(Arc::clone(&core), Arc::clone(&peer), config);
        spawn_webrtc_connection_watcher(Arc::clone(&core), Arc::clone(&peer));
        Ok(Self { core, peer })
    }
}

impl Drop for WebRtcTransportMux {
    fn drop(&mut self) {
        flush_webrtc_video_drops(&self.core, &self.peer);
        self.core.terminate_now(None);
        self.peer.terminate_now();
        flush_webrtc_video_drops(&self.core, &self.peer);
    }
}

async fn fail_webrtc(core: &SessionMuxCore, peer: &WebRtcPeerConnection, reason: String) {
    let _ = peer.close().await;
    flush_webrtc_video_drops(core, peer);
    core.fail(reason).await;
}

fn flush_webrtc_video_drops(core: &SessionMuxCore, peer: &WebRtcPeerConnection) {
    core.record_adapter_drops(TransportLane::Video, peer.take_completed_video_drops());
}

fn spawn_webrtc_connection_watcher(core: Arc<SessionMuxCore>, peer: Arc<WebRtcPeerConnection>) {
    let owner = Arc::clone(&core);
    let task = tokio::spawn(async move {
        let reason = match peer.wait_terminated().await {
            Ok(()) => "WebRTC peer connection terminated".to_owned(),
            Err(error) => format!("WebRTC connection watcher failed: {error}"),
        };
        fail_webrtc(&core, &peer, reason).await;
    });
    owner.register_task(task);
}

async fn refresh_webrtc_route(core: &SessionMuxCore, peer: &WebRtcPeerConnection) {
    let Some(stats) = peer.selected_candidate_pair_stats().await else {
        return;
    };
    if stats.local_candidate_kind == mrd_transport_webrtc::CandidateKind::Unknown
        || stats.remote_candidate_kind == mrd_transport_webrtc::CandidateKind::Unknown
    {
        return;
    }
    let kind = if stats.local_candidate_kind == mrd_transport_webrtc::CandidateKind::Relay
        || stats.remote_candidate_kind == mrd_transport_webrtc::CandidateKind::Relay
    {
        TransportRouteKind::WebRtcRelay
    } else {
        TransportRouteKind::WebRtcDirect
    };
    core.update_route(
        kind,
        stats.local_candidate_id,
        stats.remote_candidate_id,
        Some(format!("{:?}", stats.local_candidate_kind).to_ascii_lowercase()),
        Some(format!("{:?}", stats.remote_candidate_kind).to_ascii_lowercase()),
    )
    .await;
}

fn spawn_webrtc_senders(core: Arc<SessionMuxCore>, peer: Arc<WebRtcPeerConnection>) {
    for lane in TransportLane::ALL {
        let source = Arc::clone(&core);
        let peer = Arc::clone(&peer);
        let task = tokio::spawn(async move {
            while let Some(envelope) = source.next_outbound(lane).await {
                let result = match lane {
                    TransportLane::Video => {
                        let Some(metadata) = envelope.video else {
                            break;
                        };
                        if metadata.codec != "h264" {
                            break;
                        }
                        peer.send_h264_access_unit(&EncodedAccessUnit {
                            codec: VideoCodec::H264,
                            timestamp_us: metadata.timestamp_us,
                            is_keyframe: metadata.keyframe,
                            bytes: envelope.payload,
                        })
                        .await
                    }
                    TransportLane::ControlReliable => {
                        send_data_envelope(&peer, ControlLane::Reliable, &envelope).await
                    }
                    TransportLane::ControlRealtime => {
                        send_data_envelope(&peer, ControlLane::Realtime, &envelope).await
                    }
                    TransportLane::Bulk => {
                        send_data_envelope(&peer, ControlLane::Bulk, &envelope).await
                    }
                };
                if let Err(error) = result {
                    fail_webrtc(
                        &source,
                        &peer,
                        format!("WebRTC {lane:?} sender failed: {error}"),
                    )
                    .await;
                    break;
                }
            }
        });
        core.register_task(task);
    }
}

async fn send_data_envelope(
    peer: &WebRtcPeerConnection,
    lane: ControlLane,
    envelope: &TransportEnvelope,
) -> Result<usize, mrd_transport_webrtc::TransportError> {
    let payload = quic::encode_envelope(envelope)
        .map_err(|error| mrd_transport_webrtc::TransportError::Message(error.to_string()))?;
    let fragments = fragment_data_envelope(envelope.sequence, &payload)
        .map_err(mrd_transport_webrtc::TransportError::Message)?;
    if lane == ControlLane::Realtime && fragments.len() != 1 {
        return Err(mrd_transport_webrtc::TransportError::Message(
            "realtime WebRTC envelope exceeds one data-channel message".into(),
        ));
    }
    let mut bytes_sent = 0_usize;
    for fragment in fragments {
        bytes_sent = bytes_sent.saturating_add(peer.send_control(lane, &fragment).await?);
    }
    Ok(bytes_sent)
}

fn fragment_data_envelope(message_id: u64, payload: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let fragment_count = payload.len().max(1).div_ceil(DATA_FRAGMENT_PAYLOAD_LEN);
    let fragment_count = u16::try_from(fragment_count)
        .map_err(|_| "WebRTC data envelope requires too many fragments".to_owned())?;
    let total_len = u32::try_from(payload.len())
        .map_err(|_| "WebRTC data envelope exceeds wire length".to_owned())?;
    let mut fragments = Vec::with_capacity(fragment_count as usize);
    for (index, chunk) in payload.chunks(DATA_FRAGMENT_PAYLOAD_LEN).enumerate() {
        let mut encoded = Vec::with_capacity(DATA_FRAGMENT_HEADER_LEN + chunk.len());
        encoded.extend_from_slice(DATA_FRAGMENT_MAGIC);
        encoded.push(DATA_FRAGMENT_VERSION);
        encoded.extend_from_slice(&message_id.to_le_bytes());
        encoded.extend_from_slice(&(index as u16).to_le_bytes());
        encoded.extend_from_slice(&fragment_count.to_le_bytes());
        encoded.extend_from_slice(&total_len.to_le_bytes());
        encoded.extend_from_slice(chunk);
        fragments.push(encoded);
    }
    Ok(fragments)
}

#[derive(Debug)]
struct DataEnvelopeReassembler {
    max_total_len: usize,
    current: Option<PartialDataEnvelope>,
}

#[derive(Debug)]
struct PartialDataEnvelope {
    message_id: u64,
    fragment_count: u16,
    next_index: u16,
    total_len: usize,
    payload: Vec<u8>,
}

impl DataEnvelopeReassembler {
    fn new(max_total_len: usize) -> Self {
        Self {
            max_total_len: max_total_len.max(1),
            current: None,
        }
    }

    fn push(&mut self, fragment: &[u8]) -> Result<Option<Bytes>> {
        if fragment.len() < DATA_FRAGMENT_HEADER_LEN
            || &fragment[..4] != DATA_FRAGMENT_MAGIC
            || fragment[4] != DATA_FRAGMENT_VERSION
        {
            bail!("invalid WebRTC data fragment header");
        }
        let message_id = u64::from_le_bytes(fragment[5..13].try_into()?);
        let fragment_index = u16::from_le_bytes(fragment[13..15].try_into()?);
        let fragment_count = u16::from_le_bytes(fragment[15..17].try_into()?);
        let total_len = u32::from_le_bytes(fragment[17..21].try_into()?) as usize;
        if fragment_count == 0
            || fragment_index >= fragment_count
            || total_len > self.max_total_len
            || total_len.max(1).div_ceil(DATA_FRAGMENT_PAYLOAD_LEN) != fragment_count as usize
        {
            bail!("invalid WebRTC data fragment bounds");
        }
        let chunk = &fragment[DATA_FRAGMENT_HEADER_LEN..];
        let expected_chunk_len = if fragment_index + 1 == fragment_count {
            total_len.saturating_sub(
                DATA_FRAGMENT_PAYLOAD_LEN.saturating_mul(fragment_count.saturating_sub(1) as usize),
            )
        } else {
            DATA_FRAGMENT_PAYLOAD_LEN
        };
        if chunk.len() != expected_chunk_len {
            bail!("WebRTC data fragment payload length mismatch");
        }
        if self.current.is_none() {
            if fragment_index != 0 {
                bail!("WebRTC data envelope does not start at fragment zero");
            }
            self.current = Some(PartialDataEnvelope {
                message_id,
                fragment_count,
                next_index: 0,
                total_len,
                payload: Vec::with_capacity(total_len),
            });
        }
        let current = self.current.as_mut().expect("partial envelope initialized");
        if current.message_id != message_id
            || current.fragment_count != fragment_count
            || current.total_len != total_len
            || current.next_index != fragment_index
            || current.payload.len().saturating_add(chunk.len()) > current.total_len
        {
            self.current = None;
            bail!("inconsistent or reordered WebRTC data fragments");
        }
        current.payload.extend_from_slice(chunk);
        current.next_index = current.next_index.saturating_add(1);
        if current.next_index != current.fragment_count {
            return Ok(None);
        }
        let complete = self.current.take().expect("completed partial envelope");
        if complete.payload.len() != complete.total_len {
            bail!("WebRTC data envelope length mismatch");
        }
        Ok(Some(Bytes::from(complete.payload)))
    }
}

fn spawn_webrtc_receivers(
    core: Arc<SessionMuxCore>,
    peer: Arc<WebRtcPeerConnection>,
    config: TransportMuxConfig,
) {
    let max_payload_len = config.max_payload_len;
    let video_core = Arc::clone(&core);
    let video_peer = Arc::clone(&peer);
    let video_sequence = Arc::new(AtomicU64::new(0));
    let video_task = tokio::spawn(async move {
        while let Some(access_unit) = video_peer.next_h264_access_unit().await {
            flush_webrtc_video_drops(&video_core, &video_peer);
            let envelope = TransportEnvelope {
                session_id: video_core.session_id().clone(),
                lane: TransportLane::Video,
                sequence: video_sequence.fetch_add(1, Ordering::Relaxed),
                payload: access_unit.bytes,
                video: Some(VideoEnvelopeMetadata {
                    codec: "h264".into(),
                    timestamp_us: access_unit.timestamp_us,
                    keyframe: access_unit.is_keyframe,
                    width: 0,
                    height: 0,
                }),
            };
            if let Err(error) = video_core.deliver(envelope).await {
                fail_webrtc(
                    &video_core,
                    &video_peer,
                    format!("WebRTC video delivery failed: {error}"),
                )
                .await;
                return;
            }
        }
        flush_webrtc_video_drops(&video_core, &video_peer);
        fail_webrtc(
            &video_core,
            &video_peer,
            "WebRTC video receiver closed by peer".into(),
        )
        .await;
    });
    core.register_task(video_task);

    for (lane, control_lane) in [
        (TransportLane::ControlReliable, ControlLane::Reliable),
        (TransportLane::ControlRealtime, ControlLane::Realtime),
        (TransportLane::Bulk, ControlLane::Bulk),
    ] {
        let target = Arc::clone(&core);
        let peer = Arc::clone(&peer);
        let task = tokio::spawn(async move {
            let mut reassembler = DataEnvelopeReassembler::new(
                max_payload_len
                    .min(config.byte_capacity(lane))
                    .saturating_add(MAX_ENVELOPE_WIRE_OVERHEAD),
            );
            while let Some(payload) = peer.next_control(control_lane).await {
                let payload = match reassembler.push(&payload) {
                    Ok(Some(payload)) => payload,
                    Ok(None) => continue,
                    Err(error) => {
                        fail_webrtc(
                            &target,
                            &peer,
                            format!("WebRTC {lane:?} fragment invalid: {error}"),
                        )
                        .await;
                        return;
                    }
                };
                let Ok(envelope) = quic::decode_envelope(&payload, max_payload_len) else {
                    continue;
                };
                if envelope.lane != lane {
                    continue;
                }
                if let Err(error) = target.deliver(envelope).await {
                    fail_webrtc(
                        &target,
                        &peer,
                        format!("WebRTC {lane:?} delivery failed: {error}"),
                    )
                    .await;
                    return;
                }
            }
            fail_webrtc(
                &target,
                &peer,
                format!("WebRTC {lane:?} receiver closed by peer"),
            )
            .await;
        });
        core.register_task(task);
    }
}

#[async_trait::async_trait]
impl TransportMuxPort for WebRtcTransportMux {
    async fn send(&self, envelope: TransportEnvelope) -> Result<TransportSendOutcome> {
        if let Some(metadata) = &envelope.video {
            if metadata.codec != "h264" {
                bail!("WebRTC mux does not support video codec {}", metadata.codec);
            }
        }
        if envelope.lane == TransportLane::ControlRealtime
            && quic::encode_envelope(&envelope)?.len() > DATA_FRAGMENT_PAYLOAD_LEN
        {
            bail!("WebRTC realtime envelope exceeds one data-channel message");
        }
        self.core.submit(envelope).await
    }

    async fn recv(&self, lane: TransportLane) -> Result<Option<TransportEnvelope>> {
        self.core.recv(lane).await
    }

    async fn route_snapshot(&self) -> TransportRouteSnapshot {
        refresh_webrtc_route(&self.core, &self.peer).await;
        flush_webrtc_video_drops(&self.core, &self.peer);
        self.core.snapshot().await
    }

    async fn close(&self) -> Result<()> {
        flush_webrtc_video_drops(&self.core, &self.peer);
        self.core.close().await;
        let close_result = self.peer.close().await;
        flush_webrtc_video_drops(&self.core, &self.peer);
        close_result?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_transport_webrtc::PeerConnectionRole;

    fn credentials() -> ServiceTurnRelayCredentials {
        ServiceTurnRelayCredentials {
            urls: vec!["turn:relay.example.test:3478?transport=udp".into()],
            username: "temporary-user".into(),
            credential: "temporary-password".into(),
            expires_at_unix_seconds: 1_800_000_000,
        }
    }

    #[test]
    fn relay_credentials_force_relay_policy_without_debug_secret_leakage() {
        let credentials = credentials();
        let config = credentials.apply_relay_only(PeerConnectionConfig {
            role: PeerConnectionRole::Offerer,
            ..PeerConnectionConfig::default()
        });
        assert_eq!(config.ice_transport_policy, IceTransportPolicy::Relay);
        assert_eq!(config.ice_servers.len(), 1);
        assert_eq!(credentials.url_classes(), vec![RelayUrlClass::TurnUdp]);

        let debug = format!("{credentials:?}");
        assert!(!debug.contains("temporary-user"));
        assert!(!debug.contains("temporary-password"));
        assert!(debug.contains("TurnUdp"));
    }

    #[test]
    fn data_fragmentation_round_trips_payload_larger_than_sctp_message() {
        let payload = vec![0x5a; 160 * 1024];
        let fragments = fragment_data_envelope(7, &payload).expect("fragment data envelope");
        assert!(fragments.len() > 1);
        assert!(fragments.iter().all(|fragment| fragment.len() <= 65_535));

        let mut reassembler = DataEnvelopeReassembler::new(payload.len());
        let mut completed = None;
        for fragment in fragments {
            completed = reassembler
                .push(&fragment)
                .expect("reassemble ordered data fragment")
                .or(completed);
        }
        assert_eq!(completed.expect("completed payload").as_ref(), payload);
    }

    #[test]
    fn data_reassembler_rejects_nonzero_first_fragment() {
        let payload = vec![0x33; 128 * 1024];
        let fragments = fragment_data_envelope(8, &payload).expect("fragment data envelope");
        let mut reassembler = DataEnvelopeReassembler::new(payload.len());

        let error = reassembler
            .push(&fragments[1])
            .expect_err("out-of-order first fragment must fail closed");
        assert!(error.to_string().contains("fragment zero"));
    }

    #[tokio::test]
    async fn service_host_owns_exactly_one_mux_and_closes_it_with_the_session() {
        let host = ServiceWebRtcTransportHost::new();
        let session_id = SessionId("host-owned-mux".into());
        host.open_session(
            session_id.clone(),
            PeerConnectionConfig {
                role: PeerConnectionRole::Offerer,
                ..PeerConnectionConfig::default()
            },
        )
        .await
        .expect("open WebRTC session");

        let first = host
            .transport_mux(&session_id)
            .await
            .expect("first mux handle");
        let second = host
            .transport_mux(&session_id)
            .await
            .expect("second mux handle");
        assert!(Arc::ptr_eq(&first, &second));

        host.close_session(&session_id)
            .await
            .expect("host closes session mux");
        assert_eq!(
            first
                .send(TransportEnvelope {
                    session_id,
                    lane: TransportLane::ControlReliable,
                    sequence: 1,
                    payload: vec![1],
                    video: None,
                })
                .await
                .expect("closed send outcome"),
            TransportSendOutcome::Closed
        );
    }
}
