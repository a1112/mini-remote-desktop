use mrd_identity::{public_key_id, UnattendedCredential};
use mrd_ipc::{
    ConsentDecision, ConsentResponse, DecimalU64, RemoteAccessMode, RemoteAuthorizationState,
    RemoteCursorState, RemoteFailure, RemoteMediaState, RemotePermissionScope,
    RemotePresentationState, RemoteReasonCode, RemoteRouteKind, RemoteRouteState,
    RemoteSessionEvent, RemoteSessionEventEnvelope, RemoteSessionRole, RemoteSessionSnapshot,
    RouteCandidateEvidence, RouteCandidateState, RouteEvidence, SessionEventSubscription,
    SessionEventSubscriptionQuery, UnattendedAccessPolicy, UnattendedAccessSnapshot,
};
use mrd_proto::{DeviceId, SessionId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
#[cfg(any(windows, test))]
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Weak,
};
use tokio::sync::{watch, Mutex, Notify};
use tokio::time::{timeout, Duration};

const EVENT_HISTORY_LIMIT: usize = 1_024;
const AUTHORIZATION_RECORD_LIMIT: usize = 2_048;
const TERMINAL_RECORD_RETENTION_MS: u64 = 10 * 60 * 1_000;
const MAX_PENDING_INCOMING_AUTHORIZATIONS: usize = 64;
const MAX_PENDING_INCOMING_PER_PEER: usize = 4;
const PENDING_OUTGOING_PEER_KEY_PREFIX: &str = "pending_authenticated_peer:";

