//! QUIC implementation of the logical session transport mux.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportMuxPort, TransportRouteKind, TransportRouteSnapshot,
    TransportSendOutcome, VideoEnvelopeMetadata,
};
use mrd_proto::SessionId;
use mrd_transport_quic_quinn::{
    fragment_media_payload_v3, is_quic_media_v3_datagram, QuicAuReassemblerConfig, QuicMediaCodec,
    QuicMediaPayloadType, QuicMediaReassembler, QuinnDatagramEndpoint, QuinnDatagramPair,
    QuinnReliableLane,
};

use super::{SessionMuxCore, TransportMuxConfig};
use tokio::sync::{mpsc, Mutex};

const MAGIC: &[u8; 4] = b"MRMX";
const VERSION: u8 = 1;
const MAX_WIRE_OVERHEAD: usize = 38 + u16::MAX as usize + u8::MAX as usize;
const VIDEO_REORDER_DELAY: Duration = Duration::from_millis(100);

/// Session transport mux backed by one Quinn connection.
#[derive(Debug)]
pub struct QuicTransportMux {
    core: Arc<SessionMuxCore>,
    endpoint: QuinnDatagramEndpoint,
    passthrough_rx: Arc<Mutex<mpsc::Receiver<Bytes>>>,
}

impl QuicTransportMux {
    /// Create a connected localhost pair for conformance and component tests.
    pub async fn loopback(
        session_id: SessionId,
        config: TransportMuxConfig,
    ) -> Result<(Self, Self)> {
        let pair = QuinnDatagramPair::loopback()
            .await
            .context("create QUIC loopback pair")?;
        let client = Self::new(session_id.clone(), config, pair.client);
        let server = Self::new(session_id, config, pair.server);
        Ok((client, server))
    }

    /// Wrap an established Quinn endpoint in a logical session mux.
    pub fn new(
        session_id: SessionId,
        config: TransportMuxConfig,
        endpoint: QuinnDatagramEndpoint,
    ) -> Self {
        let metadata = endpoint.metadata();
        let core = SessionMuxCore::new(
            session_id,
            config,
            TransportRouteKind::QuicLan,
            metadata.local_addr.to_string(),
            metadata.peer_addr.to_string(),
        );
        let (passthrough_tx, passthrough_rx) = mpsc::channel(config.lane_capacity.max(1));
        spawn_senders(Arc::clone(&core), endpoint.clone(), config.max_payload_len);
        spawn_receivers(
            Arc::clone(&core),
            endpoint.clone(),
            config.max_payload_len,
            config.video_byte_capacity,
            passthrough_tx,
        );
        Self {
            core,
            endpoint,
            passthrough_rx: Arc::new(Mutex::new(passthrough_rx)),
        }
    }

    /// Receive a bounded legacy datagram that does not belong to a mux lane.
    ///
    /// This is an infrastructure compatibility bridge for LAN telemetry and
    /// keyframe requests; application use cases still see only `TransportMuxPort`.
    pub async fn recv_passthrough_datagram(&self) -> Option<Bytes> {
        self.passthrough_rx.lock().await.recv().await
    }

    /// Send a legacy LAN telemetry/control datagram through the adapter-owned endpoint.
    pub async fn send_passthrough_datagram(&self, payload: Bytes) -> Result<()> {
        self.endpoint.send_datagram_wait(payload).await?;
        Ok(())
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.endpoint.max_datagram_size()
    }

    pub fn close_immediately(&self, reason: &[u8]) {
        self.endpoint.close_immediately(reason);
    }
}

impl Drop for QuicTransportMux {
    fn drop(&mut self) {
        self.core.terminate_now(None);
        self.endpoint
            .close_immediately(b"transport mux owner dropped");
    }
}

async fn fail_quic(core: &SessionMuxCore, endpoint: &QuinnDatagramEndpoint, reason: String) {
    endpoint.close_immediately(b"transport mux failure");
    core.fail(reason).await;
}

fn spawn_senders(
    core: Arc<SessionMuxCore>,
    endpoint: QuinnDatagramEndpoint,
    max_payload_len: usize,
) {
    for lane in TransportLane::ALL {
        let source = Arc::clone(&core);
        let endpoint = endpoint.clone();
        let task = tokio::spawn(async move {
            while let Some(envelope) = source.next_outbound(lane).await {
                if let Err(error) = send_quic_envelope(&endpoint, envelope, max_payload_len).await {
                    fail_quic(
                        &source,
                        &endpoint,
                        format!("QUIC {lane:?} sender failed: {error}"),
                    )
                    .await;
                    break;
                }
            }
        });
        core.register_task(task);
    }
}

