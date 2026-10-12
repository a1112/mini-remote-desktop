//! Explicit local key pairing and automatic per-session authorization.
//! Durable policy is an authority; UI projections and temporary passwords are not.

use crate::{ipc_server::IpcServer, AppState};
use mrd_ipc::{
    DecimalU64, IpcResponse, PairedAccessPolicySnapshot, RemoteAccessMode,
    RemoteAuthorizationState, RemoteFailure, RemotePermissionScope, RemoteReasonCode,
    RemoteSessionRole,
};
use mrd_proto::SessionId;
use mrd_store_sqlite::{PairedAccessEnrollment, PairedAccessPasswordBinding, PairedAccessPolicy};
use std::sync::Arc;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn rejected(message: &str) -> RemoteFailure {
    RemoteFailure {
        code: RemoteReasonCode::PolicyChanged,
        message: message.to_owned(),
        suggested_action: Some("pair this controller in the active local desktop".to_owned()),
    }
}

fn ipc_error(message: &str) -> IpcResponse {
    IpcResponse::Error {
        code: "E_PAIRED_POLICY_DENIED".to_owned(),
        message: message.to_owned(),
    }
}

fn scopes(policy: &PairedAccessPolicy) -> Result<Vec<RemotePermissionScope>, RemoteFailure> {
    policy
        .scopes
        .iter()
        .map(|scope| {
            serde_json::from_value(serde_json::Value::String(scope.clone()))
                .map_err(|_| rejected("sealed paired policy has invalid scopes"))
        })
        .collect()
}

fn password_binding_matches(
    policy: &PairedAccessPolicy,
    current: Option<&PairedAccessPasswordBinding>,
) -> bool {
    policy.enabled
        && current.is_some_and(|binding| policy.password_binding.as_ref() == Some(binding))
}

fn projection(
    policy: PairedAccessPolicy,
    current: Option<&PairedAccessPasswordBinding>,
) -> Result<PairedAccessPolicySnapshot, RemoteFailure> {
    let parsed = scopes(&policy)?;
    let enabled = password_binding_matches(&policy, current);
    Ok(PairedAccessPolicySnapshot {
        peer_key_id: policy.peer_key_id,
        epoch: DecimalU64::new(policy.epoch),
        trust_revision: DecimalU64::new(policy.trust_revision),
        scopes: parsed,
        revision: DecimalU64::new(policy.revision),
        enabled,
    })
}

