#![allow(dead_code)]

// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use crate::{
    app_state::AppState,
    shell::{AutostartPortRef, UiLauncherPortRef},
};
#[cfg(test)]
use mrd_application::ports::SessionLifecycleState;
#[cfg(test)]
use mrd_application::ports::SessionSnapshot;
#[cfg(test)]
use mrd_ipc::CapabilityStatus;
use mrd_ipc::{transport, IpcRequest, IpcResponse};
use mrd_proto::{DeviceId, SessionId};
use std::{io::ErrorKind, sync::Arc};

#[cfg(windows)]
const WINDOWS_IPC_ACCEPT_BACKLOG: usize = 32;

mod dispatch;

/// IPC server - handles requests from Rdesk shell
#[derive(Clone)]
pub struct IpcServer {
    app_state: Arc<AppState>,
    endpoint: transport::IpcEndpoint,
    ui_launcher: UiLauncherPortRef,
    autostart: AutostartPortRef,
}

impl IpcServer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self::new_with_endpoint(
            app_state,
            transport::IpcEndpoint::service_from_env_or_default(),
        )
    }

    pub fn new_with_endpoint(app_state: Arc<AppState>, endpoint: transport::IpcEndpoint) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher: crate::shell::default_ui_launcher(),
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    pub fn new_with_launcher(
        app_state: Arc<AppState>,
        endpoint: transport::IpcEndpoint,
        ui_launcher: UiLauncherPortRef,
    ) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher,
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    /// Handle a single connection
    pub async fn handle_connection(&self, mut stream: transport::IpcStream) -> anyhow::Result<()> {
        loop {
            match stream.recv_request().await {
                Ok(request) => {
                    let response = self.handle_request(request).await;
                    if let Err(e) = stream.send_response(&response).await {
                        eprintln!("Failed to send IPC response: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    if !is_connection_closed_error(&e) {
                        eprintln!("IPC request error: {}", e);
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    /// Handle an IPC request and return a response
    pub async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        dispatch::dispatch_request(self, request).await
    }

    /// Get access to the app state (for testing/integration)
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.app_state
    }

    async fn local_device_id(&self) -> Option<DeviceId> {
        self.app_state
            .devices
            .lock()
            .await
            .get_local_device()
            .map(|(device_id, _)| device_id.clone())
    }

    async fn session_audit_context(
        &self,
        session_id: &SessionId,
    ) -> (Option<DeviceId>, Option<String>) {
        let sessions = self.app_state.sessions.lock().await;
        let Some(snapshot) = sessions.get(session_id) else {
            return (None, None);
        };
        let peer_device_id = snapshot
            .target_device_id
            .clone()
            .or_else(|| snapshot.source_device_id.clone());
        (peer_device_id, Some(snapshot.transport.clone()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_audit_event(
        &self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) {
        self.app_state.audit_log.lock().await.record(
            action,
            outcome,
            session_id,
            actor_device_id,
            peer_device_id,
            transport_kind,
            reason,
            details,
        );
    }

    /// Run the IPC server (accepts connections in a loop)
    pub async fn run(&self) -> anyhow::Result<()> {
        #[cfg(windows)]
        let server =
            Arc::new(transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?);

        #[cfg(not(windows))]
        let server = transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?;

        tracing::info!("IPC server listening");

        #[cfg(windows)]
        {
            let mut workers = tokio::task::JoinSet::new();
            for _ in 0..WINDOWS_IPC_ACCEPT_BACKLOG {
                let pipe_server = server.clone();
                let connection_server = self.clone();
                workers.spawn(async move {
                    loop {
                        match pipe_server.accept().await {
                            Ok(stream) => {
                                if let Err(e) = connection_server.handle_connection(stream).await {
                                    eprintln!("IPC connection error: {}", e);
                                }
                            }
                            Err(e) => {
                                eprintln!("IPC accept error: {}", e);
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            }
                        }
                    }
                });
            }

            while let Some(result) = workers.join_next().await {
                if let Err(e) = result {
                    eprintln!("IPC accept worker stopped: {}", e);
                }
            }

            Ok(())
        }

        #[cfg(not(windows))]
        {
            let app_state = self.app_state.clone();
            let ui_launcher = self.ui_launcher.clone();
            loop {
                match server.accept().await {
                    Ok(stream) => {
                        let server_clone = IpcServer {
                            app_state: app_state.clone(),
                            endpoint: self.endpoint.clone(),
                            ui_launcher: ui_launcher.clone(),
                            autostart: crate::shell::default_autostart("mrd-service"),
                        };
                        tokio::spawn(async move {
                            if let Err(e) = server_clone.handle_connection(stream).await {
                                eprintln!("IPC connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("IPC accept error: {}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

fn is_connection_closed_error(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<std::io::Error>() {
        Some(io_error) => matches!(
            io_error.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
        ),
        None => false,
    }
}

fn audit_outcome(response: &IpcResponse) -> (&'static str, Option<String>) {
    match response {
        IpcResponse::Error { message, .. } => ("error", Some(message.clone())),
        _ => ("success", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::{DeviceId, SessionId};
    use std::sync::Arc;

    #[test]
    fn closed_ipc_connection_is_not_treated_as_request_error() {
        let error = anyhow::Error::new(std::io::Error::new(ErrorKind::UnexpectedEof, "early eof"));

        assert!(is_connection_closed_error(&error));
    }

    #[tokio::test]
    async fn session_snapshot_returns_correct_ipc_format() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: Some("127.0.0.1:4433".to_string()),
            local_server_name: Some("localhost".to_string()),
            local_cert_der_b64: Some("AQID".to_string()),
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Listening,
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        server
            .app_state()
            .sessions()
            .lock()
            .await
            .insert(session_id.clone(), snapshot);

        let request = IpcRequest::SessionRuntimeSnapshot {
            session_id: session_id.clone(),
        };
        let response = server.handle_request(request).await;

        match response {
            IpcResponse::SessionSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.state, "listening"); // Only local bootstrap
                assert_eq!(snapshot.transport_kind, "quic");
            }
            _ => panic!("Expected SessionSnapshot response"),
        }
    }

    #[tokio::test]
    async fn list_sessions_returns_active_sessions() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Created,
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        server
            .app_state()
            .sessions()
            .lock()
            .await
            .insert(session_id.clone(), snapshot);

        let response = server.handle_request(IpcRequest::ListSessions).await;

        match response {
            IpcResponse::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].session_id, session_id);
                assert_eq!(sessions[0].role, "controller");
                assert_eq!(
                    sessions[0].peer_device_id,
                    Some(DeviceId("agent".to_string()))
                );
            }
            _ => panic!("Expected SessionList response"),
        }
    }

    #[tokio::test]
    async fn wake_on_lan_rejects_invalid_mac_before_sending() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = server
            .handle_request(IpcRequest::WakeOnLan {
                device_id: DeviceId("agent-device".to_string()),
                mac_address: "not-a-mac".to_string(),
                broadcast_addr: None,
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_WAKE_ON_LAN");
                assert!(message.contains("invalid Wake-on-LAN MAC address"));
            }
            _ => panic!("expected Wake-on-LAN validation error"),
        }
    }

    #[tokio::test]
    async fn list_local_capture_sources_returns_local_response_or_error() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = server
            .handle_request(IpcRequest::ListLocalCaptureSources {
                include_previews: false,
                limit: Some(4),
            })
            .await;

        match response {
            IpcResponse::LocalCaptureSourceList { sources } => {
                assert!(sources.len() <= 4);
            }
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "CAPTURE_SOURCE_LIST_FAILED");
                assert!(!message.trim().is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn runtime_snapshot_aggregates_state() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let device_id = DeviceId("test-device".to_string());
        let _ = server
            .handle_request(IpcRequest::RegisterDevice {
                device_id: device_id.clone(),
                device_name: "Test Device".to_string(),
            })
            .await;

        let response = server.handle_request(IpcRequest::RuntimeSnapshot).await;

        match response {
            IpcResponse::RuntimeSnapshot { snapshot } => {
                assert!(snapshot.is_registered);
                assert_eq!(snapshot.device_id, Some(device_id));
            }
            _ => panic!("Expected RuntimeSnapshot response"),
        }
    }

    #[tokio::test]
    async fn capability_snapshot_reports_structured_capabilities() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        match response {
            IpcResponse::CapabilitySnapshot { snapshot } => {
                assert_eq!(snapshot.schema_version, 1);
                assert!(snapshot
                    .capabilities
                    .iter()
                    .any(|item| item.id == "transport.quic_datagram"));
                assert!(snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == "lan.2k144"));
                assert!(snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == "lan.1600p165"));
            }
            _ => panic!("Expected CapabilitySnapshot response"),
        }
    }

    #[tokio::test]
    async fn capability_snapshot_returns_cached_app_state_fact_source() {
        let app_state = Arc::new(AppState::new());
        let cached = crate::capabilities::local_capability_snapshot_static();
        app_state
            .replace_capability_snapshot_for_test(cached.clone())
            .await;
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        match response {
            IpcResponse::CapabilitySnapshot { snapshot } => {
                assert_eq!(snapshot.updated_at_ms, cached.updated_at_ms);
                assert_eq!(snapshot.capabilities.len(), cached.capabilities.len());
            }
            _ => panic!("Expected CapabilitySnapshot response"),
        }
    }

    #[tokio::test]
    async fn capability_snapshot_marks_keyboard_mouse_unavailable_when_injector_is_unavailable() {
        let app_state = Arc::new(AppState::new());
        let cached = crate::capabilities::local_capability_snapshot_static();
        app_state.replace_capability_snapshot_for_test(cached).await;
        app_state
            .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new(
                "blocked by test",
            ))
            .await;
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        let snapshot = match response {
            IpcResponse::CapabilitySnapshot { snapshot } => snapshot,
            other => panic!("expected capability snapshot, got {other:?}"),
        };
        let control = snapshot
            .capabilities
            .iter()
            .find(|item| item.id == "control.keyboard_mouse")
            .expect("keyboard/mouse control capability");
        assert_eq!(control.status, CapabilityStatus::Unsupported);
        assert!(control
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("Input injector is unavailable"));
    }

    #[tokio::test]
    async fn start_session_preflight_blocks_unsupported_transport_before_start() {
        let app_state = Arc::new(AppState::new());
        let mut cached = crate::capabilities::local_capability_snapshot_static();
        set_capability_status(
            &mut cached,
            "transport.quic",
            mrd_ipc::CapabilityStatus::Unsupported,
            "QUIC disabled for preflight test",
        );
        app_state.replace_capability_snapshot_for_test(cached).await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("preflight-blocked-session".to_string());

        let response = server
            .handle_request(IpcRequest::StartSession {
                session_id: session_id.clone(),
                target_device_id: DeviceId("target".to_string()),
                transport_kind: "quic".to_string(),
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_PREFLIGHT");
                assert!(message.contains("transport.quic"));
            }
            _ => panic!("Expected preflight error response"),
        }
        assert!(app_state.sessions.lock().await.get(&session_id).is_none());
    }

    #[tokio::test]
    async fn start_lan_remote_session_preflight_blocks_before_peer_request() {
        let app_state = Arc::new(AppState::new());
        let mut cached = crate::capabilities::local_capability_snapshot_static();
        set_capability_status(
            &mut cached,
            "transport.quic_datagram",
            mrd_ipc::CapabilityStatus::Unsupported,
            "QUIC datagram disabled for preflight test",
        );
        app_state.replace_capability_snapshot_for_test(cached).await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("lan-preflight-blocked-session".to_string());

        let response = server
            .handle_request(IpcRequest::StartLanRemoteSession {
                session_id: session_id.clone(),
                target_device_id: DeviceId("target".to_string()),
                transport_kind: "quic".to_string(),
                requested_profile: Some(mrd_ipc::MediaProfile {
                    width: 2560,
                    height: 1440,
                    fps: 144,
                    bitrate_mbps: 64,
                    codec: "h264".to_string(),
                    ..mrd_ipc::MediaProfile::default()
                }),
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_PREFLIGHT");
                assert!(message.contains("transport.quic_datagram"));
            }
            _ => panic!("Expected preflight error response"),
        }
        assert!(app_state.sessions.lock().await.get(&session_id).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_h264_2k144_profile_uses_native_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(
            crate::handlers::preflight::scenario_id_for_profile(&profile),
            "lan.macos.2k144"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_hevc_2k144_profile_uses_native_hevc_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(
            crate::handlers::preflight::scenario_id_for_profile(&profile),
            "lan.macos.hevc.2k144"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_h264_smoke_profile_uses_native_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 10,
            codec: "h264".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(
            crate::handlers::preflight::scenario_id_for_profile(&profile),
            "lan.macos.2k144"
        );
    }

    #[tokio::test]
    async fn control_input_request_updates_reliable_lane_counters() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("control-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Listening,
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            })
            .await;

        assert_eq!(
            response,
            IpcResponse::ControlInputAccepted {
                session_id: session_id.clone(),
                lane: mrd_ipc::ControlInputLane::Reliable,
                event_count: 1,
            }
        );

        let snapshot = match server
            .handle_request(IpcRequest::GetControlChannelSnapshot {
                session_id: session_id.clone(),
            })
            .await
        {
            IpcResponse::ControlChannelSnapshot { snapshot } => snapshot,
            other => panic!("expected control snapshot, got {other:?}"),
        };

        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 1);
        assert_eq!(snapshot.reliable.failed_messages, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_rejects_missing_session() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("missing-control-input-session".to_string());

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("session not found"));
            }
            other => panic!("expected missing session control input error, got {other:?}"),
        }
        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.reliable.injected_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_rejects_closed_session() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("closed-control-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Closed,
                last_error: None,
                sender_active: false,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.to_ascii_lowercase().contains("closed"));
            }
            other => panic!("expected closed session control input error, got {other:?}"),
        }
        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.reliable.injected_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_for_controller_session_requires_lan_peer() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("controller-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Streaming,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("LAN peer not found"));
            }
            other => panic!("expected control input routing error, got {other:?}"),
        }

        let snapshot = app_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_for_controller_session_requires_streaming_receiver() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("controller-input-not-ready-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("streaming receiver"));
            }
            other => panic!("expected control input readiness error, got {other:?}"),
        }

        let snapshot = app_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_records_injection_failure() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new(
                "blocked by test",
            ))
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("control-input-failure-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Listening,
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("blocked by test"));
            }
            other => panic!("expected control input error, got {other:?}"),
        }

        let snapshot = match server
            .handle_request(IpcRequest::GetControlChannelSnapshot {
                session_id: session_id.clone(),
            })
            .await
        {
            IpcResponse::ControlChannelSnapshot { snapshot } => snapshot,
            other => panic!("expected control snapshot, got {other:?}"),
        };

        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 0);
        assert_eq!(snapshot.reliable.failed_messages, 1);
        assert_eq!(
            snapshot.reliable.last_error.as_deref(),
            Some("input injector unavailable: blocked by test")
        );
    }

    fn set_capability_status(
        snapshot: &mut mrd_ipc::CapabilitySnapshot,
        id: &str,
        status: mrd_ipc::CapabilityStatus,
        reason: &str,
    ) {
        let capability = snapshot
            .capabilities
            .iter_mut()
            .find(|item| item.id == id)
            .expect("capability in snapshot");
        capability.status = status;
        capability.reason = Some(reason.to_string());
    }
}
