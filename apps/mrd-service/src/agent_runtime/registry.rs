//! Service-owned registration, replacement, capability, and liveness state.

use mrd_agent_ipc::{
    AgentCapability, AgentCapabilitySnapshot, AgentChallenge, AgentHeartbeat, AgentProtocolState,
    AgentRegister, AgentRegistered, RegisteredAgentIdentity, RegistrationError,
    RegistrationProofVerifier, AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::watch;

/// Time since the last service-received agent message before health becomes stale.
pub const AGENT_HEARTBEAT_STALE_AFTER_MS: u64 = 15_000;

/// Opaque identity of one accepted private-pipe connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentConnectionId([u8; 16]);

impl AgentConnectionId {
    /// Construct a nonzero connection identifier.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, AgentRegistryError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(AgentRegistryError::InvalidConnectionId);
        }
        Ok(Self(bytes))
    }

    /// Return the opaque bytes for audit correlation.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// OS-classified caller identity for an accepted pipe connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCallerKind {
    /// Interactive local logon accepted for a desktop agent.
    InteractiveUser,
    /// Anonymous token or anonymous impersonation level.
    Anonymous,
    /// Network-only logon identity.
    Network,
    /// Service, batch, or another non-interactive token class.
    NonInteractive,
}

/// Identity independently observed from the pipe client process and token.
///
/// Production values must come only from the platform peer inspector. Agent
/// protocol fields are never a trusted source for this structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAgentIdentity {
    /// Trusted caller classification.
    pub caller_kind: AgentCallerKind,
    /// Pipe client process identifier.
    pub process_id: u32,
    /// Process creation time used to reject PID reuse.
    pub process_creation_time: u64,
    /// Hash of the token logon SID; never the raw SID.
    pub logon_sid_hash: [u8; 32],
    /// Token/terminal-services session identifier.
    pub windows_session_id: u32,
}

/// Explicit admission policy for an agent launched into an interactive session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementPolicy {
    /// No agent may already own the session when this admission activates.
    RejectExisting,
    /// Replace exactly the generation the trusted launcher observed.
    ReplaceExisting {
        /// Registration id that is allowed to be replaced.
        expected_registration_id: [u8; 16],
        /// Registration epoch that is allowed to be replaced.
        expected_registration_epoch: u64,
    },
}

/// Agent process the machine service intentionally launched or admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedAgentSession {
    /// Expected interactive Windows session.
    pub windows_session_id: u32,
    /// Expected logon SID digest for that session.
    pub logon_sid_hash: [u8; 32],
    /// Process identifier captured from the launcher-owned process handle.
    pub process_id: u32,
    /// Process creation time captured from the launcher-owned process handle.
    pub process_creation_time: u64,
    /// Signing key identity provisioned through the protected bootstrap channel.
    pub agent_key_id: [u8; 32],
    /// Exclusive expiry for this one process admission.
    pub expires_at_ms: u64,
    /// Launcher-selected policy for an existing generation.
    pub replacement_policy: ReplacementPolicy,
}

/// CSPRNG-produced values used by one registration challenge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChallengeMaterial {
    /// Unique service registration identity.
    pub registration_id: [u8; 16],
    /// Unique one-shot challenge identity.
    pub challenge_id: [u8; 16],
    /// One-shot challenge nonce.
    pub challenge_nonce: [u8; 32],
}

/// Entropy boundary used by deterministic generic-CI tests.
pub trait ChallengeMaterialSource: Send + Sync {
    /// Produce nonzero unique registration and challenge material.
    fn next_material(&self) -> Result<ChallengeMaterial, AgentRegistryError>;
}

#[derive(Debug, Default)]
struct RingChallengeMaterialSource;

impl ChallengeMaterialSource for RingChallengeMaterialSource {
    fn next_material(&self) -> Result<ChallengeMaterial, AgentRegistryError> {
        let random = SystemRandom::new();
        let mut material = ChallengeMaterial {
            registration_id: [0; 16],
            challenge_id: [0; 16],
            challenge_nonce: [0; 32],
        };
        random
            .fill(&mut material.registration_id)
            .map_err(|_| AgentRegistryError::EntropyUnavailable)?;
        random
            .fill(&mut material.challenge_id)
            .map_err(|_| AgentRegistryError::EntropyUnavailable)?;
        random
            .fill(&mut material.challenge_nonce)
            .map_err(|_| AgentRegistryError::EntropyUnavailable)?;
        validate_challenge_material(&material)?;
        Ok(material)
    }
}