/// Caller holds authorization_security_gate across policy approval/grant installation.
pub(crate) async fn verify_session_policy_under_security_gate(
    state: &AppState,
    session_id: &SessionId,
) -> Result<PairedAccessPolicy, RemoteFailure> {
    let now = now_ms();
    let snapshot = state
        .session_authorizations
        .snapshot_at(session_id, now)
        .await
        .ok_or_else(|| rejected("paired session no longer exists"))?;
    let public_key = state
        .session_authorizations
        .authenticated_peer_key_at(session_id, now)
        .await
        .ok_or_else(|| rejected("paired controller key is not authenticated"))?;
    let store = state
        .device_identities
        .paired_policy_store()
        .map_err(|_| rejected("sealed paired policy store is unavailable"))?;
    let policy = store
        .paired_access_policy(&snapshot.peer_key_id)
        .map_err(|_| rejected("sealed paired policy integrity failed"))?
        .filter(|p| p.enabled && p.public_key.as_slice() == public_key)
        .ok_or_else(|| rejected("this controller is not paired for automatic access"))?;
    let current_password = crate::public_connection::temporary::paired_password_binding(state)
        .await
        .ok_or_else(|| rejected("current temporary password is not ready for paired access"))?;
    if !password_binding_matches(&policy, Some(&current_password.binding)) {
        return Err(rejected(
            "the password changed; pair this controller again in the local desktop",
        ));
    }
    if snapshot.role != RemoteSessionRole::Agent
        || snapshot
            .authorization_expires_at_ms
            .is_none_or(|expiry| now >= expiry)
    {
        return Err(rejected(
            "paired session is expired or belongs to another role",
        ));
    }
    #[cfg(windows)]
    {
        let active =
            unsafe { windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() };
        let agent = state
            .agent_registry
            .active_for_session_at(active, now)
            .ok_or_else(|| rejected("active local Agent is unavailable"))?;
        let owner = crate::agent_runtime::paired_owner_for_observed_process(
            &crate::agent_runtime::ObservedAgentIdentity {
                caller_kind: crate::agent_runtime::AgentCallerKind::InteractiveUser,
                process_id: agent.identity.process_id,
                process_creation_time: agent.identity.process_creation_time,
                logon_sid_hash: agent.identity.logon_sid_hash,
                windows_session_id: agent.identity.windows_session_id,
            },
        )
        .map_err(|_| rejected("paired Agent owner could not be verified"))?;
        if active == 0 || active == u32::MAX || owner != policy.owner_user_sid_hash {
            return Err(rejected(
                "paired access belongs to a different local desktop owner",
            ));
        }
        state
            .agent_registry
            .bind_active_session(active, mrd_agent_ipc::AgentCapability::Capture, now)
            .map_err(|_| rejected("paired desktop capture is unavailable"))?;
    }
    #[cfg(not(windows))]
    return Err(rejected(
        "paired desktop authority is not implemented on this platform",
    ));
    let permitted = scopes(&policy)?;
    if snapshot
        .granted_scopes
        .iter()
        .any(|scope| !permitted.contains(scope))
        || state
            .session_authorizations
            .paired_policy_revision(session_id)
            .await
            .is_some_and(|revision| revision != policy.revision)
    {
        return Err(rejected(
            "paired policy changed after session authorization",
        ));
    }
    if !current_password.is_current_at(state, now_ms()) {
        return Err(rejected(
            "password changed during paired policy verification",
        ));
    }
    Ok(policy)
}

pub(crate) async fn authorize_automatic_under_security_gate(
    state: &AppState,
    session_id: &SessionId,
) -> Result<(), RemoteFailure> {
    let policy = verify_session_policy_under_security_gate(state, session_id).await?;
    let password_snapshot = crate::public_connection::temporary::paired_password_binding(state)
        .await
        .ok_or_else(|| rejected("current temporary password is not ready for paired access"))?;
    if !password_binding_matches(&policy, Some(&password_snapshot.binding)) {
        return Err(rejected("password changed before paired authorization"));
    }
    state
        .session_authorizations
        .authorize_paired_policy_at(session_id, policy.revision, &scopes(&policy)?, now_ms())
        .await?;
    if !password_snapshot.is_current_at(state, now_ms()) {
        return Err(rejected("password changed during paired authorization"));
    }
    Ok(())
}

