// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use mrd_ipc::{IpcRequest, IpcResponse, transport};
use mrd_application::ports::SessionSnapshot;
use mrd_proto::{SessionId, DeviceId};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::app_state::{AppState, SessionRegistry};

/// IPC server - handles requests from Rdesk shell
pub struct IpcServer {
    app_state: Arc<AppState>,
}

impl IpcServer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
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
                    // Connection closed or error
                    eprintln!("IPC request error: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Handle an IPC request and return a response
    pub async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        match request {
            IpcRequest::RegisterDevice { device_id, device_name } => {
                tracing::info!("Registering device: {} ({})", device_id.0, device_name);
                let mut devices = self.app_state.devices.lock().await;
                devices.register(device_id.clone(), device_name);
                IpcResponse::DeviceRegistered { device_id }
            }

            IpcRequest::ListDevices => {
                let devices = self.app_state.devices.lock().await;
                // Return the registered device, if any
                let device_list = if let Some((id, name)) = devices.get_local_device() {
                    vec![mrd_ipc::DeviceInfo {
                        device_id: id.clone(),
                        device_name: name.clone(),
                        is_online: true, // Local device is always online
                    }]
                } else {
                    vec![]
                };
                IpcResponse::DeviceList {
                    devices: device_list,
                }
            }

            IpcRequest::ListSessions => {
                let sessions = self.app_state.sessions.lock().await;
                let session_list = sessions.list_all().into_iter().map(|snap| {
                    mrd_ipc::SessionInfo {
                        session_id: snap.session_id.clone(),
                        role: if snap.target_device_id.is_some() {
                            "controller".to_string()
                        } else if snap.source_device_id.is_some() {
                            "agent".to_string()
                        } else {
                            "unknown".to_string()
                        },
                        state: if snap.local_listen_addr.is_some() && snap.remote_listen_addr.is_some() {
                            "connected".to_string()
                        } else if snap.local_listen_addr.is_some() {
                            "listening".to_string()
                        } else if snap.remote_listen_addr.is_some() {
                            "connecting".to_string()
                        } else {
                            "created".to_string()
                        },
                        transport_kind: snap.transport.clone(),
                    }
                }).collect();
                IpcResponse::SessionList { sessions: session_list }
            }

            IpcRequest::StartSession { session_id, target_device_id, transport_kind } => {
                tracing::info!("Starting session: {} -> {} via {}",
                    session_id.0, target_device_id.0, transport_kind);

                let mut sessions = self.app_state.sessions.lock().await;
                sessions.insert(session_id.clone(), SessionSnapshot {
                    session_id: session_id.clone(),
                    transport: transport_kind.clone(),
                    source_device_id: None,  // Will be set when initialized
                    target_device_id: Some(target_device_id),
                    local_listen_addr: None,
                    local_server_name: None,
                    local_cert_der_b64: None,
                    remote_listen_addr: None,
                    remote_server_name: None,
                    remote_cert_der_b64: None,
                    lifecycle_state: "connecting".to_string(),
                    last_error: None,
                    sender_active: false,
                    receiver_active: false,
                });

                IpcResponse::SessionStarted { session_id }
            }

            IpcRequest::AcceptSession { session_id, source_device_id } => {
                tracing::info!("Accepting session: {} from {}", session_id.0, source_device_id.0);

                let mut sessions = self.app_state.sessions.lock().await;
                // Update existing session or create new one
                let existing = sessions.get(&session_id);
                if let Some(snap) = existing {
                    // Update existing session
                    let new_snapshot = SessionSnapshot {
                        source_device_id: Some(source_device_id),
                        ..snap.clone()
                    };
                    sessions.insert(session_id.clone(), new_snapshot);
                } else {
                    // Create new session
                    sessions.insert(session_id.clone(), SessionSnapshot {
                        session_id: session_id.clone(),
                        transport: "unknown".to_string(),
                        source_device_id: Some(source_device_id),
                        target_device_id: None,
                        local_listen_addr: None,
                        local_server_name: None,
                        local_cert_der_b64: None,
                        remote_listen_addr: None,
                        remote_server_name: None,
                        remote_cert_der_b64: None,
                        lifecycle_state: "listening".to_string(),
                        last_error: None,
                        sender_active: false,
                        receiver_active: false,
                    });
                }

                IpcResponse::SessionAccepted { session_id }
            }

            IpcRequest::StartSender { session_id } => {
                tracing::info!("Starting sender for session: {}", session_id.0);

                let mut sessions = self.app_state.sessions.lock().await;
                let existing = sessions.get(&session_id);
                if let Some(snap) = existing {
                    // Update snapshot to mark sender as active
                    let new_snapshot = SessionSnapshot {
                        sender_active: true,
                        ..snap.clone()
                    };
                    sessions.insert(session_id.clone(), new_snapshot);
                    IpcResponse::SenderStarted { session_id }
                } else {
                    IpcResponse::Error {
                        code: "E404".to_string(),
                        message: format!("Session not found: {}", session_id.0),
                    }
                }
            }

            IpcRequest::StartReceiver { session_id } => {
                tracing::info!("Starting receiver for session: {}", session_id.0);

                let mut sessions = self.app_state.sessions.lock().await;
                let existing = sessions.get(&session_id);
                if let Some(snap) = existing {
                    // Update snapshot to mark receiver as active
                    let new_snapshot = SessionSnapshot {
                        receiver_active: true,
                        ..snap.clone()
                    };
                    sessions.insert(session_id.clone(), new_snapshot);
                    IpcResponse::ReceiverStarted { session_id }
                } else {
                    IpcResponse::Error {
                        code: "E404".to_string(),
                        message: format!("Session not found: {}", session_id.0),
                    }
                }
            }

            IpcRequest::StopSession { session_id } => {
                tracing::info!("Stopping session: {}", session_id.0);

                let mut sessions = self.app_state.sessions.lock().await;
                sessions.remove(&session_id);

                IpcResponse::SessionStopped { session_id }
            }

            IpcRequest::SessionRuntimeSnapshot { session_id } => {
                let sessions = self.app_state.sessions.lock().await;
                let snap = sessions.get(&session_id);
                match snap {
                    Some(s) => match self.snapshot_to_ipc(s) {
                        Some(snapshot) => IpcResponse::SessionSnapshot { snapshot },
                        None => IpcResponse::Error {
                            code: "E500".to_string(),
                            message: "Failed to create snapshot".to_string(),
                        }
                    },
                    None => IpcResponse::Error {
                        code: "E404".to_string(),
                        message: format!("Session not found: {}", session_id.0),
                    },
                }
            }

            IpcRequest::RuntimeSnapshot => {
                let sessions = self.app_state.sessions.lock().await;
                let devices = self.app_state.devices.lock().await;

                let session_snapshots: Vec<mrd_ipc::SessionRuntimeSnapshot> = sessions.list_all()
                    .into_iter()
                    .filter_map(|snap| self.snapshot_to_ipc(&snap))
                    .collect();

                let device_id = devices.get_local_device().map(|(id, _)| id.clone());

                IpcResponse::RuntimeSnapshot {
                    snapshot: mrd_ipc::RuntimeSnapshot {
                        sessions: session_snapshots,
                        device_id,
                        is_registered: devices.is_registered(),
                    }
                }
            }

            IpcRequest::ServiceHealth => {
                IpcResponse::ServiceHealth {
                    status: mrd_ipc::ServiceStatus {
                        running: true,
                        healthy: true,
                        pid: Some(std::process::id()),
                    }
                }
            }

            IpcRequest::ProbeSnapshot { session_id } => {
                // TODO: Implement real probe snapshot
                IpcResponse::ProbeSnapshot {
                    snapshot: mrd_ipc::ProbeSnapshot {
                        session_id,
                        frames_received: 0,
                        frames_decoded: 0,
                        frames_dropped: 0,
                        current_fps: None,
                        bitrate_mbps: None,
                        last_error: None,
                    }
                }
            }

            IpcRequest::StreamProbeEvents => {
                IpcResponse::Error {
                    code: "E501".to_string(),
                    message: "Probe streaming not implemented yet".to_string(),
                }
            }
        }
    }

    /// Convert a session snapshot to IPC format
    fn snapshot_to_ipc(&self, snap: &SessionSnapshot) -> Option<mrd_ipc::SessionRuntimeSnapshot> {
        // Determine role based on which device ID is set
        let role = if snap.target_device_id.is_some() {
            "controller"
        } else if snap.source_device_id.is_some() {
            "agent"
        } else {
            "unknown"
        }.to_string();

        // Use explicit lifecycle state from domain model
        let state = snap.lifecycle_state.clone();

        Some(mrd_ipc::SessionRuntimeSnapshot {
            session_id: snap.session_id.clone(),
            role,
            state,
            transport_kind: snap.transport.clone(),
            local_bootstrap: if snap.local_listen_addr.is_some() || snap.local_server_name.is_some() {
                Some(mrd_ipc::SessionBootstrap {
                    listen_addr: snap.local_listen_addr.clone(),
                    server_name: snap.local_server_name.clone(),
                    cert_der: snap.local_cert_der_b64.clone(),
                })
            } else {
                None
            },
            remote_bootstrap: if snap.remote_listen_addr.is_some() || snap.remote_server_name.is_some() {
                Some(mrd_ipc::SessionBootstrap {
                    listen_addr: snap.remote_listen_addr.clone(),
                    server_name: snap.remote_server_name.clone(),
                    cert_der: snap.remote_cert_der_b64.clone(),
                })
            } else {
                None
            },
            last_error: snap.last_error.clone(),
            sender_active: snap.sender_active,
            receiver_active: snap.receiver_active,
        })
    }

    /// Get access to the app state (for testing/integration)
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.app_state
    }

    /// Run the IPC server (accepts connections in a loop)
    pub async fn run(&self) -> anyhow::Result<()> {
        let server = transport::IpcServer::bind().await?;
        tracing::info!("IPC server listening");

        let app_state = self.app_state.clone();
        loop {
            match server.accept().await {
                Ok(stream) => {
                    let server_clone = IpcServer::new(app_state.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
            lifecycle_state: "listening".to_string(),
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        server.app_state().sessions().lock().await.insert(session_id.clone(), snapshot);

        let request = IpcRequest::SessionRuntimeSnapshot {
            session_id: session_id.clone(),
        };
        let response = server.handle_request(request).await;

        match response {
            IpcResponse::SessionSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.state, "listening");  // Only local bootstrap
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
            lifecycle_state: "created".to_string(),
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        server.app_state().sessions().lock().await.insert(session_id.clone(), snapshot);

        let response = server.handle_request(IpcRequest::ListSessions).await;

        match response {
            IpcResponse::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].session_id, session_id);
                assert_eq!(sessions[0].role, "controller");
            }
            _ => panic!("Expected SessionList response"),
        }
    }

    #[tokio::test]
    async fn runtime_snapshot_aggregates_state() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let device_id = DeviceId("test-device".to_string());
        let _ = server.handle_request(IpcRequest::RegisterDevice {
            device_id: device_id.clone(),
            device_name: "Test Device".to_string(),
        }).await;

        let response = server.handle_request(IpcRequest::RuntimeSnapshot).await;

        match response {
            IpcResponse::RuntimeSnapshot { snapshot } => {
                assert_eq!(snapshot.is_registered, true);
                assert_eq!(snapshot.device_id, Some(device_id));
            }
            _ => panic!("Expected RuntimeSnapshot response"),
        }
    }
}