pub(crate) fn pending_outgoing_peer_key_id(peer_device_id: &DeviceId) -> String {
    format!("{PENDING_OUTGOING_PEER_KEY_PREFIX}{}", peer_device_id.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentResolutionError {
    Rejected(RemoteFailure),
    AuditUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIncomingAuthorizationRequest {
    pub session_id: SessionId,
    pub peer_device_id: DeviceId,
    pub peer_key_id: String,
    pub peer_key_epoch: u64,
    pub access_mode: RemoteAccessMode,
    pub requested_scopes: Vec<RemotePermissionScope>,
    pub peer_permission_ceiling: Vec<RemotePermissionScope>,
    pub machine_permission_ceiling: Vec<RemotePermissionScope>,
    pub runtime_capabilities: Vec<RemotePermissionScope>,
    pub transport_kind: String,
    pub request_nonce: [u8; 16],
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct AuthorizationRecord {
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    instance_id: u64,
    request: VerifiedIncomingAuthorizationRequest,
    snapshot: RemoteSessionSnapshot,
    grant: Option<VerifiedSessionGrant>,
    peer_public_key: Option<[u8; 32]>,
    cancellation: watch::Sender<bool>,
}

pub(crate) struct SessionAuthorizationLease {
    cancellation: watch::Receiver<bool>,
    expires_at_ms: u64,
}

impl SessionAuthorizationLease {
    pub(crate) async fn wait_until_invalid(mut self) {
        let now_ms = unix_time_ms();
        if now_ms > self.expires_at_ms || *self.cancellation.borrow() {
            return;
        }
        let deadline = tokio::time::sleep(Duration::from_millis(
            self.expires_at_ms
                .saturating_sub(now_ms)
                .saturating_add(1)
                .max(1),
        ));
        tokio::pin!(deadline);
        loop {
            if *self.cancellation.borrow() {
                return;
            }
            tokio::select! {
                biased;
                changed = self.cancellation.changed() => {
                    if changed.is_err() || *self.cancellation.borrow_and_update() {
                        return;
                    }
                }
                _ = &mut deadline => return,
            }
        }
    }
}

fn new_authorization_record(
    request: VerifiedIncomingAuthorizationRequest,
    snapshot: RemoteSessionSnapshot,
    instance_id: u64,
) -> AuthorizationRecord {
    let (cancellation, _) = watch::channel(false);
    AuthorizationRecord {
        instance_id,
        request,
        snapshot,
        grant: None,
        peer_public_key: None,
        cancellation,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedSessionGrant {
    pub grant_id: String,
    pub session_id: SessionId,
    pub granted_scopes: Vec<RemotePermissionScope>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub policy_revision: u64,
    pub route_constraint: String,
    pub transport_fingerprint_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveControlAuthorization {
    pub session_id: SessionId,
    pub role: RemoteSessionRole,
    pub peer_device_id: DeviceId,
    pub peer_key_id: String,
    pub peer_public_key: [u8; 32],
    pub grant_id: [u8; 32],
    pub granted_scopes: Vec<RemotePermissionScope>,
    pub expires_at_ms: u64,
    pub policy_revision: u64,
}

#[derive(Debug, Default)]
struct AuthorizationRegistryInner {
    records: HashMap<SessionId, AuthorizationRecord>,
    events: VecDeque<RemoteSessionEventEnvelope>,
    next_sequence: u64,
    next_instance_id: u64,
    #[cfg(any(windows, test))]
    outgoing_birth_observers: HashMap<SessionId, Vec<Weak<AtomicU64>>>,
}

/// Correlates one empty slot's first birth with its local request. This private
/// receipt never grants a permission and cannot be constructed from an IPC ID.
#[cfg(any(windows, test))]
#[derive(Debug, Clone)]
pub(crate) struct OutgoingBirthReceipt {
    instance: Arc<AtomicU64>,
}

#[cfg(any(windows, test))]
impl OutgoingBirthReceipt {
    pub(crate) fn instance_id(&self) -> Option<u64> {
        let instance = self.instance.load(Ordering::Acquire);
        (instance != 0).then_some(instance)
    }
}

#[cfg(any(windows, test))]
const MAX_OUTGOING_BIRTH_OBSERVERS: usize = 128;
#[cfg(any(windows, test))]
const MAX_OUTGOING_BIRTH_OBSERVERS_PER_SESSION: usize = 8;

#[derive(Debug, Default)]
pub struct SessionAuthorizationRegistry {
    inner: Mutex<AuthorizationRegistryInner>,
    unattended: Mutex<UnattendedAuthorizationState>,
    changed: Notify,
}

#[derive(Debug)]
struct UnattendedAuthorizationState {
    credential: Option<Arc<UnattendedCredential>>,
    access_epoch: u64,
    policy_revision: u64,
    policy: UnattendedAccessPolicy,
}

impl Default for UnattendedAuthorizationState {
    fn default() -> Self {
        Self {
            credential: None,
            access_epoch: 0,
            policy_revision: 1,
            policy: UnattendedAccessPolicy {
                trusted_devices_only: true,
                allowed_peer_key_ids: Vec::new(),
                permission_ceiling: Vec::new(),
                expires_at_ms: None,
            },
        }
    }
}

impl SessionAuthorizationRegistry {
    // Retained as the service-owned primitive for Task 40 enrollment. Task 18
    // production IPC deliberately does not expose credential enrollment.
    #[allow(dead_code)]
    pub async fn enable_unattended(
        &self,
        mut policy: UnattendedAccessPolicy,
        updated_at_ms: u64,
    ) -> Result<UnattendedAccessSnapshot, RemoteFailure> {
        normalize_scopes(&mut policy.permission_ceiling);
        validate_unattended_policy(&policy, updated_at_ms)?;
        let credential =
            UnattendedCredential::generate(&ring::rand::SystemRandom::new()).map_err(|_| {
                failure(
                    RemoteReasonCode::CredentialInvalid,
                    "failed to generate unattended access material",
                )
            })?;
        let mut unattended = self.unattended.lock().await;
        unattended.credential = Some(Arc::new(credential));
        unattended.access_epoch = unattended.access_epoch.saturating_add(1).max(1);
        unattended.policy_revision = unattended.policy_revision.saturating_add(1).max(1);
        unattended.policy = policy;
        Ok(unattended_snapshot(&unattended, updated_at_ms))
    }

    pub async fn disable_unattended(
        &self,
        expected_policy_revision: u64,
        updated_at_ms: u64,
    ) -> Result<UnattendedAccessSnapshot, RemoteFailure> {
        let mut unattended = self.unattended.lock().await;
        if unattended.policy_revision != expected_policy_revision {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "unattended policy revision changed",
            ));
        }
        unattended.credential = None;
        unattended.access_epoch = unattended.access_epoch.saturating_add(1).max(1);
        unattended.policy_revision = unattended.policy_revision.saturating_add(1).max(1);
        Ok(unattended_snapshot(&unattended, updated_at_ms))
    }

    #[allow(dead_code)]
    pub async fn rotate_unattended(
        &self,
        expected_policy_revision: u64,
        updated_at_ms: u64,
    ) -> Result<UnattendedAccessSnapshot, RemoteFailure> {
        let credential =
            UnattendedCredential::generate(&ring::rand::SystemRandom::new()).map_err(|_| {
                failure(
                    RemoteReasonCode::CredentialInvalid,
                    "failed to rotate unattended access material",
                )
            })?;
        let mut unattended = self.unattended.lock().await;
        if unattended.policy_revision != expected_policy_revision {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "unattended policy revision changed",
            ));
        }
        if unattended.credential.is_none() {
            return Err(failure(
                RemoteReasonCode::CredentialInvalid,
                "unattended access is disabled",
            ));
        }
        unattended.credential = Some(Arc::new(credential));
        unattended.access_epoch = unattended.access_epoch.saturating_add(1).max(1);
        unattended.policy_revision = unattended.policy_revision.saturating_add(1).max(1);
        Ok(unattended_snapshot(&unattended, updated_at_ms))
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub async fn configure_unattended_for_test(
        &self,
        credential: UnattendedCredential,
        access_epoch: u64,
        policy_revision: u64,
        mut policy: UnattendedAccessPolicy,
    ) {
        normalize_scopes(&mut policy.permission_ceiling);
        let mut unattended = self.unattended.lock().await;
        unattended.credential = Some(Arc::new(credential));
        unattended.access_epoch = access_epoch.max(1);
        unattended.policy_revision = policy_revision.max(1);
        unattended.policy = policy;
    }

    pub async fn begin_verified_incoming(
        &self,
        mut request: VerifiedIncomingAuthorizationRequest,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        normalize_scopes(&mut request.requested_scopes);
        normalize_scopes(&mut request.peer_permission_ceiling);
        normalize_scopes(&mut request.machine_permission_ceiling);
        normalize_scopes(&mut request.runtime_capabilities);
        validate_verified_request(&request)?;

        let mut inner = self.inner.lock().await;
        prune_authorization_records(&mut inner, request.created_at_ms);
        if inner.records.contains_key(&request.session_id) {
            return Err(failure(
                RemoteReasonCode::ReplayDetected,
                "remote session id has already been used",
            ));
        }
        if inner.records.len() >= AUTHORIZATION_RECORD_LIMIT {
            return Err(failure(
                RemoteReasonCode::CredentialLocked,
                "remote authorization capacity is temporarily exhausted",
            ));
        }
        let pending_incoming = inner
            .records
            .values()
            .filter(|record| is_pending_incoming_record(record))
            .count();
        let pending_for_peer = inner
            .records
            .values()
            .filter(|record| {
                is_pending_incoming_record(record)
                    && record.request.peer_key_id == request.peer_key_id
            })
            .count();
        if pending_incoming >= MAX_PENDING_INCOMING_AUTHORIZATIONS
            || pending_for_peer >= MAX_PENDING_INCOMING_PER_PEER
        {
            return Err(failure(
                RemoteReasonCode::CredentialLocked,
                "too many remote authorization requests are already pending",
            ));
        }

        let authorization_state = match request.access_mode {
            RemoteAccessMode::Attended => RemoteAuthorizationState::AwaitingLocalConsent,
            RemoteAccessMode::Unattended => RemoteAuthorizationState::VerifyingUnattendedCredential,
        };
        let presentation_state = match request.access_mode {
            RemoteAccessMode::Attended => RemotePresentationState::IncomingApprovalRequired,
            RemoteAccessMode::Unattended => RemotePresentationState::Authenticating,
        };
        let snapshot = RemoteSessionSnapshot {
            session_id: request.session_id.clone(),
            role: RemoteSessionRole::Agent,
            peer_device_id: request.peer_device_id.clone(),
            peer_key_id: request.peer_key_id.clone(),
            access_mode: request.access_mode,
            authorization_state,
            route_state: RemoteRouteState::Idle,
            route_kind: None,
            media_state: RemoteMediaState::Idle,
            presentation_state,
            requested_scopes: request.requested_scopes.clone(),
            granted_scopes: Vec::new(),
            policy_revision: DecimalU64::new(1),
            failure: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
            authorization_expires_at_ms: Some(request.expires_at_ms),
        };
        let instance_id = next_authorization_instance(&mut inner)?;
        #[cfg(any(windows, test))]
        {
            prune_outgoing_birth_observers(&mut inner);
            // An incoming aggregate consumed this empty slot. It may not later
            // be mistaken for the observed outgoing request's original birth.
            inner.outgoing_birth_observers.remove(&request.session_id);
        }
        inner.records.insert(
            request.session_id.clone(),
            new_authorization_record(request.clone(), snapshot.clone(), instance_id),
        );
        if request.access_mode == RemoteAccessMode::Attended {
            push_event(
                &mut inner,
                request.created_at_ms,
                request.session_id,
                RemoteSessionEvent::ConsentRequested {
                    requested_scopes: request.requested_scopes,
                },
            );
        }
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub async fn begin_outgoing(
        &self,
        mut request: VerifiedIncomingAuthorizationRequest,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        normalize_scopes(&mut request.requested_scopes);
        normalize_scopes(&mut request.peer_permission_ceiling);
        normalize_scopes(&mut request.machine_permission_ceiling);
        normalize_scopes(&mut request.runtime_capabilities);
        validate_verified_request(&request)?;
        let mut inner = self.inner.lock().await;
        prune_authorization_records(&mut inner, request.created_at_ms);
        if inner.records.contains_key(&request.session_id) {
            return Err(failure(
                RemoteReasonCode::ReplayDetected,
                "remote session id has already been used",
            ));
        }
        if inner.records.len() >= AUTHORIZATION_RECORD_LIMIT {
            return Err(failure(
                RemoteReasonCode::CredentialLocked,
                "remote authorization capacity is temporarily exhausted",
            ));
        }
        let snapshot = RemoteSessionSnapshot {
            session_id: request.session_id.clone(),
            role: RemoteSessionRole::Controller,
            peer_device_id: request.peer_device_id.clone(),
            peer_key_id: request.peer_key_id.clone(),
            access_mode: request.access_mode,
            authorization_state: RemoteAuthorizationState::Authorizing,
            route_state: RemoteRouteState::Idle,
            route_kind: None,
            media_state: RemoteMediaState::Idle,
            presentation_state: RemotePresentationState::Authenticating,
            requested_scopes: request.requested_scopes.clone(),
            granted_scopes: Vec::new(),
            policy_revision: DecimalU64::new(1),
            failure: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
            authorization_expires_at_ms: Some(request.expires_at_ms),
        };
        let instance_id = next_authorization_instance(&mut inner)?;
        #[cfg(any(windows, test))]
        let session_id = request.session_id.clone();
        inner.records.insert(
            request.session_id.clone(),
            new_authorization_record(request, snapshot.clone(), instance_id),
        );
        #[cfg(any(windows, test))]
        {
            prune_outgoing_birth_observers(&mut inner);
            if let Some(observers) = inner.outgoing_birth_observers.remove(&session_id) {
                for observer in observers.into_iter().filter_map(|weak| weak.upgrade()) {
                    let _ = observer.compare_exchange(
                        0,
                        instance_id,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
        }
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub async fn snapshot(&self, session_id: &SessionId) -> Option<RemoteSessionSnapshot> {
        self.snapshot_at(session_id, unix_time_ms()).await
    }

    /// Private aggregate identity. A reused public session ID is never the same
    /// authorization instance, even with a frozen clock or matching peer fields.
    #[cfg(any(windows, test))]
    pub(crate) async fn product_audit_binding(
        &self,
        session_id: &SessionId,
    ) -> Option<(u64, RemoteSessionSnapshot)> {
        self.inner
            .lock()
            .await
            .records
            .get(session_id)
            .map(|record| (record.instance_id, record.snapshot.clone()))
    }

    /// Subscribe before the request and before taking its audit cursor. The
    /// first insertion consumes the group under the same registry lock; later
    /// same-ID aggregates can never update an already issued receipt.
    #[cfg(any(windows, test))]
    pub(crate) async fn observe_outgoing_birth(
        &self,
        session_id: &SessionId,
    ) -> Option<OutgoingBirthReceipt> {
        let mut inner = self.inner.lock().await;
        prune_outgoing_birth_observers(&mut inner);
        if inner.records.contains_key(session_id)
            || inner
                .outgoing_birth_observers
                .values()
                .map(Vec::len)
                .sum::<usize>()
                >= MAX_OUTGOING_BIRTH_OBSERVERS
            || inner
                .outgoing_birth_observers
                .get(session_id)
                .is_some_and(|group| group.len() >= MAX_OUTGOING_BIRTH_OBSERVERS_PER_SESSION)
        {
            return None;
        }
        let instance = Arc::new(AtomicU64::new(0));
        inner
            .outgoing_birth_observers
            .entry(session_id.clone())
            .or_default()
            .push(Arc::downgrade(&instance));
        Some(OutgoingBirthReceipt { instance })
    }

    pub(crate) async fn transport_kind(&self, session_id: &SessionId) -> Option<String> {
        self.inner
            .lock()
            .await
            .records
            .get(session_id)
            .map(|record| record.request.transport_kind.clone())
    }

    pub async fn snapshot_at(
        &self,
        session_id: &SessionId,
        now_ms: u64,
    ) -> Option<RemoteSessionSnapshot> {
        let mut inner = self.inner.lock().await;
        let expired = expire_grant_locked(&mut inner, session_id, now_ms);
        let snapshot = inner
            .records
            .get(session_id)
            .map(|record| record.snapshot.clone());
        drop(inner);
        if expired {
            self.changed.notify_waiters();
        }
        snapshot
    }

    #[allow(dead_code)]
    pub async fn active_grant(&self, session_id: &SessionId) -> Option<VerifiedSessionGrant> {
        self.snapshot_at(session_id, unix_time_ms()).await?;
        self.inner
            .lock()
            .await
            .records
            .get(session_id)
            .and_then(|record| record.grant.clone())
    }

    /// Project UI-safe route evidence from the exact active grant and session aggregate.
    pub async fn verified_route_evidence_at(
        &self,
        session_id: &SessionId,
        observed_at_ms: u64,
    ) -> Result<Option<RouteEvidence>, RemoteFailure> {
        let (expired, result) = {
            let mut inner = self.inner.lock().await;
            let expired = expire_grant_locked(&mut inner, session_id, observed_at_ms);
            let result = inner
                .records
                .get(session_id)
                .map(|record| project_verified_route_evidence(record, observed_at_ms))
                .transpose();
            (expired, result)
        };
        if expired {
            self.changed.notify_waiters();
        }
        result
    }

    pub async fn bind_authenticated_peer_key(
        &self,
        session_id: &SessionId,
        peer_public_key: &[u8],
        bound_at_ms: u64,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        let peer_public_key: [u8; 32] = peer_public_key.try_into().map_err(|_| {
            failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated peer key has an invalid length",
            )
        })?;
        let mut inner = self.inner.lock().await;
        let Some(record) = inner.records.get_mut(session_id) else {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated peer key has no matching authorization request",
            ));
        };
        if is_terminal_authorization_state(record.snapshot.authorization_state) {
            return Err(record.snapshot.failure.clone().unwrap_or_else(|| {
                failure(
                    RemoteReasonCode::PolicyChanged,
                    "terminal authorization cannot accept a peer key binding",
                )
            }));
        }
        if public_key_id(&peer_public_key) != record.request.peer_key_id {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated peer key does not match the requested peer identity",
            ));
        }
        if record
            .peer_public_key
            .is_some_and(|existing| existing != peer_public_key)
        {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authorization is already bound to a different peer key",
            ));
        }
        record.peer_public_key = Some(peer_public_key);
        record.snapshot.updated_at_ms = record.snapshot.updated_at_ms.max(bound_at_ms);
        let snapshot = record.snapshot.clone();
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    /// Bind the first independently verified peer key to a controller-side
    /// authorization that was created before signaling. A pending key marker
    /// can only be replaced once, for the exact requested device and while the
    /// authorization is still in its pre-grant state.
    pub async fn bind_outgoing_authenticated_peer(
        &self,
        session_id: &SessionId,
        peer_device_id: &DeviceId,
        peer_key_id: &str,
        peer_public_key: &[u8],
        bound_at_ms: u64,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        let peer_public_key: [u8; 32] = peer_public_key.try_into().map_err(|_| {
            failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated WAN peer key has an invalid length",
            )
        })?;
        if public_key_id(&peer_public_key) != peer_key_id {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated WAN peer key identifier does not match its public key",
            ));
        }
        let mut inner = self.inner.lock().await;
        let Some(record) = inner.records.get_mut(session_id) else {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated WAN peer has no matching outgoing authorization",
            ));
        };
        let pending_key_id = pending_outgoing_peer_key_id(peer_device_id);
        if record.snapshot.role != RemoteSessionRole::Controller
            || record.snapshot.authorization_state != RemoteAuthorizationState::Authorizing
            || record.request.peer_device_id != *peer_device_id
            || (record.request.peer_key_id != pending_key_id
                && record.request.peer_key_id != peer_key_id)
            || record
                .peer_public_key
                .is_some_and(|existing| existing != peer_public_key)
        {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "authenticated WAN peer does not match the pending authorization",
            ));
        }
        record.request.peer_key_id = peer_key_id.to_owned();
        record.snapshot.peer_key_id = peer_key_id.to_owned();
        record.peer_public_key = Some(peer_public_key);
        record.snapshot.updated_at_ms = record.snapshot.updated_at_ms.max(bound_at_ms);
        let snapshot = record.snapshot.clone();
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub(crate) async fn active_control_authorization(
        &self,
        session_id: &SessionId,
        now_ms: u64,
    ) -> Result<ActiveControlAuthorization, RemoteFailure> {
        let (expired, result) = {
            let mut inner = self.inner.lock().await;
            let expired = expire_grant_locked(&mut inner, session_id, now_ms);
            let result = (|| {
                let Some(record) = inner.records.get(session_id) else {
                    return Err(failure(
                        RemoteReasonCode::IdentityMismatch,
                        "control input has no matching authorization",
                    ));
                };
                if record.snapshot.authorization_state != RemoteAuthorizationState::Granted {
                    return Err(record.snapshot.failure.clone().unwrap_or_else(|| {
                        failure(
                            RemoteReasonCode::PolicyChanged,
                            "control input authorization is not granted",
                        )
                    }));
                }
                if record.snapshot.route_state != RemoteRouteState::Connected
                    || record.snapshot.media_state != RemoteMediaState::Streaming
                {
                    return Err(failure(
                        RemoteReasonCode::PolicyChanged,
                        "control input requires a connected streaming session",
                    ));
                }
                let Some(grant) = record.grant.as_ref() else {
                    return Err(failure(
                        RemoteReasonCode::PolicyChanged,
                        "control input authorization has no verified grant",
                    ));
                };
                if grant.session_id != *session_id
                    || grant.issued_at_ms > now_ms
                    || now_ms > grant.expires_at_ms
                    || grant.policy_revision != record.snapshot.policy_revision.get()
                    || grant.granted_scopes != record.snapshot.granted_scopes
                {
                    return Err(failure(
                        RemoteReasonCode::PolicyChanged,
                        "control input grant no longer matches the active session",
                    ));
                }
                let Some(peer_public_key) = record.peer_public_key else {
                    return Err(failure(
                        RemoteReasonCode::IdentityMismatch,
                        "control input peer key has not been authenticated",
                    ));
                };
                if public_key_id(&peer_public_key) != record.request.peer_key_id {
                    return Err(failure(
                        RemoteReasonCode::IdentityMismatch,
                        "control input peer key binding is invalid",
                    ));
                }
                let grant_id = parse_sha256_identifier(&grant.grant_id).ok_or_else(|| {
                    failure(
                        RemoteReasonCode::PolicyChanged,
                        "control input grant identifier is invalid",
                    )
                })?;
                Ok(ActiveControlAuthorization {
                    session_id: session_id.clone(),
                    role: record.snapshot.role,
                    peer_device_id: record.request.peer_device_id.clone(),
                    peer_key_id: record.request.peer_key_id.clone(),
                    peer_public_key,
                    grant_id,
                    granted_scopes: grant.granted_scopes.clone(),
                    expires_at_ms: grant.expires_at_ms,
                    policy_revision: grant.policy_revision,
                })
            })();
            (expired, result)
        };
        if expired {
            self.changed.notify_waiters();
        }
        result
    }

    pub async fn allows_scope(
        &self,
        session_id: &SessionId,
        scope: RemotePermissionScope,
        now_ms: u64,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        let expired = expire_grant_locked(&mut inner, session_id, now_ms);
        let allowed = inner.records.get(session_id).is_some_and(|record| {
            record.grant.as_ref().is_some_and(|grant| {
                record.snapshot.authorization_state == RemoteAuthorizationState::Granted
                    && grant.issued_at_ms <= now_ms
                    && now_ms <= grant.expires_at_ms
                    && grant.granted_scopes.contains(&scope)
            })
        });
        drop(inner);
        if expired {
            self.changed.notify_waiters();
        }
        allowed
    }

    pub(crate) async fn acquire_scope_lease(
        &self,
        session_id: &SessionId,
        scope: RemotePermissionScope,
    ) -> Option<SessionAuthorizationLease> {
        let (expired, lease) = {
            let mut inner = self.inner.lock().await;
            let now_ms = unix_time_ms();
            let expired = expire_grant_locked(&mut inner, session_id, now_ms);
            let lease = inner.records.get(session_id).and_then(|record| {
                record.grant.as_ref().and_then(|grant| {
                    (record.snapshot.authorization_state == RemoteAuthorizationState::Granted
                        && grant.issued_at_ms <= now_ms
                        && now_ms <= grant.expires_at_ms
                        && grant.granted_scopes.contains(&scope))
                    .then(|| SessionAuthorizationLease {
                        cancellation: record.cancellation.subscribe(),
                        expires_at_ms: grant.expires_at_ms,
                    })
                })
            });
            (expired, lease)
        };
        if expired {
            self.changed.notify_waiters();
        }
        lease
    }

    pub async fn install_verified_grant(
        &self,
        mut grant: VerifiedSessionGrant,
        installed_at_ms: u64,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        normalize_scopes(&mut grant.granted_scopes);
        let mut inner = self.inner.lock().await;
        let Some(record) = inner.records.get_mut(&grant.session_id) else {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "verified grant has no matching authorization request",
            ));
        };
        if record.snapshot.authorization_state != RemoteAuthorizationState::Authorizing {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "authorization is not ready to install a grant",
            ));
        }
        let scope_binding_is_valid = if record.snapshot.role == RemoteSessionRole::Controller {
            !grant.granted_scopes.is_empty()
                && grant.granted_scopes.iter().all(|scope| {
                    record.snapshot.requested_scopes.contains(scope)
                        && record.request.peer_permission_ceiling.contains(scope)
                        && record.request.machine_permission_ceiling.contains(scope)
                        && record.request.runtime_capabilities.contains(scope)
                })
        } else {
            grant.granted_scopes == record.snapshot.granted_scopes
        };
        let policy_binding_is_valid = record.snapshot.role == RemoteSessionRole::Controller
            || grant.policy_revision == record.snapshot.policy_revision.get();
        let Some(route_kind) = remote_route_kind(&record.request.transport_kind) else {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "verified grant uses an unsupported route constraint",
            ));
        };
        if grant.grant_id.trim().is_empty()
            || !scope_binding_is_valid
            || !policy_binding_is_valid
            || grant.issued_at_ms > installed_at_ms
            || installed_at_ms > grant.expires_at_ms
            || grant.route_constraint != record.request.transport_kind
            || grant
                .transport_fingerprint_sha256
                .iter()
                .all(|byte| *byte == 0)
        {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "verified grant no longer matches the approved authorization",
            ));
        }
        record.grant = Some(grant.clone());
        record.snapshot.granted_scopes = grant.granted_scopes.clone();
        record.snapshot.policy_revision = DecimalU64::new(grant.policy_revision);
        record.snapshot.authorization_state = RemoteAuthorizationState::Granted;
        record.snapshot.route_state = RemoteRouteState::Connecting;
        record.snapshot.route_kind = Some(route_kind);
        record.snapshot.presentation_state = RemotePresentationState::Connecting;
        record.snapshot.updated_at_ms = installed_at_ms;
        record.snapshot.authorization_expires_at_ms = Some(grant.expires_at_ms);
        let snapshot = record.snapshot.clone();
        push_event(
            &mut inner,
            installed_at_ms,
            grant.session_id,
            RemoteSessionEvent::AuthorizationChanged {
                state: RemoteAuthorizationState::Granted,
                failure: None,
            },
        );
        push_event(
            &mut inner,
            installed_at_ms,
            snapshot.session_id.clone(),
            RemoteSessionEvent::RouteChanged {
                state: RemoteRouteState::Connecting,
                route: Some(route_kind),
                failure: None,
            },
        );
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub async fn record_failure(
        &self,
        session_id: &SessionId,
        authorization_state: RemoteAuthorizationState,
        failure_value: RemoteFailure,
        failed_at_ms: u64,
    ) -> Option<RemoteSessionSnapshot> {
        let mut inner = self.inner.lock().await;
        let snapshot = transition_failure_locked(
            &mut inner,
            session_id,
            authorization_state,
            failure_value,
            failed_at_ms,
        )?;
        drop(inner);
        self.changed.notify_waiters();
        Some(snapshot)
    }

    pub async fn mark_streaming(
        &self,
        session_id: &SessionId,
        changed_at_ms: u64,
    ) -> Option<RemoteSessionSnapshot> {
        let mut inner = self.inner.lock().await;
        if expire_grant_locked(&mut inner, session_id, changed_at_ms) {
            let snapshot = inner
                .records
                .get(session_id)
                .map(|record| record.snapshot.clone());
            drop(inner);
            self.changed.notify_waiters();
            return snapshot;
        }
        let record = inner.records.get_mut(session_id)?;
        if record.snapshot.authorization_state != RemoteAuthorizationState::Granted {
            return None;
        }
        let route_kind = remote_route_kind(&record.request.transport_kind)?;
        if record.snapshot.route_state == RemoteRouteState::Connected
            && record.snapshot.route_kind == Some(route_kind)
            && record.snapshot.media_state == RemoteMediaState::Streaming
            && record.snapshot.presentation_state == RemotePresentationState::Streaming
        {
            return Some(record.snapshot.clone());
        }
        record.snapshot.route_state = RemoteRouteState::Connected;
        record.snapshot.route_kind = Some(route_kind);
        record.snapshot.media_state = RemoteMediaState::Streaming;
        record.snapshot.presentation_state = RemotePresentationState::Streaming;
        record.snapshot.updated_at_ms = changed_at_ms;
        let snapshot = record.snapshot.clone();
        push_event(
            &mut inner,
            changed_at_ms,
            session_id.clone(),
            RemoteSessionEvent::RouteChanged {
                state: RemoteRouteState::Connected,
                route: Some(route_kind),
                failure: None,
            },
        );
        push_event(
            &mut inner,
            changed_at_ms,
            session_id.clone(),
            RemoteSessionEvent::MediaChanged {
                state: RemoteMediaState::Streaming,
                failure: None,
            },
        );
        drop(inner);
        self.changed.notify_waiters();
        Some(snapshot)
    }

    pub async fn revoke_peer_authorizations(
        &self,
        peer_key_id: &str,
        revoked_at_ms: u64,
    ) -> Vec<SessionId> {
        let mut inner = self.inner.lock().await;
        let proven_peer_devices = inner
            .records
            .values()
            .filter(|record| {
                record.request.peer_key_id == peer_key_id
                    && !is_terminal_authorization_state(record.snapshot.authorization_state)
            })
            .map(|record| record.request.peer_device_id.clone())
            .collect::<HashSet<_>>();
        let session_ids = inner
            .records
            .iter()
            .filter(|(_, record)| {
                (record.request.peer_key_id == peer_key_id
                    || (record.request.peer_key_id
                        == pending_outgoing_peer_key_id(&record.request.peer_device_id)
                        && proven_peer_devices.contains(&record.request.peer_device_id)))
                    && !is_terminal_authorization_state(record.snapshot.authorization_state)
            })
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        let mut revoked = Vec::new();
        for session_id in session_ids {
            if transition_failure_locked(
                &mut inner,
                &session_id,
                RemoteAuthorizationState::Revoked,
                failure(
                    RemoteReasonCode::GrantRevoked,
                    "trusted device access was revoked",
                ),
                revoked_at_ms,
            )
            .is_some()
            {
                revoked.push(session_id);
            }
        }
        drop(inner);
        if !revoked.is_empty() {
            self.changed.notify_waiters();
        }
        revoked
    }

    pub async fn revoke_unattended_authorizations(&self, revoked_at_ms: u64) -> Vec<SessionId> {
        let mut inner = self.inner.lock().await;
        let session_ids = inner
            .records
            .iter()
            .filter(|(_, record)| {
                record.request.access_mode == RemoteAccessMode::Unattended
                    && !is_terminal_authorization_state(record.snapshot.authorization_state)
            })
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        let mut revoked = Vec::new();
        for session_id in session_ids {
            if transition_failure_locked(
                &mut inner,
                &session_id,
                RemoteAuthorizationState::PolicyChanged,
                failure(
                    RemoteReasonCode::GrantRevoked,
                    "unattended access material changed",
                ),
                revoked_at_ms,
            )
            .is_some()
            {
                revoked.push(session_id);
            }
        }
        drop(inner);
        if !revoked.is_empty() {
            self.changed.notify_waiters();
        }
        revoked
    }

    pub async fn verify_unattended(
        &self,
        session_id: &SessionId,
        transcript: &[u8],
        nonce: [u8; 16],
        access_epoch: u64,
        proof: &[u8],
        verified_at_ms: u64,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        // Hold policy and credential state through the authorization CAS so a
        // concurrent disable/rotation cannot authorize with stale material.
        let unattended = self.unattended.lock().await;
        let Some(credential) = unattended.credential.as_ref() else {
            return Err(failure(
                RemoteReasonCode::CredentialInvalid,
                "unattended access is disabled",
            ));
        };
        if access_epoch == 0 || access_epoch != unattended.access_epoch {
            return Err(failure(
                RemoteReasonCode::CredentialInvalid,
                "unattended access epoch is invalid",
            ));
        }
        let policy = &unattended.policy;
        let policy_revision = unattended.policy_revision;
        let mut inner = self.inner.lock().await;
        let Some(record) = inner.records.get_mut(session_id) else {
            return Err(failure(
                RemoteReasonCode::IdentityMismatch,
                "unattended authorization request was not found",
            ));
        };
        if record.snapshot.authorization_state
            != RemoteAuthorizationState::VerifyingUnattendedCredential
            || record.request.access_mode != RemoteAccessMode::Unattended
        {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "unattended authorization is no longer pending",
            ));
        }
        if verified_at_ms > record.request.expires_at_ms
            || policy
                .expires_at_ms
                .is_some_and(|expires_at_ms| verified_at_ms > expires_at_ms)
        {
            return Err(failure(
                RemoteReasonCode::AuthorizationTimeout,
                "unattended authorization request expired",
            ));
        }
        if nonce != record.request.request_nonce
            || (policy.trusted_devices_only
                && !policy.allowed_peer_key_ids.is_empty()
                && !policy
                    .allowed_peer_key_ids
                    .contains(&record.request.peer_key_id))
        {
            return Err(failure(
                RemoteReasonCode::CredentialInvalid,
                "unattended policy does not authorize this trusted peer",
            ));
        }
        if !credential.verify(transcript, nonce, proof) {
            return Err(failure(
                RemoteReasonCode::CredentialInvalid,
                "unattended credential proof is invalid",
            ));
        }
        let effective = effective_scopes(
            &record.request.requested_scopes,
            &record.request.peer_permission_ceiling,
            &record.request.machine_permission_ceiling,
            &policy.permission_ceiling,
            &record.request.runtime_capabilities,
        );
        if effective.is_empty() {
            return Err(failure(
                RemoteReasonCode::ScopeDenied,
                "unattended policy grants no requested runtime permission",
            ));
        }
        record.snapshot.authorization_state = RemoteAuthorizationState::Authorizing;
        record.snapshot.presentation_state = RemotePresentationState::Authenticating;
        record.snapshot.granted_scopes = effective;
        record.snapshot.policy_revision = DecimalU64::new(policy_revision);
        record.snapshot.failure = None;
        record.snapshot.updated_at_ms = verified_at_ms;
        let snapshot = record.snapshot.clone();
        push_event(
            &mut inner,
            verified_at_ms,
            session_id.clone(),
            RemoteSessionEvent::AuthorizationChanged {
                state: RemoteAuthorizationState::Authorizing,
                failure: None,
            },
        );
        drop(inner);
        drop(unattended);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub async fn wait_for_authorization_decision(
        &self,
        session_id: &SessionId,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (snapshot, deadline_ms) = {
                let inner = self.inner.lock().await;
                let Some(record) = inner.records.get(session_id) else {
                    return Err(failure(
                        RemoteReasonCode::IdentityMismatch,
                        "authorization request was not found",
                    ));
                };
                (record.snapshot.clone(), record.request.expires_at_ms)
            };
            match snapshot.authorization_state {
                RemoteAuthorizationState::Authorizing | RemoteAuthorizationState::Granted => {
                    return Ok(snapshot)
                }
                RemoteAuthorizationState::Denied
                | RemoteAuthorizationState::Expired
                | RemoteAuthorizationState::Revoked
                | RemoteAuthorizationState::LockedOut
                | RemoteAuthorizationState::PolicyChanged => {
                    return Err(snapshot.failure.unwrap_or_else(|| {
                        failure(
                            RemoteReasonCode::PolicyChanged,
                            "authorization request reached a terminal state",
                        )
                    }));
                }
                _ => {}
            }
            let now_ms = unix_time_ms();
            if now_ms > deadline_ms {
                let expired = self.expire_pending(session_id, now_ms).await;
                return Err(expired
                    .and_then(|snapshot| snapshot.failure)
                    .unwrap_or_else(|| {
                        failure(
                            RemoteReasonCode::AuthorizationTimeout,
                            "authorization request timed out",
                        )
                    }));
            }
            let remaining = Duration::from_millis(deadline_ms.saturating_sub(now_ms).max(1));
            if timeout(remaining, &mut notified).await.is_err() {
                let expired_at_ms = unix_time_ms().max(deadline_ms.saturating_add(1));
                let expired = self.expire_pending(session_id, expired_at_ms).await;
                return Err(expired
                    .and_then(|snapshot| snapshot.failure)
                    .unwrap_or_else(|| {
                        failure(
                            RemoteReasonCode::AuthorizationTimeout,
                            "authorization request timed out",
                        )
                    }));
            }
        }
    }

    #[allow(dead_code)]
    pub async fn respond_to_consent(
        &self,
        response: ConsentResponse,
        decided_at_ms: u64,
    ) -> Result<RemoteSessionSnapshot, RemoteFailure> {
        match self
            .respond_to_consent_with_audit(response, decided_at_ms, |_, _| true)
            .await
        {
            Ok(snapshot) => Ok(snapshot),
            Err(ConsentResolutionError::Rejected(failure)) => Err(failure),
            Err(ConsentResolutionError::AuditUnavailable) => Err(failure(
                RemoteReasonCode::PolicyChanged,
                "consent decision could not be durably audited",
            )),
        }
    }

    pub async fn respond_to_consent_with_audit<F>(
        &self,
        mut response: ConsentResponse,
        decided_at_ms: u64,
        audit: F,
    ) -> Result<RemoteSessionSnapshot, ConsentResolutionError>
    where
        F: FnOnce(&RemoteSessionSnapshot, &ConsentResponse) -> bool,
    {
        normalize_scopes(&mut response.approved_scopes);
        let mut inner = self.inner.lock().await;
        let Some(record) = inner.records.get_mut(&response.session_id) else {
            return Err(ConsentResolutionError::Rejected(failure(
                RemoteReasonCode::ConsentDenied,
                "remote consent request was not found",
            )));
        };
        if record.snapshot.authorization_state != RemoteAuthorizationState::AwaitingLocalConsent {
            return Err(ConsentResolutionError::Rejected(
                record.snapshot.failure.clone().unwrap_or_else(|| {
                    failure(
                        RemoteReasonCode::PolicyChanged,
                        "remote consent request is no longer pending",
                    )
                }),
            ));
        }
        if response.expected_policy_revision != record.snapshot.policy_revision {
            return Err(ConsentResolutionError::Rejected(failure(
                RemoteReasonCode::PolicyChanged,
                "remote consent policy revision changed",
            )));
        }
        if decided_at_ms > record.request.expires_at_ms {
            let expired = failure(
                RemoteReasonCode::AuthorizationTimeout,
                "remote authorization request expired before consent",
            );
            record.snapshot.authorization_state = RemoteAuthorizationState::Expired;
            record.snapshot.presentation_state = RemotePresentationState::Denied;
            record.snapshot.failure = Some(expired.clone());
            record.snapshot.updated_at_ms = decided_at_ms;
            let session_id = response.session_id.clone();
            push_event(
                &mut inner,
                decided_at_ms,
                session_id,
                RemoteSessionEvent::AuthorizationChanged {
                    state: RemoteAuthorizationState::Expired,
                    failure: Some(expired.clone()),
                },
            );
            drop(inner);
            self.changed.notify_waiters();
            return Err(ConsentResolutionError::Rejected(expired));
        }

        let effective = match response.decision {
            ConsentDecision::Deny => {
                if !response.approved_scopes.is_empty() {
                    return Err(ConsentResolutionError::Rejected(failure(
                        RemoteReasonCode::ScopeDenied,
                        "denial response cannot include approved scopes",
                    )));
                }
                None
            }
            ConsentDecision::Approve => {
                if response
                    .approved_scopes
                    .iter()
                    .any(|scope| !record.request.requested_scopes.contains(scope))
                {
                    return Err(ConsentResolutionError::Rejected(failure(
                        RemoteReasonCode::ScopeDenied,
                        "consent attempted to add an unrequested permission scope",
                    )));
                }
                let effective = effective_scopes(
                    &record.request.requested_scopes,
                    &record.request.peer_permission_ceiling,
                    &record.request.machine_permission_ceiling,
                    &response.approved_scopes,
                    &record.request.runtime_capabilities,
                );
                if effective.is_empty() {
                    return Err(ConsentResolutionError::Rejected(failure(
                        RemoteReasonCode::ScopeDenied,
                        "no requested permission scope is allowed by current policy",
                    )));
                }
                Some(effective)
            }
        };

        if !audit(&record.snapshot, &response) {
            return Err(ConsentResolutionError::AuditUnavailable);
        }

        match response.decision {
            ConsentDecision::Deny => {
                let denied = failure(
                    RemoteReasonCode::ConsentDenied,
                    "the local user denied this remote session",
                );
                record.snapshot.authorization_state = RemoteAuthorizationState::Denied;
                record.snapshot.presentation_state = RemotePresentationState::Denied;
                record.snapshot.failure = Some(denied);
                record.snapshot.updated_at_ms = decided_at_ms;
            }
            ConsentDecision::Approve => {
                record.snapshot.authorization_state = RemoteAuthorizationState::Authorizing;
                record.snapshot.presentation_state = RemotePresentationState::Authenticating;
                record.snapshot.granted_scopes = effective.expect("approved consent has scopes");
                record.snapshot.failure = None;
                record.snapshot.updated_at_ms = decided_at_ms;
            }
        }
        let snapshot = record.snapshot.clone();
        push_event(
            &mut inner,
            decided_at_ms,
            response.session_id,
            RemoteSessionEvent::ConsentResolved {
                decision: response.decision,
                approved_scopes: response.approved_scopes,
            },
        );
        drop(inner);
        self.changed.notify_waiters();
        Ok(snapshot)
    }

    pub async fn expire_pending(
        &self,
        session_id: &SessionId,
        expired_at_ms: u64,
    ) -> Option<RemoteSessionSnapshot> {
        let mut inner = self.inner.lock().await;
        let record = inner.records.get_mut(session_id)?;
        if record.snapshot.authorization_state != RemoteAuthorizationState::AwaitingLocalConsent
            || expired_at_ms <= record.request.expires_at_ms
        {
            return None;
        }
        let expired = failure(
            RemoteReasonCode::AuthorizationTimeout,
            "remote authorization request timed out",
        );
        record.snapshot.authorization_state = RemoteAuthorizationState::Expired;
        record.snapshot.presentation_state = RemotePresentationState::Denied;
        record.snapshot.failure = Some(expired.clone());
        record.snapshot.granted_scopes.clear();
        record.snapshot.updated_at_ms = expired_at_ms;
        let snapshot = record.snapshot.clone();
        push_event(
            &mut inner,
            expired_at_ms,
            session_id.clone(),
            RemoteSessionEvent::AuthorizationChanged {
                state: RemoteAuthorizationState::Expired,
                failure: Some(expired),
            },
        );
        drop(inner);
        self.changed.notify_waiters();
        Some(snapshot)
    }

    pub async fn subscribe(
        &self,
        query: SessionEventSubscriptionQuery,
    ) -> Result<SessionEventSubscription, RemoteFailure> {
        if query.limit == 0 || query.limit > 256 || query.wait_timeout_ms > 30_000 {
            return Err(failure(
                RemoteReasonCode::PolicyChanged,
                "session event subscription bounds are invalid",
            ));
        }
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(u64::from(query.wait_timeout_ms));
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let page = {
                let inner = self.inner.lock().await;
                build_subscription(&inner, &query)
            };
            if !page.events.is_empty()
                || !page.pending_sessions.is_empty()
                || page.cursor_state == RemoteCursorState::ResetRequired
                || query.wait_timeout_ms == 0
            {
                return Ok(page);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline || timeout(deadline - now, &mut notified).await.is_err() {
                let inner = self.inner.lock().await;
                return Ok(build_subscription(&inner, &query));
            }
        }
    }
}