#[cfg(windows)]
impl IpcServer {
    pub(crate) async fn pair_pending_session(
        &self,
        session_id: SessionId,
        mut approved_scopes: Vec<RemotePermissionScope>,
        expected_policy_revision: DecimalU64,
    ) -> IpcResponse {
        if !self.product_only || self.product_caller.is_none() {
            return ipc_error("pairing requires the verified installed local UI");
        }
        let _gate = self.app_state.authorization_security_gate.lock().await;
        if self.validate_first_pairing_caller().is_err() {
            return ipc_error("pairing requires the active local owner desktop");
        }
        let caller = self.product_caller.as_ref().expect("verified caller");
        let owner = match crate::agent_runtime::paired_owner_for_observed_process(caller) {
            Ok(owner) => owner,
            Err(_) => return ipc_error("local owner identity changed"),
        };
        let now = now_ms();
        let Some(snapshot) = self
            .app_state
            .session_authorizations
            .snapshot_at(&session_id, now)
            .await
        else {
            return ipc_error("pending session is unavailable");
        };
        if snapshot.role != RemoteSessionRole::Agent
            || snapshot.access_mode != RemoteAccessMode::Attended
            || snapshot.authorization_state != RemoteAuthorizationState::AwaitingLocalConsent
            || snapshot.policy_revision != expected_policy_revision
            || snapshot
                .authorization_expires_at_ms
                .is_none_or(|expiry| now >= expiry)
            || !approved_scopes.contains(&RemotePermissionScope::ScreenView)
            || approved_scopes.iter().any(|scope| {
                !matches!(
                    scope,
                    RemotePermissionScope::ScreenView
                        | RemotePermissionScope::InputPointer
                        | RemotePermissionScope::InputKeyboard
                ) || !snapshot.requested_scopes.contains(scope)
            })
        {
            return ipc_error("pairing no longer matches this pending screen/input request");
        }
        approved_scopes.sort_unstable();
        approved_scopes.dedup();
        let Some(public_key) = self
            .app_state
            .session_authorizations
            .authenticated_peer_key_at(&session_id, now)
            .await
        else {
            return ipc_error("pending controller key has not been authenticated");
        };
        // Open the exact global-session write lease before touching durable trust.
        // LAN consent cannot produce a policy whose later authorization would fail.
        let Some(pending) = self
            .app_state
            .session_authorizations
            .pending_pairing_lease_at(&session_id, now_ms())
            .await
        else {
            return ipc_error("pending pairing request changed or is not a global session");
        };
        let store = match self.app_state.device_identities.paired_policy_store() {
            Ok(store) => store,
            Err(_) => return ipc_error("sealed paired policy store is unavailable"),
        };
        let prior_trust = match store.trust_record(&snapshot.peer_key_id) {
            Ok(trust) => trust,
            Err(_) => return ipc_error("sealed trust integrity failed"),
        };
        let prior_policy = match store.list_paired_access_policies() {
            Ok(policies) => policies
                .into_iter()
                .find(|p| p.peer_key_id == snapshot.peer_key_id),
            Err(_) => return ipc_error("sealed paired policy integrity failed"),
        };
        let scope_names = approved_scopes
            .iter()
            .map(|scope| {
                serde_json::to_value(scope)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
            })
            .collect::<Option<Vec<_>>>();
        let Some(scope_names) = scope_names else {
            return ipc_error("pairing scopes are invalid");
        };
        let Some(password_snapshot) =
            crate::public_connection::temporary::paired_password_binding(&self.app_state).await
        else {
            return ipc_error("current temporary password is not ready for pairing");
        };
        let password_binding = password_snapshot.binding.clone();
        if let Some(prior) = prior_policy.as_ref().filter(|p| p.enabled) {
            let mut canonical = scope_names.clone();
            canonical.sort();
            canonical.dedup();
            if prior.owner_user_sid_hash != owner
                || prior.public_key.as_slice() != public_key
                || prior.scopes != canonical
            {
                return ipc_error("revoke the current pairing before changing its permissions");
            }
            if password_binding_matches(prior, Some(&password_binding)) {
                let result = self
                    .app_state
                    .session_authorizations
                    .authorize_paired_policy_at(
                        &session_id,
                        prior.revision,
                        &approved_scopes,
                        now_ms(),
                    )
                    .await;
                if !password_snapshot.is_current_at(&self.app_state, now_ms()) {
                    return ipc_error(
                        "password changed while confirming pairing; confirm a new request",
                    );
                }
                return match result {
                    Ok(session) => IpcResponse::ConsentRecorded { session },
                    Err(_) => {
                        ipc_error("the paired request expired; reconnect using paired access")
                    }
                };
            }
        }
        let enrollment = PairedAccessEnrollment {
            peer_key_id: snapshot.peer_key_id.clone(),
            public_key: public_key.to_vec(),
            epoch: prior_trust.as_ref().map_or(1, |t| t.epoch),
            expected_trust_revision: prior_trust.map(|t| t.revision),
            expected_policy_revision: prior_policy.map_or(0, |p| p.revision),
            scopes: scope_names,
            owner_user_sid_hash: owner,
            password_binding,
        };
        let server = self.clone();
        let policy = match tokio::task::spawn_blocking(move || {
            store.enroll_paired_access_guarded(enrollment, now, || {
                if !pending.is_valid_at(now_ms()) {
                    return Err(mrd_store_sqlite::StoreError::TrustTransition(
                        "pending pairing request expired or was revoked".to_owned(),
                    ));
                }
                if !password_snapshot.is_current_at(&server.app_state, now_ms()) {
                    return Err(mrd_store_sqlite::StoreError::TrustTransition(
                        "pairing password version changed".to_owned(),
                    ));
                }
                server.validate_first_pairing_caller()
            })
        })
        .await
        {
            Ok(Ok(policy)) => policy,
            _ => {
                return ipc_error(
                    "pairing could not be saved; existing trust may forbid these permissions",
                )
            }
        };
        if self.validate_first_pairing_caller().is_err() {
            return ipc_error("local desktop changed while saving pairing");
        }
        let current_password =
            crate::public_connection::temporary::paired_password_binding(&self.app_state).await;
        if !password_binding_matches(&policy, current_password.as_ref().map(|s| &s.binding)) {
            return ipc_error("password changed while saving pairing; confirm a new request");
        }
        let result = self
            .app_state
            .session_authorizations
            .authorize_paired_policy_at(&session_id, policy.revision, &approved_scopes, now_ms())
            .await;
        if current_password
            .as_ref()
            .is_none_or(|password| !password.is_current_at(&self.app_state, now_ms()))
        {
            return ipc_error("password changed while confirming pairing; confirm a new request");
        }
        match result {
            Ok(session) => IpcResponse::ConsentRecorded { session },
            Err(_) => ipc_error(
                "pairing was saved, but this request expired; reconnect using paired access",
            ),
        }
    }

