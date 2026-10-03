#[cfg(windows)]
#[tokio::test]
async fn conflicting_management_endpoint_never_announces_readiness() {
    use mrd_ipc::transport::{IpcEndpoint, IpcServer as PipeServer};
    use mrd_service::{ipc_server::IpcServer, AppState};
    use std::sync::Arc;
    let endpoint =
        IpcEndpoint::from_env_value(&format!(r"\\.\pipe\mrd-readiness-{}", std::process::id()))
            .unwrap();
    let _owner = PipeServer::bind_management_with_endpoint(endpoint.clone())
        .await
        .unwrap();
    let server = IpcServer::new_management_with_endpoint(Arc::new(AppState::new()), endpoint);
    let (ready, receiver) = tokio::sync::oneshot::channel();
    assert!(server.run_with_ready(ready).await.is_err());
    assert!(
        receiver.await.is_err(),
        "failed bind must never report ready"
    );
}