fn build_subscription(
    inner: &AuthorizationRegistryInner,
    query: &SessionEventSubscriptionQuery,
) -> SessionEventSubscription {
    let after = query.after_sequence.map(DecimalU64::get).unwrap_or(0);
    let pending_sessions = || {
        let mut pending = inner
            .records
            .values()
            .filter(|record| {
                record.snapshot.role == RemoteSessionRole::Agent
                    && record.snapshot.access_mode == RemoteAccessMode::Attended
                    && record.snapshot.authorization_state
                        == RemoteAuthorizationState::AwaitingLocalConsent
                    && query
                        .session_id
                        .as_ref()
                        .is_none_or(|session_id| session_id == &record.snapshot.session_id)
            })
            .map(|record| record.snapshot.clone())
            .collect::<Vec<_>>();
        pending.sort_by_key(|snapshot| (snapshot.created_at_ms, snapshot.session_id.0.clone()));
        pending
    };
    let oldest = inner.events.front().map(|event| event.sequence.get());
    if let Some(oldest) = oldest {
        if query.after_sequence.is_some() && after.saturating_add(1) < oldest {
            return SessionEventSubscription {
                events: Vec::new(),
                pending_sessions: pending_sessions(),
                next_after_sequence: Some(DecimalU64::new(inner.next_sequence)),
                cursor_state: RemoteCursorState::ResetRequired,
                has_more: false,
                poll_after_ms: 0,
            };
        }
    }
    let matching: Vec<_> = inner
        .events
        .iter()
        .filter(|event| {
            event.sequence.get() > after
                && query
                    .session_id
                    .as_ref()
                    .is_none_or(|session_id| &event.session_id == session_id)
        })
        .collect();
    let limit = query.limit as usize;
    let events = matching
        .iter()
        .take(limit)
        .map(|event| (*event).clone())
        .collect::<Vec<_>>();
    let has_more = matching.len() > events.len();
    let next_after_sequence = events
        .last()
        .map(|event| event.sequence)
        .or_else(|| Some(DecimalU64::new(inner.next_sequence)).filter(|value| value.get() > 0))
        .or(query.after_sequence);
    SessionEventSubscription {
        events,
        pending_sessions: if query.after_sequence.is_none() {
            pending_sessions()
        } else {
            Vec::new()
        },
        next_after_sequence,
        cursor_state: RemoteCursorState::Current,
        has_more,
        poll_after_ms: if has_more { 0 } else { 250 },
    }
}

