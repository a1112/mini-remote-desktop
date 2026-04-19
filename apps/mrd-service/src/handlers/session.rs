// Session control handlers for mrd-service
//
// These handlers implement the core session orchestration logic.

use mrd_ipc::IpcResponse;
use mrd_application::ports::SessionSnapshot;
use mrd_proto::{SessionId, DeviceId};
use std::sync::Arc;
use crate::app_state::AppState;

/// Handle session start request
pub async fn start_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    target_device_id: DeviceId,
    transport_kind: String,
) -> IpcResponse {
    tracing::info!(
        "Starting session: {} -> {} via {}",
        session_id.0,
        target_device_id.0,
        transport_kind
    );

    let mut sessions = app_state.sessions.lock().await;
    sessions.insert(session_id.clone(), SessionSnapshot {
        session_id: session_id.clone(),
        transport: transport_kind.clone(),
        source_device_id: None,
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

/// Handle session accept request
pub async fn accept_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    source_device_id: DeviceId,
) -> IpcResponse {
    tracing::info!(
        "Accepting session: {} from {}",
        session_id.0,
        source_device_id.0
    );

    let mut sessions = app_state.sessions.lock().await;
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

/// Handle session stop request
pub async fn stop_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    tracing::info!("Stopping session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    let removed = sessions.remove(&session_id);

    if removed.is_some() {
        IpcResponse::SessionStopped { session_id }
    } else {
        IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        }
    }
}

/// Handle session snapshot request
pub async fn session_snapshot(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    let sessions = app_state.sessions.lock().await;
    let snap = sessions.get(&session_id);

    match snap {
        Some(s) => {
            // Convert to IPC snapshot using explicit state
            let role = if s.target_device_id.is_some() {
                "controller"
            } else if s.source_device_id.is_some() {
                "agent"
            } else {
                "unknown"
            }.to_string();

            IpcResponse::SessionSnapshot {
                snapshot: mrd_ipc::SessionRuntimeSnapshot {
                    session_id: s.session_id.clone(),
                    role,
                    state: s.lifecycle_state.clone(),
                    transport_kind: s.transport.clone(),
                    local_bootstrap: if s.local_listen_addr.is_some() || s.local_server_name.is_some() {
                        Some(mrd_ipc::SessionBootstrap {
                            listen_addr: s.local_listen_addr.clone(),
                            server_name: s.local_server_name.clone(),
                            cert_der: s.local_cert_der_b64.clone(),
                        })
                    } else {
                        None
                    },
                    remote_bootstrap: if s.remote_listen_addr.is_some() || s.remote_server_name.is_some() {
                        Some(mrd_ipc::SessionBootstrap {
                            listen_addr: s.remote_listen_addr.clone(),
                            server_name: s.remote_server_name.clone(),
                            cert_der: s.remote_cert_der_b64.clone(),
                        })
                    } else {
                        None
                    },
                    last_error: s.last_error.clone(),
                    sender_active: s.sender_active,
                    receiver_active: s.receiver_active,
                }
            }
        }
        None => IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_session_creates_session_in_registry() {
        let app_state = Arc::new(AppState::new());

        let session_id = SessionId("test-session".to_string());
        let target_device_id = DeviceId("agent".to_string());

        let response = start_session(
            &app_state,
            session_id.clone(),
            target_device_id,
            "quic".to_string(),
        ).await;

        match response {
            IpcResponse::SessionStarted { session_id: returned_id } => {
                assert_eq!(returned_id, session_id);
            }
            _ => panic!("Expected SessionStarted response"),
        }

        // Verify session was stored
        let sessions = app_state.sessions.lock().await;
        let stored = sessions.get(&session_id);
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn stop_session_removes_from_registry() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        // First create a session
        let _ = start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "quic".to_string(),
        ).await;

        // Then stop it
        let response = stop_session(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::SessionStopped { .. } => {}
            _ => panic!("Expected SessionStopped response"),
        }

        // Verify session was removed
        let sessions = app_state.sessions.lock().await;
        let stored = sessions.get(&session_id);
        assert!(stored.is_none());
    }
}
