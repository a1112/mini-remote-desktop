// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use mrd_ipc::{IpcRequest, IpcResponse};
use mrd_application::ports::SessionSnapshot;
use mrd_proto::{SessionId, DeviceId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// In-memory session storage for the IPC server
///
/// This will be replaced by proper application state management
/// once the application layer is fully integrated.
#[derive(Debug, Default)]
pub struct IpcSessionStore {
    sessions: HashMap<SessionId, SessionSnapshot>,
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
        Some(mrd_ipc::SessionRuntimeSnapshot {
            session_id: snap.session_id.clone(),
            role: "controller".to_string(),  // Simplified for initial implementation
            state: if snap.local_listen_addr.is_some() || snap.remote_listen_addr.is_some() {
                "connected".to_string()
            } else {
                "created".to_string()
            },
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

    /// Handle an IPC request and return a response
    pub async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        match request {
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
            IpcRequest::ListDevices => {
                IpcResponse::DeviceList {
                    devices: vec![],
                }
            }
            _ => IpcResponse::Error {
                code: "E501".to_string(),
                message: "Not implemented yet".to_string(),
            },
        }
    }

    /// Get access to the session store (for testing/integration)
    pub fn session_store(&self) -> &Arc<Mutex<IpcSessionStore>> {
        &self.session_store
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
    use mrd_ipc::SessionBootstrap;

    #[tokio::test]
    async fn session_snapshot_returns_correct_ipc_format() {
        let server = IpcServer::new();

        // Add a test session
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

        // Request snapshot
        let request = IpcRequest::SessionRuntimeSnapshot {
            session_id: session_id.clone(),
        };
        let response = server.handle_request(request).await;

        match response {
            IpcResponse::SessionSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.role, "controller");
                assert_eq!(snapshot.state, "connected");
                assert_eq!(snapshot.transport_kind, "quic");
                assert_eq!(snapshot.local_bootstrap, Some(SessionBootstrap {
                    listen_addr: Some("127.0.0.1:4433".to_string()),
                    server_name: Some("localhost".to_string()),
                    cert_der: Some("AQID".to_string()),
                }));
            }
            _ => panic!("Expected SessionSnapshot response"),
        }
    }
}