fn next_authorization_instance(
    inner: &mut AuthorizationRegistryInner,
) -> Result<u64, RemoteFailure> {
    let next = inner.next_instance_id.checked_add(1).ok_or_else(|| {
        failure(
            RemoteReasonCode::CredentialLocked,
            "authorization instance capacity is exhausted",
        )
    })?;
    inner.next_instance_id = next;
    Ok(next)
}

#[cfg(any(windows, test))]
fn prune_outgoing_birth_observers(inner: &mut AuthorizationRegistryInner) {
    inner.outgoing_birth_observers.retain(|_, group| {
        group.retain(|observer| observer.strong_count() > 0);
        !group.is_empty()
    });
}

fn effective_scopes(
    requested: &[RemotePermissionScope],
    peer_ceiling: &[RemotePermissionScope],
    machine_ceiling: &[RemotePermissionScope],
    local_approval: &[RemotePermissionScope],
    runtime_capabilities: &[RemotePermissionScope],
) -> Vec<RemotePermissionScope> {
    requested
        .iter()
        .copied()
        .filter(|scope| {
            peer_ceiling.contains(scope)
                && machine_ceiling.contains(scope)
                && local_approval.contains(scope)
                && runtime_capabilities.contains(scope)
        })
        .collect()
}

fn is_terminal_authorization_state(state: RemoteAuthorizationState) -> bool {
    matches!(
        state,
        RemoteAuthorizationState::Denied
            | RemoteAuthorizationState::Expired
            | RemoteAuthorizationState::Revoked
            | RemoteAuthorizationState::LockedOut
            | RemoteAuthorizationState::PolicyChanged
    )
}

