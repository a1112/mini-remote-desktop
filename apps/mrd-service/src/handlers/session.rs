// Session control handlers for mrd-service
//
// These handlers implement the core session orchestration logic.

use crate::app_state::AppState;
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{IpcResponse, MediaProfile};
use mrd_proto::{DeviceId, SessionId};
use std::sync::Arc;

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
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
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
        },
    );

    IpcResponse::SessionStarted { session_id }
}

/// Start a LAN P2P remote session and request auto-accept on the target peer.
pub async fn start_lan_remote_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    target_device_id: DeviceId,
    transport_kind: String,
    requested_profile: Option<MediaProfile>,
) -> IpcResponse {
    tracing::info!(
        "Starting LAN remote session: {} -> {} via {}",
        session_id.0,
        target_device_id.0,
        transport_kind
    );

    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: transport_kind.clone(),
                source_device_id: None,
                target_device_id: Some(target_device_id.clone()),
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
            },
        );
    }

    match crate::lan_discovery::request_lan_remote_session(
        app_state,
        &target_device_id,
        &session_id,
        &transport_kind,
        requested_profile,
    )
    .await
    {
        Ok(_negotiation) => {
            let mut sessions = app_state.sessions.lock().await;
            if let Some(snapshot) = sessions.get(&session_id).cloned() {
                sessions.insert(
                    session_id.clone(),
                    SessionSnapshot {
                        lifecycle_state: "connected".to_string(),
                        last_error: None,
                        ..snapshot
                    },
                );
            }
            IpcResponse::SessionStarted { session_id }
        }
        Err(error) => {
            let message = error.to_string();
            let mut sessions = app_state.sessions.lock().await;
            if let Some(snapshot) = sessions.get(&session_id).cloned() {
                sessions.insert(
                    session_id.clone(),
                    SessionSnapshot {
                        lifecycle_state: "failed".to_string(),
                        last_error: Some(message.clone()),
                        ..snapshot
                    },
                );
            }
            IpcResponse::Error {
                code: "E_LAN_REMOTE".to_string(),
                message,
            }
        }
    }
}

/// Handle a runtime media profile switch request.
pub async fn update_media_profile(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    requested_profile: MediaProfile,
) -> IpcResponse {
    tracing::info!(
        "Updating media profile: {} -> {}x{}@{} {}Mbps {}",
        session_id.0,
        requested_profile.width,
        requested_profile.height,
        requested_profile.fps,
        requested_profile.bitrate_mbps,
        requested_profile.codec
    );

    match crate::lan_discovery::request_lan_media_profile_update(
        app_state,
        &session_id,
        requested_profile,
    )
    .await
    {
        Ok(negotiation) => IpcResponse::MediaProfileUpdated {
            session_id,
            negotiation,
        },
        Err(error) => IpcResponse::Error {
            code: "E_MEDIA_PROFILE".to_string(),
            message: error.to_string(),
        },
    }
}

/// Handle a remote capture source listing request.
pub async fn list_remote_capture_sources(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    include_previews: bool,
    limit: Option<u32>,
) -> IpcResponse {
    match crate::lan_discovery::request_lan_capture_sources(
        app_state,
        &session_id,
        include_previews,
        limit,
    )
    .await
    {
        Ok(sources) => IpcResponse::CaptureSourceList {
            session_id,
            sources,
        },
        Err(error) => IpcResponse::Error {
            code: "E_CAPTURE_SOURCES".to_string(),
            message: error.to_string(),
        },
    }
}

/// Handle a remote capture source selection request.
pub async fn select_remote_capture_source(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    source_id: String,
) -> IpcResponse {
    match crate::lan_discovery::request_lan_capture_source_select(app_state, &session_id, source_id)
        .await
    {
        Ok(selection) => IpcResponse::CaptureSourceSelected {
            session_id,
            selection,
        },
        Err(error) => IpcResponse::Error {
            code: "E_CAPTURE_SOURCE_SELECT".to_string(),
            message: error.to_string(),
        },
    }
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
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
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
            },
        );
    }

    IpcResponse::SessionAccepted { session_id }
}

/// Handle session stop request
pub async fn stop_session(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    tracing::info!("Stopping session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                lifecycle_state: "closed".to_string(),
                last_error: None,
                sender_active: false,
                receiver_active: false,
                ..snapshot
            },
        );
        drop(sessions);
        app_state
            .media_tasks
            .lock()
            .await
            .abort_session(&session_id);
        app_state.media_profiles.lock().await.remove(&session_id);
        app_state.capture_sources.lock().await.remove(&session_id);
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .remove(&session_id);
        app_state.media_pipelines.lock().await.remove(&session_id);
        return IpcResponse::SessionStopped { session_id };
    }

    IpcResponse::Error {
        code: "E404".to_string(),
        message: format!("Session not found: {}", session_id.0),
    }
}

/// Handle session failure request.
pub async fn fail_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    reason: String,
) -> IpcResponse {
    tracing::warn!("Failing session: {} reason={}", session_id.0, reason);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                lifecycle_state: "failed".to_string(),
                last_error: Some(reason.clone()),
                sender_active: false,
                receiver_active: false,
                ..snapshot
            },
        );
        drop(sessions);
        app_state
            .media_tasks
            .lock()
            .await
            .abort_session(&session_id);
        app_state.media_pipelines.lock().await.remove(&session_id);

        let mut shell = app_state.shell.lock().await;
        shell.last_error = Some(reason);

        return IpcResponse::SessionFailed { session_id };
    }

    IpcResponse::Error {
        code: "E404".to_string(),
        message: format!("Session not found: {}", session_id.0),
    }
}

