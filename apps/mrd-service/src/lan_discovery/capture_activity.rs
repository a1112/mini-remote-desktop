use crate::app_state::AppState;
use std::sync::Arc;

pub(super) async fn active_window_capture_count(app_state: &Arc<AppState>) -> u32 {
    let sessions = app_state.sessions.lock().await;
    let capture_sources = app_state.capture_sources.lock().await;
    capture_sources
        .active_window_capture_count(&sessions)
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
    use mrd_ipc::{CaptureSource, CaptureSourceSelection};
    use mrd_proto::{DeviceId, SessionId};

    fn sender_snapshot(session_id: &SessionId) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Streaming,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        }
    }

    fn capture_source(id: &str, kind: &str) -> CaptureSource {
        CaptureSource {
            id: id.to_string(),
            platform: "windows".to_string(),
            source_kind: kind.to_string(),
            title: id.to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1920,
            height: 1080,
            process_id: 4242,
            app_name: Some(id.to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    async fn select_source(
        app_state: &Arc<AppState>,
        session_id: &SessionId,
        source: CaptureSource,
    ) {
        app_state.capture_sources.lock().await.set(
            session_id.clone(),
            CaptureSourceSelection {
                session_id: session_id.clone(),
                source,
                status: "selected".to_string(),
                reason: None,
            },
        );
    }

    #[tokio::test]
    async fn counts_only_active_window_sender_sessions() {
        let app_state = Arc::new(AppState::default());
        let active_window = SessionId("active-window".to_string());
        let inactive_window = SessionId("inactive-window".to_string());
        let active_display = SessionId("active-display".to_string());

        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(active_window.clone(), sender_snapshot(&active_window));
            sessions.insert(
                inactive_window.clone(),
                SessionSnapshot {
                    sender_active: false,
                    lifecycle_state: SessionLifecycleState::Failed {
                        message: "failed".to_string(),
                    },
                    ..sender_snapshot(&inactive_window)
                },
            );
            sessions.insert(active_display.clone(), sender_snapshot(&active_display));
        }

        select_source(
            &app_state,
            &active_window,
            capture_source("windows:window:0x1111", "window"),
        )
        .await;
        select_source(
            &app_state,
            &inactive_window,
            capture_source("windows:window:0x2222", "window"),
        )
        .await;
        select_source(
            &app_state,
            &active_display,
            capture_source("windows:display-shared:0", "display"),
        )
        .await;

        assert_eq!(active_window_capture_count(&app_state).await, 1);
    }
}
