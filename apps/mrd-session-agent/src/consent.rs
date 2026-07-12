//! Agent-local consent authority state.
// The write-side state machine is intentionally wired by the next consent-
// manager slice. Registry writes stay crate-private; the public source trait is
// a transitional test-injection seam until B2.4 production wiring removes it.
#![cfg_attr(not(test), allow(dead_code))]

use mrd_agent_ipc::{
    CancelConsent, ConsentDecision, ConsentRequest, ConsentResult, DesktopKind, PeerBinding,
    AGENT_CONSENT_MAX_LIFETIME_MS, AGENT_IPC_MAX_IDENTIFIER_BYTES,
};
use mrd_proto::SessionId;
use mrd_session::PermissionScopes;
use std::{collections::HashMap, sync::Mutex};
use thiserror::Error;

/// Maximum simultaneously live consent-derived session bindings.
pub const MAX_ACTIVE_BINDINGS: usize = 64;
/// Maximum prompts that may be awaiting a local decision.
pub const MAX_PENDING_CONSENTS: usize = 32;
/// Maximum completed/cancelled consent identities retained against replay.
pub const MAX_CONSENT_TOMBSTONES: usize = 4_096;

/// Independently trusted, agent-local authorization for one product session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedSessionBinding {
    /// Consent request that created this authority.
    pub consent_request_id: [u8; 16],
    /// Exact service registration authorized by the local user decision.
    pub registration_id: [u8; 16],
    /// Exact service registration generation authorized by the decision.
    pub registration_epoch: u64,
    /// Active product session.
    pub session_id: SessionId,
    /// Authenticated remote peer shown to the local user.
    pub peer: PeerBinding,
    /// Exact permission scopes approved locally.
    pub approved_scopes: PermissionScopes,
    /// Local-policy revision shown to the user.
    pub policy_revision: u64,
    /// Interactive Windows session in which approval occurred.
    pub windows_session_id: u32,
    /// Exact input-desktop generation in which approval occurred.
    pub desktop_epoch: u64,
    /// Input-desktop kind in which approval occurred.
    pub desktop_kind: DesktopKind,
    /// Exclusive expiry of the local authorization.
    pub authorization_expires_at_ms: u64,
    /// Sole execute-grant issuer accepted for this authority.
    pub expected_issuer_key_id: [u8; 32],
}

/// Read-only source of agent-local consent bindings used during execution.
pub trait TrustedSessionBindingSource: Send + Sync {
    /// Resolve an unexpired binding using trusted current wall-clock time.
    fn resolve(&self, session_id: &SessionId, now_ms: u64) -> Option<TrustedSessionBinding>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedConsentContext {
    pub(crate) registration_id: [u8; 16],
    pub(crate) registration_epoch: u64,
    pub(crate) windows_session_id: u32,
    pub(crate) desktop_epoch: u64,
    pub(crate) desktop_kind: DesktopKind,
    pub(crate) expected_issuer_key_id: [u8; 32],
    pub(crate) now_ms: u64,
}

impl TrustedConsentContext {
    fn is_valid_for(&self, request: &ConsentRequest) -> bool {
        self.registration_id.iter().any(|byte| *byte != 0)
            && self.registration_epoch != 0
            && self.windows_session_id == request.windows_session_id
            && self.desktop_epoch != 0
            && self.desktop_kind == DesktopKind::Default
            && self.expected_issuer_key_id.iter().any(|byte| *byte != 0)
    }