/// Registration and registry state failures.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum AgentRegistryError {
    /// Connection ids cannot use the all-zero sentinel.
    #[error("agent connection id is invalid")]
    InvalidConnectionId,
    /// Expected process admission is malformed.
    #[error("expected agent session is invalid")]
    InvalidExpectedSession,
    /// Another unconsumed admission is installed for this session.
    #[error("expected agent session conflicts with existing identity")]
    ExpectedSessionConflict,
    /// The launcher admission reached its exclusive expiry.
    #[error("expected agent process admission expired")]
    ExpectedSessionExpired,
    /// Caller token is anonymous, network-only, or non-interactive.
    #[error("agent caller token is not an interactive local logon")]
    UntrustedCaller,
    /// Register fields do not match process/token observations.
    #[error("agent register identity does not match observed OS identity")]
    ObservedIdentityMismatch,
    /// The service did not expect an agent for this Windows session.
    #[error("agent Windows session is not expected")]
    UnexpectedWindowsSession,
    /// The expected logon identity changed or does not match the token.
    #[error("agent logon identity does not match expected session")]
    ExpectedLogonMismatch,
    /// The process object differs from the process launched by the service.
    #[error("agent process does not match launcher admission")]
    ExpectedProcessMismatch,
    /// The agent attempted to substitute an untrusted registration key.
    #[error("agent signing key does not match launcher admission")]
    ExpectedAgentKeyMismatch,
    /// This connection already started or completed registration.
    #[error("agent connection already registered")]
    DuplicateConnection,
    /// Another connection is completing registration for the same session.
    #[error("agent session registration is already in progress")]
    RegistrationInProgress,
    /// Admission policy rejects a second active session agent.
    #[error("agent session already has an active process")]
    ActiveSessionConflict,
    /// The active generation no longer matches the launcher's replacement target.
    #[error("agent replacement target no longer matches the active generation")]
    ReplacementTargetMismatch,
    /// Completion arrived without an outstanding one-shot challenge.
    #[error("agent connection has no pending registration")]
    NoPendingRegistration,
    /// A proof was already accepted and only initial capabilities are expected.
    #[error("agent registration proof was already authenticated")]
    RegistrationAlreadyAuthenticated,
    /// Challenge reached its exclusive expiry.
    #[error("agent registration challenge expired")]
    ChallengeExpired,
    /// Service registration epoch cannot advance further.
    #[error("agent registration epoch exhausted")]
    EpochExhausted,
    /// Challenge expiry time cannot be represented.
    #[error("agent registration challenge time overflowed")]
    ChallengeTimeOverflow,
    /// Challenge CSPRNG failed or returned a sentinel/collision.
    #[error("agent registration challenge entropy is unavailable")]
    EntropyUnavailable,
    /// Connection is not the active registration it claims to be.
    #[error("agent connection is not active")]
    NotActive,
    /// Capability or heartbeat bindings differ from registration state.
    #[error("agent message binding does not match registration")]
    MessageBindingMismatch,
    /// Capability revisions must increase strictly.
    #[error("agent capability revision is not monotonic")]
    NonMonotonicCapabilityRevision,
    /// Capability desktop generations cannot move backwards.
    #[error("agent desktop generation moved backwards")]
    DesktopEpochRollback,
    /// Lifecycle event sequence must increase strictly.
    #[error("agent event sequence is not monotonic")]
    NonMonotonicEventSequence,
    /// Service receipt times cannot move backwards.
    #[error("agent service receipt time is not monotonic")]
    NonMonotonicReceiveTime,
    /// Registry mutex was poisoned by a previous panic.
    #[error("agent registry state is unavailable")]
    StateUnavailable,
    /// A global security failure permanently disabled agent admission.
    #[error("agent registration is disabled after a security failure")]
    SecurityUnavailable,
    /// Underlying connection-local protocol validation failed.
    #[error("agent registration protocol failed: {0}")]
    Protocol(RegistrationError),
}

/// Active registration plus any superseded connection to stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationOutcome {
    /// Immutable authenticated agent identity.
    pub identity: RegisteredAgentIdentity,
    /// Previous connection atomically revoked by replacement policy.
    pub replaced_connection: Option<AgentConnectionId>,
}

/// Registration invalidated by disconnect or explicit replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidatedRegistration {
    /// Revoked service registration id.
    pub registration_id: [u8; 16],
    /// Revoked service registration epoch.
    pub registration_epoch: u64,
    /// Windows session formerly served by the registration.
    pub windows_session_id: u32,
}

/// Service-derived agent liveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHealth {
    /// A message was received within the bounded heartbeat window.
    Healthy,
    /// No service-observed message arrived within the heartbeat window.
    Unresponsive,
}

/// Read-only active agent state exposed to routing code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAgentSnapshot {
    /// Private connection owning the registration.
    pub connection_id: AgentConnectionId,
    /// Immutable authenticated identity.
    pub identity: RegisteredAgentIdentity,
    /// Latest strictly monotonic capability snapshot.
    pub capabilities: AgentCapabilitySnapshot,
    /// Latest strictly monotonic lifecycle/heartbeat sequence.
    pub last_event_sequence: u64,
    /// Latest agent-asserted observation time, never used for health.
    pub last_agent_observed_at_ms: u64,
    /// Latest receipt time measured by the service clock.
    pub last_received_at_ms: u64,
    /// Health derived exclusively from the service receipt clock.
    pub health: AgentHealth,
}

/// Immutable selection of one exact agent generation and desktop capability.
///
/// A binding can only be created from a healthy active registration. Routing
/// code must resolve this exact value before every request; it must never use
/// the Windows session id alone to select a replacement generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBinding {
    windows_session_id: u32,
    connection_id: AgentConnectionId,
    registration_id: [u8; 16],
    registration_epoch: u64,
    desktop_epoch: u64,
    required_capability: AgentCapability,
}

impl AgentBinding {
    /// Interactive Windows session selected for this product session.
    pub fn windows_session_id(&self) -> u32 {
        self.windows_session_id
    }

    /// Exact private connection that owned the selected registration.
    pub fn connection_id(&self) -> AgentConnectionId {
        self.connection_id
    }

