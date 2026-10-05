use std::sync::{Arc, Mutex};

use mrd_ipc::{IpcRequest, IpcResponse, OpenUiReason, RemoteDevicePowerAction, ShutdownMode};
use mrd_proto::{DeviceId, SessionId};
use mrd_service::{ipc_server::IpcServer, AppState};

#[tokio::test]
async fn management_pipe_denies_files_session_remote_power_and_ui_process_control() {
    let state = Arc::new(AppState::new());
    let server = IpcServer::new_management(state.clone());
    let denied = [
        IpcRequest::ListDirectory {
            path: Some(".".into()),
        },
        IpcRequest::ListDevices,
        IpcRequest::ListSessions,
        IpcRequest::StartSession {
            session_id: SessionId("denied".into()),
            target_device_id: DeviceId("denied".into()),
            transport_kind: "quic".into(),
        },
        IpcRequest::RequestRemoteDevicePowerAction {
            device_id: DeviceId("denied".into()),
            action: RemoteDevicePowerAction::Shutdown,
        },
        IpcRequest::UiAttached {
            pid: 1234,
            executable_path: Some("C:\\malicious.exe".into()),
        },
        IpcRequest::OpenUi {
            reason: OpenUiReason::UserRequest,
        },
    ];
    for request in denied {
        let response = server.handle_request(request).await;
        assert!(
            matches!(response, IpcResponse::Error { code, .. } if code == "E_MANAGEMENT_COMMAND_DENIED")
        );
    }
    assert!(state.sessions.lock().await.list_all().is_empty());
    assert_eq!(state.shell.lock().await.ui_pid, None);
}

#[tokio::test]
async fn management_pipe_allows_health_shell_autostart_and_shutdown() {
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let server = IpcServer::new_management(state).with_autostart(Arc::new(Mutex::new(
        mrd_service::NoOpAutostart::new("test"),
    )));
    assert!(matches!(
        server.handle_request(IpcRequest::ServiceHealth).await,
        IpcResponse::ServiceHealth { .. }
    ));
    assert!(matches!(
        server.handle_request(IpcRequest::GetShellStatus).await,
        IpcResponse::ShellStatus { .. }
    ));
    for request in [
        IpcRequest::GetAutostartStatus,
        IpcRequest::SetAutostart { enabled: false },
    ] {
        let response = server.handle_request(request).await;
        assert!(
            !matches!(response, IpcResponse::Error { code, .. } if code == "E_MANAGEMENT_COMMAND_DENIED")
        );
    }
    assert!(matches!(
        server
            .handle_request(IpcRequest::ShutdownService {
                mode: ShutdownMode::Graceful
            })
            .await,
        IpcResponse::Ack
    ));
}

#[tokio::test]
async fn real_management_pipe_enforces_the_allowlist_and_delivers_shutdown_ack() {
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    #[cfg(windows)]
    let endpoint = mrd_ipc::transport::IpcEndpoint::named_pipe(format!(
        r"\\.\pipe\mrd-management-smoke-{unique}"
    ));
    #[cfg(unix)]
    let endpoint = {
        let mrd_ipc::transport::IpcEndpoint::UnixSocket(default) =
            mrd_ipc::transport::IpcEndpoint::default_service();
        mrd_ipc::transport::IpcEndpoint::unix_socket(
            std::path::Path::new(&default)
                .with_file_name(format!("mrd-management-smoke-{unique}.sock"))
                .to_string_lossy(),
        )
    };
    let server = IpcServer::new_management_with_endpoint(state.clone(), endpoint.clone())
        .with_autostart(Arc::new(Mutex::new(mrd_service::NoOpAutostart::new(
            "test",
        ))));
    let task = tokio::spawn(async move { server.run().await });
    let mut client = mrd_ipc::client::IpcClient::management_with_endpoint(endpoint.clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(response) = client
                .send_request_no_reconnect(IpcRequest::ServiceHealth)
                .await
            {
                assert!(matches!(response, IpcResponse::ServiceHealth { .. }));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        client
            .send_request_no_reconnect(IpcRequest::GetShellStatus)
            .await
            .unwrap(),
        IpcResponse::ShellStatus { .. }
    ));
    assert!(
        matches!(client.send_request_no_reconnect(IpcRequest::ListDirectory { path: Some(".".into()) }).await.unwrap(), IpcResponse::Error { code, .. } if code == "E_MANAGEMENT_COMMAND_DENIED")
    );
    assert!(matches!(
        client
            .send_request_no_reconnect(IpcRequest::ShutdownService {
                mode: ShutdownMode::Graceful
            })
            .await
            .unwrap(),
        IpcResponse::Ack
    ));
    assert_eq!(
        state
            .shutdown
            .ready_mode_at_epoch(0, state.shutdown.admission_epoch()),
        Some(ShutdownMode::Graceful)
    );
    client.disconnect();
    task.abort();
    let _ = task.await;
    #[cfg(unix)]
    {
        let mrd_ipc::transport::IpcEndpoint::UnixSocket(path) = endpoint;
        assert!(!std::path::Path::new(&path).exists());
    }
}
