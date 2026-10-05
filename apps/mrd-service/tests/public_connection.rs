use mrd_ipc::{IpcRequest, IpcResponse};
use mrd_service::{ipc_server::IpcServer, AppState};
use std::sync::Arc;

#[test]
fn public_management_credentials_are_redacted_in_debug_but_keep_wire_compatibility() {
    let request: IpcRequest = serde_json::from_str(
        r#"{"type":"RecoverPublicDevice","device_token":"very-sensitive-device.jwt.token"}"#,
    )
    .unwrap();
    assert!(!format!("{request:?}").contains("very-sensitive"));
    assert_eq!(
        serde_json::to_value(request).unwrap()["device_token"],
        "very-sensitive-device.jwt.token"
    );
}

#[tokio::test]
async fn management_reports_public_connection_without_exposing_security_or_sessions() {
    let state = Arc::new(AppState::new());
    let server = IpcServer::new_management(state);
    let request: IpcRequest = serde_json::from_str(r#"{"type":"GetPublicServerStatus"}"#).unwrap();
    let response = server.handle_request(request).await;
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["type"], "PublicServerStatus");
    assert_eq!(json["status"]["device_registered"], false);
    assert_eq!(json["status"]["signaling_state"], "disabled");
    assert_eq!(json["status"]["api_reachable"], serde_json::Value::Null);
    let text = json.to_string();
    for secret in [
        "access_token",
        "private_key",
        "sessions",
        "motherboard_serial",
    ] {
        assert!(!text.contains(secret));
    }
}

#[tokio::test]
async fn management_rejects_invalid_enrollment_without_starting_or_changing_identity() {
    let state = Arc::new(AppState::new());
    let server = IpcServer::new_management(state.clone());
    let request: IpcRequest = serde_json::from_str(
        r#"{"type":"EnrollPublicDevice","enrollment_token":"bad-secret","device_name":"Office"}"#,
    )
    .unwrap();
    assert!(
        matches!(server.handle_request(request).await, IpcResponse::Error {code, message} if code == "E_PUBLIC_ENROLLMENT" && !message.contains("bad-secret"))
    );
    assert!(state.devices.lock().await.get_local_device().is_none());
}