async fn send_quic_envelope(
    endpoint: &QuinnDatagramEndpoint,
    envelope: TransportEnvelope,
    max_payload_len: usize,
) -> Result<()> {
    let payload = Bytes::from(encode_envelope(&envelope)?);
    match envelope.lane {
        TransportLane::Video => {
            let metadata = envelope.video.as_ref().expect("validated video metadata");
            let codec = match metadata.codec.as_str() {
                "h264" => QuicMediaCodec::H264,
                "hevc" => QuicMediaCodec::Hevc,
                "av1" => QuicMediaCodec::Av1,
                codec => bail!("unsupported QUIC video codec {codec}"),
            };
            if metadata.keyframe {
                endpoint
                    .send_reliable_lane_message(QuinnReliableLane::Video, payload)
                    .await?;
            } else {
                let fragments = fragment_media_payload_v3(
                    QuicMediaPayloadType::AccessUnit,
                    codec,
                    0,
                    envelope.sequence as u32,
                    metadata.timestamp_us,
                    false,
                    &payload,
                    endpoint.max_datagram_size().unwrap_or(1200),
                )?;
                for fragment in fragments {
                    endpoint.send_datagram_wait(fragment).await?;
                }
            }
        }
        TransportLane::ControlRealtime => endpoint.send_datagram_wait(payload).await?,
        TransportLane::ControlReliable => {
            endpoint
                .send_reliable_lane_message(QuinnReliableLane::Control, payload)
                .await?;
        }
        TransportLane::Bulk => {
            endpoint
                .send_reliable_lane_message(QuinnReliableLane::Bulk, payload)
                .await?;
        }
    }
    if envelope.payload.len() > max_payload_len {
        bail!("transport payload exceeds configured limit");
    }
    Ok(())
}

