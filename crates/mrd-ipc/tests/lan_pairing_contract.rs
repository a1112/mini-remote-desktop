use mrd_ipc::{IpcRequest, IpcResponse};
use serde_json::json;

fn approval() -> serde_json::Value {
    json!({
        "candidate_id": "07".repeat(16),
        "device_id": "lan-MSLGKRSZGOVODD",
        "peer_key_id": "86e02445b5059bc92cbc86e3fae3ac6d262e97d0b9407b91ac7331992780c638",
        "key_epoch": "1",
        "discovery_endpoint": "192.168.1.241:21116",
        "permission_ceiling": ["screen.view"]
    })
}

#[test]
fn explicit_lan_pairing_request_preserves_the_displayed_peer_binding() {
    let value = json!({"type": "ApproveLanPairing", "approval": approval()});
    let request: IpcRequest = serde_json::from_value(value.clone())
        .expect("the explicit installed-UI LAN pairing request must exist");
    assert_eq!(serde_json::to_value(request).unwrap(), value);
}

#[test]
fn pairing_candidate_request_and_response_are_read_only_public_metadata() {
    let request: IpcRequest = serde_json::from_value(json!({"type": "ListLanPairingCandidates"}))
        .expect("the installed UI must be able to inspect current verified candidates");
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({"type": "ListLanPairingCandidates"})
    );
    let mut candidate = approval();
    candidate["device_name"] = json!("MS-LGKRSZGOVODD");
    candidate["expires_at_ms"] = json!(1791359600000_u64);
    let response_value = json!({"type": "LanPairingCandidateList", "candidates": [candidate]});
    let response: IpcResponse = serde_json::from_value(response_value.clone())
        .expect("candidate response must contain public binding metadata only");
    assert_eq!(serde_json::to_value(response).unwrap(), response_value);
}

#[test]
fn explicit_lan_pairing_never_accepts_client_public_key_as_evidence() {
    let mut value = approval();
    value["public_key"] = json!(vec![9_u8; 32]);
    assert!(serde_json::from_value::<IpcRequest>(
        json!({"type": "ApproveLanPairing", "approval": value})
    )
    .is_err());
}
