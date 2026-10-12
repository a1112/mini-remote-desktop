use mrd_agent_ipc::ServiceToAgent;
use serde_json::json;

#[test]
fn paired_authority_install_is_a_distinct_wire_command() {
    let wire = json!({
        "type": "install_paired_session_authority",
        "payload": {
            "request_token": 17,
            "authority": {
                "claims": {
                    "authority_id": vec![1; 32],
                    "registration_id": vec![2; 16],
                    "registration_epoch": 3,
                    "session_id": "paired-session",
                    "peer": {"device_id": "paired-peer", "key_id": vec![4; 32]},
                    "scopes": ["ScreenView", "InputPointer"],
                    "policy_revision": 5,
                    "windows_session_id": 1,
                    "desktop_epoch": 6,
                    "desktop_kind": "default",
                    "issued_at_ms": 1000,
                    "not_before_ms": 1000,
                    "expires_at_ms": 2000,
                    "audience": "session_agent"
                },
                "issuer_key_id": vec![7; 32],
                "signature": vec![8; 64]
            }
        }
    });
    let parsed = serde_json::from_value::<ServiceToAgent>(wire);
    assert!(
        parsed.is_ok(),
        "paired install must have its own strict wire variant: {parsed:?}"
    );
}
