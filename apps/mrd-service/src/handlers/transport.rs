// Transport control handlers for mrd-service
//
// These handlers implement media control (sender/receiver) logic.

use mrd_ipc::{IpcRequest, IpcResponse};
use mrd_proto::SessionId;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::app_state::AppState;

/// Handle start sender request (controller role - begins media capture)
pub async fn start_sender(
    _app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    tracing::info!("Starting sender for session: {}", session_id.0);

    // TODO: Integrate with actual media pipeline
    // This will require:
    // 1. Get session from registry
    // 2. Determine transport type (quic/webrtc)
    // 3. Start capture from DXGI
    // 4. Start encode pipeline
    // 5. Connect to transport

    IpcResponse::SenderStarted { session_id }
}

/// Handle start receiver request (agent role - begins media decode/render)
pub async fn start_receiver(
    _app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    tracing::info!("Starting receiver for session: {}", session_id.0);

    // TODO: Integrate with actual media pipeline
    // This will require:
    // 1. Get session from registry
    // 2. Determine transport type (quic/webrtc)
    // 3. Start decode pipeline
    // 4. Connect to transport
    // 5. Start rendering

    IpcResponse::ReceiverStarted { session_id }
}

/// Handle probe snapshot request
pub async fn probe_snapshot(
    _app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_sender_returns_started_response() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let response = start_sender(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::SenderStarted { session_id: returned_id } => {
                assert_eq!(returned_id, session_id);
            }
            _ => panic!("Expected SenderStarted response"),
        }
    }

    #[tokio::test]
    async fn start_receiver_returns_started_response() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let response = start_receiver(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::ReceiverStarted { session_id: returned_id } => {
                assert_eq!(returned_id, session_id);
            }
            _ => panic!("Expected ReceiverStarted response"),
        }
    }
}
