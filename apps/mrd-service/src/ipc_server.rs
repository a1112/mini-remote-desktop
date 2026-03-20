// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use mrd_ipc::{IpcRequest, IpcResponse, transport};
use mrd_application::ports::SessionSnapshot;
use mrd_proto::{SessionId, DeviceId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// In-memory session storage for the IPC server
#[derive(Debug, Default)]
pub struct IpcSessionStore {
    pub(crate) sessions: HashMap<SessionId, SessionSnapshot>,
}

impl IpcSessionStore {
    pub fn insert(&mut self, session_id: SessionId, snapshot: SessionSnapshot) {
        self.sessions.insert(session_id, snapshot);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<&SessionSnapshot> {
        self.sessions.get(session_id)
    }

    pub fn snapshot_to_ipc(&self, session_id: &SessionId) -> Option<mrd_ipc::SessionRuntimeSnapshot> {
        let snap = self.sessions.get(session_id)?;

        // Determine role based on which device ID is set
        // - target_device_id set → controller (initiating session)
        // - source_device_id set → agent (accepting session)
        let role = if snap.target_device_id.is_some() {
            "controller"
        } else if snap.source_device_id.is_some() {
            "agent"
        } else {
            "unknown"
        }.to_string();

        // Determine state based on bootstrap information
        // TODO: This is still simplified - real state machine needs integration
        let state = if snap.local_listen_addr.is_some() && snap.remote_listen_addr.is_some() {
            // Both sides have bootstrap - bidirectional established
            "connected"
        } else if snap.local_listen_addr.is_some() || snap.local_server_name.is_some() {
            // Local side initialized - listening
            "listening"
        } else if snap.remote_listen_addr.is_some() || snap.remote_server_name.is_some() {
            // Remote side info available - connecting
            "connecting"
        } else {
            // Just created, no bootstrap yet
            "created"
        }.to_string();

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
        })
    }
}

/// IPC server - handles requests from Rdesk shell
pub struct IpcServer {
    session_store: Arc<Mutex<IpcSessionStore>>,
}

impl IpcServer {
    pub fn new() -> Self {
        Self {
            session_store: Arc::new(Mutex::new(IpcSessionStore::default())),
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
                IpcResponse::DeviceRegistered { device_id }
            }

            IpcRequest::ListDevices => {
                IpcResponse::DeviceList {
                    devices: vec![],
                }
            }

            IpcRequest::StartSession { session_id, target_device_id, transport_kind } => {
                tracing::info!("Starting session: {} -> {} via {}",
                    session_id.0, target_device_id.0, transport_kind);

                let mut store = self.session_store.lock().await;
                store.insert(session_id.clone(), SessionSnapshot {
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
                });

                IpcResponse::SessionStarted { session_id }
            }

            IpcRequest::AcceptSession { session_id, source_device_id } => {
                tracing::info!("Accepting session: {} from {}", session_id.0, source_device_id.0);

                let mut store = self.session_store.lock().await;
                // Update existing session or create new one
                let snapshot = store.sessions.entry(session_id.clone()).or_insert_with(|| SessionSnapshot {
                    session_id: session_id.clone(),
                    transport: "unknown".to_string(),
                    source_device_id: None,
                    target_device_id: None,
                    local_listen_addr: None,
                    local_server_name: None,
                    local_cert_der_b64: None,
                    remote_listen_addr: None,
                    remote_server_name: None,
                    remote_cert_der_b64: None,
                });
                snapshot.source_device_id = Some(source_device_id);

                IpcResponse::SessionAccepted { session_id }
            }

            IpcRequest::StartSender { session_id } => {
                tracing::info!("Starting sender for session: {}", session_id.0);
                // TODO: Integrate with actual media pipeline
                IpcResponse::SenderStarted { session_id }
            }

            IpcRequest::StartReceiver { session_id } => {
                tracing::info!("Starting receiver for session: {}", session_id.0);
                // TODO: Integrate with actual media pipeline
                IpcResponse::ReceiverStarted { session_id }
            }

            IpcRequest::StopSession { session_id } => {
                tracing::info!("Stopping session: {}", session_id.0);

                let mut store = self.session_store.lock().await;
                store.sessions.remove(&session_id);

                IpcResponse::SessionStopped { session_id }
            }

            IpcRequest::SessionRuntimeSnapshot { session_id } => {
                let store = self.session_store.lock().await;
                match store.snapshot_to_ipc(&session_id) {
                    Some(snapshot) => IpcResponse::SessionSnapshot { snapshot },
                    None => IpcResponse::Error {
                        code: "E404".to_string(),
                        message: format!("Session not found: {}", session_id.0),
                    },
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

    /// Get access to the session store (for testing/integration)
    pub fn session_store(&self) -> &Arc<Mutex<IpcSessionStore>> {
        &self.session_store
    }

    /// Run the IPC server (accepts connections in a loop)
    pub async fn run(&self) -> anyhow::Result<()> {
        let server = transport::IpcServer::bind().await?;
        tracing::info!("IPC server listening");

        loop {
            match server.accept().await {
                Ok(stream) => {
                    let self_clone = IpcServer {
                        session_store: self.session_store.clone(),
                    };
                    tokio::spawn(async move {
                        if let Err(e) = self_clone.handle_connection(stream).await {
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

impl Default for IpcServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_snapshot_returns_correct_ipc_format() {
        let server = IpcServer::new();

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
        };

        server.session_store().lock().await.insert(session_id.clone(), snapshot);

        let request = IpcRequest::SessionRuntimeSnapshot {
            session_id: session_id.clone(),
        };
        let response = server.handle_request(request).await;

        match response {
            IpcResponse::SessionSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.state, "connected");
                assert_eq!(snapshot.transport_kind, "quic");
            }
            _ => panic!("Expected SessionSnapshot response"),
        }
    }
}
