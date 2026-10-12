use mrd_proto::{DeviceId, SessionId};
use mrd_signal_proto::{SignalProtocolError, SignalReplayGuard, VerifiedSignalMetadata};

fn metadata(device: &str, counter: u64, nonce: u8) -> VerifiedSignalMetadata {
    VerifiedSignalMetadata {
        issuer_device_id: DeviceId(device.into()),
        issuer_key_id: "ab".repeat(32),
        intended_peer_device_id: DeviceId("target-1".into()),
        counter,
        nonce: [nonce; 16],
    }
}

#[test]
fn authenticated_browser_namespaces_keep_strict_stream_replay_checks() {
    let mut guard = SignalReplayGuard::new(2, 8);
    let a = SessionId("session-a".into());
    let b = SessionId("session-b".into());
    guard
        .accept_browser_session(&metadata("browser_a", 100, 1), &a, 1000, 1)
        .unwrap();
    guard
        .accept_browser_session(&metadata("browser_b", 1, 1), &b, 1000, 2)
        .unwrap();
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_a", 99, 2), &a, 1000, 3),
        Err(SignalProtocolError::CounterRollback)
    );
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_a", 101, 1), &a, 1000, 4),
        Err(SignalProtocolError::RepeatedNonce)
    );
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_b", 1, 2), &b, 1000, 5),
        Err(SignalProtocolError::CounterRollback)
    );
    guard
        .accept_browser_session(&metadata("browser_a", 101, 2), &a, 1000, 6)
        .unwrap();
}

#[test]
fn browser_replay_capacity_expires_only_after_authenticated_principal_deadline() {
    let mut guard = SignalReplayGuard::new(2, 8);
    guard.accept("physical-key", 10, [10; 16]).unwrap();
    let a = SessionId("session-a".into());
    let b = SessionId("session-b".into());
    guard
        .accept_browser_session(&metadata("browser_a", 1, 1), &a, 100, 1)
        .unwrap();
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_b", 1, 1), &b, 200, 2),
        Err(SignalProtocolError::ReplayCapacity)
    );
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_a", 2, 2), &a, 100, 100),
        Err(SignalProtocolError::Expired)
    );
    guard
        .accept_browser_session(&metadata("browser_b", 1, 1), &b, 200, 100)
        .unwrap();
    assert_eq!(
        guard.accept("physical-key", 9, [9; 16]),
        Err(SignalProtocolError::CounterRollback)
    );
}

#[test]
fn browser_retention_extension_keeps_counter_and_nonce_and_only_exact_live_lane() {
    let mut guard = SignalReplayGuard::new(2, 8);
    let a = SessionId("session-a".into());
    let b = SessionId("session-b".into());
    guard
        .accept_browser_session(&metadata("browser_a", 100, 1), &a, 30, 1)
        .unwrap();
    assert!(!guard
        .extend_browser_session_retention(&metadata("browser_b", 102, 2), &a, 60, 15)
        .unwrap());
    assert!(!guard
        .extend_browser_session_retention(&metadata("browser_a", 102, 2), &b, 60, 15)
        .unwrap());
    let mut wrong_key = metadata("browser_a", 102, 2);
    wrong_key.issuer_key_id = "cd".repeat(32);
    assert!(!guard
        .extend_browser_session_retention(&wrong_key, &a, 60, 15)
        .unwrap());
    assert!(guard
        .extend_browser_session_retention(&metadata("browser_a", 102, 2), &a, 60, 15)
        .unwrap());
    // Extension does not count as ordered-stream acceptance and never clears
    // any old replay evidence. A shorter claim cannot reduce retention.
    assert!(guard
        .extend_browser_session_retention(&metadata("browser_a", 103, 3), &a, 25, 20)
        .unwrap());
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_a", 99, 4), &a, 90, 31),
        Err(SignalProtocolError::CounterRollback)
    );
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_a", 101, 1), &a, 90, 31),
        Err(SignalProtocolError::RepeatedNonce)
    );
    guard
        .accept_browser_session(&metadata("browser_a", 101, 5), &a, 60, 31)
        .unwrap();
    guard
        .accept_browser_session(&metadata("browser_b", 1, 1), &b, 100, 31)
        .unwrap();
    assert!(!guard
        .extend_browser_session_retention(&metadata("browser_a", 104, 6), &a, 90, 60)
        .unwrap());
    assert_eq!(
        guard.extend_browser_session_retention(&metadata("browser_b", 2, 2), &b, 50, 60),
        Err(SignalProtocolError::Expired)
    );
    assert_eq!(
        guard.accept_browser_session(&metadata("browser_b", 1, 3), &b, 100, 60),
        Err(SignalProtocolError::CounterRollback)
    );
}
