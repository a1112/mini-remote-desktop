// Session control handlers for mrd-service
//
// These handlers implement the core session orchestration logic.

use crate::app_state::AppState;
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_ipc::{
    ControlInputEvent, CrossE2EFaultInjectionResult, DisplayMode, IpcResponse, MediaProfile,
    MediaTestImpairmentSnapshot,
};
use mrd_proto::{DeviceId, SessionId};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
            lifecycle_state: SessionLifecycleState::Connecting,
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
                lifecycle_state: SessionLifecycleState::Connecting,
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
                        lifecycle_state: SessionLifecycleState::Connected,
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
                        lifecycle_state: SessionLifecycleState::Failed {
                            message: message.clone(),
                        },
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

/// Handle a runtime LAN media adaptation configuration request.
pub async fn configure_media_adaptation(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    config: mrd_ipc::AdaptiveMediaConfig,
) -> IpcResponse {
    tracing::info!(
        "Configuring media adaptation: {} enabled={} mode={}",
        session_id.0,
        config.enabled,
        config.mode
    );

    match crate::media_adaptation::configure_media_adaptation(app_state, session_id.clone(), config)
        .await
    {
        Ok(snapshot) => IpcResponse::MediaAdaptationConfigured {
            session_id,
            snapshot,
        },
        Err(error) => IpcResponse::Error {
            code: "E_MEDIA_ADAPTATION".to_string(),
            message: error.to_string(),
        },
    }
}

/// Handle a control input request.
pub async fn send_control_input(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    event: ControlInputEvent,
) -> IpcResponse {
    let route_to_peer = match {
        let sessions = app_state.sessions.lock().await;
        sessions.get(&session_id).cloned()
    } {
        Some(snapshot) if snapshot.lifecycle_state.is_terminal() => {
            return IpcResponse::Error {
                code: "E_CONTROL_INPUT".to_string(),
                message: format!(
                    "control input rejected for {} session",
                    snapshot.lifecycle_state
                ),
            };
        }
        Some(snapshot) => {
            let route_to_peer = snapshot.target_device_id.is_some();
            if route_to_peer
                && (snapshot.lifecycle_state != SessionLifecycleState::Streaming
                    || !snapshot.receiver_active)
            {
                return IpcResponse::Error {
                    code: "E_CONTROL_INPUT".to_string(),
                    message: format!(
                        "control input requires a streaming receiver for session {}",
                        session_id.0
                    ),
                };
            }
            if !route_to_peer && !snapshot.sender_active {
                return IpcResponse::Error {
                    code: "E_CONTROL_INPUT".to_string(),
                    message: format!(
                        "control input requires an active local sender for session {}",
                        session_id.0
                    ),
                };
            }
            route_to_peer
        }
        None => {
            return IpcResponse::Error {
                code: "E_CONTROL_INPUT".to_string(),
                message: format!("session not found: {}", session_id.0),
            };
        }
    };

    let result = if route_to_peer {
        crate::lan_discovery::request_lan_control_input(app_state, &session_id, event).await
    } else {
        app_state
            .control_input()
            .lock()
            .await
            .handle_session_event(&session_id, &event)
            .map_err(Into::into)
    };

    match result {
        Ok(result) => IpcResponse::ControlInputAccepted {
            session_id,
            lane: result.lane,
            event_count: result.event_count,
        },
        Err(error) => IpcResponse::Error {
            code: "E_CONTROL_INPUT".to_string(),
            message: error.to_string(),
        },
    }
}

