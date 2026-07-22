use super::media_profile::{clamp_media_profile_to_lan_capability, default_media_profile};
use crate::app_state::AppState;
use anyhow::Result;
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_ipc::{
    MediaProfile, MediaProfileNegotiation, RemoteAuthorizationState, RemoteFailure,
    RemotePermissionScope, RemoteReasonCode,
};
use mrd_proto::SessionId;
use std::sync::Arc;

pub(super) async fn session_allows_media(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> bool {
    if app_state
        .session_authorizations
        .snapshot(session_id)
        .await
        .is_some()
        && !app_state
            .session_authorizations
            .allows_scope(session_id, RemotePermissionScope::ScreenView, now_ms())
            .await
    {
        return false;
    }
    let sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id) else {
        return false;
    };
    !snapshot.lifecycle_state.is_terminal()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(super) async fn mark_session_failed(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    reason: String,
) {
    let _authorization_guard = app_state.authorization_security_gate.lock().await;
    let failed_at_ms = now_ms();
    if app_state
        .session_authorizations
        .snapshot_at(session_id, failed_at_ms)
        .await
        .is_some()
    {
        let _ = app_state
            .session_authorizations
            .record_failure(
                session_id,
                RemoteAuthorizationState::Revoked,
                RemoteFailure {
                    code: RemoteReasonCode::RouteLost,
                    message: reason.clone(),
                    suggested_action: Some("retry the LAN connection".to_string()),
                },
                failed_at_ms,
            )
            .await;
    }
    super::release_control_state_for_session(app_state, session_id).await;
    let mut sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id).cloned() else {
        return;
    };
    if snapshot.lifecycle_state == SessionLifecycleState::Closed {
        return;
    }
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
            lifecycle_state: SessionLifecycleState::Failed {
                message: reason.clone(),
            },
            last_error: Some(reason),
            sender_active: false,
            receiver_active: false,
            ..snapshot
        },
    );
}

pub(super) async fn selected_media_profile(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> MediaProfile {
    app_state
        .media_profiles
        .lock()
        .await
        .get(session_id)
        .map(|negotiation| negotiation.selected)
        .unwrap_or_else(default_media_profile)
}

pub(super) fn negotiate_media_profile(
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    clamp_media_profile_to_lan_capability(requested_profile)
}