    fn same_authority(&self, other: &Self) -> bool {
        self.registration_id == other.registration_id
            && self.registration_epoch == other.registration_epoch
            && self.windows_session_id == other.windows_session_id
            && self.desktop_epoch == other.desktop_epoch
            && self.desktop_kind == other.desktop_kind
            && self.expected_issuer_key_id == other.expected_issuer_key_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConsentFingerprint {
    session_id: SessionId,
    peer: PeerBinding,
    requested_scopes: PermissionScopes,
    policy_revision: u64,
    windows_session_id: u32,
    issued_at_ms: u64,
    expires_at_ms: u64,
    authorization_expires_at_ms: u64,
}

impl From<&ConsentRequest> for ConsentFingerprint {
    fn from(request: &ConsentRequest) -> Self {
        Self {
            session_id: request.session_id.clone(),
            peer: request.peer.clone(),
            requested_scopes: request.requested_scopes.clone(),
            policy_revision: request.policy_revision,
            windows_session_id: request.windows_session_id,
            issued_at_ms: request.issued_at_ms,
            expires_at_ms: request.expires_at_ms,
            authorization_expires_at_ms: request.authorization_expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentPrompt {
    pub(crate) attempt_id: u64,
    pub(crate) request: ConsentRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentBeginOutcome {
    Prompt(ConsentPrompt),
    Cached(ConsentResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsentCompletionRejection {
    InvalidLocalContext,
    PromptExpired,
    ScopeEscalation,
    UnexpectedApprovedScopes,
    BindingCapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsentCompletionDisposition {
    Approved,
    NonApproved,
    Rejected(ConsentCompletionRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentCompletion {
    pub(crate) result: ConsentResult,
    pub(crate) binding_changed: bool,
    pub(crate) disposition: ConsentCompletionDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentCompletionOutcome {
    Completed(ConsentCompletion),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentCancelOutcome {
    Cancelled(ConsentResult),
    Ignored,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub(crate) enum ConsentRegistryError {
    #[error("consent request shape is invalid")]
    InvalidRequest,
    #[error("consent request is outside its prompt window")]
    InactiveRequest,
    #[error("trusted local consent context does not match the request")]
    InvalidLocalContext,
    #[error("consent request id was reused for different semantics")]
    ConsentReplayConflict,
    #[error("an equivalent consent request is already pending")]
    ConsentAlreadyPending,
    #[error("the pending consent capacity is full")]
    PendingCapacityExceeded,
    #[error("the consent replay capacity is full")]
    TombstoneCapacityExceeded,
    #[error("consent attempt identities are exhausted")]
    AttemptIdExhausted,
    #[error("the consent authority registry lock is unavailable")]
    Unavailable,
}

#[derive(Debug, Clone)]
struct PendingConsent {
    attempt_id: u64,
    request: ConsentRequest,
    fingerprint: ConsentFingerprint,
    context: TrustedConsentContext,
}

#[derive(Debug, Clone)]
struct ConsentTombstone {
    fingerprint: ConsentFingerprint,
    result: ConsentResult,
    retain_until_ms: u64,
}

#[derive(Debug, Clone, Copy)]
struct RegistryLimits {
    active_bindings: usize,
    pending_consents: usize,
    consent_tombstones: usize,
}

impl Default for RegistryLimits {
    fn default() -> Self {
        Self {
            active_bindings: MAX_ACTIVE_BINDINGS,
            pending_consents: MAX_PENDING_CONSENTS,
            consent_tombstones: MAX_CONSENT_TOMBSTONES,
        }
    }
}

#[derive(Default)]
struct ConsentAuthorityState {
    bindings: HashMap<SessionId, TrustedSessionBinding>,
    pending: HashMap<[u8; 16], PendingConsent>,
    pending_attempts: HashMap<u64, [u8; 16]>,
    tombstones: HashMap<[u8; 16], ConsentTombstone>,
    next_attempt_id: u64,
}

impl ConsentAuthorityState {
    fn prune_for_capacity(&mut self, now_ms: u64) {
        self.prune_bindings(now_ms);
        self.tombstones
            .retain(|_, tombstone| now_ms < tombstone.retain_until_ms);
        let expired_request_ids = self
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                (now_ms >= pending.request.expires_at_ms).then_some(*request_id)
            })
            .collect::<Vec<_>>();
        for request_id in expired_request_ids {
            let Some(pending) = self.pending.remove(&request_id) else {
                continue;
            };
            self.pending_attempts.remove(&pending.attempt_id);
            if now_ms < pending.request.authorization_expires_at_ms {
                self.tombstones.insert(
                    request_id,
                    ConsentTombstone {
                        fingerprint: pending.fingerprint,
                        result: terminal_result(
                            &pending.request,
                            ConsentDecision::Expired,
                            pending.request.expires_at_ms.saturating_sub(1),
                        ),
                        retain_until_ms: pending.request.authorization_expires_at_ms,
                    },
                );
            }
        }
    }

    fn prune_bindings(&mut self, now_ms: u64) {
        self.bindings
            .retain(|_, binding| now_ms < binding.authorization_expires_at_ms);
    }

    fn allocate_attempt_id(&mut self) -> Result<u64, ConsentRegistryError> {
        let next = self
            .next_attempt_id
            .checked_add(1)
            .ok_or(ConsentRegistryError::AttemptIdExhausted)?;
        self.next_attempt_id = next;
        Ok(next)
    }
}

/// Bounded in-memory source of consent-derived authority.
///
/// Registry mutation is crate-private. The public binding/source types remain
/// a transitional test-injection seam until B2.4 seals production wiring; an
/// arbitrary source must not be treated as product consent authority.
pub struct ConsentAuthorityRegistry {
    state: Mutex<ConsentAuthorityState>,
    limits: RegistryLimits,
}

impl Default for ConsentAuthorityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsentAuthorityRegistry {
    /// Construct an empty fail-closed consent registry.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ConsentAuthorityState::default()),
            limits: RegistryLimits::default(),
        }
    }

    #[cfg(test)]
    fn with_limits(active_bindings: usize, pending_consents: usize, tombstones: usize) -> Self {
        Self {
            state: Mutex::new(ConsentAuthorityState::default()),
            limits: RegistryLimits {
                active_bindings,
                pending_consents,
                consent_tombstones: tombstones,
            },
        }
    }

    pub(crate) fn begin(
        &self,
        request: ConsentRequest,
        context: TrustedConsentContext,
    ) -> Result<ConsentBeginOutcome, ConsentRegistryError> {
        if !valid_request_shape(&request) {
            return Err(ConsentRegistryError::InvalidRequest);
        }
        let fingerprint = ConsentFingerprint::from(&request);
        let mut state = self
            .state
            .lock()
            .map_err(|_| ConsentRegistryError::Unavailable)?;
        state.prune_for_capacity(context.now_ms);

        if let Some(tombstone) = state.tombstones.get(&request.request_id) {
            if tombstone.fingerprint != fingerprint {
                return Err(ConsentRegistryError::ConsentReplayConflict);
            }
            let mut cached = tombstone.result.clone();
            cached.request_token = request.request_token;
            return Ok(ConsentBeginOutcome::Cached(cached));
        }
        if let Some(pending) = state.pending.get(&request.request_id) {
            return if pending.fingerprint == fingerprint {
                Err(ConsentRegistryError::ConsentAlreadyPending)
            } else {
                Err(ConsentRegistryError::ConsentReplayConflict)
            };
        }
        if context.now_ms < request.issued_at_ms || context.now_ms >= request.expires_at_ms {
            return Err(ConsentRegistryError::InactiveRequest);
        }
        if !context.is_valid_for(&request) {
            return Err(ConsentRegistryError::InvalidLocalContext);
        }
        if state.pending.len() >= self.limits.pending_consents {
            return Err(ConsentRegistryError::PendingCapacityExceeded);
        }
        if state.tombstones.len().saturating_add(state.pending.len())
            >= self.limits.consent_tombstones
        {
            return Err(ConsentRegistryError::TombstoneCapacityExceeded);
        }

        let attempt_id = state.allocate_attempt_id()?;
        state.pending.insert(
            request.request_id,
            PendingConsent {
                attempt_id,
                request: request.clone(),
                fingerprint,
                context,
            },
        );
        state
            .pending_attempts
            .insert(attempt_id, request.request_id);
        Ok(ConsentBeginOutcome::Prompt(ConsentPrompt {
            attempt_id,
            request,
        }))
    }

    pub(crate) fn complete(
        &self,
        attempt_id: u64,
        decision: ConsentDecision,
        approved_scopes: PermissionScopes,
        context: TrustedConsentContext,
    ) -> Result<ConsentCompletionOutcome, ConsentRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ConsentRegistryError::Unavailable)?;
        state.prune_bindings(context.now_ms);
        let Some(request_id) = state.pending_attempts.remove(&attempt_id) else {
            return Ok(ConsentCompletionOutcome::Ignored);
        };
        let Some(pending) = state.pending.remove(&request_id) else {
            return Ok(ConsentCompletionOutcome::Ignored);
        };

        let (mut final_decision, mut final_scopes, mut disposition) =
            normalize_decision(&pending, decision, approved_scopes, &context);
        if final_decision == ConsentDecision::Approved
            && !state.bindings.contains_key(&pending.request.session_id)
            && state.bindings.len() >= self.limits.active_bindings
        {
            final_decision = ConsentDecision::Dismissed;
            final_scopes.clear();
            disposition = ConsentCompletionDisposition::Rejected(
                ConsentCompletionRejection::BindingCapacityExceeded,
            );
        }

        let result = ConsentResult {
            request_token: pending.request.request_token,
            request_id: pending.request.request_id,
            session_id: pending.request.session_id.clone(),
            peer: pending.request.peer.clone(),
            policy_revision: pending.request.policy_revision,
            windows_session_id: pending.request.windows_session_id,
            decision: final_decision,
            approved_scopes: final_scopes.clone(),
            decided_at_ms: clamped_decided_at(&pending.request, context.now_ms),
        };
        let binding_changed = if final_decision == ConsentDecision::Approved {
            state.bindings.insert(
                pending.request.session_id.clone(),
                TrustedSessionBinding {
                    consent_request_id: pending.request.request_id,
                    registration_id: pending.context.registration_id,
                    registration_epoch: pending.context.registration_epoch,
                    session_id: pending.request.session_id.clone(),
                    peer: pending.request.peer.clone(),
                    approved_scopes: final_scopes,
                    policy_revision: pending.request.policy_revision,
                    windows_session_id: pending.context.windows_session_id,
                    desktop_epoch: pending.context.desktop_epoch,
                    desktop_kind: pending.context.desktop_kind,
                    authorization_expires_at_ms: pending.request.authorization_expires_at_ms,
                    expected_issuer_key_id: pending.context.expected_issuer_key_id,
                },
            );
            true
        } else {
            false
        };
        state.tombstones.insert(
            request_id,
            ConsentTombstone {
                fingerprint: pending.fingerprint,
                result: result.clone(),
                retain_until_ms: pending.request.authorization_expires_at_ms,
            },
        );
        Ok(ConsentCompletionOutcome::Completed(ConsentCompletion {
            result,
            binding_changed,
            disposition,
        }))
    }

    pub(crate) fn cancel(
        &self,
        cancel: &CancelConsent,
        now_ms: u64,
    ) -> Result<ConsentCancelOutcome, ConsentRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ConsentRegistryError::Unavailable)?;
        let Some(pending) = state.pending.get(&cancel.request_id) else {
            return Ok(ConsentCancelOutcome::Ignored);
        };
        if pending.request.request_token != cancel.request_token
            || pending.request.session_id != cancel.session_id
        {
            return Ok(ConsentCancelOutcome::Ignored);
        }
        let Some(pending) = state.pending.remove(&cancel.request_id) else {
            return Ok(ConsentCancelOutcome::Ignored);
        };
        state.pending_attempts.remove(&pending.attempt_id);
        let decision = if now_ms >= pending.request.expires_at_ms {
            ConsentDecision::Expired
        } else {
            ConsentDecision::Dismissed
        };
        let result = terminal_result(
            &pending.request,
            decision,
            clamped_decided_at(&pending.request, now_ms),
        );
        if now_ms < pending.request.authorization_expires_at_ms {
            state.tombstones.insert(
                cancel.request_id,
                ConsentTombstone {
                    fingerprint: pending.fingerprint,
                    result: result.clone(),
                    retain_until_ms: pending.request.authorization_expires_at_ms,
                },
            );
        }
        Ok(ConsentCancelOutcome::Cancelled(result))
    }
}

impl TrustedSessionBindingSource for ConsentAuthorityRegistry {
    fn resolve(&self, session_id: &SessionId, now_ms: u64) -> Option<TrustedSessionBinding> {
        let mut state = self.state.lock().ok()?;
        if state
            .bindings
            .get(session_id)
            .is_some_and(|binding| now_ms >= binding.authorization_expires_at_ms)
        {
            state.bindings.remove(session_id);
            return None;
        }
        state.bindings.get(session_id).cloned()
    }
}

fn normalize_decision(
    pending: &PendingConsent,
    decision: ConsentDecision,
    approved_scopes: PermissionScopes,
    context: &TrustedConsentContext,
) -> (
    ConsentDecision,
    PermissionScopes,
    ConsentCompletionDisposition,
) {
    if context.now_ms < pending.context.now_ms {
        return (
            ConsentDecision::Dismissed,
            PermissionScopes::new(),
            ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::InvalidLocalContext),
        );
    }
    if context.now_ms < pending.request.issued_at_ms
        || context.now_ms >= pending.request.expires_at_ms
    {
        return (
            ConsentDecision::Expired,
            PermissionScopes::new(),
            ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::PromptExpired),
        );
    }
    if !context.is_valid_for(&pending.request) || !context.same_authority(&pending.context) {
        return (
            ConsentDecision::Dismissed,
            PermissionScopes::new(),
            ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::InvalidLocalContext),
        );
    }
    match decision {
        ConsentDecision::Approved if approved_scopes.is_empty() => (
            ConsentDecision::Dismissed,
            PermissionScopes::new(),
            ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::ScopeEscalation),
        ),
        ConsentDecision::Approved
            if !pending
                .request
                .requested_scopes
                .is_superset(&approved_scopes) =>
        {
            (
                ConsentDecision::Dismissed,
                PermissionScopes::new(),
                ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::ScopeEscalation),
            )
        }
        ConsentDecision::Approved => (
            ConsentDecision::Approved,
            approved_scopes,
            ConsentCompletionDisposition::Approved,
        ),
        _non_approved if !approved_scopes.is_empty() => (
            ConsentDecision::Dismissed,
            PermissionScopes::new(),
            ConsentCompletionDisposition::Rejected(
                ConsentCompletionRejection::UnexpectedApprovedScopes,
            ),
        ),
        non_approved => (
            non_approved,
            PermissionScopes::new(),
            ConsentCompletionDisposition::NonApproved,
        ),
    }
}

fn clamped_decided_at(request: &ConsentRequest, now_ms: u64) -> u64 {
    now_ms
        .max(request.issued_at_ms)
        .min(request.expires_at_ms.saturating_sub(1))
}

fn terminal_result(
    request: &ConsentRequest,
    decision: ConsentDecision,
    decided_at_ms: u64,
) -> ConsentResult {
    ConsentResult {
        request_token: request.request_token,
        request_id: request.request_id,
        session_id: request.session_id.clone(),
        peer: request.peer.clone(),
        policy_revision: request.policy_revision,
        windows_session_id: request.windows_session_id,
        decision,
        approved_scopes: PermissionScopes::new(),
        decided_at_ms,
    }
}

fn valid_request_shape(request: &ConsentRequest) -> bool {
    request.request_token != 0
        && request.request_id.iter().any(|byte| *byte != 0)
        && !request.session_id.0.is_empty()
        && request.session_id.0.len() <= AGENT_IPC_MAX_IDENTIFIER_BYTES
        && !request.peer.device_id.0.is_empty()
        && request.peer.device_id.0.len() <= AGENT_IPC_MAX_IDENTIFIER_BYTES
        && request.peer.key_id.iter().any(|byte| *byte != 0)
        && !request.requested_scopes.is_empty()
        && request.policy_revision != 0
        && request.windows_session_id != 0
        && request.issued_at_ms != 0
        && request.expires_at_ms > request.issued_at_ms
        && request.authorization_expires_at_ms >= request.expires_at_ms
        && request
            .authorization_expires_at_ms
            .saturating_sub(request.issued_at_ms)
            <= AGENT_CONSENT_MAX_LIFETIME_MS
}

#[cfg(test)]
mod tests;
