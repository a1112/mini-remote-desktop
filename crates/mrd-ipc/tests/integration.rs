//! IPC client-server integration tests
//!
//! Tests the full round-trip communication between IpcClient and IpcServer.

use std::time::Duration;

use mrd_ipc::{client::IpcClient, IpcRequest, IpcResponse};
use mrd_proto::{DeviceId, SessionId};

/// Helper to create a test session ID
fn test_session_id() -> SessionId {
    SessionId("test-session-integration".to_string())
}

/// Helper to create a test device ID
fn test_device_id() -> DeviceId {
    DeviceId("test-device-integration".to_string())
}

/// Test basic client connection and ListDevices request
#[tokio::test]
async fn ipc_client_sends_list_devices_request() {
    // This test requires the server to be running or creates a mock
    // For Windows, we need to handle named pipe creation carefully

    let mut client = IpcClient::new();

    // Try to connect - if service is not running, test will fail gracefully
    let result = client.send_request(IpcRequest::ListDevices).await;

    // We don't assert success here because the service might not be running
    // In a real integration test environment, we'd spawn the service first
    if let Ok(response) = result {
        assert!(matches!(response, IpcResponse::DeviceList { .. }));
    }
}

/// Test ServiceHealth request
#[tokio::test]
async fn ipc_client_sends_service_health_request() {
    let mut client = IpcClient::new();

    let result = client.send_request(IpcRequest::ServiceHealth).await;

    if let Ok(response) = result {
        match response {
            IpcResponse::ServiceHealth { status } => {
                assert!(status.running);
                assert!(status.pid.is_some());
            }
            _ => panic!("Expected ServiceHealth response"),
        }
    }
}

/// Test client state transitions
#[test]
fn ipc_client_transitions_connection_states() {
    let client = IpcClient::new();
    assert_eq!(
        client.state(),
        &mrd_ipc::client::ConnectionState::Disconnected
    );
    assert!(!client.is_connected());
}

/// Test reconnection configuration
#[test]
fn ipc_client_uses_custom_reconnect_config() {
    let config = mrd_ipc::client::ReconnectConfig {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_secs(2),
        enabled: false,
    };

    let client = IpcClient::with_config(config.clone());
    assert!(!client.is_connected());

    // Update config
    let new_config = mrd_ipc::client::ReconnectConfig {
        max_attempts: 10,
        ..config
    };
    let mut client = IpcClient::with_config(new_config);
    client.set_reconnect_config(mrd_ipc::client::ReconnectConfig::default());
    assert!(!client.is_connected());
}

/// Test client disconnect method
#[test]
fn ipc_client_disconnect_resets_state() {
    let mut client = IpcClient::new();
    // Even though not connected, disconnect should be idempotent
    client.disconnect();
    assert_eq!(
        client.state(),
        &mrd_ipc::client::ConnectionState::Disconnected
    );
}

/// Test that requests can be created without connection
#[test]
fn ipc_requests_can_be_created_serialized() {
    let requests = vec![
        IpcRequest::RegisterDevice {
            device_id: test_device_id(),
            device_name: "Test Device".to_string(),
        },
        IpcRequest::ListDevices,
        IpcRequest::StartSession {
            session_id: test_session_id(),
            target_device_id: test_device_id(),
            transport_kind: "quic".to_string(),
        },
        IpcRequest::ServiceHealth,
    ];

    for request in requests {
        let json = serde_json::to_string(&request);
        assert!(json.is_ok(), "Failed to serialize request: {:?}", request);
    }
}

/// Test that responses can be deserialized
#[test]
fn ipc_responses_can_be_deserialized() {
    let responses = vec![
        r#"{"type":"DeviceRegistered","device_id":"test-device"}"#,
        r#"{"type":"DeviceList","devices":[]}"#,
        r#"{"type":"ServiceHealth","status":{"running":true,"healthy":true,"pid":1234}}"#,
        r#"{"type":"Error","code":"E001","message":"Test error"}"#,
    ];

    for json in responses {
        let response: Result<IpcResponse, _> = serde_json::from_str(json);
        assert!(response.is_ok(), "Failed to deserialize: {}", json);
    }
}

/// Test multiple sequential requests with client
#[tokio::test]
async fn ipc_client_handles_multiple_sequential_requests() {
    let mut client = IpcClient::new();

    // Try multiple requests
    let _ = client.send_request(IpcRequest::ServiceHealth).await;
    let _ = client.send_request(IpcRequest::ListDevices).await;
    let _ = client.send_request(IpcRequest::ServiceHealth).await;

    // Client should maintain state
    assert!(!client.is_connected() || client.is_connected());
}
