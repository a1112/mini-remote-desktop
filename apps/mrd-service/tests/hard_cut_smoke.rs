// Hard-cut migration smoke tests for mrd-service
//
// These tests verify that mrd-service can independently handle
// the core session flow without any shell-owned runtime.

use mrd_ipc::{IpcRequest, IpcResponse, client::IpcClient};
use mrd_proto::{SessionId, DeviceId};

#[tokio::test]
async fn hard_cut_service_responds_to_health_check() {
    // This test verifies mrd-service can serve health checks
    let mut client = IpcClient::new();

    // Service health should be accessible
    let response = client.send_request(IpcRequest::ServiceHealth).await;

    match response {
        Ok(IpcResponse::ServiceHealth { status }) => {
            assert!(status.running, "Service should report running");
            assert!(status.healthy, "Service should report healthy");
        }
        Ok(_) => panic!("Expected ServiceHealth response"),
        Err(e) => panic!("IPC error: {}", e),
    }
}

#[tokio::test]
async fn hard_cut_full_session_flow() {
    let mut client = IpcClient::new();

    // Step 1: Register device
    let device_id = DeviceId("smoke-test-device".to_string());
    let register_response = client.send_request(IpcRequest::RegisterDevice {
        device_id: device_id.clone(),
        device_name: "Smoke Test Device".to_string(),
    }).await;

    match register_response {
        Ok(IpcResponse::DeviceRegistered { .. }) => {}
        Ok(_) => panic!("Expected DeviceRegistered response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Step 2: List devices (should return our device)
    let list_response = client.send_request(IpcRequest::ListDevices).await;

    match list_response {
        Ok(IpcResponse::DeviceList { .. }) => {
            // TODO: Verify our device is in the list
        }
        Ok(_) => panic!("Expected DeviceList response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Step 3: Start session as controller
    let session_id = SessionId("smoke-test-session".to_string());
    let target_device_id = DeviceId("remote-agent".to_string());

    let start_response = client.send_request(IpcRequest::StartSession {
        session_id: session_id.clone(),
        target_device_id,
        transport_kind: "quic".to_string(),
    }).await;

    match start_response {
        Ok(IpcResponse::SessionStarted { .. }) => {}
        Ok(_) => panic!("Expected SessionStarted response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Step 4: Get session snapshot
    let snapshot_response = client.send_request(IpcRequest::SessionRuntimeSnapshot {
        session_id: session_id.clone(),
    }).await;

    match snapshot_response {
        Ok(IpcResponse::SessionSnapshot { snapshot }) => {
            assert_eq!(snapshot.session_id, session_id);
            assert_eq!(snapshot.role, "controller");
            // State could be "created" or "connecting" depending on implementation
            assert!(!snapshot.state.is_empty());
        }
        Ok(_) => panic!("Expected SessionSnapshot response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Step 5: Stop session
    let stop_response = client.send_request(IpcRequest::StopSession {
        session_id: session_id.clone(),
    }).await;

    match stop_response {
        Ok(IpcResponse::SessionStopped { .. }) => {}
        Ok(_) => panic!("Expected SessionStopped response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Step 6: Verify session is gone
    let snapshot_response = client.send_request(IpcRequest::SessionRuntimeSnapshot {
        session_id,
    }).await;

    match snapshot_response {
        Ok(IpcResponse::Error { code, .. }) => {
            assert_eq!(code, "E404", "Session should not exist after stop");
        }
        Ok(_) => panic!("Expected error for non-existent session"),
        Err(e) => panic!("IPC error: {}", e),
    }
}

#[tokio::test]
async fn hard_cut_runtime_snapshot_aggregates_state() {
    let mut client = IpcClient::new();

    // Start a session first
    let session_id = SessionId("snapshot-test-session".to_string());
    let _ = client.send_request(IpcRequest::StartSession {
        session_id: session_id.clone(),
        target_device_id: DeviceId("agent".to_string()),
        transport_kind: "webrtc".to_string(),
    }).await;

    // Get runtime snapshot
    let response = client.send_request(IpcRequest::RuntimeSnapshot).await;

    match response {
        Ok(IpcResponse::RuntimeSnapshot { snapshot }) => {
            // Should have our session
            assert!(!snapshot.sessions.is_empty(), "Should have at least one session");
            assert!(snapshot.is_registered, "Should be registered after earlier operations");
        }
        Ok(_) => panic!("Expected RuntimeSnapshot response"),
        Err(e) => panic!("IPC error: {}", e),
    }
}

#[tokio::test]
async fn hard_cut_list_sessions_returns_active_sessions() {
    let mut client = IpcClient::new();

    // Start two sessions
    let session1 = SessionId("list-test-1".to_string());
    let session2 = SessionId("list-test-2".to_string());

    for session_id in [&session1, &session2] {
        let _ = client.send_request(IpcRequest::StartSession {
            session_id: session_id.clone(),
            target_device_id: DeviceId("agent".to_string()),
            transport_kind: "quic".to_string(),
        }).await;
    }

    // List sessions
    let response = client.send_request(IpcRequest::ListSessions).await;

    match response {
        Ok(IpcResponse::SessionList { sessions }) => {
            assert!(sessions.len() >= 2, "Should have at least 2 sessions");
        }
        Ok(_) => panic!("Expected SessionList response"),
        Err(e) => panic!("IPC error: {}", e),
    }

    // Cleanup
    for session_id in [&session1, &session2] {
        let _ = client.send_request(IpcRequest::StopSession {
            session_id: session_id.clone(),
        }).await;
    }
}
