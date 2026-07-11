use mrd_agent_ipc::{
    derive_registration_public_key, read_agent_bootstrap, windows_agent_bootstrap_pipe_name,
    write_agent_bootstrap, AgentBootstrap, BoundEd25519RegistrationVerifier,
    RegistrationProofVerifier,
};
use zeroize::Zeroizing;

#[tokio::test]
async fn bootstrap_codec_round_trips_secret_without_serde_or_environment_state() {
    let seed = [7_u8; 32];
    let key = derive_registration_public_key(&seed).expect("derive registration key");
    let endpoint = r"\\.\pipe\mrd-agent-control-test";
    let (mut writer, mut reader) = tokio::io::duplex(2_048);
    let writing = tokio::spawn(async move {
        write_agent_bootstrap(
            &mut writer,
            AgentBootstrap {
                control_endpoint: endpoint,
                service_process_id: 44,
                service_process_creation_time: 55,
                heartbeat_interval_ms: 1_000,
                handshake_timeout_ms: 5_000,
                registration_seed: Zeroizing::new(seed),
                expected_agent_key_id: key.key_id,
            },
        )
        .await
    });
    let received = read_agent_bootstrap(&mut reader).await.unwrap();
    writing.await.unwrap().unwrap();

    assert_eq!(received.control_endpoint(), endpoint);
    assert_eq!(received.service_process_id(), 44);
    assert_eq!(received.service_process_creation_time(), 55);
    assert_eq!(received.heartbeat_interval_ms(), 1_000);
    assert_eq!(received.handshake_timeout_ms(), 5_000);
    assert_eq!(received.expected_agent_key_id(), &key.key_id);
    assert_eq!(&*received.into_registration_seed(), &seed);
}

#[test]
fn derived_verifier_is_bound_to_the_bootstrap_key_id() {
    let seed = [8_u8; 32];
    let key = derive_registration_public_key(&seed).unwrap();
    let signer = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    let signature = signer.sign(b"bound transcript");
    let verifier = BoundEd25519RegistrationVerifier::new(key.key_id, key.public_key).unwrap();

    assert!(verifier.verify(
        &key.key_id,
        b"bound transcript",
        signature.as_ref().try_into().unwrap()
    ));
    assert!(!verifier.verify(
        &[99; 32],
        b"bound transcript",
        signature.as_ref().try_into().unwrap()
    ));
    assert!(!verifier.verify(
        &key.key_id,
        b"different transcript",
        signature.as_ref().try_into().unwrap()
    ));
}

#[test]
fn bootstrap_pipe_name_is_derived_only_from_os_process_identity() {
    assert_eq!(
        windows_agent_bootstrap_pipe_name(7, 42, 0x1234),
        r"\\.\pipe\mrd-agent-bootstrap-v1-s7-p42-c0000000000001234"
    );
}