fn spawn_receivers(
    core: Arc<SessionMuxCore>,
    endpoint: QuinnDatagramEndpoint,
    max_payload_len: usize,
    video_byte_capacity: usize,
    passthrough_tx: mpsc::Sender<Bytes>,
) {
    let (video_tx, video_rx) = mpsc::channel(1);
    spawn_video_orderer(
        Arc::clone(&core),
        endpoint.clone(),
        video_rx,
        video_byte_capacity,
        core.configured_lane_capacity(),
    );
    let datagram_core = Arc::clone(&core);
    let datagram_endpoint = endpoint.clone();
    let datagram_video_tx = video_tx.clone();
    let datagram_task = tokio::spawn(async move {
        let mut media = QuicMediaReassembler::new(QuicAuReassemblerConfig::default())
            .with_max_frame_bytes(max_payload_len.saturating_add(MAX_WIRE_OVERHEAD))
            .with_max_total_bytes(video_byte_capacity.saturating_add(MAX_WIRE_OVERHEAD));
        loop {
            let payload = match datagram_endpoint.read_datagram().await {
                Ok(payload) => payload,
                Err(error) => {
                    fail_quic(
                        &datagram_core,
                        &datagram_endpoint,
                        format!("QUIC datagram receiver failed: {error}"),
                    )
                    .await;
                    break;
                }
            };
            if is_quic_media_v3_datagram(&payload) {
                let raw_payload = payload.clone();
                let Ok(Some(frame)) = media.push_datagram(&payload) else {
                    continue;
                };
                if frame.payload_type != QuicMediaPayloadType::AccessUnit {
                    continue;
                }
                let Ok(envelope) = decode_envelope(&frame.payload, max_payload_len) else {
                    let _ = passthrough_tx.try_send(raw_payload);
                    continue;
                };
                if envelope.lane != TransportLane::Video {
                    continue;
                }
                if datagram_video_tx.send(envelope).await.is_err() {
                    fail_quic(
                        &datagram_core,
                        &datagram_endpoint,
                        "QUIC video ordering input closed".into(),
                    )
                    .await;
                    break;
                }
                continue;
            }
            let Ok(envelope) = decode_envelope(&payload, max_payload_len) else {
                let _ = passthrough_tx.try_send(payload);
                continue;
            };
            if envelope.lane == TransportLane::ControlRealtime {
                if let Err(error) = datagram_core.deliver(envelope).await {
                    fail_quic(
                        &datagram_core,
                        &datagram_endpoint,
                        format!("QUIC realtime delivery failed: {error}"),
                    )
                    .await;
                    break;
                }
            } else {
                let _ = passthrough_tx.try_send(payload);
            }
        }
    });
    core.register_task(datagram_task);

    for (lane, quic_lane) in [
        (TransportLane::Video, QuinnReliableLane::Video),
        (TransportLane::ControlReliable, QuinnReliableLane::Control),
        (TransportLane::Bulk, QuinnReliableLane::Bulk),
    ] {
        let target = Arc::clone(&core);
        let endpoint = endpoint.clone();
        let reliable_video_tx = video_tx.clone();
        let task = tokio::spawn(async move {
            loop {
                let payload = match endpoint
                    .read_reliable_lane_message(
                        quic_lane,
                        max_payload_len.saturating_add(MAX_WIRE_OVERHEAD),
                    )
                    .await
                {
                    Ok(payload) => payload,
                    Err(error) => {
                        fail_quic(
                            &target,
                            &endpoint,
                            format!("QUIC {lane:?} receiver failed: {error}"),
                        )
                        .await;
                        break;
                    }
                };
                let envelope = match decode_envelope(&payload, max_payload_len) {
                    Ok(envelope) if envelope.lane == lane => envelope,
                    Ok(_) => continue,
                    Err(error) => {
                        fail_quic(
                            &target,
                            &endpoint,
                            format!("QUIC {lane:?} frame invalid: {error}"),
                        )
                        .await;
                        break;
                    }
                };
                let delivery = if lane == TransportLane::Video {
                    reliable_video_tx
                        .send(envelope)
                        .await
                        .map_err(|_| anyhow::anyhow!("video ordering input closed"))
                } else {
                    target.deliver(envelope).await
                };
                if let Err(error) = delivery {
                    fail_quic(
                        &target,
                        &endpoint,
                        format!("QUIC {lane:?} delivery failed: {error}"),
                    )
                    .await;
                    break;
                }
            }
        });
        core.register_task(task);
    }
}

fn spawn_video_orderer(
    core: Arc<SessionMuxCore>,
    endpoint: QuinnDatagramEndpoint,
    mut input: mpsc::Receiver<TransportEnvelope>,
    byte_capacity: usize,
    envelope_capacity: usize,
) {
    let owner = Arc::clone(&core);
    let task = tokio::spawn(async move {
        let mut pending = BTreeMap::<u64, (Instant, TransportEnvelope)>::new();
        let mut pending_bytes = 0_usize;
        let mut last_delivered = None::<u64>;

        loop {
            while let Some(next) = last_delivered
                .and_then(|sequence| sequence.checked_add(1))
                .and_then(|sequence| pending.remove(&sequence))
            {
                pending_bytes = pending_bytes.saturating_sub(next.1.payload.len());
                if let Err(error) = core.deliver(next.1).await {
                    fail_quic(
                        &core,
                        &endpoint,
                        format!("QUIC ordered video delivery failed: {error}"),
                    )
                    .await;
                    return;
                }
                last_delivered = last_delivered.and_then(|value| value.checked_add(1));
            }

            let over_budget =
                pending.len() > envelope_capacity.max(1) || pending_bytes > byte_capacity.max(1);
            let deadline = pending
                .values()
                .map(|(received_at, _)| *received_at + VIDEO_REORDER_DELAY)
                .min();
            let initial_keyframe_ready = last_delivered.is_none()
                && pending
                    .first_key_value()
                    .and_then(|(_, (_, envelope))| envelope.video.as_ref())
                    .is_some_and(|metadata| metadata.keyframe);
            if over_budget
                || initial_keyframe_ready
                || deadline.is_some_and(|deadline| deadline <= Instant::now())
            {
                let Some((sequence, (_, envelope))) = pending.pop_first() else {
                    continue;
                };
                pending_bytes = pending_bytes.saturating_sub(envelope.payload.len());
                if last_delivered.is_some_and(|last| sequence <= last) {
                    continue;
                }
                if let Err(error) = core.deliver(envelope).await {
                    fail_quic(
                        &core,
                        &endpoint,
                        format!("QUIC ordered video delivery failed: {error}"),
                    )
                    .await;
                    return;
                }
                last_delivered = Some(sequence);
                continue;
            }

            let received = if let Some(deadline) = deadline {
                tokio::select! {
                    received = input.recv() => received,
                    _ = tokio::time::sleep_until(deadline.into()) => continue,
                }
            } else {
                input.recv().await
            };
            let Some(envelope) = received else {
                while let Some((sequence, (_, envelope))) = pending.pop_first() {
                    if last_delivered.is_some_and(|last| sequence <= last) {
                        continue;
                    }
                    if core.deliver(envelope).await.is_err() {
                        return;
                    }
                    last_delivered = Some(sequence);
                }
                return;
            };
            if last_delivered.is_some_and(|last| envelope.sequence <= last)
                || pending.contains_key(&envelope.sequence)
            {
                continue;
            }
            pending_bytes = pending_bytes.saturating_add(envelope.payload.len());
            pending.insert(envelope.sequence, (Instant::now(), envelope));
        }
    });
    owner.register_task(task);
}

