use mrd_ipc::{IpcRequest, RemoteAccessMode};

#[test]
fn paired_mode_and_explicit_policy_commands_are_secure_remote_contracts() {
    let mode: Result<RemoteAccessMode, _> = serde_json::from_str("\"paired\"");
    assert!(mode.is_ok(), "paired mode must have an explicit wire value");
    for body in [
        r#"{"type":"PairPendingSession","session_id":"pending-signed-session","approved_scopes":["screen.view","input.pointer"],"expected_policy_revision":"1"}"#,
        r#"{"type":"ListPairedAccessPolicies"}"#,
        r#"{"type":"RevokePairedAccess","peer_key_id":"pinned-peer","expected_revision":"1"}"#,
    ] {
        let request: IpcRequest = serde_json::from_str(body).expect("formal paired policy command");
        assert!(request.is_secure_remote());
    }
}

#[test]
fn paired_policy_enrollment_cannot_supply_an_arbitrary_public_key() {
    let body = r#"{"type":"PairPendingSession","session_id":"pending-signed-session","approved_scopes":["screen.view"],"expected_policy_revision":"1","public_key":[1,2,3]}"#;
    assert!(serde_json::from_str::<IpcRequest>(body).is_err());
}
