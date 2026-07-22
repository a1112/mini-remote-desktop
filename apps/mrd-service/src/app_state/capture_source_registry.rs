use super::SessionRegistry;
use mrd_ipc::CaptureSourceSelection;
use mrd_proto::SessionId;
use std::collections::HashMap;

/// Runtime capture source selection state keyed by session.
#[derive(Debug, Default)]
pub struct CaptureSourceRegistry {
    selections: HashMap<SessionId, CaptureSourceSelection>,
}

impl CaptureSourceRegistry {
    pub fn set(&mut self, session_id: SessionId, selection: CaptureSourceSelection) {
        self.selections.insert(session_id, selection);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.remove(session_id)
    }

    pub fn active_window_capture_count(&self, sessions: &SessionRegistry) -> usize {
        self.selections
            .iter()
            .filter(|(session_id, selection)| {
                selection.source.source_kind == "window"
                    && sessions.get(session_id).is_some_and(|snapshot| {
                        snapshot.sender_active && !snapshot.lifecycle_state.is_terminal()
                    })
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::SessionRegistry;
    use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
    use mrd_ipc::{CaptureSource, CaptureSourceSelection};
    use mrd_proto::{DeviceId, SessionId};

    fn sender_snapshot(session_id: &SessionId, active: bool) -> SessionSnapshot {
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
            lifecycle_state: if active {
                SessionLifecycleState::Streaming
            } else {
                SessionLifecycleState::Failed {
                    message: "failed".to_string(),
                }
            },
            last_error: None,
            sender_active: active,
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

    fn selection(session_id: &SessionId, source: CaptureSource) -> CaptureSourceSelection {
        CaptureSourceSelection {
            session_id: session_id.clone(),
            source,
            status: "selected".to_string(),
            reason: None,
        }
    }

    #[test]
    fn active_window_count_ignores_display_and_inactive_sessions() {
        let active_window = SessionId("active-window".to_string());
        let failed_window = SessionId("failed-window".to_string());
        let active_display = SessionId("active-display".to_string());
        let mut sessions = SessionRegistry::default();
        sessions.insert(active_window.clone(), sender_snapshot(&active_window, true));
        sessions.insert(
            failed_window.clone(),
            sender_snapshot(&failed_window, false),
        );
        sessions.insert(
            active_display.clone(),
            sender_snapshot(&active_display, true),
        );

        let mut registry = CaptureSourceRegistry::default();
        registry.set(
            active_window.clone(),
            selection(
                &active_window,
                capture_source("windows:window:0x1", "window"),
            ),
        );
        registry.set(
            failed_window.clone(),
            selection(
                &failed_window,
                capture_source("windows:window:0x2", "window"),
            ),
        );
        registry.set(
            active_display.clone(),
            selection(
                &active_display,
                capture_source("windows:display-shared:0", "display_shared"),
            ),
        );

        assert_eq!(registry.active_window_capture_count(&sessions), 1);
    }
}