    /// Registration identity selected for execution.
    pub fn registration_id(&self) -> &[u8; 16] {
        &self.registration_id
    }

    /// Registration generation selected for execution.
    pub fn registration_epoch(&self) -> u64 {
        self.registration_epoch
    }

    /// Desktop generation on which the capability was advertised.
    pub fn desktop_epoch(&self) -> u64 {
        self.desktop_epoch
    }

    /// Capability for which this binding was selected.
    pub fn required_capability(&self) -> AgentCapability {
        self.required_capability
    }
}

/// Failures selecting or resolving an exact desktop-agent route.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum AgentRouteError {
    /// No active registration owns the requested Windows session.
    #[error("no active agent owns the requested Windows session")]
    SessionUnavailable,
    /// The selected registration has not recently contacted the service.
    #[error("the selected agent is unhealthy")]
    Unhealthy,
    /// The selected registration does not currently advertise the capability.
    #[error("the selected agent capability is unavailable")]
    CapabilityUnavailable,
    /// A binding created for one capability was reused for another.
    #[error("the requested capability does not match the agent binding")]
    CapabilityBindingMismatch,
    /// Disconnect or replacement revoked the exact selected generation.
    #[error("the selected agent registration was revoked")]
    BindingRevoked,
    /// The interactive desktop changed after selection.
    #[error("the selected agent desktop changed")]
    DesktopChanged,
    /// Registry state could not be inspected safely.
    #[error("agent routing state is unavailable")]
    StateUnavailable,
    /// The selected registration negotiated a protocol too old for the request.
    #[error("the selected agent protocol version is unavailable")]
    ProtocolVersionUnavailable,
}

/// Immediately revocable handle for a successfully revalidated exact route.
#[derive(Debug, Clone)]
pub struct ExactAgentRoute {
    binding: AgentBinding,
    lease: AgentRegistrationLease,
}

impl ExactAgentRoute {
    /// Exact immutable binding revalidated by the registry.
    pub fn binding(&self) -> &AgentBinding {
        &self.binding
    }

    /// Private connection to which a request may be queued.
    pub fn connection_id(&self) -> AgentConnectionId {
        self.binding.connection_id
    }

    /// Whether the generation was revoked after this route was resolved.
    pub fn is_revoked(&self) -> bool {
        self.lease.is_revoked()
    }

    /// Borrow the generation lease for cancellation-aware request handling.
    pub fn lease(&self) -> &AgentRegistrationLease {
        &self.lease
    }

    /// Consume the route and return its generation lease.
    pub fn into_lease(self) -> AgentRegistrationLease {
        self.lease
    }
}

/// Revocation handle bound to one exact active registration generation.
#[derive(Debug, Clone)]
pub struct AgentRegistrationLease {
    registration_id: [u8; 16],
    registration_epoch: u64,
    revoked: watch::Receiver<bool>,
}

impl AgentRegistrationLease {
    /// Registration id to bind into an execution grant.
    pub fn registration_id(&self) -> &[u8; 16] {
        &self.registration_id
    }

    /// Registration epoch to bind into an execution grant.
    pub fn registration_epoch(&self) -> u64 {
        self.registration_epoch
    }

    /// Whether disconnect, replacement, or global invalidation revoked this generation.
    pub fn is_revoked(&self) -> bool {
        *self.revoked.borrow() || self.revoked.has_changed().is_err()
    }

    /// Wait until this exact generation is revoked.
    pub async fn wait_revoked(&mut self) {
        while !self.is_revoked() {
            if self.revoked.changed().await.is_err() {
                return;
            }
        }
    }
}

enum PendingPhase {
    Challenged(AgentProtocolState),
    Authenticated(RegisteredAgentIdentity),
}

struct PendingRegistration {
    expected: ExpectedAgentSession,
    verifier: Arc<dyn RegistrationProofVerifier + Send + Sync>,
    registration_id: [u8; 16],
    challenge_id: [u8; 16],
    challenge_nonce: [u8; 32],
    phase: PendingPhase,
}

#[derive(Clone)]
struct ExpectedAgentAdmission {
    session: ExpectedAgentSession,
    verifier: Arc<dyn RegistrationProofVerifier + Send + Sync>,
}

struct ActiveAgent {
    connection_id: AgentConnectionId,
    identity: RegisteredAgentIdentity,
    capabilities: AgentCapabilitySnapshot,
    last_event_sequence: u64,
    last_agent_observed_at_ms: u64,
    last_received_at_ms: u64,
    revocation: watch::Sender<bool>,
}

#[derive(Default)]
struct RegistryState {
    accepting_registrations: bool,
    expected: HashMap<u32, ExpectedAgentAdmission>,
    pending: HashMap<AgentConnectionId, PendingRegistration>,
    pending_by_session: HashMap<u32, AgentConnectionId>,
    active_by_session: HashMap<u32, ActiveAgent>,
    active_by_connection: HashMap<AgentConnectionId, u32>,
    next_epoch: u64,
}

/// Thread-safe service-owned registry for interactive desktop agents.
pub struct AgentRegistry {
    state: Mutex<RegistryState>,
    challenge_source: Arc<dyn ChallengeMaterialSource>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::with_challenge_source(Arc::new(RingChallengeMaterialSource))
    }
}

