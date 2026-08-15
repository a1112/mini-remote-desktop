use std::{env, time::Duration};

use mrd_transport_webrtc::{
    CandidateKind, IceServerConfig, IceTransportPolicy, PeerConnectionConfig, PeerConnectionRole,
    WebRtcPeerConnection,
};

const WAIT: Duration = Duration::from_secs(15);

fn relay_config(
    role: PeerConnectionRole,
    url: &str,
    username: &str,
    credential: &str,
) -> PeerConnectionConfig {
    PeerConnectionConfig {
        role,
        ice_servers: vec![IceServerConfig::new(
            vec![url.to_owned()],
            username.to_owned(),
            credential.to_owned(),
        )],
        ice_transport_policy: IceTransportPolicy::Relay,
        ..PeerConnectionConfig::default()
    }
}

#[test]
fn ice_server_debug_output_redacts_temporary_credentials() {
    let server = IceServerConfig::new(
        vec!["turn:relay.example.test:3478?transport=udp".to_owned()],
        "temporary-user".to_owned(),
        "temporary-password".to_owned(),
    );
    let debug = format!("{server:?}");
    assert!(!debug.contains("temporary-user"));
    assert!(!debug.contains("temporary-password"));
    assert!(debug.contains("[REDACTED]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_candidate_pair_is_relay_when_forced() {
    let Ok(url) = env::var("MRD_TEST_TURN_URL") else {
        eprintln!("skipping live forced-relay proof: MRD_TEST_TURN_URL is not configured");
        return;
    };
    let username = env::var("MRD_TEST_TURN_USERNAME").expect("TURN username is required");
    let credential = env::var("MRD_TEST_TURN_CREDENTIAL").expect("TURN credential is required");

    let offerer = WebRtcPeerConnection::new(relay_config(
        PeerConnectionRole::Offerer,
        &url,
        &username,
        &credential,
    ))
    .await
    .expect("relay-only offerer should start");
    let answerer = WebRtcPeerConnection::new(relay_config(
        PeerConnectionRole::Answerer,
        &url,
        &username,
        &credential,
    ))
    .await
    .expect("relay-only answerer should start");

    let offer = offerer.create_offer().await.expect("create offer");
    let answer = answerer.accept_offer(offer).await.expect("create answer");
    offerer.accept_answer(answer).await.expect("accept answer");
    let offer_candidate = tokio::time::timeout(WAIT, offerer.next_local_candidate())
        .await
        .expect("offer candidate timed out")
        .expect("offer candidate missing");
    let answer_candidate = tokio::time::timeout(WAIT, answerer.next_local_candidate())
        .await
        .expect("answer candidate timed out")
        .expect("answer candidate missing");
    answerer
        .add_ice_candidate(offer_candidate)
        .await
        .expect("answerer accepts relay candidate");
    offerer
        .add_ice_candidate(answer_candidate)
        .await
        .expect("offerer accepts relay candidate");

    tokio::time::timeout(WAIT, offerer.wait_connected())
        .await
        .expect("relay connection timed out")
        .expect("relay connection failed");
    let stats = offerer
        .selected_candidate_pair_stats()
        .await
        .expect("selected relay pair stats missing");
    assert_eq!(stats.local_candidate_kind, CandidateKind::Relay);
    assert_eq!(stats.remote_candidate_kind, CandidateKind::Relay);

    offerer.close().await.expect("close offerer");
    answerer.close().await.expect("close answerer");
}