    pub(crate) async fn list_paired_access(&self) -> IpcResponse {
        if !self.product_only {
            return ipc_error("paired policies require the verified local UI");
        }
        let Some(caller) = self.product_caller.as_ref() else {
            return ipc_error("paired policies require the verified local UI");
        };
        let owner = match crate::agent_runtime::paired_owner_for_observed_process(caller) {
            Ok(owner) => owner,
            Err(_) => return ipc_error("local owner identity changed"),
        };
        let _gate = self.app_state.authorization_security_gate.lock().await;
        if self.validate_first_pairing_caller().is_err() {
            return ipc_error("paired policies require the active local owner");
        }
        let store = match self.app_state.device_identities.paired_policy_store() {
            Ok(store) => store,
            Err(_) => return ipc_error("sealed paired policy store is unavailable"),
        };
        let current_password =
            crate::public_connection::temporary::paired_password_binding(&self.app_state).await;
        match store.list_paired_access_policies() {
            Ok(policies) => match policies
                .into_iter()
                .filter(|p| p.owner_user_sid_hash == owner)
                .map(|policy| projection(policy, current_password.as_ref().map(|s| &s.binding)))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(policies) => IpcResponse::PairedAccessPolicies { policies },
                Err(_) => ipc_error("sealed policy projection failed"),
            },
            Err(_) => ipc_error("sealed paired policy integrity failed"),
        }
    }

    pub(crate) async fn revoke_paired_access(
        &self,
        peer_key_id: String,
        expected_revision: DecimalU64,
    ) -> IpcResponse {
        if !self.product_only || self.product_caller.is_none() {
            return ipc_error("revocation requires the verified installed local UI");
        }
        let _gate = self.app_state.authorization_security_gate.lock().await;
        if self.validate_first_pairing_caller().is_err() {
            return ipc_error("revocation requires the active local owner");
        }
        let store = match self.app_state.device_identities.paired_policy_store() {
            Ok(store) => store,
            Err(_) => return ipc_error("sealed paired policy store is unavailable"),
        };
        let owner = match crate::agent_runtime::paired_owner_for_observed_process(
            self.product_caller.as_ref().expect("verified caller"),
        ) {
            Ok(owner) => owner,
            Err(_) => return ipc_error("local owner identity changed"),
        };
        let policy = match store.list_paired_access_policies() {
            Ok(policies) => match policies
                .into_iter()
                .find(|p| p.peer_key_id == peer_key_id && p.owner_user_sid_hash == owner)
            {
                Some(policy) => policy,
                None => return ipc_error("paired policy is unavailable for this local owner"),
            },
            _ => return ipc_error("sealed paired policy integrity failed"),
        };
        if policy.revision != expected_revision.get() {
            return ipc_error("paired policy revision changed");
        }
        let server = self.clone();
        let key = peer_key_id.clone();
        let policy = match tokio::task::spawn_blocking(move || {
            store.revoke_paired_access_guarded(&key, expected_revision.get(), now_ms(), || {
                server.validate_first_pairing_caller()
            })
        })
        .await
        {
            Ok(Ok(policy)) => policy,
            _ => return ipc_error("paired access revocation could not be saved"),
        };
        revoke_peer_sessions_under_security_gate(&self.app_state, &peer_key_id).await;
        match projection(policy, None) {
            Ok(policy) => IpcResponse::PairedAccessPolicyUpdated { policy },
            Err(_) => ipc_error("revoked paired policy projection failed"),
        }
    }
}