impl AgentRegistry {
    /// Construct a registry with injectable challenge entropy.
    pub fn with_challenge_source(challenge_source: Arc<dyn ChallengeMaterialSource>) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                accepting_registrations: true,
                next_epoch: 1,
                ..RegistryState::default()
            }),
            challenge_source,
        }
    }

    /// Admit exactly one launcher-bound process for a Windows session.
    pub fn expect_session(
        &self,
        expected: ExpectedAgentSession,
        verifier: Arc<dyn RegistrationProofVerifier + Send + Sync>,
    ) -> Result<(), AgentRegistryError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(u64::MAX);
        self.expect_session_at(expected, verifier, now_ms)
    }

    /// Admit a launcher-bound process using an injected trusted wall clock.
    pub fn expect_session_at(
        &self,
        expected: ExpectedAgentSession,
        verifier: Arc<dyn RegistrationProofVerifier + Send + Sync>,
        now_ms: u64,
    ) -> Result<(), AgentRegistryError> {
        validate_expected_session(&expected)?;
        if now_ms >= expected.expires_at_ms {
            return Err(AgentRegistryError::ExpectedSessionExpired);
        }
        let mut state = self.lock_state()?;
        if !state.accepting_registrations {
            return Err(AgentRegistryError::SecurityUnavailable);
        }
        validate_replacement_target(
            expected.replacement_policy,
            state.active_by_session.get(&expected.windows_session_id),
        )?;
        match state.expected.get(&expected.windows_session_id) {
            Some(current)
                if current.session == expected && Arc::ptr_eq(&current.verifier, &verifier) =>
            {
                Ok(())
            }
            Some(current)
                if current.session.expires_at_ms <= now_ms
                    && !state
                        .pending_by_session
                        .contains_key(&expected.windows_session_id) =>
            {
                state.expected.insert(
                    expected.windows_session_id,
                    ExpectedAgentAdmission {
                        session: expected,
                        verifier,
                    },
                );
                Ok(())
            }
            Some(_) => Err(AgentRegistryError::ExpectedSessionConflict),
            None => {
                state.expected.insert(
                    expected.windows_session_id,
                    ExpectedAgentAdmission {
                        session: expected,
                        verifier,
                    },
                );
                Ok(())
            }
        }
    }

    /// Cancel exactly one unconsumed launcher admission without touching a replacement.
    pub fn cancel_expected_session(
        &self,
        expected: &ExpectedAgentSession,
    ) -> Result<bool, AgentRegistryError> {
        let mut state = self.lock_state()?;
        if state
            .pending_by_session
            .contains_key(&expected.windows_session_id)
        {
            return Ok(false);
        }
        if state
            .expected
            .get(&expected.windows_session_id)
            .is_some_and(|current| current.session == *expected)
        {
            state.expected.remove(&expected.windows_session_id);
            return Ok(true);
        }
        Ok(false)
    }

    /// Validate OS observations and issue a one-shot registration challenge.
    pub fn begin_registration(
        &self,
        connection_id: AgentConnectionId,
        register: AgentRegister,
        observed: ObservedAgentIdentity,
        now_ms: u64,
    ) -> Result<AgentChallenge, AgentRegistryError> {
        validate_observed_registration(&register, &observed)?;

        let mut protocol = AgentProtocolState::new();
        protocol
            .accept_register(register.clone())
            .map_err(AgentRegistryError::Protocol)?;
        let material = self.challenge_source.next_material()?;
        validate_challenge_material(&material)?;

        let mut state = self.lock_state()?;
        if !state.accepting_registrations {
            return Err(AgentRegistryError::SecurityUnavailable);
        }
        let expected = state
            .expected
            .get(&observed.windows_session_id)
            .cloned()
            .ok_or(AgentRegistryError::UnexpectedWindowsSession)?;
        validate_expected_binding(&expected.session, &register, &observed, now_ms)?;
        if state.pending.contains_key(&connection_id)
            || state.active_by_connection.contains_key(&connection_id)
        {
            return Err(AgentRegistryError::DuplicateConnection);
        }
        if state
            .pending_by_session
            .contains_key(&observed.windows_session_id)
        {
            return Err(AgentRegistryError::RegistrationInProgress);
        }
        validate_replacement_target(
            expected.session.replacement_policy,
            state.active_by_session.get(&observed.windows_session_id),
        )?;
        if challenge_material_in_use(&state, &material) {
            return Err(AgentRegistryError::EntropyUnavailable);
        }

        let registration_epoch = state.next_epoch;
        state.next_epoch = state
            .next_epoch
            .checked_add(1)
            .ok_or(AgentRegistryError::EpochExhausted)?;
        let expires_at_ms = now_ms
            .checked_add(AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS)
            .ok_or(AgentRegistryError::ChallengeTimeOverflow)?;
        let challenge = AgentChallenge {
            registration_id: material.registration_id,
            registration_epoch,
            challenge_id: material.challenge_id,
            challenge_nonce: material.challenge_nonce,
            expected_agent_instance_id: register.agent_instance_id,
            expected_process_id: observed.process_id,
            expected_process_creation_time: observed.process_creation_time,
            expected_logon_sid_hash: observed.logon_sid_hash,
            expected_windows_session_id: observed.windows_session_id,
            issued_at_ms: now_ms,
            expires_at_ms,
        };
        protocol
            .issue_challenge(challenge.clone())
            .map_err(AgentRegistryError::Protocol)?;
        state.pending.insert(
            connection_id,
            PendingRegistration {
                expected: expected.session,
                verifier: expected.verifier,
                registration_id: material.registration_id,
                challenge_id: material.challenge_id,
                challenge_nonce: material.challenge_nonce,
                phase: PendingPhase::Challenged(protocol),
            },
        );
        state
            .pending_by_session
            .insert(observed.windows_session_id, connection_id);
        Ok(challenge)
    }

    /// Consume the one-shot proof while leaving any old active agent in place.
    ///
    /// A newly authenticated process is not activated until its first bound
    /// capability snapshot is validated by [`Self::activate_registration`].
    pub fn complete_registration(
        &self,
        connection_id: AgentConnectionId,
        proof: AgentRegistered,
        now_ms: u64,
    ) -> Result<RegisteredAgentIdentity, AgentRegistryError> {
        let pending = {
            let mut state = self.lock_state()?;
            let pending = state
                .pending
                .remove(&connection_id)
                .ok_or(AgentRegistryError::NoPendingRegistration)?;
            if matches!(pending.phase, PendingPhase::Authenticated(_)) {
                state.pending.insert(connection_id, pending);
                return Err(AgentRegistryError::RegistrationAlreadyAuthenticated);
            }
            pending
        };

        let PendingPhase::Challenged(mut protocol) = pending.phase else {
            unreachable!("authenticated pending registration returned above");
        };
        let verification = protocol.complete_registration(proof, now_ms, pending.verifier.as_ref());
        let mut state = self.lock_state()?;
        if state
            .pending_by_session
            .get(&pending.expected.windows_session_id)
            != Some(&connection_id)
        {
            return Err(AgentRegistryError::NoPendingRegistration);
        }
        let identity = match verification {
            Ok(identity) => identity,
            Err(error) => {
                state
                    .pending_by_session
                    .remove(&pending.expected.windows_session_id);
                return Err(map_registration_error(error));
            }
        };
        if !state
            .expected
            .get(&pending.expected.windows_session_id)
            .is_some_and(|current| current.session == pending.expected)
        {
            state
                .pending_by_session
                .remove(&pending.expected.windows_session_id);
            return Err(AgentRegistryError::UnexpectedWindowsSession);
        }
        state.pending.insert(
            connection_id,
            PendingRegistration {
                phase: PendingPhase::Authenticated(identity.clone()),
                ..pending
            },
        );
        Ok(identity)
    }

    /// Validate initial capabilities and atomically activate or replace an agent.
    pub fn activate_registration(
        &self,
        connection_id: AgentConnectionId,
        snapshot: AgentCapabilitySnapshot,
        received_at_ms: u64,
    ) -> Result<RegistrationOutcome, AgentRegistryError> {
        let mut state = self.lock_state()?;
        let pending = state
            .pending
            .remove(&connection_id)
            .ok_or(AgentRegistryError::NoPendingRegistration)?;
        if state
            .pending_by_session
            .get(&pending.expected.windows_session_id)
            == Some(&connection_id)
        {
            state
                .pending_by_session
                .remove(&pending.expected.windows_session_id);
        }
        let PendingPhase::Authenticated(identity) = pending.phase else {
            return Err(AgentRegistryError::NoPendingRegistration);
        };
        validate_initial_capabilities(&identity, &snapshot, received_at_ms)?;
        if received_at_ms >= pending.expected.expires_at_ms {
            return Err(AgentRegistryError::ExpectedSessionExpired);
        }
        if !state
            .expected
            .get(&pending.expected.windows_session_id)
            .is_some_and(|current| current.session == pending.expected)
        {
            return Err(AgentRegistryError::UnexpectedWindowsSession);
        }
        validate_replacement_target(
            pending.expected.replacement_policy,
            state
                .active_by_session
                .get(&pending.expected.windows_session_id),
        )?;

        let replaced = state
            .active_by_session
            .remove(&pending.expected.windows_session_id);
        let replaced_connection = replaced.as_ref().map(|active| active.connection_id);
        if let Some(active) = replaced {
            state.active_by_connection.remove(&active.connection_id);
            active.revocation.send_replace(true);
        }

        let (revocation, _) = watch::channel(false);
        state
            .active_by_connection
            .insert(connection_id, pending.expected.windows_session_id);
        state.active_by_session.insert(
            pending.expected.windows_session_id,
            ActiveAgent {
                connection_id,
                identity: identity.clone(),
                last_event_sequence: 0,
                last_agent_observed_at_ms: snapshot.observed_at_ms,
                last_received_at_ms: received_at_ms,
                capabilities: snapshot,
                revocation,
            },
        );
        state.expected.remove(&pending.expected.windows_session_id);
        Ok(RegistrationOutcome {
            identity,
            replaced_connection,
        })
    }

    /// Record a capability snapshot only for the exact active registration.
    pub fn record_capabilities(
        &self,
        connection_id: AgentConnectionId,
        snapshot: AgentCapabilitySnapshot,
        received_at_ms: u64,
    ) -> Result<(), AgentRegistryError> {
        let mut state = self.lock_state()?;
        let active = active_for_connection_mut(&mut state, connection_id)?;
        validate_capability_binding(&active.identity, &snapshot)?;
        if snapshot.revision == 0 || snapshot.revision <= active.capabilities.revision {
            return Err(AgentRegistryError::NonMonotonicCapabilityRevision);
        }
        if snapshot.desktop_epoch < active.capabilities.desktop_epoch {
            return Err(AgentRegistryError::DesktopEpochRollback);
        }
        validate_received_at(active.last_received_at_ms, received_at_ms)?;
        active.last_received_at_ms = received_at_ms;
        active.last_agent_observed_at_ms = active
            .last_agent_observed_at_ms
            .max(snapshot.observed_at_ms);
        active.capabilities = snapshot;
        Ok(())
    }

    /// Record strictly monotonic heartbeat state for the active connection.
    pub fn record_heartbeat(
        &self,
        connection_id: AgentConnectionId,
        heartbeat: AgentHeartbeat,
        received_at_ms: u64,
    ) -> Result<(), AgentRegistryError> {
        let mut state = self.lock_state()?;
        let active = active_for_connection_mut(&mut state, connection_id)?;
        let context = heartbeat.context;
        if context.registration_id != active.identity.registration_id
            || context.registration_epoch != active.identity.registration_epoch
            || context.windows_session_id != active.identity.windows_session_id
            || context.desktop_epoch != active.capabilities.desktop_epoch
            || context.observed_at_ms == 0
        {
            return Err(AgentRegistryError::MessageBindingMismatch);
        }
        if context.sequence == 0 || context.sequence <= active.last_event_sequence {
            return Err(AgentRegistryError::NonMonotonicEventSequence);
        }
        validate_received_at(active.last_received_at_ms, received_at_ms)?;
        active.last_event_sequence = context.sequence;
        active.last_received_at_ms = received_at_ms;
        active.last_agent_observed_at_ms =
            active.last_agent_observed_at_ms.max(context.observed_at_ms);
        Ok(())
    }

    /// Disconnect a connection and revoke its pending or active registration.
    pub fn disconnect(&self, connection_id: AgentConnectionId) -> Option<InvalidatedRegistration> {
        let mut state = self.state.lock().ok()?;
        if let Some(pending) = state.pending.remove(&connection_id) {
            if state
                .pending_by_session
                .get(&pending.expected.windows_session_id)
                == Some(&connection_id)
            {
                state
                    .pending_by_session
                    .remove(&pending.expected.windows_session_id);
            }
        } else {
            state
                .pending_by_session
                .retain(|_, pending_connection| *pending_connection != connection_id);
        }

        let windows_session_id = state.active_by_connection.remove(&connection_id)?;
        let matches_connection = state
            .active_by_session
            .get(&windows_session_id)
            .is_some_and(|active| active.connection_id == connection_id);
        if !matches_connection {
            return None;
        }
        let active = state.active_by_session.remove(&windows_session_id)?;
        active.revocation.send_replace(true);
        Some(InvalidatedRegistration {
            registration_id: active.identity.registration_id,
            registration_epoch: active.identity.registration_epoch,
            windows_session_id,
        })
    }

    /// Revoke every pending and active generation after a global security failure.
    pub fn invalidate_all(&self) -> Result<Vec<InvalidatedRegistration>, AgentRegistryError> {
        let mut state = self.lock_state()?;
        state.accepting_registrations = false;
        state.expected.clear();
        state.pending.clear();
        state.pending_by_session.clear();
        state.active_by_connection.clear();
        let active = std::mem::take(&mut state.active_by_session);
        Ok(active
            .into_iter()
            .map(|(windows_session_id, active)| {
                active.revocation.send_replace(true);
                InvalidatedRegistration {
                    registration_id: active.identity.registration_id,
                    registration_epoch: active.identity.registration_epoch,
                    windows_session_id,
                }
            })
            .collect())
    }

    /// Whether a grant-bound registration is still the active session owner.
    pub fn is_registration_active(&self, registration_id: &[u8; 16], epoch: u64) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.active_by_session.values().any(|active| {
                &active.identity.registration_id == registration_id
                    && active.identity.registration_epoch == epoch
            })
        })
    }

    /// Whether this exact private connection still owns an active generation.
    pub fn is_connection_active(&self, connection_id: AgentConnectionId) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .active_by_connection
                .get(&connection_id)
                .and_then(|session_id| state.active_by_session.get(session_id))
                .is_some_and(|active| active.connection_id == connection_id)
        })
    }

    /// Obtain an immediately revocable lease for the active session generation.
    pub fn lease_for_session(&self, windows_session_id: u32) -> Option<AgentRegistrationLease> {
        let state = self.state.lock().ok()?;
        let active = state.active_by_session.get(&windows_session_id)?;
        Some(registration_lease(active))
    }

    /// Select one healthy active generation for an exact capability.
    ///
    /// This is the sole operation that may select by Windows session id. The
    /// returned binding must be persisted by the product session and passed to
    /// [`Self::resolve_exact`] for every later operation.
    pub fn bind_active_session(
        &self,
        windows_session_id: u32,
        required_capability: AgentCapability,
        now_ms: u64,
    ) -> Result<AgentBinding, AgentRouteError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AgentRouteError::StateUnavailable)?;
        let active = state
            .active_by_session
            .get(&windows_session_id)
            .ok_or(AgentRouteError::SessionUnavailable)?;
        validate_route_readiness(active, required_capability, now_ms)?;
        Ok(AgentBinding {
            windows_session_id,
            connection_id: active.connection_id,
            registration_id: active.identity.registration_id,
            registration_epoch: active.identity.registration_epoch,
            desktop_epoch: active.capabilities.desktop_epoch,
            required_capability,
        })
    }

    /// Revalidate one previously selected generation without automatic fallback.
    ///
    /// A replacement process in the same Windows session is deliberately not a
    /// match. Callers must pause their product session and perform an explicit
    /// rebind flow before a new generation can receive work.
    pub fn resolve_exact(
        &self,
        binding: &AgentBinding,
        required_capability: AgentCapability,
        now_ms: u64,
    ) -> Result<ExactAgentRoute, AgentRouteError> {
        self.resolve_exact_with_minimum_minor(binding, required_capability, 0, now_ms)
    }

    /// Revalidate one exact route and require a negotiated protocol minor.
    ///
    /// This gates mandatory-field additions without silently delivering them to
    /// an older agent that negotiated a compatible major but lacks the fields.
    pub fn resolve_exact_with_minimum_minor(
        &self,
        binding: &AgentBinding,
        required_capability: AgentCapability,
        minimum_protocol_minor: u16,
        now_ms: u64,
    ) -> Result<ExactAgentRoute, AgentRouteError> {
        if binding.required_capability != required_capability {
            return Err(AgentRouteError::CapabilityBindingMismatch);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| AgentRouteError::StateUnavailable)?;
        let active = state
            .active_by_session
            .get(&binding.windows_session_id)
            .ok_or(AgentRouteError::BindingRevoked)?;
        if active.connection_id != binding.connection_id
            || active.identity.registration_id != binding.registration_id
            || active.identity.registration_epoch != binding.registration_epoch
            || active.identity.windows_session_id != binding.windows_session_id
        {
            return Err(AgentRouteError::BindingRevoked);
        }
        if active.capabilities.desktop_epoch != binding.desktop_epoch {
            return Err(AgentRouteError::DesktopChanged);
        }
        if active.identity.protocol_minor < minimum_protocol_minor {
            return Err(AgentRouteError::ProtocolVersionUnavailable);
        }
        validate_route_readiness(active, required_capability, now_ms)?;
        Ok(ExactAgentRoute {
            binding: binding.clone(),
            lease: registration_lease(active),
        })
    }

    /// Snapshot the active agent and derive health from the service receipt clock.
    pub fn active_for_session_at(
        &self,
        windows_session_id: u32,
        now_ms: u64,
    ) -> Option<ActiveAgentSnapshot> {
        self.state
            .lock()
            .ok()?
            .active_by_session
            .get(&windows_session_id)
            .map(|active| ActiveAgentSnapshot {
                connection_id: active.connection_id,
                identity: active.identity.clone(),
                capabilities: active.capabilities.clone(),
                last_event_sequence: active.last_event_sequence,
                last_agent_observed_at_ms: active.last_agent_observed_at_ms,
                last_received_at_ms: active.last_received_at_ms,
                health: agent_health(active, now_ms),
            })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, RegistryState>, AgentRegistryError> {
        self.state
            .lock()
            .map_err(|_| AgentRegistryError::StateUnavailable)
    }
}