fn is_pending_incoming_record(record: &AuthorizationRecord) -> bool {
    record.snapshot.role == RemoteSessionRole::Agent
        && matches!(
            record.snapshot.authorization_state,
            RemoteAuthorizationState::AwaitingLocalConsent
                | RemoteAuthorizationState::VerifyingUnattendedCredential
                | RemoteAuthorizationState::Authorizing
        )
}

fn prune_authorization_records(inner: &mut AuthorizationRegistryInner, now_ms: u64) {
    inner.records.retain(|_, record| {
        !is_terminal_authorization_state(record.snapshot.authorization_state)
            || now_ms.saturating_sub(record.snapshot.updated_at_ms) <= TERMINAL_RECORD_RETENTION_MS
    });

    while inner.records.len() >= AUTHORIZATION_RECORD_LIMIT {
        let oldest_terminal = inner
            .records
            .iter()
            .filter(|(_, record)| {
                is_terminal_authorization_state(record.snapshot.authorization_state)
            })
            .min_by_key(|(_, record)| record.snapshot.updated_at_ms)
            .map(|(session_id, _)| session_id.clone());
        let Some(session_id) = oldest_terminal else {
            break;
        };
        inner.records.remove(&session_id);
    }
}

fn expire_grant_locked(
    inner: &mut AuthorizationRegistryInner,
    session_id: &SessionId,
    now_ms: u64,
) -> bool {
    let should_expire = inner.records.get(session_id).is_some_and(|record| {
        record.snapshot.authorization_state == RemoteAuthorizationState::Granted
            && record
                .grant
                .as_ref()
                .is_some_and(|grant| now_ms > grant.expires_at_ms)
    });
    if !should_expire {
        return false;
    }
    transition_failure_locked(
        inner,
        session_id,
        RemoteAuthorizationState::Expired,
        failure(
            RemoteReasonCode::GrantExpired,
            "remote session grant expired",
        ),
        now_ms,
    )
    .is_some()
}