/// Inject a test-only cross-device E2E fault into an active session.
pub async fn cross_e2e_inject_fault(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    fault_type: String,
    duration_ms: Option<u64>,
) -> IpcResponse {
    if let Some(error) = validate_fault_session(app_state, &session_id).await {
        return error;
    }

    match fault_type.as_str() {
        "renderer.detach_surface" => {
            let surface_ids: Vec<String> = app_state
                .media_pipelines
                .lock()
                .await
                .snapshot(&session_id)
                .attached_surfaces
                .into_iter()
                .map(|surface| surface.surface_id)
                .collect();
            if surface_ids.is_empty() {
                return IpcResponse::Error {
                    code: "E_CROSS_E2E_FAULT".to_string(),
                    message: format!("no attached render surfaces for session {}", session_id.0),
                };
            }

            {
                let mut pipelines = app_state.media_pipelines.lock().await;
                for surface_id in &surface_ids {
                    pipelines.detach_surface(&session_id, surface_id);
                }
            }
            #[cfg(any(windows, target_os = "macos"))]
            {
                let mut renderers = app_state.media_surface_renderers.lock().await;
                for surface_id in &surface_ids {
                    renderers.detach_surface(&session_id, surface_id);
                }
            }

            IpcResponse::CrossE2EFaultInjected {
                result: CrossE2EFaultInjectionResult {
                    session_id,
                    fault_type,
                    status: "injected".to_string(),
                    message: format!("detached {} native render surface(s)", surface_ids.len()),
                    duration_ms,
                    affected_surface_ids: surface_ids,
                    impairment: None,
                },
            }
        }
        "network.pause_peer" => {
            let pause_ms = duration_ms.unwrap_or(1_000).max(1);
            let impairment = MediaTestImpairmentSnapshot {
                loss_pct: 1.0,
                base_delay_ms: pause_ms,
                jitter_ms: 0,
                mtu_bytes: None,
                seed: now_unix_ms_lossy(),
                datagrams_sent: 0,
                datagrams_dropped: 0,
                datagrams_delayed: 0,
                datagrams_fragmented_by_mtu: 0,
            };
            app_state
                .media_pipelines
                .lock()
                .await
                .set_test_impairment(session_id.clone(), Some(impairment.clone()));
            app_state.probes.lock().await.record_transient_frame_drop(
                &session_id,
                0,
                now_unix_ms_lossy(),
            );

            let app_state_for_restore = app_state.clone();
            let session_id_for_restore = session_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(pause_ms)).await;
                app_state_for_restore
                    .media_pipelines
                    .lock()
                    .await
                    .set_test_impairment(session_id_for_restore, None);
            });

            IpcResponse::CrossE2EFaultInjected {
                result: CrossE2EFaultInjectionResult {
                    session_id,
                    fault_type,
                    status: "injected".to_string(),
                    message: format!("recorded test network pause impairment for {} ms", pause_ms),
                    duration_ms: Some(pause_ms),
                    affected_surface_ids: vec![],
                    impairment: Some(impairment),
                },
            }
        }
        _ => IpcResponse::Error {
            code: "E_CROSS_E2E_FAULT".to_string(),
            message: format!("unsupported cross-device E2E fault: {fault_type}"),
        },
    }
}

