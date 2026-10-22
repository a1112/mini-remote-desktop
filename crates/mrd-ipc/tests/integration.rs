//! IPC client-server integration tests
//!
//! Tests the full round-trip communication between IpcClient and IpcServer.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mrd_ipc::{
    client::IpcClient,
    transport::{IpcEndpoint, IpcServer},
    IpcRequest, IpcResponse, ServiceStatus,
};
use mrd_proto::{DeviceId, SessionId};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_ENDPOINT: AtomicU64 = AtomicU64::new(1);

#[cfg(unix)]
struct PrivateFixtureDirectory(std::path::PathBuf);

#[cfg(unix)]
impl Drop for PrivateFixtureDirectory {
    fn drop(&mut self) {
        // The transport unlinks only its own socket inode before this directory.
        // Never recursively remove a directory or replace an existing endpoint.
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Use a real isolated transport server, rather than an ambient installed service.
/// Production product-client/kernel authentication is covered by its own tests.
async fn isolated_server(requests: Vec<IpcRequest>) -> (IpcClient, tokio::task::JoinHandle<()>) {
    let id = format!(
        "{}-{:x}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ENDPOINT.fetch_add(1, Ordering::Relaxed)
    );
    #[cfg(windows)]
    let endpoint = IpcEndpoint::named_pipe(format!(r"\\.\pipe\mrd-ipc-fixture-{id}"));
    #[cfg(unix)]
    let (endpoint, directory) = {
        use std::os::unix::fs::DirBuilderExt;
        // Keep the pathname below macOS's Unix-socket length limit.
        let directory = std::path::PathBuf::from("/tmp").join(format!("mrd-ipc-fixture-{id}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        (
            IpcEndpoint::unix_socket(directory.join("service.sock").to_string_lossy()),
            PrivateFixtureDirectory(directory),
        )
    };
    let server = IpcServer::bind_with_endpoint(endpoint.clone())
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        #[cfg(unix)]
        let directory = directory;
        let mut stream = server.accept().await.unwrap();
        for expected in requests {
            let request = stream.recv_request().await.unwrap();
            assert_eq!(request, expected);
            let response = match request {
                IpcRequest::ListDevices => IpcResponse::DeviceList { devices: vec![] },
                IpcRequest::ServiceHealth => IpcResponse::ServiceHealth {
                    status: ServiceStatus {
                        running: true,
                        healthy: true,
                        pid: Some(std::process::id()),
                    },
                },
                other => panic!("unexpected fixture request: {other:?}"),
            };
            stream.send_response(&response).await.unwrap();
        }
        drop(stream);
        drop(server);
        #[cfg(unix)]
        drop(directory);
    });
    let client = IpcClient::with_config_and_endpoint(
        mrd_ipc::client::ReconnectConfig {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(100),
            enabled: true,
        },
        endpoint,
    );
    (client, task)
}

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
    let (mut client, server) = isolated_server(vec![IpcRequest::ListDevices]).await;
    let response = tokio::time::timeout(
        FIXTURE_TIMEOUT,
        client.send_request(IpcRequest::ListDevices),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response, IpcResponse::DeviceList { devices: vec![] });
    assert!(client.is_connected());
    tokio::time::timeout(FIXTURE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
}

/// Test ServiceHealth request
#[tokio::test]
async fn ipc_client_sends_service_health_request() {
    let (mut client, server) = isolated_server(vec![IpcRequest::ServiceHealth]).await;
    let response = tokio::time::timeout(
        FIXTURE_TIMEOUT,
        client.send_request(IpcRequest::ServiceHealth),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        response,
        IpcResponse::ServiceHealth {
            status: ServiceStatus {
                running: true,
                healthy: true,
                pid: Some(std::process::id()),
            },
        }
    );
    tokio::time::timeout(FIXTURE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
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
    let requests = vec![
        IpcRequest::ServiceHealth,
        IpcRequest::ListDevices,
        IpcRequest::ServiceHealth,
    ];
    let (mut client, server) = isolated_server(requests.clone()).await;
    for request in requests {
        let response = tokio::time::timeout(FIXTURE_TIMEOUT, client.send_request(request.clone()))
            .await
            .unwrap()
            .unwrap();
        match request {
            IpcRequest::ListDevices => {
                assert_eq!(response, IpcResponse::DeviceList { devices: vec![] });
            }
            IpcRequest::ServiceHealth => assert!(matches!(
                response,
                IpcResponse::ServiceHealth { status }
                    if status.running && status.healthy && status.pid == Some(std::process::id())
            )),
            _ => unreachable!(),
        }
        assert!(client.is_connected());
    }
    client.disconnect();
    assert!(!client.is_connected());
    tokio::time::timeout(FIXTURE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
}