fn registration_lease(active: &ActiveAgent) -> AgentRegistrationLease {
    AgentRegistrationLease {
        registration_id: active.identity.registration_id,
        registration_epoch: active.identity.registration_epoch,
        revoked: active.revocation.subscribe(),
    }
}

fn agent_health(active: &ActiveAgent, now_ms: u64) -> AgentHealth {
    if now_ms >= active.last_received_at_ms
        && now_ms - active.last_received_at_ms <= AGENT_HEARTBEAT_STALE_AFTER_MS
    {
        AgentHealth::Healthy
    } else {
        AgentHealth::Unresponsive
    }
}

fn validate_route_readiness(
    active: &ActiveAgent,
    required_capability: AgentCapability,
    now_ms: u64,
) -> Result<(), AgentRouteError> {
    if agent_health(active, now_ms) != AgentHealth::Healthy {
        return Err(AgentRouteError::Unhealthy);
    }
    if !active
        .capabilities
        .capabilities
        .contains(&required_capability)
    {
        return Err(AgentRouteError::CapabilityUnavailable);
    }
    Ok(())
}

fn validate_expected_session(expected: &ExpectedAgentSession) -> Result<(), AgentRegistryError> {
    let replacement_is_valid = match expected.replacement_policy {
        ReplacementPolicy::RejectExisting => true,
        ReplacementPolicy::ReplaceExisting {
            expected_registration_id,
            expected_registration_epoch,
        } => {
            expected_registration_id.iter().any(|byte| *byte != 0)
                && expected_registration_epoch != 0
        }
    };
    if expected.windows_session_id == 0
        || expected.logon_sid_hash.iter().all(|byte| *byte == 0)
        || expected.process_id == 0
        || expected.process_creation_time == 0
        || expected.agent_key_id.iter().all(|byte| *byte == 0)
        || expected.expires_at_ms == 0
        || !replacement_is_valid
    {
        return Err(AgentRegistryError::InvalidExpectedSession);
    }
    Ok(())
}

