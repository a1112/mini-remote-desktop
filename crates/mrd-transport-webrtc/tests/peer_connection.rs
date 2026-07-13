use std::time::Duration;

use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
use mrd_transport_webrtc::{
    ControlLane, H264CodecConfig, H264CodecProfile, IceCandidate, PeerConnectionConfig,
    PeerConnectionRole, VideoCodecConfig, WebRtcPeerConnection, CTRL_REL_LABEL, CTRL_RT_LABEL,
};

const WAIT: Duration = Duration::from_secs(10);

fn loopback_config(role: PeerConnectionRole) -> PeerConnectionConfig {
    PeerConnectionConfig {
        role,
        include_loopback_candidates: true,
        ..PeerConnectionConfig::default()
    }
}

async fn exchange_candidate(
    from: &WebRtcPeerConnection,
    to: &WebRtcPeerConnection,
) -> IceCandidate {
    let candidate = tokio::time::timeout(WAIT, from.next_local_candidate())
        .await
        .expect("candidate gathering timed out")
        .expect("candidate stream closed before yielding a candidate");
    to.add_ice_candidate(candidate.clone())
        .await
        .expect("remote candidate should be accepted");
    candidate
}

async fn connect_loopback() -> (WebRtcPeerConnection, WebRtcPeerConnection) {
    let offerer = WebRtcPeerConnection::new(loopback_config(PeerConnectionRole::Offerer))
        .await
        .expect("offerer preflight and creation should succeed");
    let answerer = WebRtcPeerConnection::new(loopback_config(PeerConnectionRole::Answerer))
        .await
        .expect("answerer preflight and creation should succeed");

    let offer = offerer
        .create_offer()
        .await
        .expect("offer should be created");
    let answer = answerer
        .accept_offer(offer)
        .await
        .expect("offer should produce an answer");
    offerer
        .accept_answer(answer)
        .await
        .expect("answer should be accepted");

    let (offer_candidate, answer_candidate) = tokio::join!(
        exchange_candidate(&offerer, &answerer),
        exchange_candidate(&answerer, &offerer)
    );
    assert!(!offer_candidate.candidate.is_empty());
    assert!(!answer_candidate.candidate.is_empty());

    tokio::time::timeout(WAIT, offerer.wait_connected())
        .await
        .expect("offerer connection timed out")
        .expect("offerer should connect");
    tokio::time::timeout(WAIT, answerer.wait_connected())
        .await
        .expect("answerer connection timed out")
        .expect("answerer should connect");

    (offerer, answerer)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exchanges_offer_answer_candidates_video_control_and_stats() {
    let (offerer, answerer) = connect_loopback().await;

    let channels = offerer.control_channels().await;
    assert_eq!(channels.reliable.label, CTRL_REL_LABEL);
    assert!(channels.reliable.ordered);
    assert_eq!(channels.reliable.max_retransmits, None);
    assert_eq!(channels.realtime.label, CTRL_RT_LABEL);
    assert!(!channels.realtime.ordered);
    assert_eq!(channels.realtime.max_retransmits, Some(0));

    offerer
        .send_control(ControlLane::Reliable, b"clipboard-sync")
        .await
        .expect("reliable control should send");
    let reliable = tokio::time::timeout(WAIT, answerer.next_control(ControlLane::Reliable))
        .await
        .expect("reliable control timed out")
        .expect("reliable control channel closed");
    assert_eq!(reliable.as_ref(), b"clipboard-sync");

    offerer
        .send_control(ControlLane::Realtime, b"mouse-move")
        .await
        .expect("realtime control should send");
    let realtime = tokio::time::timeout(WAIT, answerer.next_control(ControlLane::Realtime))
        .await
        .expect("realtime control timed out")
        .expect("realtime control channel closed");
    assert_eq!(realtime.as_ref(), b"mouse-move");

    let access_unit = EncodedAccessUnit {
        codec: VideoCodec::H264,
        timestamp_us: 42_000,
        is_keyframe: true,
        bytes: vec![
            0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0xe5, 0x88, 0x68, 0x54, 0, 0, 0, 1, 0x68, 0xce,
            0x06, 0xe2, 0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21,
        ],
    };
    offerer
        .send_h264_access_unit(&access_unit)
        .await
        .expect("H.264 access unit should send");
    let received = tokio::time::timeout(WAIT, answerer.next_h264_access_unit())
        .await
        .expect("H.264 receive timed out")
        .expect("H.264 receive queue closed");
    assert_eq!(received.codec, VideoCodec::H264);
    assert_eq!(received.bytes, access_unit.bytes);

    let stats = offerer
        .selected_candidate_pair_stats()
        .await
        .expect("selected candidate pair should be reported");
    assert!(stats.nominated);
    assert!(!stats.local_candidate_id.is_empty());
    assert!(!stats.remote_candidate_id.is_empty());
    assert_ne!(
        stats.local_candidate_kind,
        mrd_transport_webrtc::CandidateKind::Unknown
    );
    assert_ne!(
        stats.remote_candidate_kind,
        mrd_transport_webrtc::CandidateKind::Unknown
    );

    assert!(offerer.active_task_count() >= 1);
    assert!(answerer.active_task_count() >= 1);
    offerer.close().await.expect("offerer should close cleanly");
    answerer
        .close()
        .await
        .expect("answerer should close cleanly");
    assert_eq!(offerer.active_task_count(), 0);
    assert_eq!(answerer.active_task_count(), 0);
}

#[tokio::test]
async fn unsupported_codec_or_profile_fails_preflight() {
    let unsupported_codec = PeerConnectionConfig {
        video_codec: VideoCodecConfig::Unsupported(VideoCodec::Hevc),
        ..loopback_config(PeerConnectionRole::Offerer)
    };
    let error = WebRtcPeerConnection::new(unsupported_codec)
        .await
        .expect_err("HEVC is not an interoperable WebRTC preflight codec yet");
    assert!(error.to_string().contains("unsupported WebRTC video codec"));

    let invalid_profile = PeerConnectionConfig {
        video_codec: VideoCodecConfig::H264(H264CodecConfig {
            profile: H264CodecProfile::High,
            profile_level_id: "42e01f".to_owned(),
            packetization_mode: 1,
        }),
        ..loopback_config(PeerConnectionRole::Offerer)
    };
    let error = WebRtcPeerConnection::new(invalid_profile)
        .await
        .expect_err("baseline profile-level-id must not pass high-profile preflight");
    assert!(error.to_string().contains("profile-level-id"));

    let invalid_packetization = PeerConnectionConfig {
        video_codec: VideoCodecConfig::H264(H264CodecConfig {
            packetization_mode: 0,
            ..H264CodecConfig::default()
        }),
        ..loopback_config(PeerConnectionRole::Offerer)
    };
    let error = WebRtcPeerConnection::new(invalid_packetization)
        .await
        .expect_err("packetization-mode 0 must fail preflight");
    assert!(error.to_string().contains("packetization-mode"));
}
