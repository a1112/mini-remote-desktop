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

/// Public version metadata only; this never returns password or credential bytes.
pub(crate) struct PairedPasswordBindingSnapshot {
    pub(crate) binding: mrd_store_sqlite::PairedAccessPasswordBinding,
    temporary: crate::temporary_access::TemporaryPairedPasswordSnapshot,
}

fn current_device_auth_version(public: &PublicConnectionState) -> Option<u64> {
    decoded_auth_version(&public.registration()?.access_token)
}

impl PairedPasswordBindingSnapshot {
    /// Synchronous post-write-lock guard. The gate serializes material changes;
    /// explicit user intent remains an immediate fence even while it is held.
    pub(crate) fn is_current_at(&self, state: &AppState, now: u64) -> bool {
        self.temporary.is_valid_at(now)
            && online(state)
            && current_device_auth_version(&state.public_connection)
                == Some(self.binding.target_auth_version)
            && state
                .public_connection
                .temporary_auth_version
                .load(Ordering::Acquire)
                == self.binding.target_auth_version
            && self.temporary.is_valid_at(now)
    }
}

pub(crate) async fn paired_password_binding(
    state: &AppState,
) -> Option<PairedPasswordBindingSnapshot> {
    let now = now_ms();
    let auth = current_device_auth_version(&state.public_connection)?;
    let temporary = state
        .public_connection
        .temporary
        .lock()
        .await
        .paired_password_snapshot(now, online(state))?;
    let snapshot = PairedPasswordBindingSnapshot {
        binding: mrd_store_sqlite::PairedAccessPasswordBinding {
            temporary_generation: temporary.generation,
            target_auth_version: auth,
        },
        temporary,
    };
    snapshot.is_current_at(state, now).then_some(snapshot)
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

/// Called only after validating the remote ACK, under the authorization gate
/// and temporary-state lock. Registration replacement has its own lock.
fn mark_publication_ack(
    public: &PublicConnectionState,
    local: &mut crate::temporary_access::TemporaryAccessState,
    epoch: u64,
    auth: u64,
    document: &crate::temporary_access::TemporaryAccessDocument,
) -> Result<(), &'static str> {
    check_epoch(public, epoch)?;
    // Keep this read lock through the synchronous publication/cache commit so
    // registration.save cannot replace the auth version between them.
    let registration = public
        .registration
        .read()
        .map_err(|_| "temporary_publication_superseded")?;
    if registration
        .as_ref()
        .and_then(|saved| decoded_auth_version(&saved.access_token))
        != Some(auth)
    {
        return Err("temporary_publication_superseded");
    }
    if local.enabled() != document.enabled
        || !local.mark_published(document.generation, document.expires_at_ms)
    {
        return Err("temporary_publication_superseded");
    }
    public.temporary_auth_version.store(auth, Ordering::Release);
    Ok(())
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
        let _security = state.authorization_security_gate.lock().await;
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
    let _security = state.authorization_security_gate.lock().await;
    let mut local = public.temporary.lock().await;
    mark_publication_ack(public, &mut local, epoch, auth, &document)
}