pub(crate) fn encode_envelope(envelope: &TransportEnvelope) -> Result<Vec<u8>> {
    let session = envelope.session_id.0.as_bytes();
    let session_len = u16::try_from(session.len()).context("session identifier too long")?;
    let payload_len = u32::try_from(envelope.payload.len()).context("payload too long")?;
    let codec = envelope
        .video
        .as_ref()
        .map(|metadata| metadata.codec.as_bytes())
        .unwrap_or_default();
    let codec_len = u8::try_from(codec.len()).context("codec identifier too long")?;
    let lane = match envelope.lane {
        TransportLane::Video => 0,
        TransportLane::ControlReliable => 1,
        TransportLane::ControlRealtime => 2,
        TransportLane::Bulk => 3,
    };
    let mut encoded = Vec::with_capacity(40 + session.len() + codec.len() + envelope.payload.len());
    encoded.extend_from_slice(MAGIC);
    encoded.push(VERSION);
    encoded.push(lane);
    encoded.extend_from_slice(&envelope.sequence.to_le_bytes());
    encoded.extend_from_slice(&session_len.to_le_bytes());
    encoded.push(codec_len);
    if let Some(video) = &envelope.video {
        encoded.extend_from_slice(&video.timestamp_us.to_le_bytes());
        encoded.push(u8::from(video.keyframe));
        encoded.extend_from_slice(&video.width.to_le_bytes());
        encoded.extend_from_slice(&video.height.to_le_bytes());
    } else {
        encoded.extend_from_slice(&0_u64.to_le_bytes());
        encoded.push(0);
        encoded.extend_from_slice(&0_u32.to_le_bytes());
        encoded.extend_from_slice(&0_u32.to_le_bytes());
    }
    encoded.extend_from_slice(&payload_len.to_le_bytes());
    encoded.extend_from_slice(session);
    encoded.extend_from_slice(codec);
    encoded.extend_from_slice(&envelope.payload);
    Ok(encoded)
}

