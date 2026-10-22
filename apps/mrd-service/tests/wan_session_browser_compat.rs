//! Standard browser semantics against the production service transport, without
//! the localhost preview API or native-controller peer wrapper on the remote side.
use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportMuxPort, TransportSendOutcome, VideoEnvelopeMetadata,
};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoDecoder, VideoEncoder};
use mrd_proto::SessionId;
use mrd_service::transports::webrtc::ServiceWebRtcTransportHost;
use mrd_transport_webrtc::{
    H264RtpIngress, PeerConnectionConfig, PeerConnectionRole, SessionDescription,
    SessionDescriptionType,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
        setting_engine::SettingEngine, APIBuilder,
    },
    data_channel::{
        data_channel_init::RTCDataChannelInit, data_channel_state::RTCDataChannelState,
    },
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
    },
    rtp_transceiver::{
        rtp_codec::RTPCodecType, rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCRtpTransceiverInit,
    },
};

fn browser_control_frame(session: &SessionId, payload: &[u8]) -> Vec<u8> {
    let mut mux = Vec::new();
    mux.extend_from_slice(b"MRMX");
    mux.extend_from_slice(&[1, 1]);
    mux.extend_from_slice(&1u64.to_le_bytes());
    mux.extend_from_slice(&(session.0.len() as u16).to_le_bytes());
    mux.extend_from_slice(&[0; 18]); // no codec or video metadata
    mux.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    mux.extend_from_slice(session.0.as_bytes());
    mux.extend_from_slice(payload);
    let mut frame = Vec::new();
    frame.extend_from_slice(b"MRDF");
    frame.push(1);
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&1u16.to_le_bytes());
    frame.extend_from_slice(&(mux.len() as u32).to_le_bytes());
    frame.extend_from_slice(&mux);
    frame
}

