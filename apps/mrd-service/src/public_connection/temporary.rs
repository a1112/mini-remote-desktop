//! Device-authenticated publication; account binding is deliberately not a prerequisite.
use super::{validate_api_url, PublicConnectionState, Registration};
use crate::{
    temporary_access::{decoded_auth_version, signed_publication},
    AppState,
};
use mrd_ipc::{TemporaryAccessSecret, TemporaryAccessStatus};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
fn online(state: &AppState) -> bool {
    state.signaling_status.snapshot().state
        == crate::signaling::SignalingConnectionState::Authenticated
}
fn endpoint(state: &PublicConnectionState, saved: &Registration) -> Result<String, &'static str> {
    let configured = state
        .api_url
        .read()
        .map_err(|_| "temporary_configuration_unavailable")?;
    validate_api_url(&configured).map_err(|_| "temporary_origin_invalid")?;
    if configured.trim_end_matches('/') != saved.api_url.trim_end_matches('/') {
        return Err("temporary_origin_mismatch");
    }
    Ok(format!(
        "{}/devices/temporary-access",
        configured.trim_end_matches('/')
    ))
}

pub(crate) async fn status(state: &AppState) -> TemporaryAccessStatus {
    state
        .public_connection
        .temporary
        .lock()
        .await
        .status(now_ms(), online(state))
}
pub(crate) async fn secret(state: &AppState) -> TemporaryAccessSecret {
    state
        .public_connection
        .temporary
        .lock()
        .await
        .secret(now_ms(), online(state))
}

fn check_epoch(public: &PublicConnectionState, epoch: u64) -> Result<(), &'static str> {
    if public.temporary_epoch.is_current(epoch) {
        Ok(())
    } else {
        Err("temporary_publication_superseded")
    }
}

async fn update(
    state: &Arc<AppState>,
    force_rotate: bool,
    disable: bool,
    epoch: u64,
) -> Result<(), &'static str> {
    let public = &state.public_connection;
    let _operation = public.temporary_operation.lock().await;
    check_epoch(public, epoch)?;
    let saved = public
        .registration()
        .ok_or("temporary_device_unregistered")?;
    let auth =
        decoded_auth_version(&saved.access_token).ok_or("temporary_device_credential_invalid")?;
    let url = endpoint(public, &saved)?;
    let client = crate::temporary_access_http::client()?;
    let now = now_ms();
    let needs_disable = {
        let local = public.temporary.lock().await;
        check_epoch(public, epoch)?;
        if !force_rotate
            && !disable
            && !local.enabled()
            && !local.needs_publication()
            && local.publication_epoch_current()
        {
            return Ok(());
        }
        disable || (!force_rotate && !local.enabled())
    };
    let needs_rotation = {
        let local = public.temporary.lock().await;
        check_epoch(public, epoch)?;
        force_rotate
            || local.needs_rotation(now)
            || !local.publication_epoch_current()
            || public.temporary_auth_version.load(Ordering::Acquire) != auth
    };
    if needs_rotation || needs_disable {
        let remote =
            crate::temporary_access_http::metadata(&client, &url, &saved.access_token).await?;
        check_epoch(public, epoch)?;
        if remote.generation > i64::MAX as u64 {
            return Err("temporary_generation_invalid");
        }
        let mut local = public.temporary.lock().await;
        check_epoch(public, epoch)?;
        local.apply_operation_epoch(epoch)?;
        if !force_rotate
            && !disable
            && local.generation() == 0
            && remote.generation > 0
            && !remote.enabled
        {
            local.disable(remote.generation)?;
            local.mark_published(remote.generation, remote.expires_at_ms);
            public.temporary_auth_version.store(auth, Ordering::Release);
            return Ok(());
        }
        let generation = local
            .generation()
            .max(remote.generation)
            .checked_add(1)
            .ok_or("temporary_generation_invalid")?;
        if needs_disable {
            local.disable(generation)?;
        } else {
            local.rotate(generation, now)?;
        }
    }
    let document = {
        let mut local = public.temporary.lock().await;
        check_epoch(public, epoch)?;
        local.apply_operation_epoch(epoch)?;
        local.renew_publication(now)?;
        if !local.needs_publication() {
            return Ok(());
        }
        local.access_document(&saved.device_id, auth)?
    };
    check_epoch(public, epoch)?;
    let proof = signed_publication(&state.device_identities.machine_identity(), &url, &document)?;
    check_epoch(public, epoch)?;
    let ack =
        crate::temporary_access_http::publish(&client, &url, &saved.access_token, &proof).await?;
    check_epoch(public, epoch)?;
    if ack.enabled != document.enabled
        || ack.generation != document.generation
        || ack.expires_at_ms != document.expires_at_ms
        || (document.enabled && !ack.ready)
    {
        return Err("temporary_publication_binding_mismatch");
    }
    let mut local = public.temporary.lock().await;
    check_epoch(public, epoch)?;
    if local.enabled() != document.enabled
        || !local.mark_published(document.generation, document.expires_at_ms)
    {
        return Err("temporary_publication_superseded");
    }
    public.temporary_auth_version.store(auth, Ordering::Release);
    Ok(())
}