fn validate_observed_registration(
    register: &AgentRegister,
    observed: &ObservedAgentIdentity,
) -> Result<(), AgentRegistryError> {
    if observed.caller_kind != AgentCallerKind::InteractiveUser {
        return Err(AgentRegistryError::UntrustedCaller);
    }
    if register.process_id != observed.process_id
        || register.process_creation_time != observed.process_creation_time
        || register.logon_sid_hash != observed.logon_sid_hash
        || register.windows_session_id != observed.windows_session_id
    {
        return Err(AgentRegistryError::ObservedIdentityMismatch);
    }
    Ok(())
}

fn validate_expected_binding(
    expected: &ExpectedAgentSession,
    register: &AgentRegister,
    observed: &ObservedAgentIdentity,
    now_ms: u64,
) -> Result<(), AgentRegistryError> {
    if now_ms >= expected.expires_at_ms {
        return Err(AgentRegistryError::ExpectedSessionExpired);
    }
    if expected.logon_sid_hash != observed.logon_sid_hash {
        return Err(AgentRegistryError::ExpectedLogonMismatch);
    }
    if expected.process_id != observed.process_id
        || expected.process_creation_time != observed.process_creation_time
    {
        return Err(AgentRegistryError::ExpectedProcessMismatch);
    }
    if expected.agent_key_id != register.agent_key_id {
        return Err(AgentRegistryError::ExpectedAgentKeyMismatch);
    }
    Ok(())
}