async fn revoke_peer_sessions_under_security_gate(state: &Arc<AppState>, peer_key_id: &str) {
    let sessions = state
        .session_authorizations
        .revoke_peer_authorizations(peer_key_id, now_ms())
        .await;
    let mut lan = Vec::new();
    for session_id in sessions {
        match crate::wan_session::service::resolve_session_kind_under_security_gate(
            state,
            &session_id,
        )
        .await
        {
            crate::wan_session::service::ServiceSessionKind::Wan => {
                if crate::wan_session::service::terminalize_wan_session_under_security_gate(
                    state,
                    &session_id,
                    crate::wan_session::service::ServiceWanTerminalRequest::Fail {
                        failure: crate::wan_session::model::WanSessionFailure::Cancelled,
                        remote_failure: RemoteFailure {
                            code: RemoteReasonCode::GrantRevoked,
                            message: "paired access was revoked".to_owned(),
                            suggested_action: None,
                        },
                    },
                )
                .await
                .is_err()
                {
                    state.mark_security_unhealthy();
                }
            }
            crate::wan_session::service::ServiceSessionKind::Lan => lan.push(session_id),
            crate::wan_session::service::ServiceSessionKind::Unknown => {
                #[cfg(windows)]
                state.console_capture.stop(state, &session_id).await;
                state.mark_security_unhealthy();
            }
        }
    }
    crate::lan_discovery::terminate_authorized_remote_sessions_under_security_gate(state, &lan)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::IpcRequest;

    fn password_binding(generation: u64, auth: u64) -> PairedAccessPasswordBinding {
        PairedAccessPasswordBinding {
            temporary_generation: generation,
            target_auth_version: auth,
        }
    }

    fn password_policy() -> PairedAccessPolicy {
        PairedAccessPolicy {
            peer_key_id: "00".repeat(32),
            public_key: vec![7; 32],
            epoch: 1,
            owner_user_sid_hash: [8; 32],
            trust_revision: 1,
            scopes: vec!["screen.view".to_owned()],
            revision: 1,
            enabled: true,
            password_binding: Some(password_binding(4, 2)),
        }
    }

    #[test]
    fn paired_password_policy_requires_exact_nonlegacy_current_versions() {
        let mut policy = password_policy();
        let current = password_binding(4, 2);
        assert!(password_binding_matches(&policy, Some(&current)));
        assert!(!password_binding_matches(&policy, None));
        assert!(!password_binding_matches(
            &policy,
            Some(&password_binding(5, 2))
        ));
        assert!(!password_binding_matches(
            &policy,
            Some(&password_binding(4, 3))
        ));
        assert!(projection(policy.clone(), Some(&current)).unwrap().enabled);
        assert!(
            !projection(policy.clone(), Some(&password_binding(5, 2)))
                .unwrap()
                .enabled
        );
        policy.password_binding = None;
        assert!(!password_binding_matches(&policy, Some(&current)));
        assert!(!projection(policy.clone(), Some(&current)).unwrap().enabled);
        policy.password_binding = Some(current);
        policy.enabled = false;
        assert!(!password_binding_matches(&policy, Some(&current)));
    }

    #[test]
    fn paired_password_reenrollment_advances_revision_and_restores_exact_binding() {
        use mrd_store_sqlite::{AeadSecretProtector, PersistentStore};
        let directory = tempfile::tempdir().unwrap();
        let store = PersistentStore::open(
            directory.path().join("pairing.sqlite"),
            Arc::new(AeadSecretProtector::from_key([19; 32]).unwrap()),
        )
        .unwrap();
        let peer =
            mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
        let mut enrollment = PairedAccessEnrollment {
            peer_key_id: peer.key_id().to_owned(),
            public_key: peer.public_key().to_vec(),
            epoch: 1,
            owner_user_sid_hash: [8; 32],
            expected_trust_revision: None,
            expected_policy_revision: 0,
            scopes: vec!["screen.view".to_owned()],
            password_binding: password_binding(4, 2),
        };
        let first = store
            .enroll_paired_access_guarded(enrollment.clone(), 1, || Ok(()))
            .unwrap();
        let current = password_binding(5, 2);
        assert!(!password_binding_matches(&first, Some(&current)));
        enrollment.expected_trust_revision = Some(first.trust_revision);
        enrollment.expected_policy_revision = first.revision;
        enrollment.password_binding = current;
        let confirmed = store
            .enroll_paired_access_guarded(enrollment, 2, || Ok(()))
            .unwrap();
        assert_eq!(confirmed.revision, first.revision + 1);
        assert_eq!(confirmed.scopes, first.scopes);
        assert_eq!(confirmed.public_key, first.public_key);
        assert!(password_binding_matches(&confirmed, Some(&current)));
        assert_eq!(
            store.paired_access_policy(peer.key_id()).unwrap(),
            Some(confirmed)
        );
    }

    #[tokio::test]
    async fn paired_policy_management_refuses_an_unverified_caller() {
        let state = Arc::new(AppState::new());
        let server = IpcServer::new(state);
        for request in [
            IpcRequest::PairPendingSession {
                session_id: SessionId("unverified".to_owned()),
                approved_scopes: vec![RemotePermissionScope::ScreenView],
                expected_policy_revision: DecimalU64::new(1),
            },
            IpcRequest::ListPairedAccessPolicies,
            IpcRequest::RevokePairedAccess {
                peer_key_id: "00".repeat(32),
                expected_revision: DecimalU64::new(1),
            },
        ] {
            assert!(
                matches!(server.handle_request(request).await, IpcResponse::Error { code, .. } if code == "E_PAIRED_POLICY_DENIED")
            );
        }
    }

    #[tokio::test]
    async fn legacy_memory_trust_is_never_a_durable_paired_policy() {
        let state = AppState::new();
        let now = now_ms();
        let key = [7; 32];
        let session_id = SessionId("memory-trust-is-not-pairing".to_owned());
        state
            .session_authorizations
            .begin_verified_incoming(
                crate::session_authorization::VerifiedIncomingAuthorizationRequest {
                    session_id: session_id.clone(),
                    peer_device_id: mrd_proto::DeviceId("controller".to_owned()),
                    peer_key_id: mrd_identity::public_key_id(&key),
                    peer_key_epoch: 1,
                    access_mode: RemoteAccessMode::Paired,
                    requested_scopes: vec![RemotePermissionScope::ScreenView],
                    peer_permission_ceiling: vec![RemotePermissionScope::ScreenView],
                    machine_permission_ceiling: vec![RemotePermissionScope::ScreenView],
                    runtime_capabilities: vec![RemotePermissionScope::ScreenView],
                    transport_kind: "webrtc_relay".to_owned(),
                    request_nonce: [1; 16],
                    created_at_ms: now,
                    expires_at_ms: now + 60_000,
                },
            )
            .await
            .unwrap();
        state
            .session_authorizations
            .bind_authenticated_peer_key(&session_id, &key, now)
            .await
            .unwrap();
        assert!(authorize_automatic_under_security_gate(&state, &session_id)
            .await
            .is_err());
        assert_eq!(
            state
                .session_authorizations
                .snapshot(&session_id)
                .await
                .unwrap()
                .authorization_state,
            RemoteAuthorizationState::Authenticating
        );
    }
}
