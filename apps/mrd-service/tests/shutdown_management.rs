use mrd_ipc::{IpcRequest, IpcResponse, ShutdownMode};
use mrd_service::{ipc_server::IpcServer, AppState};
use std::sync::Arc;

#[test]
fn shutdown_waits_for_response_and_session_drain() {
    let coordinator = Arc::new(mrd_service::shutdown::ShutdownCoordinator::default());
    let _runtime = coordinator.bind_runtime().unwrap();
    let admission = coordinator.admit().unwrap();
    coordinator.request(ShutdownMode::AfterSessions).unwrap();
    assert!(coordinator.admit().is_err());
    assert_eq!(
        coordinator.ready_mode_at_epoch(0, coordinator.admission_epoch()),
        None
    );
    coordinator.acknowledge(ShutdownMode::AfterSessions);
    assert_eq!(
        coordinator.ready_mode_at_epoch(0, coordinator.admission_epoch()),
        None
    );
    drop(admission);
    assert_eq!(
        coordinator.ready_mode_at_epoch(1, coordinator.admission_epoch()),
        None
    );
    assert_eq!(
        coordinator.ready_mode_at_epoch(0, coordinator.admission_epoch()),
        Some(ShutdownMode::AfterSessions)
    );
}

#[test]
fn shutdown_escalation_never_downgrades_or_exits_before_its_ack() {
    let coordinator = Arc::new(mrd_service::shutdown::ShutdownCoordinator::default());
    let _runtime = coordinator.bind_runtime().unwrap();
    coordinator.request(ShutdownMode::AfterSessions).unwrap();
    coordinator.acknowledge(ShutdownMode::AfterSessions);
    coordinator.request(ShutdownMode::Force).unwrap();
    assert_eq!(
        coordinator.ready_mode_at_epoch(2, coordinator.admission_epoch()),
        None
    );
    coordinator.acknowledge(ShutdownMode::Force);
    coordinator.request(ShutdownMode::Graceful).unwrap();
    assert_eq!(
        coordinator.ready_mode_at_epoch(2, coordinator.admission_epoch()),
        Some(ShutdownMode::Force)
    );
}

#[test]
fn runtime_registration_does_not_remain_available_after_drop() {
    let coordinator = Arc::new(mrd_service::shutdown::ShutdownCoordinator::default());
    let runtime = coordinator.bind_runtime().unwrap();
    assert!(coordinator.bind_runtime().is_err());
    drop(runtime);
    assert!(coordinator.request(ShutdownMode::Graceful).is_err());
}

#[tokio::test]
async fn pending_shutdown_blocks_new_sessions_but_allows_status() {
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let server = IpcServer::new(state);
    assert!(matches!(
        server
            .handle_request(IpcRequest::ShutdownService {
                mode: ShutdownMode::AfterSessions
            })
            .await,
        IpcResponse::Ack
    ));
    let response = server
        .handle_request(IpcRequest::StartSession {
            session_id: mrd_proto::SessionId("blocked".into()),
            target_device_id: mrd_proto::DeviceId("target".into()),
            transport_kind: "quic".into(),
        })
        .await;
    assert!(
        matches!(response, IpcResponse::Error { code, .. } if code == "E_SERVICE_SHUTTING_DOWN")
    );
    assert!(matches!(
        server.handle_request(IpcRequest::ServiceHealth).await,
        IpcResponse::ServiceHealth { status } if status.running && !status.healthy
    ));
}

#[tokio::test]
async fn shutdown_without_bound_runtime_reports_unavailable() {
    let server = IpcServer::new(Arc::new(AppState::new()));
    let response = server
        .handle_request(IpcRequest::ShutdownService {
            mode: ShutdownMode::Graceful,
        })
        .await;
    assert!(
        matches!(response, IpcResponse::Error { code, .. } if code == "E_SERVICE_SHUTDOWN_UNAVAILABLE")
    );
}

#[cfg(windows)]
#[tokio::test]
async fn shutdown_ack_is_received_before_runtime_closes_ipc() {
    use mrd_ipc::transport::{IpcClient, IpcEndpoint};
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let endpoint = IpcEndpoint::from_env_value(&format!(
        r"\\.\pipe\mrd-shutdown-test-{}",
        std::process::id()
    ))
    .unwrap();
    let server = IpcServer::new_with_endpoint(state.clone(), endpoint.clone());
    let task = tokio::spawn(async move { server.run().await });
    let mut stream = loop {
        match IpcClient::connect_with_endpoint(&endpoint).await {
            Ok(stream) => break stream,
            Err(_) => tokio::task::yield_now().await,
        }
    };
    stream
        .send_request(&IpcRequest::ShutdownService {
            mode: ShutdownMode::Graceful,
        })
        .await
        .unwrap();
    let runtime = tokio::spawn(async move {
        mrd_service::shutdown::wait_for_shutdown(&state).await;
        task.abort();
        let _ = task.await;
    });
    runtime.await.unwrap();
    assert!(matches!(
        stream.recv_response().await.unwrap(),
        IpcResponse::Ack
    ));
}

#[test]
fn stale_empty_session_snapshot_cannot_complete_drain_after_admission_finishes() {
    let coordinator = Arc::new(mrd_service::shutdown::ShutdownCoordinator::default());
    let _runtime = coordinator.bind_runtime().unwrap();
    let admission = coordinator.admit().unwrap();
    let epoch = coordinator.admission_epoch();
    // The runtime saw no projected sessions before this accepted admission
    // committed a session and released its permit.
    coordinator.request(ShutdownMode::AfterSessions).unwrap();
    coordinator.acknowledge(ShutdownMode::AfterSessions);
    drop(admission);
    assert_eq!(coordinator.ready_mode_at_epoch(0, epoch), None);
    assert_eq!(
        coordinator.ready_mode_at_epoch(1, coordinator.admission_epoch()),
        None
    );
}

#[tokio::test]
async fn force_shutdown_does_not_wait_for_session_registry_lock() {
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let _sessions = state.sessions.lock().await;
    state.shutdown.request(ShutdownMode::Force).unwrap();
    state.shutdown.acknowledge(ShutdownMode::Force);
    let mode = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        mrd_service::shutdown::wait_for_shutdown(&state),
    )
    .await;
    assert_eq!(mode.unwrap(), ShutdownMode::Force);
}

#[tokio::test]
async fn force_escalation_cancels_waiting_for_drain_snapshot() {
    let state = Arc::new(AppState::new());
    let _runtime = state.shutdown.bind_runtime().unwrap();
    let _sessions = state.sessions.lock().await;
    state.shutdown.request(ShutdownMode::AfterSessions).unwrap();
    state.shutdown.acknowledge(ShutdownMode::AfterSessions);
    let waiter_state = state.clone();
    let waiter =
        tokio::spawn(async move { mrd_service::shutdown::wait_for_shutdown(&waiter_state).await });
    tokio::task::yield_now().await;
    state.shutdown.request(ShutdownMode::Force).unwrap();
    state.shutdown.acknowledge(ShutdownMode::Force);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
            .await
            .unwrap()
            .unwrap(),
        ShutdownMode::Force
    );
}