pub(crate) fn decode_envelope(encoded: &[u8], max_payload_len: usize) -> Result<TransportEnvelope> {
    const FIXED: usize = 4 + 1 + 1 + 8 + 2 + 1 + 8 + 1 + 4 + 4 + 4;
    if encoded.len() < FIXED || &encoded[..4] != MAGIC || encoded[4] != VERSION {
        bail!("invalid transport mux frame header");
    }
    let lane = match encoded[5] {
        0 => TransportLane::Video,
        1 => TransportLane::ControlReliable,
        2 => TransportLane::ControlRealtime,
        3 => TransportLane::Bulk,
        _ => bail!("unknown transport lane"),
    };
    let sequence = u64::from_le_bytes(encoded[6..14].try_into()?);
    let session_len = u16::from_le_bytes(encoded[14..16].try_into()?) as usize;
    let codec_len = encoded[16] as usize;
    let timestamp_us = u64::from_le_bytes(encoded[17..25].try_into()?);
    let keyframe = encoded[25] != 0;
    let width = u32::from_le_bytes(encoded[26..30].try_into()?);
    let height = u32::from_le_bytes(encoded[30..34].try_into()?);
    let payload_len = u32::from_le_bytes(encoded[34..38].try_into()?) as usize;
    if payload_len > max_payload_len {
        bail!("transport mux payload exceeds configured limit");
    }
    let session_start = FIXED;
    let codec_start = session_start
        .checked_add(session_len)
        .context("frame overflow")?;
    let payload_start = codec_start
        .checked_add(codec_len)
        .context("frame overflow")?;
    let end = payload_start
        .checked_add(payload_len)
        .context("frame overflow")?;
    if end != encoded.len() {
        bail!("transport mux frame length mismatch");
    }
    let session_id = SessionId(String::from_utf8(
        encoded[session_start..codec_start].to_vec(),
    )?);
    let video = if lane == TransportLane::Video {
        Some(VideoEnvelopeMetadata {
            codec: String::from_utf8(encoded[codec_start..payload_start].to_vec())?,
            timestamp_us,
            keyframe,
            width,
            height,
        })
    } else {
        if codec_len != 0 {
            bail!("non-video frame carries video metadata");
        }
        None
    };
    Ok(TransportEnvelope {
        session_id,
        lane,
        sequence,
        payload: encoded[payload_start..end].to_vec(),
        video,
    })
}

#[async_trait::async_trait]
impl TransportMuxPort for QuicTransportMux {
    async fn send(&self, envelope: TransportEnvelope) -> Result<TransportSendOutcome> {
        if let Some(metadata) = &envelope.video {
            if !matches!(metadata.codec.as_str(), "h264" | "hevc" | "av1") {
                bail!("QUIC mux does not support video codec {}", metadata.codec);
            }
        }
        self.core.submit(envelope).await
    }

    async fn recv(&self, lane: TransportLane) -> Result<Option<TransportEnvelope>> {
        self.core.recv(lane).await
    }

    async fn route_snapshot(&self) -> TransportRouteSnapshot {
        self.core.snapshot().await
    }

    async fn close(&self) -> Result<()> {
        self.core.close().await;
        self.endpoint.close_immediately(b"transport mux closed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_frame_round_trips_video_metadata() {
        let envelope = TransportEnvelope {
            session_id: SessionId("session".into()),
            lane: TransportLane::Video,
            sequence: 42,
            payload: vec![1, 2, 3],
            video: Some(VideoEnvelopeMetadata {
                codec: "h264".into(),
                timestamp_us: 99,
                keyframe: true,
                width: 1280,
                height: 720,
            }),
        };
        let encoded = encode_envelope(&envelope).expect("encode");
        assert_eq!(decode_envelope(&encoded, 1024).expect("decode"), envelope);
    }

    #[tokio::test]
    async fn video_orderer_merges_reliable_and_datagram_paths_by_sequence() {
        let session_id = SessionId("cross-path-video-order".into());
        let config = TransportMuxConfig::test();
        let pair = QuinnDatagramPair::loopback()
            .await
            .expect("create cross-path QUIC pair");
        let mux = QuicTransportMux::new(session_id.clone(), config, pair.server);
        let video = |sequence, keyframe, marker| TransportEnvelope {
            session_id: session_id.clone(),
            lane: TransportLane::Video,
            sequence,
            payload: vec![0, 0, 0, 1, marker],
            video: Some(VideoEnvelopeMetadata {
                codec: "h264".into(),
                timestamp_us: sequence * 1_000,
                keyframe,
                width: 1280,
                height: 720,
            }),
        };

        send_quic_envelope(&pair.client, video(8, false, 0x41), config.max_payload_len)
            .await
            .expect("send later datagram video first");
        send_quic_envelope(&pair.client, video(7, true, 0x65), config.max_payload_len)
            .await
            .expect("send earlier reliable keyframe second");

        let first = tokio::time::timeout(Duration::from_secs(1), mux.recv(TransportLane::Video))
            .await
            .expect("first ordered video timeout")
            .expect("first ordered video receive")
            .expect("first ordered video envelope");
        let second = tokio::time::timeout(Duration::from_secs(1), mux.recv(TransportLane::Video))
            .await
            .expect("second ordered video timeout")
            .expect("second ordered video receive")
            .expect("second ordered video envelope");
        assert_eq!((first.sequence, second.sequence), (7, 8));
    }
}