/// Recover a failed or closed session into the startup state for its role.
pub async fn recover_session(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    tracing::info!("Recovering session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        let lifecycle_state = recovery_state_for(&snapshot).to_string();
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                lifecycle_state,
                last_error: None,
                sender_active: false,
                receiver_active: false,
                ..snapshot
            },
        );
        drop(sessions);

        let mut shell = app_state.shell.lock().await;
        shell.last_error = None;

        return IpcResponse::SessionRecovered { session_id };
    }

    IpcResponse::Error {
        code: "E404".to_string(),
        message: format!("Session not found: {}", session_id.0),
    }
}

/// Handle session snapshot request
pub async fn session_snapshot(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
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
            }
            .to_string();

            IpcResponse::SessionSnapshot {
                snapshot: mrd_ipc::SessionRuntimeSnapshot {
                    session_id: s.session_id.clone(),
                    role,
                    state: s.lifecycle_state.clone(),
                    transport_kind: s.transport.clone(),
                    local_bootstrap: if s.local_listen_addr.is_some()
                        || s.local_server_name.is_some()
                    {
                        Some(mrd_ipc::SessionBootstrap {
                            listen_addr: s.local_listen_addr.clone(),
                            server_name: s.local_server_name.clone(),
                            cert_der: s.local_cert_der_b64.clone(),
                        })
                    } else {
                        None
                    },
                    remote_bootstrap: if s.remote_listen_addr.is_some()
                        || s.remote_server_name.is_some()
                    {
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
                },
            }
        }
        None => IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        },
    }
}

fn recovery_state_for(snapshot: &SessionSnapshot) -> &'static str {
    if snapshot.target_device_id.is_some() {
        "connecting"
    } else if snapshot.source_device_id.is_some() {
        "listening"
    } else {
        "created"
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
        )
        .await;

        match response {
            IpcResponse::SessionStarted {
                session_id: returned_id,
            } => {
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
    async fn start_lan_remote_session_marks_missing_peer_failure() {
        let app_state = Arc::new(AppState::new());
        app_state
            .devices
            .lock()
            .await
            .register(DeviceId("controller".to_string()), "Controller".to_string());
        let session_id = SessionId("lan-session".to_string());

        let response = start_lan_remote_session(
            &app_state,
            session_id.clone(),
            DeviceId("missing-peer".to_string()),
            "webrtc".to_string(),
            None,
        )
        .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_LAN_REMOTE");
                assert!(message.contains("missing-peer"));
            }
            _ => panic!("Expected LAN remote error response"),
        }

        let sessions = app_state.sessions.lock().await;
        let stored = sessions.get(&session_id).expect("failed LAN session");
        assert_eq!(stored.lifecycle_state, "failed");
        assert!(stored.last_error.is_some());
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
        )
        .await;

        // Then stop it
        let response = stop_session(&app_state, session_id.clone()).await;

        match response {
            IpcResponse::SessionStopped { .. } => {}
            _ => panic!("Expected SessionStopped response"),
        }

        // Verify session was retained as closed so UI can observe the stop.
        let sessions = app_state.sessions.lock().await;
        let stored = sessions
            .get(&session_id)
            .expect("closed session should remain");
        assert_eq!(stored.lifecycle_state, "closed");
        assert!(!stored.sender_active);
        assert!(!stored.receiver_active);
    }

    #[tokio::test]
    async fn stop_session_aborts_registered_media_tasks() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let _ = start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "quic".to_string(),
        )
        .await;

        let task = tokio::spawn(async { std::future::pending::<()>().await });
        app_state
            .media_tasks
            .lock()
            .await
            .register(session_id.clone(), task.abort_handle());

        let response = stop_session(&app_state, session_id.clone()).await;

        assert!(matches!(response, IpcResponse::SessionStopped { .. }));
        tokio::task::yield_now().await;
        assert!(task.is_finished(), "media task should be aborted on stop");
        assert_eq!(
            app_state.media_tasks.lock().await.active_count(&session_id),
            0
        );
    }

    #[tokio::test]
    async fn fail_and_recover_session_updates_lifecycle_state() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("test-session".to_string());

        let _ = start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "quic".to_string(),
        )
        .await;

        let response =
            fail_session(&app_state, session_id.clone(), "transport lost".to_string()).await;
        assert!(matches!(response, IpcResponse::SessionFailed { .. }));

        {
            let sessions = app_state.sessions.lock().await;
            let stored = sessions.get(&session_id).expect("failed session");
            assert_eq!(stored.lifecycle_state, "failed");
            assert_eq!(stored.last_error.as_deref(), Some("transport lost"));
        }

        let response = recover_session(&app_state, session_id.clone()).await;
        assert!(matches!(response, IpcResponse::SessionRecovered { .. }));

        let sessions = app_state.sessions.lock().await;
        let stored = sessions.get(&session_id).expect("recovered session");
        assert_eq!(stored.lifecycle_state, "connecting");
        assert!(stored.last_error.is_none());
    }
}
