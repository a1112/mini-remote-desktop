use std::{sync::Arc, time::Duration};

use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportMuxPort, TransportRouteKind, TransportSendOutcome,
    VideoEnvelopeMetadata,
};
use mrd_proto::SessionId;
use mrd_service::transports::{
    memory::MemoryTransportMux, quic::QuicTransportMux, webrtc::WebRtcTransportMux,
    TransportMuxConfig,
};

const WAIT: Duration = Duration::from_secs(10);

type Mux = Arc<dyn TransportMuxPort>;

fn envelope(
    session_id: &SessionId,
    lane: TransportLane,
    sequence: u64,
    payload: impl Into<Vec<u8>>,
) -> TransportEnvelope {
    TransportEnvelope {
        session_id: session_id.clone(),
        lane,
        sequence,
        payload: payload.into(),
        video: None,
    }
}

async fn receive(mux: &Mux, lane: TransportLane) -> TransportEnvelope {
    let received = match tokio::time::timeout(WAIT, mux.recv(lane)).await {
        Ok(received) => received.expect("lane receive failed"),
        Err(_) => {
            let snapshot = mux.route_snapshot().await;
            panic!(
                "lane {lane:?} receive timed out; closed={}, last_error={:?}",
                snapshot.closed, snapshot.last_error
            );
        }
    };
    match received {
        Some(envelope) => envelope,
        None => {
            let snapshot = mux.route_snapshot().await;
            panic!(
                "lane {lane:?} closed before the expected envelope; last_error={:?}",
                snapshot.last_error
            );
        }
    }
}