pub(crate) async fn rotate(state: &Arc<AppState>) -> Result<TemporaryAccessStatus, &'static str> {
    let epoch = state.public_connection.temporary_epoch.begin(false)?;
    // Suppress old password reads and requested-session admission immediately.
    {
        let mut local = state.public_connection.temporary.lock().await;
        local.apply_operation_epoch(epoch)?;
        local.invalidate_publication();
    }
    update(state, true, false, epoch).await?;
    let result = status(state).await;
    check_epoch(&state.public_connection, epoch)?;
    Ok(result)
}

pub(crate) async fn disable(state: &Arc<AppState>) -> Result<TemporaryAccessStatus, &'static str> {
    let epoch = state.public_connection.temporary_epoch.begin(true)?;
    let sessions = {
        let mut local = state.public_connection.temporary.lock().await;
        if state.public_connection.temporary_epoch.is_current(epoch) {
            local.apply_operation_epoch(epoch)?;
            local.freeze();
        }
        local.guest_session_ids_before_epoch(epoch)
    };
    // Monotonic registry revocation invalidates input/media leases immediately.
    // Grant installation uses the same registry mutex and refuses terminal
    // authorizations, so this denial cannot be undone by an in-flight approval.
    for session in &sessions {
        let _ = state
            .session_authorizations
            .record_failure(
                &mrd_proto::SessionId(session.clone()),
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "temporary access was disabled".into(),
                    suggested_action: None,
                },
                now_ms(),
            )
            .await;
    }
    // Freeze happens before any network request. Existing input, media and peers
    // use the established gated WAN terminalizer rather than a new bypass.
    {
        let _security = state.authorization_security_gate.lock().await;
        let mut cleanup_failed = false;
        for session_id in sessions {
            let request = crate::wan_session::service::ServiceWanTerminalRequest::Fail {
                failure: crate::wan_session::model::WanSessionFailure::Cancelled,
                remote_failure: mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "temporary access was disabled".into(),
                    suggested_action: None,
                },
            };
            if crate::wan_session::service::terminalize_wan_session_under_security_gate(
                state,
                &mrd_proto::SessionId(session_id),
                request,
            )
            .await
            .is_err()
            {
                state.mark_security_unhealthy();
                cleanup_failed = true;
            }
        }
        if cleanup_failed {
            return Err("temporary_session_cleanup_incomplete");
        }
    }
    update(state, false, true, epoch).await?;
    Ok(status(state).await)
}

pub(crate) struct TemporaryTask {
    stop: oneshot::Sender<()>,
    join: JoinHandle<()>,
}
impl TemporaryTask {
    pub(crate) async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.join.await;
    }
}
pub(crate) fn spawn(state: Arc<AppState>) -> TemporaryTask {
    let (stop, mut stopping) = oneshot::channel();
    let join = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {biased;_=&mut stopping=>break,_=ticker.tick()=>{}}
            if state.public_connection.registration().is_none() {
                continue;
            }
            // Errors remain safe, fixed codes; never serialize a credential or
            // upstream response body into events or diagnostics.
            let Ok(epoch) = state.public_connection.temporary_epoch.current() else {
                continue;
            };
            tokio::select! {biased;_=&mut stopping=>break,_=update(&state,false,false,epoch)=>{}}
        }
        if let Ok(epoch) = state.public_connection.temporary_epoch.begin(true) {
            let mut local = state.public_connection.temporary.lock().await;
            if local.apply_operation_epoch(epoch).is_ok() {
                local.freeze();
            }
        }
    });
    TemporaryTask { stop, join }
}
