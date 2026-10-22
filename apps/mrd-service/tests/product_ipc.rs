#![cfg(windows)]

use mrd_ipc::{
    client::{IpcClient, ReconnectConfig},
    transport::IpcEndpoint,
    IpcRequest, IpcResponse,
};
use mrd_service::{app_state::AppState, ipc_server::IpcServer};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn ordinary_uninstalled_process_is_denied_before_any_product_command() {
    let endpoint = IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\product-untrusted-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state = Arc::new(AppState::new());
    let server = IpcServer::new_product_with_endpoint(state.clone(), endpoint.clone());
    let (ready, announced) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move { server.run_with_ready(ready).await });
    tokio::time::timeout(Duration::from_secs(3), announced)
        .await
        .unwrap()
        .unwrap();
    // Transport data-only access succeeds; authorization rejects the real
    // pipe caller executable because this test lives in a build directory.
    for request in [
        IpcRequest::RuntimeSnapshot,
        IpcRequest::GetPublicDeviceBindingProtocol,
        IpcRequest::BindPublicDevice {
            protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
            user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                .unwrap(),
        },
        IpcRequest::UnbindPublicDevice {
            protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
            user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                .unwrap(),
        },
        IpcRequest::UiAttached {
            pid: std::process::id(),
            executable_path: Some(r"C:\Program Files\MiniRemoteDesktop\Rdesk.exe".into()),
        },
        IpcRequest::UiAttached {
            pid: 4,
            executable_path: Some(r"C:\Program Files\MiniRemoteDesktop\Rdesk.exe".into()),
        },
        IpcRequest::StartSession {
            session_id: mrd_proto::SessionId("unauthorized".into()),
            target_device_id: mrd_proto::DeviceId("peer".into()),
            transport_kind: "quic".into(),
        },
    ] {
        let mut client = IpcClient::management_with_config_and_endpoint(
            ReconnectConfig {
                enabled: false,
                ..Default::default()
            },
            endpoint.clone(),
        );
        let response = tokio::time::timeout(Duration::from_secs(3), client.send_request(request))
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(response, IpcResponse::Error { code, .. } if code == "E_PRODUCT_CALLER_DENIED")
        );
    }
    assert!(state.shell.lock().await.ui_pid.is_none());
    assert!(state
        .sessions
        .lock()
        .await
        .get(&mrd_proto::SessionId("unauthorized".into()))
        .is_none());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn interactive_management_pipe_cannot_proxy_device_binding() {
    let endpoint = IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\binding-management-deny-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state = Arc::new(AppState::new());
    let server = IpcServer::new_management_with_endpoint(state, endpoint.clone());
    let (ready, announced) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move { server.run_with_ready(ready).await });
    tokio::time::timeout(Duration::from_secs(3), announced)
        .await
        .unwrap()
        .unwrap();
    let mut client = IpcClient::management_with_config_and_endpoint(
        ReconnectConfig {
            enabled: false,
            ..Default::default()
        },
        endpoint,
    );
    assert!(matches!(
        client
            .send_request_no_reconnect(IpcRequest::GetPublicDeviceBindingProtocol)
            .await
            .unwrap(),
        IpcResponse::PublicDeviceBindingProtocol {
            protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR
        }
    ));
    for request in [
        IpcRequest::BindPublicDevice {
            protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
            user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                .unwrap(),
        },
        IpcRequest::UnbindPublicDevice {
            protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
            user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                .unwrap(),
        },
    ] {
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            client.send_request_no_reconnect(request),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(response, IpcResponse::Error { ref code, .. } if code == "E_MANAGEMENT_COMMAND_DENIED")
        );
        assert!(!format!("{response:?}").contains("user.access.token"));
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn product_frame_budget_rejects_the_length_before_reading_untrusted_payload() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let endpoint = IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\product-frame-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let server = IpcServer::new_product_with_endpoint(Arc::new(AppState::new()), endpoint.clone());
    let (ready, announced) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move { server.run_with_ready(ready).await });
    announced.await.unwrap();
    let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(endpoint.as_windows_pipe_name())
        .unwrap();
    client.write_all(&u32::MAX.to_le_bytes()).await.unwrap();
    let mut response = [0_u8; 1];
    let result = tokio::time::timeout(Duration::from_millis(500), client.read(&mut response))
        .await
        .unwrap();
    assert!(
        matches!(result, Ok(0) | Err(_)),
        "Oversized caller was retained"
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn product_idle_unauthenticated_connection_is_closed_within_the_admission_budget() {
    use tokio::io::AsyncReadExt;
    let endpoint = IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\product-idle-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let server = IpcServer::new_product_with_endpoint(Arc::new(AppState::new()), endpoint.clone());
    let (ready, announced) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move { server.run_with_ready(ready).await });
    announced.await.unwrap();
    let mrd_ipc::transport::IpcStream::Client(mut client) =
        mrd_ipc::transport::IpcClient::connect_management_with_endpoint(&endpoint)
            .await
            .unwrap()
    else {
        panic!("Expected pipe client");
    };
    let mut response = [0_u8; 1];
    let result = tokio::time::timeout(Duration::from_secs(4), client.read(&mut response))
        .await
        .unwrap();
    assert!(
        matches!(result, Ok(0) | Err(_)),
        "Unauthenticated idle caller was retained"
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn product_server_has_no_unauthenticated_in_process_dispatch_bypass() {
    let server = IpcServer::new_product(Arc::new(AppState::new()));
    let response = server.handle_request(IpcRequest::RuntimeSnapshot).await;
    assert!(
        matches!(response, IpcResponse::Error { code, .. } if code == "E_PRODUCT_CALLER_DENIED")
    );
}

#[tokio::test]
async fn product_binding_rejects_first_instance_squatting() {
    let endpoint = IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\product-collision-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _hostile = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .create(endpoint.as_windows_pipe_name())
        .unwrap();
    let server = IpcServer::new_product_with_endpoint(Arc::new(AppState::new()), endpoint);
    let (ready, announced) = tokio::sync::oneshot::channel();
    assert!(server.run_with_ready(ready).await.is_err());
    assert!(announced.await.is_err());
}