fn transition_failure_locked(
    inner: &mut AuthorizationRegistryInner,
    session_id: &SessionId,
    authorization_state: RemoteAuthorizationState,
    failure_value: RemoteFailure,
    failed_at_ms: u64,
) -> Option<RemoteSessionSnapshot> {
    let record = inner.records.get_mut(session_id)?;
    if record.snapshot.authorization_state == authorization_state
        && record.snapshot.failure.as_ref().map(|failure| failure.code) == Some(failure_value.code)
    {
        return Some(record.snapshot.clone());
    }
    if is_terminal_authorization_state(record.snapshot.authorization_state) {
        return None;
    }

    record.cancellation.send_replace(true);
    let had_active_route = record.grant.is_some()
        || !matches!(
            record.snapshot.route_state,
            RemoteRouteState::Idle | RemoteRouteState::Closed
        );
    let had_active_media = !matches!(
        record.snapshot.media_state,
        RemoteMediaState::Idle | RemoteMediaState::Stopped
    );
    let clean_shutdown = matches!(
        failure_value.code,
        RemoteReasonCode::GrantExpired
            | RemoteReasonCode::GrantRevoked
            | RemoteReasonCode::TrustRequired
            | RemoteReasonCode::PolicyChanged
    );
    let authorization_denial = matches!(
        failure_value.code,
        RemoteReasonCode::IdentityMismatch
            | RemoteReasonCode::TrustRequired
            | RemoteReasonCode::ConsentDenied
            | RemoteReasonCode::CredentialInvalid
            | RemoteReasonCode::CredentialLocked
            | RemoteReasonCode::AuthorizationTimeout
            | RemoteReasonCode::GrantExpired
            | RemoteReasonCode::GrantRevoked
            | RemoteReasonCode::PolicyChanged
            | RemoteReasonCode::ReplayDetected
            | RemoteReasonCode::ScopeDenied
            | RemoteReasonCode::ProtocolDowngradeBlocked
    );
    let route_failure = matches!(
        failure_value.code,
        RemoteReasonCode::LanUnreachable
            | RemoteReasonCode::IceDirectFailed
            | RemoteReasonCode::TurnAllocationFailed
            | RemoteReasonCode::RouteLost
            | RemoteReasonCode::RouteMigrationTimeout
    );
    let media_failure = matches!(
        failure_value.code,
        RemoteReasonCode::EncoderUnavailable
            | RemoteReasonCode::DecoderUnavailable
            | RemoteReasonCode::CaptureSourceLost
            | RemoteReasonCode::ProfileDowngraded
            | RemoteReasonCode::CongestionDownshift
            | RemoteReasonCode::RenderBudgetExceeded
    );
    record.snapshot.authorization_state = authorization_state;
    record.snapshot.presentation_state = if had_active_route || had_active_media {
        if clean_shutdown {
            RemotePresentationState::Closed
        } else {
            RemotePresentationState::Failed
        }
    } else if !authorization_denial {
        RemotePresentationState::Failed
    } else {
        RemotePresentationState::Denied
    };
    record.snapshot.failure = Some(failure_value.clone());
    record.snapshot.granted_scopes.clear();
    record.snapshot.updated_at_ms = failed_at_ms;
    record.grant = None;
    if had_active_route || route_failure {
        record.snapshot.route_state = if clean_shutdown {
            RemoteRouteState::Closed
        } else {
            RemoteRouteState::Failed
        };
    }
    if had_active_media || media_failure {
        record.snapshot.media_state = if clean_shutdown {
            RemoteMediaState::Stopped
        } else {
            RemoteMediaState::Failed
        };
    }
    let snapshot = record.snapshot.clone();
    push_event(
        inner,
        failed_at_ms,
        session_id.clone(),
        RemoteSessionEvent::AuthorizationChanged {
            state: authorization_state,
            failure: Some(failure_value.clone()),
        },
    );
    if had_active_route || route_failure {
        push_event(
            inner,
            failed_at_ms,
            session_id.clone(),
            RemoteSessionEvent::RouteChanged {
                state: snapshot.route_state,
                route: snapshot.route_kind,
                failure: Some(failure_value.clone()),
            },
        );
    }
    if had_active_media || media_failure {
        push_event(
            inner,
            failed_at_ms,
            session_id.clone(),
            RemoteSessionEvent::MediaChanged {
                state: snapshot.media_state,
                failure: Some(failure_value.clone()),
            },
        );
    }
    if had_active_route
        || had_active_media
        || route_failure
        || media_failure
        || snapshot.presentation_state == RemotePresentationState::Failed
    {
        push_event(
            inner,
            failed_at_ms,
            session_id.clone(),
            RemoteSessionEvent::SessionClosed {
                failure: Some(failure_value),
            },
        );
    }
    Some(snapshot)
}

fn validate_verified_request(
    request: &VerifiedIncomingAuthorizationRequest,
) -> Result<(), RemoteFailure> {
    if request.session_id.0.trim().is_empty()
        || request.peer_device_id.0.trim().is_empty()
        || request.peer_key_id.trim().is_empty()
        || request.transport_kind.trim().is_empty()
        || request.peer_key_epoch == 0
    {
        return Err(failure(
            RemoteReasonCode::IdentityMismatch,
            "verified remote request has invalid identity binding",
        ));
    }
    if request.request_nonce.iter().all(|byte| *byte == 0) {
        return Err(failure(
            RemoteReasonCode::ReplayDetected,
            "verified remote request has an invalid nonce",
        ));
    }
    if request.created_at_ms >= request.expires_at_ms {
        return Err(failure(
            RemoteReasonCode::AuthorizationTimeout,
            "remote authorization request has expired",
        ));
    }
    if request.requested_scopes.is_empty() {
        return Err(failure(
            RemoteReasonCode::ScopeDenied,
            "remote request contains no permission scopes",
        ));
    }
    Ok(())
}

fn normalize_scopes(scopes: &mut Vec<RemotePermissionScope>) {
    scopes.sort_unstable();
    scopes.dedup();
}

fn remote_route_kind(transport_kind: &str) -> Option<RemoteRouteKind> {
    match transport_kind {
        "quic" | "lan_quic" => Some(RemoteRouteKind::LanQuic),
        "webrtc" | "webrtc_direct" => Some(RemoteRouteKind::WebRtcDirect),
        "webrtc_relay" => Some(RemoteRouteKind::WebRtcRelay),
        _ => None,
    }
}