fn validate_replacement_target(
    policy: ReplacementPolicy,
    active: Option<&ActiveAgent>,
) -> Result<(), AgentRegistryError> {
    match (policy, active) {
        (ReplacementPolicy::RejectExisting, None) => Ok(()),
        (ReplacementPolicy::RejectExisting, Some(_)) => {
            Err(AgentRegistryError::ActiveSessionConflict)
        }
        (
            ReplacementPolicy::ReplaceExisting {
                expected_registration_id,
                expected_registration_epoch,
            },
            Some(active),
        ) if active.identity.registration_id == expected_registration_id
            && active.identity.registration_epoch == expected_registration_epoch =>
        {
            Ok(())
        }
        (ReplacementPolicy::ReplaceExisting { .. }, _) => {
            Err(AgentRegistryError::ReplacementTargetMismatch)
        }
    }
}

fn validate_challenge_material(material: &ChallengeMaterial) -> Result<(), AgentRegistryError> {
    if material.registration_id.iter().all(|byte| *byte == 0)
        || material.challenge_id.iter().all(|byte| *byte == 0)
        || material.challenge_nonce.iter().all(|byte| *byte == 0)
    {
        return Err(AgentRegistryError::EntropyUnavailable);
    }
    Ok(())
}

fn challenge_material_in_use(state: &RegistryState, material: &ChallengeMaterial) -> bool {
    state.pending.values().any(|pending| {
        pending.registration_id == material.registration_id
            || pending.challenge_id == material.challenge_id
            || pending.challenge_nonce == material.challenge_nonce
            || match &pending.phase {
                PendingPhase::Authenticated(identity) => {
                    identity.registration_id == material.registration_id
                }
                PendingPhase::Challenged(_) => false,
            }
    }) || state
        .active_by_session
        .values()
        .any(|active| active.identity.registration_id == material.registration_id)
}