pub(crate) async fn rotate(state: &Arc<AppState>) -> Result<TemporaryAccessStatus, &'static str> {
    let epoch = state.public_connection.temporary_epoch.begin(false)?;
    {
        // Suppress old password reads and requested-session admission immediately.
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    fn set_auth(state: &AppState, version: u64) {
        let payload =
            serde_json::json!({"token_type":"device","role":"device","auth_version":version});
        *state.public_connection.registration.write().unwrap() = Some(super::super::Registration {
            device_id: "1501515774".to_owned(),
            device_name: "synthetic binding fixture".to_owned(),
            access_token: format!(
                "fixture.{}.signature",
                URL_SAFE_NO_PAD.encode(payload.to_string())
            ),
            refresh_token: None,
            api_url: "https://127.0.0.1:9/rdesk/api/v1".to_owned(),
            machine_serial: "synthetic".to_owned(),
        });
    }

    async fn ready_state() -> Arc<AppState> {
        let state = Arc::new(AppState::new());
        set_auth(&state, 2);
        state.signaling_status.note_authenticated(now_ms());
        state
            .public_connection
            .temporary_auth_version
            .store(2, Ordering::Release);
        let mut temporary = state.public_connection.temporary.lock().await;
        temporary.rotate(4, now_ms()).unwrap();
        let expiry = temporary
            .access_document("fixture", 2)
            .unwrap()
            .expires_at_ms;
        assert!(temporary.mark_published(4, expiry));
        drop(temporary);
        state
    }

    #[tokio::test]
    async fn paired_password_binding_rechecks_current_device_auth_not_only_publication_cache() {
        let state = ready_state().await;
        let snapshot = paired_password_binding(&state).await.unwrap();
        assert_eq!(snapshot.binding.temporary_generation, 4);
        assert_eq!(snapshot.binding.target_auth_version, 2);
        assert!(snapshot.is_current_at(&state, now_ms()));
        set_auth(&state, 3);
        assert!(!snapshot.is_current_at(&state, now_ms()));
        assert!(paired_password_binding(&state).await.is_none());
        state
            .public_connection
            .temporary_auth_version
            .store(3, Ordering::Release);
        let current = paired_password_binding(&state).await.unwrap();
        assert_eq!(current.binding.target_auth_version, 3);
        assert_ne!(current.binding, snapshot.binding);
    }

    #[tokio::test]
    async fn paired_password_late_auth_ack_cannot_publish_old_material_or_auth_cache() {
        let state = ready_state().await;
        let _gate = state.authorization_security_gate.lock().await;
        let epoch = state.public_connection.temporary_epoch.current().unwrap();
        let mut local = state.public_connection.temporary.lock().await;
        local.rotate(5, now_ms()).unwrap();
        let document = local.access_document("fixture", 2).unwrap();
        state
            .public_connection
            .temporary_auth_version
            .store(1, Ordering::Release);
        // The publication's HTTP request used auth=2; a new registration arrived
        // before its otherwise valid remote ACK. No network is used here.
        set_auth(&state, 3);
        assert_eq!(
            mark_publication_ack(&state.public_connection, &mut local, epoch, 2, &document),
            Err("temporary_publication_superseded")
        );
        assert!(!local.status(now_ms(), true).ready);
        assert_eq!(
            state
                .public_connection
                .temporary_auth_version
                .load(Ordering::Acquire),
            1
        );
        set_auth(&state, 2);
        assert_eq!(
            mark_publication_ack(&state.public_connection, &mut local, epoch, 2, &document),
            Ok(())
        );
        assert!(local.status(now_ms(), true).ready);
        assert_eq!(
            state
                .public_connection
                .temporary_auth_version
                .load(Ordering::Acquire),
            2
        );
    }

    #[tokio::test]
    async fn paired_password_enrollment_guard_rechecks_after_the_sql_write_lock_wait() {
        use mrd_store_sqlite::{
            AeadSecretProtector, PairedAccessEnrollment, PersistentStore, StoreError,
        };
        for change_auth in [false, true] {
            let state = ready_state().await;
            let snapshot = paired_password_binding(&state).await.unwrap();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("pairing.sqlite");
            let store = Arc::new(
                PersistentStore::open(
                    &path,
                    Arc::new(AeadSecretProtector::from_key([21; 32]).unwrap()),
                )
                .unwrap(),
            );
            let peer =
                mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
            let enrollment = PairedAccessEnrollment {
                peer_key_id: peer.key_id().to_owned(),
                public_key: peer.public_key().to_vec(),
                epoch: 1,
                owner_user_sid_hash: [8; 32],
                expected_trust_revision: None,
                expected_policy_revision: 0,
                scopes: vec!["screen.view".to_owned()],
                password_binding: snapshot.binding,
            };
            let blocker = rusqlite::Connection::open(&path).unwrap();
            blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
            let (started, started_rx) = oneshot::channel();
            let worker_state = state.clone();
            let worker_store = store.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                worker_store.enroll_paired_access_guarded(enrollment, 1, || {
                    if snapshot.is_current_at(&worker_state, now_ms()) {
                        Ok(())
                    } else {
                        Err(StoreError::TrustTransition(
                            "pairing password version changed".to_owned(),
                        ))
                    }
                })
            });
            started_rx.await.unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(25), &mut worker)
                .await
                .is_err());
            if change_auth {
                // Registration replacement is not serialized by the pairing gate.
                set_auth(&state, 3);
            } else {
                // Explicit refresh must invalidate the write even before material
                // can change under a held pairing gate.
                state
                    .public_connection
                    .temporary_epoch
                    .begin(false)
                    .unwrap();
            }
            blocker.execute_batch("ROLLBACK").unwrap();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), worker)
                    .await
                    .unwrap()
                    .unwrap(),
                Err(StoreError::TrustTransition(_))
            ));
            assert!(store.paired_access_policy(peer.key_id()).unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn paired_password_explicit_intent_fences_before_waiting_for_authorization_gate() {
        for disable_password in [false, true] {
            let state = ready_state().await;
            // Refuse before all HTTP: this test isolates the real intent fence.
            *state.public_connection.registration.write().unwrap() = None;
            let current_epoch = state.public_connection.temporary_epoch.current().unwrap();
            let gate = state.authorization_security_gate.lock().await;
            let worker_state = state.clone();
            let (started, started_rx) = oneshot::channel();
            let worker = tokio::spawn(async move {
                started.send(()).unwrap();
                if disable_password {
                    disable(&worker_state).await
                } else {
                    rotate(&worker_state).await
                }
            });
            started_rx.await.unwrap();
            tokio::task::yield_now().await;
            assert!(
                state.public_connection.temporary_epoch.current().unwrap() > current_epoch,
                "explicit password intent must fence old authority before any gate await"
            );
            assert!(state
                .public_connection
                .temporary
                .lock()
                .await
                .paired_password_snapshot(now_ms(), true)
                .is_none());
            drop(gate);
            assert!(matches!(
                worker.await.unwrap(),
                Err("temporary_device_unregistered")
            ));
        }
    }

    #[tokio::test]
    async fn paired_password_disable_revokes_active_guest_leases_while_security_gate_is_held() {
        use crate::session_authorization::{
            VerifiedIncomingAuthorizationRequest, VerifiedSessionGrant,
        };
        use mrd_ipc::{ConsentDecision, ConsentResponse, RemoteAccessMode, RemotePermissionScope};
        use mrd_proto::{DeviceId, SessionId};

        let state = ready_state().await;
        let now = now_ms();
        let session = SessionId("synthetic-active-guest".to_owned());
        let scopes = vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
        ];
        // Domain-only fixture: no Agent consent, capture, input or real session.
        let pending = state
            .session_authorizations
            .begin_verified_incoming(VerifiedIncomingAuthorizationRequest {
                session_id: session.clone(),
                peer_device_id: DeviceId("synthetic-controller".to_owned()),
                peer_key_id: mrd_identity::public_key_id(&[7; 32]),
                peer_key_epoch: 1,
                access_mode: RemoteAccessMode::Attended,
                requested_scopes: scopes.clone(),
                peer_permission_ceiling: scopes.clone(),
                machine_permission_ceiling: scopes.clone(),
                runtime_capabilities: scopes.clone(),
                transport_kind: "webrtc_relay".to_owned(),
                request_nonce: [1; 16],
                created_at_ms: now,
                expires_at_ms: now + 60_000,
            })
            .await
            .unwrap();
        state
            .session_authorizations
            .respond_to_consent(
                ConsentResponse {
                    session_id: session.clone(),
                    decision: ConsentDecision::Approve,
                    approved_scopes: scopes.clone(),
                    expected_policy_revision: pending.policy_revision,
                },
                now,
            )
            .await
            .unwrap();
        state
            .session_authorizations
            .install_verified_grant(
                VerifiedSessionGrant {
                    grant_id: format!("sha256:{}", "ab".repeat(32)),
                    session_id: session.clone(),
                    granted_scopes: scopes,
                    issued_at_ms: now,
                    expires_at_ms: now + 60_000,
                    policy_revision: pending.policy_revision.get(),
                    route_constraint: "webrtc_relay".to_owned(),
                    transport_fingerprint_sha256: [9; 32],
                },
                now,
            )
            .await
            .unwrap();
        let authority = crate::temporary_access::TemporaryGuestAuthority {
            generation: 4,
            target_auth_version: 2,
        };
        {
            let mut temporary = state.public_connection.temporary.lock().await;
            assert!(
                temporary.authorize_guest(&session.0, authority, "fixture", false, now, true, 2)
            );
            assert!(temporary.authorize_guest(&session.0, authority, "fixture", true, now, true, 2));
        }
        let lease = state
            .session_authorizations
            .acquire_scope_lease(&session, RemotePermissionScope::InputPointer)
            .await
            .unwrap();
        let watcher = state
            .session_authorizations
            .acquire_scope_lease(&session, RemotePermissionScope::InputPointer)
            .await
            .unwrap();
        assert!(lease.is_valid_at(now_ms()));
        let gate = state.authorization_security_gate.lock().await;
        let worker_state = state.clone();
        let worker = tokio::spawn(async move { disable(&worker_state).await });
        let revoked_without_gate =
            tokio::time::timeout(Duration::from_secs(1), watcher.wait_until_invalid())
                .await
                .is_ok();
        let fresh_lease = state
            .session_authorizations
            .acquire_scope_lease(&session, RemotePermissionScope::InputPointer)
            .await;
        // Cancel before the gated full resource terminalizer. This fixture owns
        // no OS or WAN resources; only the immediate registry denial is tested.
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        drop(gate);
        assert!(
            revoked_without_gate,
            "disable must revoke existing leases before waiting for the security gate"
        );
        assert!(!lease.is_valid_at(now_ms()));
        assert!(fresh_lease.is_none());
    }
}