fn parse_sha256_identifier(value: &str) -> Option<[u8; 32]> {
    let digest = value.strip_prefix("sha256:")?;
    if digest.len() != 64 || !digest.is_ascii() {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in digest.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(bytes)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[allow(dead_code)]
fn validate_unattended_policy(
    policy: &UnattendedAccessPolicy,
    now_ms: u64,
) -> Result<(), RemoteFailure> {
    if !policy.trusted_devices_only
        || policy.permission_ceiling.is_empty()
        || policy
            .expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    {
        return Err(failure(
            RemoteReasonCode::ScopeDenied,
            "unattended access requires trusted devices, non-empty scopes, and a future expiry",
        ));
    }
    Ok(())
}

fn unattended_snapshot(
    unattended: &UnattendedAuthorizationState,
    updated_at_ms: u64,
) -> UnattendedAccessSnapshot {
    UnattendedAccessSnapshot {
        enabled: unattended.credential.is_some(),
        policy_revision: DecimalU64::new(unattended.policy_revision),
        access_epoch: DecimalU64::new(unattended.access_epoch),
        policy: unattended.policy.clone(),
        locked_until_ms: None,
        updated_at_ms,
    }
}

fn failure(code: RemoteReasonCode, message: impl Into<String>) -> RemoteFailure {
    RemoteFailure {
        code,
        message: message.into(),
        suggested_action: None,
    }
}

fn project_verified_route_evidence(
    record: &AuthorizationRecord,
    observed_at_ms: u64,
) -> Result<RouteEvidence, RemoteFailure> {
    if record.snapshot.authorization_state != RemoteAuthorizationState::Granted {
        return Err(record.snapshot.failure.clone().unwrap_or_else(|| {
            failure(
                RemoteReasonCode::PolicyChanged,
                "route evidence requires an active verified session grant",
            )
        }));
    }
    let Some(grant) = record.grant.as_ref() else {
        return Err(failure(
            RemoteReasonCode::PolicyChanged,
            "route evidence has no verified session grant",
        ));
    };
    if record.request.session_id != record.snapshot.session_id
        || record.request.peer_device_id != record.snapshot.peer_device_id
        || record.request.peer_key_id != record.snapshot.peer_key_id
        || grant.session_id != record.snapshot.session_id
        || grant.issued_at_ms > observed_at_ms
        || observed_at_ms > grant.expires_at_ms
        || grant.policy_revision != record.snapshot.policy_revision.get()
        || grant.granted_scopes != record.snapshot.granted_scopes
        || grant.route_constraint != record.request.transport_kind
        || grant.route_constraint != "quic"
        || grant
            .transport_fingerprint_sha256
            .iter()
            .all(|byte| *byte == 0)
    {
        return Err(failure(
            RemoteReasonCode::PolicyChanged,
            "route evidence grant no longer matches the active session",
        ));
    }
    let Some(peer_public_key) = record.peer_public_key else {
        return Err(failure(
            RemoteReasonCode::IdentityMismatch,
            "route evidence peer key has not been authenticated",
        ));
    };
    if public_key_id(&peer_public_key) != record.request.peer_key_id {
        return Err(failure(
            RemoteReasonCode::IdentityMismatch,
            "route evidence peer key binding is invalid",
        ));
    }
    if record.snapshot.route_kind != Some(RemoteRouteKind::LanQuic) {
        return Err(failure(
            RemoteReasonCode::PolicyChanged,
            "route evidence selected route does not match the verified LAN grant",
        ));
    }

    let candidate_state = match record.snapshot.route_state {
        RemoteRouteState::Idle => RouteCandidateState::NotTried,
        RemoteRouteState::Gathering
        | RemoteRouteState::Connecting
        | RemoteRouteState::Migrating
        | RemoteRouteState::Reconnecting => RouteCandidateState::Connecting,
        RemoteRouteState::Connected => RouteCandidateState::Connected,
        RemoteRouteState::Failed | RemoteRouteState::Closed => RouteCandidateState::Failed,
    };
    let completed_at_ms = matches!(
        candidate_state,
        RouteCandidateState::Connected | RouteCandidateState::Failed
    )
    .then_some(record.snapshot.updated_at_ms.max(grant.issued_at_ms));
    let candidate_failure = (candidate_state == RouteCandidateState::Failed)
        .then(|| record.snapshot.failure.clone())
        .flatten();
    Ok(RouteEvidence {
        session_id: record.snapshot.session_id.clone(),
        route_state: record.snapshot.route_state,
        selected_route: record.snapshot.route_kind,
        policy_revision: record.snapshot.policy_revision,
        transport_fingerprint_sha256: Some(format!(
            "sha256:{}",
            hex_bytes(&grant.transport_fingerprint_sha256)
        )),
        candidates: vec![RouteCandidateEvidence {
            route: RemoteRouteKind::LanQuic,
            state: candidate_state,
            started_at_ms: (candidate_state != RouteCandidateState::NotTried)
                .then_some(grant.issued_at_ms),
            completed_at_ms,
            round_trip_ms: None,
            failure: candidate_failure,
        }],
        observed_at_ms,
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn push_event(
    inner: &mut AuthorizationRegistryInner,
    timestamp_ms: u64,
    session_id: SessionId,
    event: RemoteSessionEvent,
) {
    inner.next_sequence = inner.next_sequence.saturating_add(1).max(1);
    inner.events.push_back(RemoteSessionEventEnvelope {
        sequence: DecimalU64::new(inner.next_sequence),
        timestamp_ms,
        session_id,
        event,
    });
    while inner.events.len() > EVENT_HISTORY_LIMIT {
        inner.events.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATED_AT_MS: u64 = 1_000;
    const EXPIRES_AT_MS: u64 = 20_000;

    #[tokio::test]
    async fn outgoing_birth_receipts_capture_only_the_first_real_insert() {
        let registry = SessionAuthorizationRegistry::default();
        let request = control_request("observed-outgoing", &[7; 32]);
        let first = registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .unwrap();
        let concurrent = registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .unwrap();
        assert_eq!(first.instance_id(), None);
        registry.begin_outgoing(request.clone()).await.unwrap();
        assert_eq!(first.instance_id(), Some(1));
        assert_eq!(concurrent.instance_id(), Some(1));
        assert!(registry.begin_outgoing(request.clone()).await.is_err());
        registry
            .record_failure(
                &request.session_id,
                RemoteAuthorizationState::Revoked,
                failure(RemoteReasonCode::GrantRevoked, "fixture cleanup"),
                CREATED_AT_MS,
            )
            .await;
        prune_authorization_records(
            &mut *registry.inner.lock().await,
            CREATED_AT_MS + TERMINAL_RECORD_RETENTION_MS + 1,
        );
        let later = registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .unwrap();
        registry.begin_outgoing(request).await.unwrap();
        assert_eq!(later.instance_id(), Some(2));
        assert_eq!(first.instance_id(), Some(1));
        assert_eq!(concurrent.instance_id(), Some(1));
    }

    #[tokio::test]
    async fn outgoing_birth_observation_cannot_adopt_an_existing_or_retained_terminal_record() {
        let registry = SessionAuthorizationRegistry::default();
        let request = control_request("existing-outgoing", &[7; 32]);
        registry.begin_outgoing(request.clone()).await.unwrap();
        assert!(registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .is_none());
        registry
            .record_failure(
                &request.session_id,
                RemoteAuthorizationState::Revoked,
                failure(RemoteReasonCode::GrantRevoked, "fixture cleanup"),
                CREATED_AT_MS,
            )
            .await;
        assert!(registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn incoming_birth_consumes_the_empty_slot_without_filling_outgoing_receipts() {
        let registry = SessionAuthorizationRegistry::default();
        let request = control_request("incoming-consumed-slot", &[7; 32]);
        let receipt = registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .unwrap();
        registry
            .begin_verified_incoming(request.clone())
            .await
            .unwrap();
        assert!(registry
            .inner
            .lock()
            .await
            .outgoing_birth_observers
            .is_empty());
        assert_eq!(receipt.instance_id(), None);
        registry
            .record_failure(
                &request.session_id,
                RemoteAuthorizationState::Revoked,
                failure(RemoteReasonCode::GrantRevoked, "fixture cleanup"),
                CREATED_AT_MS,
            )
            .await;
        prune_authorization_records(
            &mut *registry.inner.lock().await,
            CREATED_AT_MS + TERMINAL_RECORD_RETENTION_MS + 1,
        );
        let fresh = registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .unwrap();
        registry.begin_outgoing(request).await.unwrap();
        assert_eq!(
            receipt.instance_id(),
            None,
            "the earlier incoming lifetime can never become this request's outgoing proof"
        );
        assert_eq!(fresh.instance_id(), Some(2));
    }

    #[tokio::test]
    async fn outgoing_birth_observers_bound_each_slot_without_evicting_live_receipts() {
        let registry = SessionAuthorizationRegistry::default();
        let request = control_request("bounded-outgoing-slot", &[7; 32]);
        let mut live = Vec::new();
        for _ in 0..MAX_OUTGOING_BIRTH_OBSERVERS_PER_SESSION {
            live.push(
                registry
                    .observe_outgoing_birth(&request.session_id)
                    .await
                    .unwrap(),
            );
        }
        assert!(registry
            .observe_outgoing_birth(&request.session_id)
            .await
            .is_none());
        live.remove(0);
        live.push(
            registry
                .observe_outgoing_birth(&request.session_id)
                .await
                .unwrap(),
        );
        registry.begin_outgoing(request).await.unwrap();
        assert!(live.iter().all(|receipt| receipt.instance_id() == Some(1)));
    }

    #[tokio::test]
    async fn outgoing_birth_observers_bound_live_keys_and_reclaim_dead_weak_references() {
        let registry = SessionAuthorizationRegistry::default();
        let mut live = Vec::new();
        for i in 0..MAX_OUTGOING_BIRTH_OBSERVERS {
            live.push(
                registry
                    .observe_outgoing_birth(&SessionId(format!("observer-{i}")))
                    .await
                    .unwrap(),
            );
        }
        assert!(registry
            .observe_outgoing_birth(&SessionId("over-capacity".into()))
            .await
            .is_none());
        assert_eq!(
            registry.inner.lock().await.outgoing_birth_observers.len(),
            MAX_OUTGOING_BIRTH_OBSERVERS
        );
        live.remove(0);
        assert!(registry
            .observe_outgoing_birth(&SessionId("reclaimed-slot".into()))
            .await
            .is_some());
        assert!(live.iter().all(|receipt| receipt.instance_id().is_none()));
    }

    #[tokio::test]
    async fn outgoing_birth_insert_reclaims_dead_observers_for_other_slots() {
        let registry = SessionAuthorizationRegistry::default();
        drop(
            registry
                .observe_outgoing_birth(&SessionId("dead-observer".into()))
                .await
                .unwrap(),
        );
        let still_live = registry
            .observe_outgoing_birth(&SessionId("live-observer".into()))
            .await
            .unwrap();
        registry
            .begin_outgoing(control_request("actual-birth", &[7; 32]))
            .await
            .unwrap();
        let inner = registry.inner.lock().await;
        assert_eq!(inner.outgoing_birth_observers.len(), 1);
        assert!(inner
            .outgoing_birth_observers
            .contains_key(&SessionId("live-observer".into())));
        assert_eq!(still_live.instance_id(), None);
    }

    #[tokio::test]
    async fn audit_instance_distinguishes_reuse_with_a_frozen_clock() {
        let registry = SessionAuthorizationRegistry::default();
        let request = control_request("audit-frozen-clock", &[7; 32]);
        let first = registry.begin_outgoing(request.clone()).await.unwrap();
        let first_instance = registry
            .product_audit_binding(&request.session_id)
            .await
            .unwrap()
            .0;
        registry
            .record_failure(
                &request.session_id,
                RemoteAuthorizationState::Revoked,
                failure(RemoteReasonCode::GrantRevoked, "fixture cleanup"),
                CREATED_AT_MS,
            )
            .await;
        prune_authorization_records(
            &mut *registry.inner.lock().await,
            CREATED_AT_MS + TERMINAL_RECORD_RETENTION_MS + 1,
        );
        let second = registry.begin_outgoing(request.clone()).await.unwrap();
        assert_eq!(
            first, second,
            "a frozen clock can produce identical public snapshots"
        );
        assert!(
            registry
                .product_audit_binding(&request.session_id)
                .await
                .unwrap()
                .0
                > first_instance
        );
    }

    #[tokio::test]
    async fn audit_instance_is_shared_by_both_birth_paths_and_changes_only_on_birth() {
        let registry = SessionAuthorizationRegistry::default();
        let outgoing = control_request("audit-outgoing", &[7; 32]);
        let incoming = control_request("audit-incoming", &[7; 32]);
        registry.begin_outgoing(outgoing.clone()).await.unwrap();
        registry
            .begin_verified_incoming(incoming.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .product_audit_binding(&outgoing.session_id)
                .await
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            registry
                .product_audit_binding(&incoming.session_id)
                .await
                .unwrap()
                .0,
            2
        );
        assert!(registry.begin_outgoing(outgoing.clone()).await.is_err());
        assert_eq!(
            registry
                .product_audit_binding(&outgoing.session_id)
                .await
                .unwrap()
                .0,
            1
        );
        assert_eq!(registry.inner.lock().await.next_instance_id, 2);
    }

    #[tokio::test]
    async fn audit_instance_exhaustion_rejects_both_birth_paths_without_mutation() {
        for incoming in [false, true] {
            let registry = SessionAuthorizationRegistry::default();
            registry.inner.lock().await.next_instance_id = u64::MAX;
            let request = control_request("audit-exhausted", &[7; 32]);
            let result = if incoming {
                registry.begin_verified_incoming(request).await
            } else {
                registry.begin_outgoing(request).await
            };
            assert!(result.is_err());
            let inner = registry.inner.lock().await;
            assert!(inner.records.is_empty());
            assert!(inner.events.is_empty());
            assert_eq!(inner.next_instance_id, u64::MAX);
        }
    }

    fn control_request(
        session_id: &str,
        peer_public_key: &[u8; 32],
    ) -> VerifiedIncomingAuthorizationRequest {
        let peer_key_id = public_key_id(peer_public_key);
        VerifiedIncomingAuthorizationRequest {
            session_id: SessionId(session_id.to_string()),
            peer_device_id: DeviceId("controller-device".to_string()),
            peer_key_id,
            peer_key_epoch: 1,
            access_mode: RemoteAccessMode::Attended,
            requested_scopes: vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ],
            peer_permission_ceiling: vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ],
            machine_permission_ceiling: vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ],
            runtime_capabilities: vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ],
            transport_kind: "quic".to_string(),
            request_nonce: [1; 16],
            created_at_ms: CREATED_AT_MS,
            expires_at_ms: EXPIRES_AT_MS,
        }
    }

    fn control_grant(session_id: &SessionId) -> VerifiedSessionGrant {
        VerifiedSessionGrant {
            grant_id: format!("sha256:{}", "ab".repeat(32)),
            session_id: session_id.clone(),
            granted_scopes: vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ],
            issued_at_ms: CREATED_AT_MS + 1,
            expires_at_ms: EXPIRES_AT_MS,
            policy_revision: 7,
            route_constraint: "quic".to_string(),
            transport_fingerprint_sha256: [9; 32],
        }
    }

    fn screen_and_pointer_request(session_id: &str) -> VerifiedIncomingAuthorizationRequest {
        let mut request = control_request(session_id, &[7; 32]);
        let scopes = vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
        ];
        request.requested_scopes = scopes.clone();
        request.peer_permission_ceiling = scopes.clone();
        request.machine_permission_ceiling = scopes.clone();
        request.runtime_capabilities = scopes;
        request
    }

    async fn assert_controller_grant_exceeding_ceiling_is_rejected(
        request: VerifiedIncomingAuthorizationRequest,
    ) {
        let registry = SessionAuthorizationRegistry::default();
        let session_id = request.session_id.clone();
        let mut grant = control_grant(&session_id);
        grant.granted_scopes = request.requested_scopes.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization with a narrower effective ceiling");
        let before = registry
            .bind_authenticated_peer_key(&session_id, &[7; 32], CREATED_AT_MS + 1)
            .await
            .expect("bind the authenticated peer");

        let rejection = registry
            .install_verified_grant(grant, CREATED_AT_MS + 2)
            .await
            .expect_err("a verified grant must respect every local permission ceiling");
        assert_eq!(rejection.code, RemoteReasonCode::PolicyChanged);
        let after = registry
            .snapshot_at(&session_id, CREATED_AT_MS + 2)
            .await
            .expect("rejected authorization remains queryable");
        assert_eq!(
            after, before,
            "rejecting an overbroad grant must not mutate authorization"
        );
    }

    #[tokio::test]
    async fn controller_grant_installation_rejects_peer_ceiling_expansion() {
        let mut request = screen_and_pointer_request("controller-peer-ceiling");
        request.peer_permission_ceiling = vec![RemotePermissionScope::ScreenView];
        assert_controller_grant_exceeding_ceiling_is_rejected(request).await;
    }

    #[tokio::test]
    async fn controller_grant_installation_rejects_machine_ceiling_expansion() {
        let mut request = screen_and_pointer_request("controller-machine-ceiling");
        request.machine_permission_ceiling = vec![RemotePermissionScope::ScreenView];
        assert_controller_grant_exceeding_ceiling_is_rejected(request).await;
    }

    #[tokio::test]
    async fn controller_grant_installation_rejects_runtime_capability_expansion() {
        let mut request = screen_and_pointer_request("controller-runtime-ceiling");
        request.runtime_capabilities = vec![RemotePermissionScope::ScreenView];
        assert_controller_grant_exceeding_ceiling_is_rejected(request).await;
    }

    #[tokio::test]
    async fn controller_grant_installation_accepts_screen_view_within_all_ceilings() {
        let registry = SessionAuthorizationRegistry::default();
        let mut request = screen_and_pointer_request("controller-screen-view-only");
        request.peer_permission_ceiling = vec![RemotePermissionScope::ScreenView];
        request.machine_permission_ceiling = vec![RemotePermissionScope::ScreenView];
        request.runtime_capabilities = vec![RemotePermissionScope::ScreenView];
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing request with screen-only effective permissions");
        registry
            .bind_authenticated_peer_key(&session_id, &[7; 32], CREATED_AT_MS + 1)
            .await
            .expect("bind the authenticated peer");
        let mut grant = control_grant(&session_id);
        grant.granted_scopes = vec![RemotePermissionScope::ScreenView];

        let authorized = registry
            .install_verified_grant(grant, CREATED_AT_MS + 2)
            .await
            .expect("a verified screen-view-only grant remains valid");
        assert_eq!(
            authorized.granted_scopes,
            vec![RemotePermissionScope::ScreenView]
        );
        assert_eq!(
            authorized.authorization_state,
            RemoteAuthorizationState::Granted
        );
        assert_eq!(authorized.route_state, RemoteRouteState::Connecting);
        assert_eq!(authorized.route_kind, Some(RemoteRouteKind::LanQuic));
    }

    #[tokio::test]
    async fn peer_key_binding_is_exact_and_idempotent() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("bind-idempotent", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");

        let first = registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 1)
            .await
            .expect("bind requested peer key");
        let second = registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 2)
            .await
            .expect("repeat the same binding");

        assert_eq!(first.peer_key_id, public_key_id(&peer_public_key));
        assert_eq!(second.peer_key_id, first.peer_key_id);
        assert_eq!(second.updated_at_ms, CREATED_AT_MS + 2);
    }

    #[tokio::test]
    async fn peer_key_binding_rejects_non_ed25519_key_length() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("bind-rejects-shape", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");

        let wrong_shape = registry
            .bind_authenticated_peer_key(&session_id, &[7; 31], CREATED_AT_MS + 1)
            .await
            .expect_err("non-Ed25519 key length must fail closed");
        assert_eq!(wrong_shape.code, RemoteReasonCode::IdentityMismatch);
    }

    #[tokio::test]
    async fn peer_key_binding_rejects_key_id_mismatch() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("bind-rejects-identity", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");

        let wrong_key = registry
            .bind_authenticated_peer_key(&session_id, &[8; 32], CREATED_AT_MS + 1)
            .await
            .expect_err("key id mismatch must fail closed");
        assert_eq!(wrong_key.code, RemoteReasonCode::IdentityMismatch);
    }

    #[tokio::test]
    async fn peer_key_binding_rejects_terminal_authorization() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("bind-terminal", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");
        registry
            .record_failure(
                &session_id,
                RemoteAuthorizationState::Revoked,
                failure(RemoteReasonCode::GrantRevoked, "revoked"),
                CREATED_AT_MS + 1,
            )
            .await
            .expect("record terminal authorization");

        let failure = registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 2)
            .await
            .expect_err("terminal authorization cannot gain a peer key binding");

        assert_eq!(failure.code, RemoteReasonCode::GrantRevoked);
    }

    #[tokio::test]
    async fn active_control_authorization_requires_streaming_and_projects_exact_bindings() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("active-control", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");
        registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 1)
            .await
            .expect("bind peer key");
        registry
            .install_verified_grant(control_grant(&session_id), CREATED_AT_MS + 2)
            .await
            .expect("install verified grant");

        let before_streaming = registry
            .active_control_authorization(&session_id, CREATED_AT_MS + 3)
            .await
            .expect_err("connecting route cannot authorize control input");
        assert_eq!(before_streaming.code, RemoteReasonCode::PolicyChanged);

        registry
            .mark_streaming(&session_id, CREATED_AT_MS + 4)
            .await
            .expect("mark session streaming");
        let active = registry
            .active_control_authorization(&session_id, CREATED_AT_MS + 5)
            .await
            .expect("project active control authorization");

        assert_eq!(active.session_id, session_id);
        assert_eq!(
            active.peer_device_id,
            DeviceId("controller-device".to_string())
        );
        assert_eq!(active.peer_key_id, public_key_id(&peer_public_key));
        assert_eq!(active.peer_public_key, peer_public_key);
        assert_eq!(active.grant_id, [0xab; 32]);
        assert_eq!(active.policy_revision, 7);
        assert_eq!(active.expires_at_ms, EXPIRES_AT_MS);
        assert_eq!(
            active.granted_scopes,
            vec![
                RemotePermissionScope::InputPointer,
                RemotePermissionScope::InputKeyboard,
            ]
        );
    }

    #[tokio::test]
    async fn active_control_authorization_expires_grant_before_projection() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let request = control_request("active-control-expired", &peer_public_key);
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing authorization");
        registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 1)
            .await
            .expect("bind peer key");
        registry
            .install_verified_grant(control_grant(&session_id), CREATED_AT_MS + 2)
            .await
            .expect("install verified grant");
        registry
            .mark_streaming(&session_id, CREATED_AT_MS + 3)
            .await
            .expect("mark session streaming");

        let failure = registry
            .active_control_authorization(&session_id, EXPIRES_AT_MS + 1)
            .await
            .expect_err("expired grant cannot authorize control input");

        assert_eq!(failure.code, RemoteReasonCode::GrantExpired);
        let snapshot = registry
            .snapshot_at(&session_id, EXPIRES_AT_MS + 1)
            .await
            .expect("expired snapshot remains queryable");
        assert_eq!(
            snapshot.authorization_state,
            RemoteAuthorizationState::Expired
        );
    }

    #[tokio::test]
    async fn mark_streaming_preserves_the_verified_wan_route_kind() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let mut request = control_request("active-wan-control", &peer_public_key);
        request.transport_kind = "webrtc_relay".to_string();
        let session_id = request.session_id.clone();
        registry
            .begin_outgoing(request)
            .await
            .expect("begin outgoing WAN authorization");
        registry
            .bind_authenticated_peer_key(&session_id, &peer_public_key, CREATED_AT_MS + 1)
            .await
            .expect("bind WAN peer key");
        let mut grant = control_grant(&session_id);
        grant.route_constraint = "webrtc_relay".to_string();
        registry
            .install_verified_grant(grant, CREATED_AT_MS + 2)
            .await
            .expect("install verified WAN grant");

        let streaming = registry
            .mark_streaming(&session_id, CREATED_AT_MS + 3)
            .await
            .expect("mark WAN authorization streaming");

        assert_eq!(streaming.route_kind, Some(RemoteRouteKind::WebRtcRelay));
        assert_eq!(streaming.route_state, RemoteRouteState::Connected);
        assert_eq!(streaming.media_state, RemoteMediaState::Streaming);
    }

    #[tokio::test]
    async fn peer_revoke_only_fences_pending_outgoing_for_the_proven_device() {
        let registry = SessionAuthorizationRegistry::default();
        let peer_public_key = [7; 32];
        let peer_key_id = public_key_id(&peer_public_key);

        let mut exact = control_request("revoke-exact-device-a", &peer_public_key);
        exact.peer_device_id = DeviceId("device-a".to_owned());
        exact.transport_kind = "webrtc_relay".to_owned();
        let exact_session_id = exact.session_id.clone();
        registry
            .begin_outgoing(exact)
            .await
            .expect("begin exact authorization for device A");

        let mut pending_a = control_request("revoke-pending-device-a", &peer_public_key);
        pending_a.peer_device_id = DeviceId("device-a".to_owned());
        pending_a.peer_key_id = pending_outgoing_peer_key_id(&pending_a.peer_device_id);
        pending_a.transport_kind = "webrtc_relay".to_owned();
        let pending_a_session_id = pending_a.session_id.clone();
        registry
            .begin_outgoing(pending_a)
            .await
            .expect("begin pending authorization for device A");

        let mut pending_b = control_request("revoke-pending-device-b", &peer_public_key);
        pending_b.peer_device_id = DeviceId("device-b".to_owned());
        pending_b.peer_key_id = pending_outgoing_peer_key_id(&pending_b.peer_device_id);
        pending_b.transport_kind = "webrtc_relay".to_owned();
        let pending_b_session_id = pending_b.session_id.clone();
        registry
            .begin_outgoing(pending_b)
            .await
            .expect("begin unrelated pending authorization for device B");

        let mut revoked = registry
            .revoke_peer_authorizations(&peer_key_id, CREATED_AT_MS + 1)
            .await;
        revoked.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            revoked,
            vec![exact_session_id.clone(), pending_a_session_id.clone()]
        );
        for session_id in [&exact_session_id, &pending_a_session_id] {
            let snapshot = registry
                .snapshot_at(session_id, CREATED_AT_MS + 1)
                .await
                .expect("revoked authorization remains queryable");
            assert_eq!(
                snapshot.authorization_state,
                RemoteAuthorizationState::Revoked
            );
        }
        let unrelated = registry
            .snapshot_at(&pending_b_session_id, CREATED_AT_MS + 1)
            .await
            .expect("unrelated pending authorization remains queryable");
        assert_eq!(
            unrelated.authorization_state,
            RemoteAuthorizationState::Authorizing
        );
    }
}