fn validate_initial_capabilities(
    identity: &RegisteredAgentIdentity,
    snapshot: &AgentCapabilitySnapshot,
    received_at_ms: u64,
) -> Result<(), AgentRegistryError> {
    validate_capability_binding(identity, snapshot)?;
    if snapshot.revision == 0 {
        return Err(AgentRegistryError::NonMonotonicCapabilityRevision);
    }
    if snapshot.desktop_epoch == 0 || snapshot.observed_at_ms == 0 || received_at_ms == 0 {
        return Err(AgentRegistryError::MessageBindingMismatch);
    }
    Ok(())
}

fn validate_capability_binding(
    identity: &RegisteredAgentIdentity,
    snapshot: &AgentCapabilitySnapshot,
) -> Result<(), AgentRegistryError> {
    if snapshot.registration_id != identity.registration_id
        || snapshot.agent_instance_id != identity.agent_instance_id
        || snapshot.windows_session_id != identity.windows_session_id
    {
        return Err(AgentRegistryError::MessageBindingMismatch);
    }
    Ok(())
}

fn validate_received_at(previous: u64, received_at_ms: u64) -> Result<(), AgentRegistryError> {
    if received_at_ms == 0 || received_at_ms < previous {
        return Err(AgentRegistryError::NonMonotonicReceiveTime);
    }
    Ok(())
}

fn map_registration_error(error: RegistrationError) -> AgentRegistryError {
    match error {
        RegistrationError::ChallengeExpired => AgentRegistryError::ChallengeExpired,
        other => AgentRegistryError::Protocol(other),
    }
}

fn active_for_connection_mut(
    state: &mut RegistryState,
    connection_id: AgentConnectionId,
) -> Result<&mut ActiveAgent, AgentRegistryError> {
    let session_id = state
        .active_by_connection
        .get(&connection_id)
        .copied()
        .ok_or(AgentRegistryError::NotActive)?;
    let active = state
        .active_by_session
        .get_mut(&session_id)
        .ok_or(AgentRegistryError::NotActive)?;
    if active.connection_id != connection_id {
        return Err(AgentRegistryError::NotActive);
    }
    Ok(active)
}