#[tokio::test]
async fn standard_recvonly_h264_peer_gets_remote_rtp_and_bidirectional_control() {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let interceptors = register_default_interceptors(Registry::new(), &mut media).unwrap();
    let mut settings = SettingEngine::default();
    settings.set_include_loopback_candidate(true);
    let browser = Arc::new(
        APIBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(interceptors)
            .with_setting_engine(settings)
            .build()
            .new_peer_connection(RTCConfiguration::default())
            .await
            .unwrap(),
    );
    browser
        .add_transceiver_from_kind(
            RTPCodecType::Video,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                send_encodings: vec![],
            }),
        )
        .await
        .unwrap();
    let rel = browser
        .create_data_channel(
            "ctrl_rel",
            Some(RTCDataChannelInit {
                ordered: Some(true),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let rt = browser
        .create_data_channel(
            "ctrl_rt",
            Some(RTCDataChannelInit {
                ordered: Some(false),
                max_retransmits: Some(0),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let bulk = browser
        .create_data_channel(
            "bulk",
            Some(RTCDataChannelInit {
                ordered: Some(true),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let (control_tx, mut control_rx) = mpsc::channel(4);
    rel.on_message(Box::new(move |message| {
        let tx = control_tx.clone();
        Box::pin(async move {
            let _ = tx.send(message.data).await;
        })
    }));
    let (video_tx, mut video_rx) = mpsc::channel(2);
    browser.on_track(Box::new(move |track, _, _| {
        let tx = video_tx.clone();
        Box::pin(async move {
            assert_eq!(
                track.codec().capability.mime_type.to_lowercase(),
                "video/h264"
            );
            tokio::spawn(async move {
                let mut ingress = H264RtpIngress::with_max_access_unit_bytes(4 * 1024 * 1024);
                while let Ok((packet, _)) = track.read_rtp().await {
                    if let Some(unit) = ingress.push_packet(
                        &packet.payload,
                        packet.header.marker,
                        packet.header.sequence_number,
                        u64::from(packet.header.timestamp) * 1_000_000 / 90_000,
                    ) {
                        let _ = tx.send(unit).await;
                        break;
                    }
                }
            });
        })
    }));
    let (ice_tx, mut ice_rx) = mpsc::channel(32);
    browser.on_ice_candidate(Box::new(move |candidate| {
        let tx = ice_tx.clone();
        Box::pin(async move {
            if let Some(candidate) = candidate {
                let _ = tx.send(candidate.to_json().unwrap()).await;
            }
        })
    }));
    let offer = browser.create_offer(None).await.unwrap();
    assert!(offer.sdp.contains("a=recvonly"));
    assert!(offer.sdp.contains("H264/90000"));
    browser.set_local_description(offer.clone()).await.unwrap();

    let host = ServiceWebRtcTransportHost::new();
    let session = SessionId("browser-standard-remote".into());
    host.open_session(
        session.clone(),
        PeerConnectionConfig {
            role: PeerConnectionRole::Answerer,
            include_loopback_candidates: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let answer = host
        .accept_offer(
            &session,
            SessionDescription::from_wire(SessionDescriptionType::Offer, offer.sdp, 0, None)
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(answer.sdp.contains("a=sendonly"));
    assert!(answer.sdp.contains("H264/90000"));
    browser
        .set_remote_description(RTCSessionDescription::answer(answer.sdp.clone()).unwrap())
        .await
        .unwrap();
    let remote_candidate = tokio::time::timeout(Duration::from_secs(5), ice_rx.recv())
        .await
        .unwrap()
        .unwrap();
    host.add_ice_candidate(&session, remote_candidate.into())
        .await
        .unwrap();
    let local_candidate =
        tokio::time::timeout(Duration::from_secs(5), host.next_local_candidate(&session))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    browser
        .add_ice_candidate(webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: local_candidate.candidate.clone(),
            sdp_mid: local_candidate.sdp_mid.clone(),
            sdp_mline_index: local_candidate.sdp_mline_index,
            username_fragment: local_candidate.username_fragment.clone(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), host.wait_connected(&session))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while rel.ready_state() != RTCDataChannelState::Open
            || rt.ready_state() != RTCDataChannelState::Open
            || bulk.ready_state() != RTCDataChannelState::Open
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mux = host.transport_mux(&session).await.unwrap();
    rel.send(&browser_control_frame(&session, b"browser-control-wire").into())
        .await
        .unwrap();
    let incoming = tokio::time::timeout(
        Duration::from_secs(3),
        mux.recv(TransportLane::ControlReliable),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(incoming.payload, b"browser-control-wire");
    assert_eq!(incoming.session_id, session);
    assert_eq!(incoming.sequence, 1);
    assert_eq!(
        mux.send(TransportEnvelope {
            session_id: session.clone(),
            lane: TransportLane::ControlReliable,
            sequence: 1,
            payload: b"host-control-wire".to_vec(),
            video: None
        })
        .await
        .unwrap(),
        TransportSendOutcome::Enqueued
    );
    let reply = tokio::time::timeout(Duration::from_secs(3), control_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply[..5], b"MRDF\x01");
    assert!(reply.ends_with(b"host-control-wire"));

    let frame = CapturedFrame::from_cpu(
        1920,
        1080,
        FramePixelFormat::Rgb24,
        1_000_000,
        vec![73; 1920 * 1080 * 3],
    );
    let mut encoder = OpenH264Encoder::new_with_bitrate(1920, 1080, 60, 8_000_000).unwrap();
    for unit in encoder.encode(&frame).unwrap() {
        assert_eq!(
            mux.send(TransportEnvelope {
                session_id: session.clone(),
                lane: TransportLane::Video,
                sequence: 1,
                payload: unit.bytes,
                video: Some(VideoEnvelopeMetadata {
                    codec: "h264".into(),
                    timestamp_us: unit.timestamp_us,
                    keyframe: unit.is_keyframe,
                    width: 1920,
                    height: 1080
                })
            })
            .await
            .unwrap(),
            TransportSendOutcome::Enqueued
        );
    }
    let unit = tokio::time::timeout(Duration::from_secs(5), video_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(unit.is_keyframe);
    let mut decoder = mrd_decode::H264SoftwareDecoder::new().unwrap();
    decoder.push_access_unit(&unit.bytes).unwrap();
    let frames = decoder.drain_decoded_frames();
    assert!(!frames.is_empty());
    assert_eq!((frames[0].width, frames[0].height), (1920, 1080));
    browser.close().await.unwrap();
    host.shutdown().await.unwrap();
}
