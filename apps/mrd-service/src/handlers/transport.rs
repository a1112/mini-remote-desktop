// Transport control handlers for mrd-service
//
// These handlers implement media control (sender/receiver) logic.

use crate::app_state::AppState;
use mrd_ipc::IpcResponse;
use mrd_proto::SessionId;
use std::sync::Arc;

/// Handle start sender request (controller role - begins media capture)
pub async fn start_sender(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    tracing::info!("Starting sender for session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        sessions.insert(
            session_id.clone(),
            mrd_application::ports::SessionSnapshot {
                sender_active: true,
                lifecycle_state: "streaming".to_string(),
                last_error: None,
                ..snapshot
            },
        );
        IpcResponse::SenderStarted { session_id }
    } else {
        IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        }
    }
}

/// Handle start receiver request (agent role - begins media decode/render)
pub async fn start_receiver(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    tracing::info!("Starting receiver for session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        sessions.insert(
            session_id.clone(),
            mrd_application::ports::SessionSnapshot {
                receiver_active: true,
                lifecycle_state: "streaming".to_string(),
                last_error: None,
                ..snapshot
            },
        );
        IpcResponse::ReceiverStarted { session_id }
    } else {
        IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        }
    }
}

/// Handle probe snapshot request
pub async fn probe_snapshot(_app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    // TODO: Implement real probe snapshot
    // This requires access to the actual media telemetry
    IpcResponse::ProbeSnapshot {
        snapshot: mrd_ipc::ProbeSnapshot {
            session_id,
            frames_received: 0,
            frames_decoded: 0,
            frames_dropped: 0,
            current_fps: None,
            bitrate_mbps: None,
            last_error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::session;
    use mrd_proto::DeviceId;

    #[tokio::test]
    async fn start_sender_returns_started_response() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let _ = session::start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "quic".to_string(),
        )
        .await;

        let response = start_sender(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::SenderStarted {
                session_id: returned_id,
            } => {
                assert_eq!(returned_id, session_id);
            }
            _ => panic!("Expected SenderStarted response"),
        }

        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions.get(&session_id).expect("session snapshot");
        assert!(snapshot.sender_active, "sender should be marked active");
    }

    #[tokio::test]
    async fn start_sender_returns_not_found_for_missing_session() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("missing-session".to_string());

        let response = start_sender(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E404");
                assert!(message.contains(&session_id.0));
            }
            _ => panic!("Expected Error response"),
        }
    }

    #[tokio::test]
    async fn start_receiver_returns_started_response() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let _ = session::start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "webrtc".to_string(),
        )
        .await;

        let response = start_receiver(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::ReceiverStarted {
                session_id: returned_id,
            } => {
                assert_eq!(returned_id, session_id);
            }
            _ => panic!("Expected ReceiverStarted response"),
        }

        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions.get(&session_id).expect("session snapshot");
        assert!(snapshot.receiver_active, "receiver should be marked active");
    }

    #[tokio::test]
    async fn start_receiver_returns_not_found_for_missing_session() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("missing-session".to_string());

        let response = start_receiver(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E404");
                assert!(message.contains(&session_id.0));
            }
            _ => panic!("Expected Error response"),
        }
    }
}