async fn wait_for_received(mux: &Mux, lane: TransportLane, expected: u64) {
    tokio::time::timeout(WAIT, async {
        loop {
            let snapshot = mux.route_snapshot().await;
            if snapshot.lane(lane).received >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("route counters did not observe the expected messages");
}

async fn assert_transport_mux_conformance(
    session_id: SessionId,
    left: Mux,
    right: Mux,
    expected_route: TransportRouteKind,
) {
    let mut video_payload = vec![0x5a; 128 * 1024];
    video_payload[..5].copy_from_slice(&[0, 0, 0, 1, 0x65]);
    let video = TransportEnvelope {
        session_id: session_id.clone(),
        lane: TransportLane::Video,
        sequence: 7,
        payload: video_payload,
        video: Some(VideoEnvelopeMetadata {
            codec: "h264".into(),
            timestamp_us: 42_000,
            keyframe: true,
            width: 1920,
            height: 1080,
        }),
    };
    assert_eq!(
        left.send(video.clone()).await.expect("send video"),
        TransportSendOutcome::Enqueued
    );
    let second_video = TransportEnvelope {
        session_id: session_id.clone(),
        lane: TransportLane::Video,
        sequence: 8,
        payload: vec![0, 0, 0, 1, 0x41, 0x11, 0x22, 0x33],
        video: Some(VideoEnvelopeMetadata {
            codec: "h264".into(),
            timestamp_us: 43_000,
            keyframe: false,
            width: 1920,
            height: 1080,
        }),
    };
    assert_eq!(
        left.send(second_video.clone())
            .await
            .expect("send second video"),
        TransportSendOutcome::Enqueued
    );

    let received_video = receive(&right, TransportLane::Video).await;
    let first_received_video_sequence = received_video.sequence;
    assert_eq!(received_video.session_id, video.session_id);
    assert_eq!(received_video.lane, video.lane);
    assert_eq!(received_video.payload, video.payload);
    let received_metadata = received_video.video.expect("received video metadata");
    let sent_metadata = video.video.expect("sent video metadata");
    assert_eq!(received_metadata.codec, sent_metadata.codec);
    assert!(received_metadata.timestamp_us > 0);
    assert_eq!(received_metadata.keyframe, sent_metadata.keyframe);
    assert!(
        (received_metadata.width, received_metadata.height)
            == (sent_metadata.width, sent_metadata.height)
            || (received_metadata.width, received_metadata.height) == (0, 0),
        "an adapter must either preserve dimensions or explicitly report them unknown"
    );

    let second_received_video = receive(&right, TransportLane::Video).await;
    assert!(second_received_video.sequence > first_received_video_sequence);
    assert_eq!(second_received_video.payload, second_video.payload);

    for sequence in 1..=4 {
        let outcome = left
            .send(envelope(
                &session_id,
                TransportLane::ControlReliable,
                sequence,
                vec![sequence as u8],
            ))
            .await
            .expect("send reliable control");
        assert_eq!(outcome, TransportSendOutcome::Enqueued);
    }
    for sequence in 1..=4 {
        let received = receive(&right, TransportLane::ControlReliable).await;
        assert_eq!(received.sequence, sequence);
        assert_eq!(received.payload, vec![sequence as u8]);
    }

    for sequence in 10..=12 {
        left.send(envelope(
            &session_id,
            TransportLane::ControlRealtime,
            sequence,
            vec![sequence as u8],
        ))
        .await
        .expect("send realtime control");
    }
    wait_for_received(&right, TransportLane::ControlRealtime, 1).await;
    let latest = receive(&right, TransportLane::ControlRealtime).await;
    assert_eq!(
        latest.sequence, 12,
        "stale realtime values must be replaced"
    );
    let sender_replaced = left
        .route_snapshot()
        .await
        .lane(TransportLane::ControlRealtime)
        .stale_replaced;
    let receiver_replaced = right
        .route_snapshot()
        .await
        .lane(TransportLane::ControlRealtime)
        .stale_replaced;
    assert_eq!(sender_replaced + receiver_replaced, 2);

    for sequence in 20..=22 {
        left.send(envelope(
            &session_id,
            TransportLane::ControlReliable,
            sequence,
            vec![sequence as u8],
        ))
        .await
        .expect("queue reliable control before bulk");
    }
    let bulk = envelope(
        &session_id,
        TransportLane::Bulk,
        100,
        b"clipboard-or-file-chunk".to_vec(),
    );
    left.send(bulk.clone()).await.expect("send bulk");
    assert_eq!(receive(&right, TransportLane::Bulk).await, bulk);

    let fragmented_bulk = envelope(
        &session_id,
        TransportLane::Bulk,
        101,
        vec![0x6b; 128 * 1024],
    );
    assert_eq!(
        left.send(fragmented_bulk.clone())
            .await
            .expect("send fragmented bulk payload"),
        TransportSendOutcome::Enqueued
    );
    assert_eq!(
        receive(&right, TransportLane::Bulk).await,
        fragmented_bulk,
        "each adapter must deliver a reliable payload larger than one WebRTC message"
    );

    let left_snapshot = left.route_snapshot().await;
    assert_eq!(left_snapshot.kind, expected_route);
    assert_eq!(left_snapshot.session_id, session_id);
    assert!(!left_snapshot.local_endpoint.is_empty());
    assert!(!left_snapshot.peer_endpoint.is_empty());
    if matches!(
        expected_route,
        TransportRouteKind::WebRtcDirect | TransportRouteKind::WebRtcRelay
    ) {
        assert!(left_snapshot.local_candidate_kind.is_some());
        assert!(left_snapshot.remote_candidate_kind.is_some());
        assert!(!left_snapshot.local_endpoint.contains("pending"));
        assert!(!left_snapshot.peer_endpoint.contains("pending"));
    }
    assert!(left_snapshot.lane(TransportLane::Video).sent >= 1);
    assert!(left_snapshot.lane(TransportLane::Bulk).sent >= 1);

    let mut observed_backpressure = false;
    for sequence in 1_000..10_000 {
        let outcome = left
            .send(envelope(
                &session_id,
                TransportLane::Bulk,
                sequence,
                vec![0x5a; 64 * 1024],
            ))
            .await
            .expect("bulk send should report pressure without failing");
        if outcome == TransportSendOutcome::Backpressured {
            observed_backpressure = true;
            break;
        }
    }
    assert!(
        observed_backpressure,
        "bounded bulk queue must expose pressure"
    );

    let interactive = envelope(
        &session_id,
        TransportLane::ControlReliable,
        50_000,
        b"interactive-while-bulk-stalled".to_vec(),
    );
    assert_eq!(
        left.send(interactive.clone())
            .await
            .expect("stalled bulk must not block interactive control submission"),
        TransportSendOutcome::Enqueued
    );
    assert_eq!(
        receive(&right, TransportLane::ControlReliable).await,
        envelope(&session_id, TransportLane::ControlReliable, 20, vec![20],)
    );
    assert_eq!(
        receive(&right, TransportLane::ControlReliable).await,
        envelope(&session_id, TransportLane::ControlReliable, 21, vec![21],)
    );
    assert_eq!(
        receive(&right, TransportLane::ControlReliable).await,
        envelope(&session_id, TransportLane::ControlReliable, 22, vec![22],)
    );
    assert_eq!(
        receive(&right, TransportLane::ControlReliable).await,
        interactive
    );

    let waiting_mux = Arc::clone(&left);
    let waiting_receiver = tokio::spawn(async move {
        waiting_mux
            .recv(TransportLane::Video)
            .await
            .expect("blocked receive should wake cleanly")
    });
    tokio::task::yield_now().await;
    left.close().await.expect("first close");
    assert!(tokio::time::timeout(WAIT, waiting_receiver)
        .await
        .expect("close did not wake a blocked receiver")
        .expect("blocked receiver task panicked")
        .is_none());
    left.close().await.expect("idempotent close");
    assert_eq!(
        left.send(envelope(
            &session_id,
            TransportLane::ControlReliable,
            99_999,
            b"after-close".to_vec(),
        ))
        .await
        .expect("closed send is an outcome"),
        TransportSendOutcome::Closed
    );
    assert!(left.route_snapshot().await.closed);
    right.close().await.expect("close peer");
}

#[tokio::test]
async fn memory_adapter_conforms_to_transport_mux() {
    let session_id = SessionId("mux-memory".into());
    let (left, right) = MemoryTransportMux::pair(session_id.clone(), TransportMuxConfig::test());
    assert_transport_mux_conformance(
        session_id,
        Arc::new(left),
        Arc::new(right),
        TransportRouteKind::TestMemory,
    )
    .await;
}

#[tokio::test]
async fn quic_loopback_adapter_conforms_to_transport_mux() {
    let session_id = SessionId("mux-quic".into());
    let (left, right) = QuicTransportMux::loopback(session_id.clone(), TransportMuxConfig::test())
        .await
        .expect("create QUIC mux loopback");
    assert_transport_mux_conformance(
        session_id,
        Arc::new(left),
        Arc::new(right),
        TransportRouteKind::QuicLan,
    )
    .await;
}

#[tokio::test]
async fn webrtc_loopback_adapter_conforms_to_transport_mux() {
    let session_id = SessionId("mux-webrtc".into());
    let (left, right) =
        WebRtcTransportMux::loopback(session_id.clone(), TransportMuxConfig::test())
            .await
            .expect("create WebRTC mux loopback");
    let left: Mux = Arc::new(left);
    let right: Mux = Arc::new(right);
    let unsupported = left
        .send(TransportEnvelope {
            session_id: session_id.clone(),
            lane: TransportLane::Video,
            sequence: 0,
            payload: vec![1],
            video: Some(VideoEnvelopeMetadata {
                codec: "hevc".into(),
                timestamp_us: 1,
                keyframe: true,
                width: 1,
                height: 1,
            }),
        })
        .await
        .expect_err("unsupported WebRTC codecs must fail before queueing");
    assert!(unsupported
        .to_string()
        .contains("does not support video codec"));
    let oversized_realtime = left
        .send(envelope(
            &session_id,
            TransportLane::ControlRealtime,
            1,
            vec![0x44; 64 * 1024],
        ))
        .await
        .expect_err("realtime payloads cannot span unreliable WebRTC messages");
    assert!(oversized_realtime
        .to_string()
        .contains("exceeds one data-channel message"));
    assert_transport_mux_conformance(session_id, left, right, TransportRouteKind::WebRtcDirect)
        .await;
}

async fn assert_peer_close_propagates(session_id: SessionId, left: Mux, right: Mux) {
    let waiting = Arc::clone(&left);
    let receiver = tokio::spawn(async move {
        waiting
            .recv(TransportLane::Video)
            .await
            .expect("receive after peer close")
    });
    tokio::task::yield_now().await;

    right.close().await.expect("close remote peer");

    assert!(tokio::time::timeout(WAIT, receiver)
        .await
        .expect("peer close did not wake receiver")
        .expect("receiver task panicked")
        .is_none());
    let snapshot = left.route_snapshot().await;
    assert!(snapshot.closed);
    assert!(snapshot.last_error.is_some());
    assert_eq!(
        left.send(envelope(
            &session_id,
            TransportLane::ControlReliable,
            1,
            b"after-peer-close".to_vec(),
        ))
        .await
        .expect("closed send outcome"),
        TransportSendOutcome::Closed
    );
    left.close().await.expect("close local mux");
}

async fn assert_peer_drop_propagates(left: Mux, right: Mux) {
    let waiting = Arc::clone(&left);
    let receiver = tokio::spawn(async move {
        waiting
            .recv(TransportLane::Video)
            .await
            .expect("receive after peer drop")
    });
    tokio::task::yield_now().await;

    drop(right);

    assert!(tokio::time::timeout(WAIT, receiver)
        .await
        .expect("peer drop did not wake receiver")
        .expect("receiver task panicked")
        .is_none());
    let snapshot = left.route_snapshot().await;
    assert!(snapshot.closed);
    assert!(snapshot.last_error.is_some());
    left.close().await.expect("close surviving peer");
}

#[tokio::test]
async fn memory_peer_close_closes_mux_and_wakes_receivers() {
    let session_id = SessionId("mux-memory-peer-close".into());
    let (left, right) = MemoryTransportMux::pair(session_id.clone(), TransportMuxConfig::test());
    assert_peer_close_propagates(session_id, Arc::new(left), Arc::new(right)).await;
}

#[tokio::test]
async fn memory_peer_drop_terminates_the_session() {
    let session_id = SessionId("mux-memory-peer-drop".into());
    let (left, right) = MemoryTransportMux::pair(session_id, TransportMuxConfig::test());
    assert_peer_drop_propagates(Arc::new(left), Arc::new(right)).await;
}

#[tokio::test]
async fn quic_peer_close_closes_mux_and_wakes_receivers() {
    let session_id = SessionId("mux-quic-peer-close".into());
    let (left, right) = QuicTransportMux::loopback(session_id.clone(), TransportMuxConfig::test())
        .await
        .expect("create QUIC mux loopback");
    assert_peer_close_propagates(session_id, Arc::new(left), Arc::new(right)).await;
}

#[tokio::test]
async fn quic_peer_drop_terminates_the_connection() {
    let session_id = SessionId("mux-quic-peer-drop".into());
    let (left, right) = QuicTransportMux::loopback(session_id, TransportMuxConfig::test())
        .await
        .expect("create QUIC mux loopback");
    assert_peer_drop_propagates(Arc::new(left), Arc::new(right)).await;
}

#[tokio::test]
async fn webrtc_peer_close_closes_mux_and_wakes_receivers() {
    let session_id = SessionId("mux-webrtc-peer-close".into());
    let (left, right) =
        WebRtcTransportMux::loopback(session_id.clone(), TransportMuxConfig::test())
            .await
            .expect("create WebRTC mux loopback");
    assert_peer_close_propagates(session_id, Arc::new(left), Arc::new(right)).await;
}

#[tokio::test]
async fn webrtc_peer_drop_terminates_the_connection() {
    let session_id = SessionId("mux-webrtc-peer-drop".into());
    let (left, right) = WebRtcTransportMux::loopback(session_id, TransportMuxConfig::test())
        .await
        .expect("create WebRTC mux loopback");
    assert_peer_drop_propagates(Arc::new(left), Arc::new(right)).await;
}
