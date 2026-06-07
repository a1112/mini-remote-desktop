use super::media_profile::{clamp_media_profile_to_lan_capability, default_media_profile};
use crate::app_state::AppState;
use anyhow::Result;
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_ipc::{MediaProfile, MediaProfileNegotiation};
use mrd_proto::SessionId;
use std::sync::Arc;

pub(super) async fn session_allows_media(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> bool {
    let sessions = app_state.sessions.lock().await;
    let Some(snapshot) = sessions.get(session_id) else {
        return false;
    };
    !snapshot.lifecycle_state.is_terminal()
}

pub(super) async fn mark_session_failed(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    reason: String,
) {
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