async fn validate_fault_session(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Option<IpcResponse> {
    match app_state.sessions.lock().await.get(session_id).cloned() {
        Some(snapshot) if snapshot.lifecycle_state.is_terminal() => Some(IpcResponse::Error {
            code: "E_CROSS_E2E_FAULT".to_string(),
            message: format!(
                "fault injection rejected for {} session",
                snapshot.lifecycle_state
            ),
        }),
        Some(_) => None,
        None => Some(IpcResponse::Error {
            code: "E_CROSS_E2E_FAULT".to_string(),
            message: format!("session not found: {}", session_id.0),
        }),
    }
}

fn now_unix_ms_lossy() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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

/// Handle a remote display mode listing request.
pub async fn list_remote_display_modes(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    let source_id = app_state
        .capture_sources
        .lock()
        .await
        .get(&session_id)
        .map(|selection| selection.source.id);
    match crate::lan_discovery::request_lan_display_modes(app_state, &session_id, source_id).await {
        Ok(modes) => IpcResponse::DisplayModeList { session_id, modes },
        Err(error) => IpcResponse::Error {
            code: "E_DISPLAY_MODES".to_string(),
            message: error.to_string(),
        },
    }
}

/// Handle a remote display mode set request.
pub async fn set_remote_display_mode(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    mode: DisplayMode,
    restore_after_session: bool,
) -> IpcResponse {
    match crate::lan_discovery::request_lan_display_mode_set(
        app_state,
        &session_id,
        mode,
        restore_after_session,
    )
    .await
    {
        Ok(change) => IpcResponse::DisplayModeChanged { session_id, change },
        Err(error) => IpcResponse::Error {
            code: "E_DISPLAY_MODE_SET".to_string(),
            message: error.to_string(),
        },
    }
}

/// Handle a remote display mode restore request.
pub async fn restore_remote_display_mode(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    match crate::lan_discovery::request_lan_display_mode_restore(app_state, &session_id).await {
        Ok(change) => IpcResponse::DisplayModeChanged { session_id, change },
        Err(error) => IpcResponse::Error {
            code: "E_DISPLAY_MODE_RESTORE".to_string(),
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
                lifecycle_state: SessionLifecycleState::Listening,
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

    let snapshot = {
        let sessions = app_state.sessions.lock().await;
        sessions.get(&session_id).cloned()
    };

    if let Some(snapshot) = snapshot {
        release_control_input_for_terminal_session(app_state, &session_id, &snapshot, "stopping")
            .await;

        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                lifecycle_state: SessionLifecycleState::Closed,
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
        clear_session_media_state(app_state, &session_id).await;
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

    let snapshot = {
        let sessions = app_state.sessions.lock().await;
        sessions.get(&session_id).cloned()
    };

    if let Some(snapshot) = snapshot {
        release_control_input_for_terminal_session(app_state, &session_id, &snapshot, "failing")
            .await;

        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                lifecycle_state: SessionLifecycleState::Failed {
                    message: reason.clone(),
                },
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
        clear_session_media_state(app_state, &session_id).await;

        let mut shell = app_state.shell.lock().await;
        shell.last_error = Some(reason);

        return IpcResponse::SessionFailed { session_id };
    }

    IpcResponse::Error {
        code: "E404".to_string(),
        message: format!("Session not found: {}", session_id.0),
    }
}

async fn clear_session_media_state(app_state: &Arc<AppState>, session_id: &SessionId) {
    app_state.media_profiles.lock().await.remove(session_id);
    app_state.capture_sources.lock().await.remove(session_id);
    app_state
        .peer_media_capabilities
        .lock()
        .await
        .remove(session_id);
    #[cfg(windows)]
    app_state
        .media_surface_renderers
        .lock()
        .await
        .detach_session(session_id);
    app_state.media_pipelines.lock().await.remove(session_id);
}

async fn release_control_input_for_terminal_session(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    snapshot: &SessionSnapshot,
    action: &'static str,
) {
    let route_release_to_peer = snapshot.target_device_id.is_some()
        && snapshot.lifecycle_state == SessionLifecycleState::Streaming
        && snapshot.receiver_active;
    let result = if route_release_to_peer {
        crate::lan_discovery::request_lan_control_input(
            app_state,
            session_id,
            ControlInputEvent::ReleaseAll,
        )
        .await
        .map(|_| ())
    } else {
        app_state
            .control_input()
            .lock()
            .await
            .handle_session_event(session_id, &ControlInputEvent::ReleaseAll)
            .map(|_| ())
            .map_err(Into::into)
    };
    if let Err(error) = result {
        tracing::warn!(
            session_id = %session_id.0,
            %error,
            "failed to release active control input while {action} session"
        );
    }
}

/// Recover a failed or closed session into the startup state for its role.
pub async fn recover_session(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    tracing::info!("Recovering session: {}", session_id.0);

    let mut sessions = app_state.sessions.lock().await;
    if let Some(snapshot) = sessions.get(&session_id).cloned() {
        let lifecycle_state = recovery_state_for(&snapshot);
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

/// Handle session list request.
pub async fn list_sessions(app_state: &Arc<AppState>) -> IpcResponse {
    let sessions = app_state.sessions.lock().await;
    let session_list = sessions
        .list_all()
        .into_iter()
        .map(|snap| mrd_ipc::SessionInfo {
            session_id: snap.session_id.clone(),
            role: session_role(&snap),
            state: snap.lifecycle_state.as_str().to_string(),
            transport_kind: snap.transport.clone(),
            last_error: snap.last_error.clone(),
            sender_active: snap.sender_active,
            receiver_active: snap.receiver_active,
            peer_device_id: peer_device_id(&snap),
        })
        .collect();

    IpcResponse::SessionList {
        sessions: session_list,
    }
}

/// Handle aggregated runtime snapshot request.
pub async fn runtime_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    let sessions = app_state.sessions.lock().await;
    let session_snapshots: Vec<mrd_ipc::SessionRuntimeSnapshot> = sessions
        .list_all()
        .into_iter()
        .map(|snap| session_runtime_snapshot(&snap))
        .collect();
    drop(sessions);

    let devices = app_state.devices.lock().await;
    let device_id = devices.get_local_device().map(|(id, _)| id.clone());

    IpcResponse::RuntimeSnapshot {
        snapshot: mrd_ipc::RuntimeSnapshot {
            sessions: session_snapshots,
            device_id,
            is_registered: devices.is_registered(),
        },
    }
}

/// Handle session snapshot request
pub async fn session_snapshot(app_state: &Arc<AppState>, session_id: SessionId) -> IpcResponse {
    let sessions = app_state.sessions.lock().await;
    let snap = sessions.get(&session_id);

    match snap {
        Some(s) => IpcResponse::SessionSnapshot {
            snapshot: session_runtime_snapshot(s),
        },
        None => IpcResponse::Error {
            code: "E404".to_string(),
            message: format!("Session not found: {}", session_id.0),
        },
    }
}

fn session_runtime_snapshot(s: &SessionSnapshot) -> mrd_ipc::SessionRuntimeSnapshot {
    mrd_ipc::SessionRuntimeSnapshot {
        session_id: s.session_id.clone(),
        role: session_role(s),
        state: s.lifecycle_state.as_str().to_string(),
        transport_kind: s.transport.clone(),
        local_bootstrap: bootstrap(
            &s.local_listen_addr,
            &s.local_server_name,
            &s.local_cert_der_b64,
        ),
        remote_bootstrap: bootstrap(
            &s.remote_listen_addr,
            &s.remote_server_name,
            &s.remote_cert_der_b64,
        ),
        last_error: s.last_error.clone(),
        sender_active: s.sender_active,
        receiver_active: s.receiver_active,
        peer_device_id: peer_device_id(s),
    }
}

fn session_role(s: &SessionSnapshot) -> String {
    if s.target_device_id.is_some() {
        "controller"
    } else if s.source_device_id.is_some() {
        "agent"
    } else {
        "unknown"
    }
    .to_string()
}

fn peer_device_id(s: &SessionSnapshot) -> Option<DeviceId> {
    s.target_device_id
        .clone()
        .or_else(|| s.source_device_id.clone())
}

fn bootstrap(
    listen_addr: &Option<String>,
    server_name: &Option<String>,
    cert_der: &Option<String>,
) -> Option<mrd_ipc::SessionBootstrap> {
    if listen_addr.is_some() || server_name.is_some() {
        Some(mrd_ipc::SessionBootstrap {
            listen_addr: listen_addr.clone(),
            server_name: server_name.clone(),
            cert_der: cert_der.clone(),
        })
    } else {
        None
    }
}

fn recovery_state_for(snapshot: &SessionSnapshot) -> SessionLifecycleState {
    if snapshot.target_device_id.is_some() {
        SessionLifecycleState::Connecting
    } else if snapshot.source_device_id.is_some() {
        SessionLifecycleState::Listening
    } else {
        SessionLifecycleState::Created
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
        assert!(matches!(
            stored.lifecycle_state,
            SessionLifecycleState::Failed { .. }
        ));
        assert!(stored.last_error.is_some());
    }

    #[tokio::test]
    async fn list_sessions_returns_peer_device_context() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("listed-session".to_string());
        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
                session_id.clone(),
                SessionSnapshot {
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
                    lifecycle_state: SessionLifecycleState::Streaming,
                    last_error: None,
                    sender_active: false,
                    receiver_active: true,
                },
            );
        }

        let response = list_sessions(&app_state).await;

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
            other => panic!("Expected SessionList response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn runtime_snapshot_reports_device_and_session_state() {
        let app_state = Arc::new(AppState::new());
        let device_id = DeviceId("local-device".to_string());
        app_state
            .devices
            .lock()
            .await
            .register(device_id.clone(), "Local Device".to_string());
        let session_id = SessionId("runtime-session".to_string());
        let _ = start_session(
            &app_state,
            session_id.clone(),
            DeviceId("agent".to_string()),
            "quic".to_string(),
        )
        .await;

        let response = runtime_snapshot(&app_state).await;

        match response {
            IpcResponse::RuntimeSnapshot { snapshot } => {
                assert!(snapshot.is_registered);
                assert_eq!(snapshot.device_id, Some(device_id));
                assert_eq!(snapshot.sessions.len(), 1);
                assert_eq!(snapshot.sessions[0].session_id, session_id);
                assert_eq!(
                    snapshot.sessions[0].peer_device_id,
                    Some(DeviceId("agent".to_string()))
                );
            }
            other => panic!("Expected RuntimeSnapshot response, got {other:?}"),
        }
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
        assert_eq!(stored.lifecycle_state, SessionLifecycleState::Closed);
        assert!(!stored.sender_active);
        assert!(!stored.receiver_active);
    }

    #[tokio::test]
    async fn stop_session_releases_active_control_input() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let session_id = SessionId("control-stop-session".to_string());
        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
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
        }

        app_state
            .control_input()
            .lock()
            .await
            .handle_event(&mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            })
            .expect("key down");

        let response = stop_session(&app_state, session_id.clone()).await;
        assert!(matches!(response, IpcResponse::SessionStopped { .. }));

        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 2);
        assert_eq!(snapshot.reliable.injected_messages, 2);
    }

    #[tokio::test]
    async fn fail_session_releases_active_control_input() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let session_id = SessionId("control-fail-session".to_string());
        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
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
        }

        app_state
            .control_input()
            .lock()
            .await
            .handle_event(&mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            })
            .expect("key down");

        let response =
            fail_session(&app_state, session_id.clone(), "transport lost".to_string()).await;
        assert!(matches!(response, IpcResponse::SessionFailed { .. }));

        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 2);
        assert_eq!(snapshot.reliable.injected_messages, 2);
    }

    #[tokio::test]
    async fn fail_session_clears_media_negotiation_state() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("media-fail-session".to_string());
        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
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
                    lifecycle_state: SessionLifecycleState::Streaming,
                    last_error: None,
                    sender_active: true,
                    receiver_active: false,
                },
            );
        }

        let profile = MediaProfile {
            codec: "av1".to_string(),
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            ..MediaProfile::default()
        };
        app_state.media_profiles.lock().await.set(
            session_id.clone(),
            mrd_ipc::MediaProfileNegotiation {
                requested: profile.clone(),
                selected: profile,
                status: "accepted".to_string(),
                reason: None,
                selected_source_id: Some("display:0".to_string()),
                selected_width: Some(2560),
                selected_height: Some(1440),
                downgrade_reason: None,
            },
        );
        app_state.capture_sources.lock().await.set(
            session_id.clone(),
            mrd_ipc::CaptureSourceSelection {
                session_id: session_id.clone(),
                source: mrd_ipc::CaptureSource {
                    id: "display:0".to_string(),
                    platform: "windows".to_string(),
                    source_kind: "display".to_string(),
                    title: "Primary".to_string(),
                    class_name: String::new(),
                    width: 2560,
                    height: 1440,
                    process_id: 0,
                    app_name: None,
                    bundle_identifier: None,
                    preview_data_url: None,
                    preview_width: None,
                    preview_height: None,
                },
                status: "selected".to_string(),
                reason: None,
            },
        );
        app_state.peer_media_capabilities.lock().await.set(
            session_id.clone(),
            vec![
                "media.codec.av1".to_string(),
                "media.color_mode_v1".to_string(),
            ],
        );

        let response =
            fail_session(&app_state, session_id.clone(), "transport lost".to_string()).await;
        assert!(matches!(response, IpcResponse::SessionFailed { .. }));

        assert!(app_state
            .media_profiles
            .lock()
            .await
            .get(&session_id)
            .is_none());
        assert!(app_state
            .capture_sources
            .lock()
            .await
            .get(&session_id)
            .is_none());
        assert!(!app_state
            .peer_media_capabilities
            .lock()
            .await
            .supports(&session_id, "media.codec.av1"));
    }

    #[tokio::test]
    async fn inactive_local_sender_session_rejects_control_input_without_injection() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let session_id = SessionId("inactive-local-input-session".to_string());
        {
            let mut sessions = app_state.sessions.lock().await;
            sessions.insert(
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
                    lifecycle_state: SessionLifecycleState::Connected,
                    last_error: None,
                    sender_active: false,
                    receiver_active: false,
                },
            );
        }

        let response = send_control_input(
            &app_state,
            session_id.clone(),
            mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            },
        )
        .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("active local sender"));
            }
            other => panic!("expected inactive local sender control input error, got {other:?}"),
        }

        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.reliable.injected_messages, 0);
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
            assert!(matches!(
                stored.lifecycle_state,
                SessionLifecycleState::Failed { .. }
            ));
            assert_eq!(stored.last_error.as_deref(), Some("transport lost"));
        }

        let response = recover_session(&app_state, session_id.clone()).await;
        assert!(matches!(response, IpcResponse::SessionRecovered { .. }));

        let sessions = app_state.sessions.lock().await;
        let stored = sessions.get(&session_id).expect("recovered session");
        assert_eq!(stored.lifecycle_state, SessionLifecycleState::Connecting);
        assert!(stored.last_error.is_none());
    }
}
