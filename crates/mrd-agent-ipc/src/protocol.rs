//! Wire messages and registration state for machine-service/session-agent IPC.

use crate::{ExecuteGrant, AGENT_IPC_PROTOCOL_MAJOR, AGENT_IPC_PROTOCOL_MINOR};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use ring::digest::{Context as DigestContext, SHA256};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum lifetime of a one-shot registration challenge.
pub const AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS: u64 = 30_000;
/// Domain separator for hashes of validated Windows logon SID bytes.
pub const AGENT_LOGON_SID_HASH_CONTEXT: &[u8] = b"mrd-agent-logon-sid-v1\0";
/// Largest binary SID accepted by Windows (`SECURITY_MAX_SID_SIZE`).
pub const AGENT_LOGON_SID_MAX_BYTES: usize = 68;

/// Hash validated binary Windows logon SID bytes for protocol comparison.
///
/// The raw SID remains local to the OS security boundary. This helper rejects
/// impossible lengths; callers must additionally use `IsValidSid` on Windows.
pub fn hash_windows_logon_sid(sid_bytes: &[u8]) -> Option<[u8; 32]> {
    if !(8..=AGENT_LOGON_SID_MAX_BYTES).contains(&sid_bytes.len()) {
        return None;
    }
    let mut context = DigestContext::new(&SHA256);
    context.update(AGENT_LOGON_SID_HASH_CONTEXT);
    context.update(sid_bytes);
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(context.finish().as_ref());
    Some(digest)
}
/// Maximum lifetime of consent-derived authorization from request issuance.
///
/// The prompt deadline is bounded by this same outer authorization deadline.
pub const AGENT_CONSENT_MAX_LIFETIME_MS: u64 = 5 * 60 * 1_000;
/// Maximum UTF-8 bytes in protocol identifiers inherited from domain types.
pub const AGENT_IPC_MAX_IDENTIFIER_BYTES: usize = 256;
/// Maximum encoded media payload carried by one agent IPC message.
pub const AGENT_IPC_MAX_MEDIA_ACCESS_UNIT_BYTES: usize = 768 * 1024;
/// Domain separator for resource-bound input event commitments.
pub const AGENT_INPUT_EVENT_COMMITMENT_CONTEXT: &[u8] = b"mrd-agent-ipc/input-event/v1\0";

/// Serde adapter for fixed-size Ed25519 signatures. Serde's built-in array
/// implementations intentionally stop below this wire size on supported MSRVs.
pub(crate) mod bytes_64 {
    use serde::{
        de::{Error as _, SeqAccess, Visitor},
        Deserializer, Serializer,
    };
    use std::fmt;

    pub(crate) fn serialize<S>(value: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(value.iter())
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SignatureVisitor;

        impl<'de> Visitor<'de> for SignatureVisitor {
            type Value = [u8; 64];

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("exactly 64 Ed25519 signature bytes")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut signature = [0_u8; 64];
                for (index, byte) in signature.iter_mut().enumerate() {
                    *byte = sequence.next_element()?.ok_or_else(|| {
                        A::Error::invalid_length(index, &"exactly 64 signature bytes")
                    })?;
                }
                if sequence.next_element::<u8>()?.is_some() {
                    return Err(A::Error::invalid_length(65, &"exactly 64 signature bytes"));
                }
                Ok(signature)
            }
        }

        deserializer.deserialize_tuple(64, SignatureVisitor)
    }
}

mod request_token {
    use serde::{de::Error as _, Deserialize, Deserializer};

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = u64::deserialize(deserializer)?;
        if token == 0 {
            return Err(D::Error::custom("request token must be nonzero"));
        }
        Ok(token)
    }
}

/// A remote peer identity bound to an authorization decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerBinding {
    /// Stable product device identifier.
    pub device_id: DeviceId,
    /// Thumbprint of the authenticated peer key.
    pub key_id: [u8; 32],
}

/// Immutable process and interactive-session identity presented by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRegister {
    /// Per-process random identity, regenerated for every agent launch.
    pub agent_instance_id: [u8; 16],
    /// Operating-system process identifier.
    pub process_id: u32,
    /// Operating-system process creation time used to prevent PID reuse.
    pub process_creation_time: u64,
    /// One-way digest of the logon SID; the raw SID never crosses this protocol.
    pub logon_sid_hash: [u8; 32],
    /// Windows interactive session identifier.
    pub windows_session_id: u32,
    /// Identifier of the pre-trusted agent signing key.
    pub agent_key_id: [u8; 32],
    /// Per-registration nonce.
    pub agent_nonce: [u8; 32],
}

/// Service challenge that binds a registration to the observed process identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentChallenge {
    /// Service-assigned registration identifier.
    pub registration_id: [u8; 16],
    /// Monotonic registration generation.
    pub registration_epoch: u64,
    /// One-shot challenge identifier.
    pub challenge_id: [u8; 16],
    /// Service-generated challenge nonce.
    pub challenge_nonce: [u8; 32],
    /// Agent instance expected to answer this challenge.
    pub expected_agent_instance_id: [u8; 16],
    /// Process identifier observed by the service.
    pub expected_process_id: u32,
    /// Process creation time observed by the service.
    pub expected_process_creation_time: u64,
    /// Logon SID hash observed by the service.
    pub expected_logon_sid_hash: [u8; 32],
    /// Windows session observed by the service.
    pub expected_windows_session_id: u32,
    /// Inclusive challenge activation time.
    pub issued_at_ms: u64,
    /// Exclusive challenge expiry time.
    pub expires_at_ms: u64,
}

/// Agent signature over the registration transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRegistered {
    /// Service-assigned registration identifier.
    pub registration_id: [u8; 16],
    /// Registration generation being accepted.
    pub registration_epoch: u64,
    /// One-shot challenge being answered.
    pub challenge_id: [u8; 16],
    /// Agent process answering the challenge.
    pub agent_instance_id: [u8; 16],
    /// Protocol major selected by the agent.
    pub accepted_protocol_major: u16,
    /// Protocol minor selected by the agent.
    pub accepted_protocol_minor: u16,
    /// Time at which the proof was signed.
    pub signed_at_ms: u64,
    /// Signature over [`registration_proof_signing_bytes`].
    #[serde(with = "bytes_64")]
    pub signature: [u8; 64],
}

/// Product capabilities an agent can truthfully provide in its current desktop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    /// Interactive user-consent surface.
    Consent,
    /// Screen capture.
    Capture,
    /// Pointer and keyboard injection.
    Input,
    /// Audio capture or playback.
    Audio,
    /// Clipboard synchronization.
    Clipboard,
    /// File transfer.
    File,
    /// Remote display rendering.
    Render,
}

/// Point-in-time capabilities of a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentCapabilitySnapshot {
    /// Agent process that produced the snapshot.
    pub agent_instance_id: [u8; 16],
    /// Active registration.
    pub registration_id: [u8; 16],
    /// Interactive Windows session served by the agent.
    pub windows_session_id: u32,
    /// Monotonic snapshot revision.
    pub revision: u64,
    /// Desktop generation to which the capabilities apply.
    pub desktop_epoch: u64,
    /// Observation time.
    pub observed_at_ms: u64,
    /// Capabilities currently available.
    pub capabilities: std::collections::BTreeSet<AgentCapability>,
}

/// A service request for interactive user consent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConsentRequest {
    /// Service-assigned transport correlation token. This is not authorization data.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// One-shot consent request identifier.
    pub request_id: [u8; 16],
    /// Session requesting consent.
    pub session_id: SessionId,
    /// Authenticated remote peer.
    pub peer: PeerBinding,
    /// Scopes shown to the local user.
    pub requested_scopes: PermissionScopes,
    /// Policy revision used to form the request.
    pub policy_revision: u64,
    /// Interactive session in which consent must be shown.
    pub windows_session_id: u32,
    /// Inclusive request activation time.
    pub issued_at_ms: u64,
    /// Exclusive prompt/decision expiry time.
    pub expires_at_ms: u64,
    /// Exclusive expiry of authorization derived from an approval.
    ///
    /// This is independent of the prompt deadline and must never be replaced
    /// with `expires_at_ms` when binding a product session.
    pub authorization_expires_at_ms: u64,
}

/// Why the service is withdrawing an already-delivered consent prompt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConsentCancelReason {
    /// The caller that requested consent abandoned its future.
    CallerAborted,
    /// The service-side response deadline elapsed.
    TimedOut,
    /// The logical product session ended.
    SessionClosed,
    /// Local policy invalidated the request.
    PolicyChanged,
}

/// Cleanup for one exact consent request already delivered to an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CancelConsent {
    /// Exact nonzero transport token from the delivered request.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Exact one-shot consent request identifier.
    pub request_id: [u8; 16],
    /// Exact logical session from the delivered request.
    pub session_id: SessionId,
    /// Reason the prompt is being withdrawn.
    pub reason: ConsentCancelReason,
}

/// Local user's decision for a consent request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConsentDecision {
    /// The user approved at least the returned scopes.
    Approved,
    /// The user explicitly denied the request.
    Denied,
    /// The request expired before a decision was made.
    Expired,
    /// The consent surface was dismissed without approval.
    Dismissed,
}

/// Agent response to an interactive consent request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConsentResult {
    /// Exact transport token copied from the request.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Consent request being answered.
    pub request_id: [u8; 16],
    /// Session from the original request.
    pub session_id: SessionId,
    /// Peer from the original request.
    pub peer: PeerBinding,
    /// Policy revision used for the decision.
    pub policy_revision: u64,
    /// Windows session in which the decision occurred.
    pub windows_session_id: u32,
    /// User decision.
    pub decision: ConsentDecision,
    /// Approved subset; empty for non-approved decisions.
    pub approved_scopes: PermissionScopes,
    /// Decision time.
    pub decided_at_ms: u64,
}

/// Failures while correlating an untrusted consent result with its request.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ConsentValidationError {
    /// The service-created request itself violates protocol shape bounds.
    #[error("consent request has an invalid shape")]
    InvalidRequest,
    /// Result identity, session, peer, policy, or Windows-session bindings differ.
    #[error("consent result does not match its request")]
    RequestMismatch,
    /// Trusted current time is before the request activation time.
    #[error("consent request is not active yet")]
    NotYetValid,
    /// Trusted current time reached the exclusive request expiry.
    #[error("consent request has expired")]
    Expired,
    /// The agent's decision timestamp is outside the active request window.
    #[error("consent decision time is invalid")]
    InvalidDecisionTime,
    /// An approved result returned no concrete scope.
    #[error("approved consent result has no scopes")]
    EmptyApproval,
    /// The result approved a scope that was never requested.
    #[error("consent result expands the requested scopes")]
    ScopeEscalation,
    /// A non-approved decision carried approved scopes.
    #[error("non-approved consent result carries approved scopes")]
    UnexpectedApprovedScopes,
}

/// A consent result that passed request correlation and scope-subset checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedConsent {
    request_id: [u8; 16],
    session_id: SessionId,
    peer: PeerBinding,
    policy_revision: u64,
    windows_session_id: u32,
    decision: ConsentDecision,
    approved_scopes: PermissionScopes,
    decided_at_ms: u64,
    authorization_expires_at_ms: u64,
}

impl ValidatedConsent {
    /// Return the correlated request identifier.
    pub fn request_id(&self) -> &[u8; 16] {
        &self.request_id
    }

    /// Return the session whose consent was validated.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Return the authenticated peer shown to the user.
    pub fn peer(&self) -> &PeerBinding {
        &self.peer
    }

    /// Return the policy revision under which consent was requested.
    pub fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    /// Return the interactive Windows session that produced consent.
    pub fn windows_session_id(&self) -> u32 {
        self.windows_session_id
    }

    /// Return the user's decision.
    pub fn decision(&self) -> ConsentDecision {
        self.decision
    }

    /// Return the validated subset of approved scopes.
    pub fn approved_scopes(&self) -> &PermissionScopes {
        &self.approved_scopes
    }

    /// Return the trusted-window decision timestamp.
    pub fn decided_at_ms(&self) -> u64 {
        self.decided_at_ms
    }

    /// Return the authorization deadline independently of the prompt deadline.
    pub fn authorization_expires_at_ms(&self) -> u64 {
        self.authorization_expires_at_ms
    }
}

/// Correlate an untrusted agent result with the exact service-created request.
pub fn validate_consent_result(
    request: &ConsentRequest,
    result: &ConsentResult,
    now_ms: u64,
) -> Result<ValidatedConsent, ConsentValidationError> {
    if request.request_token == 0
        || request.request_id.iter().all(|byte| *byte == 0)
        || request.session_id.0.is_empty()
        || request.session_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
        || request.peer.device_id.0.is_empty()
        || request.peer.device_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
        || request.peer.key_id.iter().all(|byte| *byte == 0)
        || request.requested_scopes.is_empty()
        || request.policy_revision == 0
        || request.windows_session_id == 0
        || request.issued_at_ms == 0
        || request.expires_at_ms <= request.issued_at_ms
        || request.authorization_expires_at_ms < request.expires_at_ms
        || request
            .authorization_expires_at_ms
            .saturating_sub(request.issued_at_ms)
            > AGENT_CONSENT_MAX_LIFETIME_MS
    {
        return Err(ConsentValidationError::InvalidRequest);
    }
    if result.request_token == 0
        || result.request_token != request.request_token
        || result.request_id != request.request_id
        || result.session_id != request.session_id
        || result.peer != request.peer
        || result.policy_revision != request.policy_revision
        || result.windows_session_id != request.windows_session_id
    {
        return Err(ConsentValidationError::RequestMismatch);
    }
    if now_ms < request.issued_at_ms {
        return Err(ConsentValidationError::NotYetValid);
    }
    if now_ms >= request.expires_at_ms {
        return Err(ConsentValidationError::Expired);
    }
    if result.decided_at_ms < request.issued_at_ms
        || result.decided_at_ms >= request.expires_at_ms
        || result.decided_at_ms > now_ms
    {
        return Err(ConsentValidationError::InvalidDecisionTime);
    }

    match result.decision {
        ConsentDecision::Approved => {
            if result.approved_scopes.is_empty() {
                return Err(ConsentValidationError::EmptyApproval);
            }
            if !request
                .requested_scopes
                .is_superset(&result.approved_scopes)
            {
                return Err(ConsentValidationError::ScopeEscalation);
            }
        }
        ConsentDecision::Denied | ConsentDecision::Expired | ConsentDecision::Dismissed => {
            if !result.approved_scopes.is_empty() {
                return Err(ConsentValidationError::UnexpectedApprovedScopes);
            }
        }
    }

    Ok(ValidatedConsent {
        request_id: request.request_id,
        session_id: request.session_id.clone(),
        peer: request.peer.clone(),
        policy_revision: request.policy_revision,
        windows_session_id: request.windows_session_id,
        decision: result.decision,
        approved_scopes: result.approved_scopes.clone(),
        decided_at_ms: result.decided_at_ms,
        authorization_expires_at_ms: request.authorization_expires_at_ms,
    })
}

/// Audio flow requested from the interactive agent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AudioDirection {
    /// Send local audio to the remote peer.
    Listen,
    /// Play remote audio locally.
    Talk,
    /// Enable both directions.
    Duplex,
}

/// File flow requested from the interactive agent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileDirection {
    /// Read local files for the remote peer.
    Download,
    /// Write files supplied by the remote peer.
    Upload,
    /// Permit both directions.
    Bidirectional,
}

/// Mouse button carried by a service-to-agent input event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InputButton {
    /// Primary mouse button.
    Left,
    /// Secondary mouse button.
    Right,
    /// Middle mouse button.
    Middle,
    /// First extended mouse button.
    X1,
    /// Second extended mouse button.
    X2,
}

/// Keyboard key carried by a service-to-agent input event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputKey {
    /// Windows virtual-key code.
    VirtualKey {
        /// Numeric virtual-key code.
        code: u16,
    },
}

/// Normalized input operation executed inside one authorized input resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEventPayload {
    /// Absolute mouse position in target desktop coordinates.
    MouseMove {
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
    },
    /// Mouse button transition.
    MouseButton {
        /// Button identifier.
        button: InputButton,
        /// Whether the button is pressed.
        pressed: bool,
    },
    /// Vertical mouse-wheel movement.
    MouseWheel {
        /// Wheel delta.
        delta: i32,
    },
    /// Horizontal mouse-wheel movement.
    MouseHorizontalWheel {
        /// Wheel delta.
        delta: i32,
    },
    /// Keyboard transition.
    Key {
        /// Key identifier.
        key: InputKey,
        /// Whether the key is pressed.
        pressed: bool,
    },
    /// Release every pressed key and button owned by this resource.
    ReleaseAll,
}

impl InputEventPayload {
    /// Permission needed to inject this event, or `None` for cleanup-only release-all.
    pub fn required_scope(&self) -> Option<PermissionScope> {
        match self {
            Self::MouseMove { .. }
            | Self::MouseButton { .. }
            | Self::MouseWheel { .. }
            | Self::MouseHorizontalWheel { .. } => Some(PermissionScope::InputPointer),
            Self::Key { .. } => Some(PermissionScope::InputKeyboard),
            Self::ReleaseAll => None,
        }
    }

    fn has_valid_shape(&self) -> bool {
        !matches!(
            self,
            Self::Key {
                key: InputKey::VirtualKey { code: 0 },
                ..
            }
        )
    }
}

/// One input operation bound to an authorized input resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InputEventEnvelope {
    /// Service-assigned transport correlation token.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Product session that owns the input resource.
    pub session_id: SessionId,
    /// Input-resource identity created by `StartInput`.
    pub resource_id: [u8; 16],
    /// Unique execute grant that authorized `StartInput`.
    pub start_grant_id: [u8; 32],
    /// Strictly increasing sequence local to this resource.
    pub sequence: u64,
    /// Input operation. This payload is never echoed in an acknowledgment.
    pub event: InputEventPayload,
}

impl InputEventEnvelope {
    /// Validate bounded identifiers, non-sentinel bindings, sequence, and payload shape.
    pub fn validate_shape(&self) -> Result<(), InputRejection> {
        if self.request_token == 0
            || self.session_id.0.trim().is_empty()
            || self.session_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
            || self.session_id.0.contains('\0')
            || self.resource_id.iter().all(|byte| *byte == 0)
            || self.start_grant_id.iter().all(|byte| *byte == 0)
            || self.sequence == 0
            || self.sequence == u64::MAX
            || !self.event.has_valid_shape()
        {
            return Err(InputRejection::InvalidEvent);
        }
        Ok(())
    }

    /// Return a domain-separated commitment covering every semantic event field.
    ///
    /// The service-assigned request token is transport correlation metadata and
    /// is deliberately excluded so a retry cannot become a different input.
    pub fn commitment(&self) -> Result<[u8; 32], InputRejection> {
        self.validate_shape()?;
        #[derive(Serialize)]
        struct SemanticInputEvent<'a> {
            session_id: &'a SessionId,
            resource_id: &'a [u8; 16],
            start_grant_id: &'a [u8; 32],
            sequence: u64,
            event: &'a InputEventPayload,
        }
        let encoded = serde_json::to_vec(&SemanticInputEvent {
            session_id: &self.session_id,
            resource_id: &self.resource_id,
            start_grant_id: &self.start_grant_id,
            sequence: self.sequence,
            event: &self.event,
        })
        .map_err(|_| InputRejection::InvalidEvent)?;
        let mut context = DigestContext::new(&SHA256);
        context.update(AGENT_INPUT_EVENT_COMMITMENT_CONTEXT);
        context.update(&(encoded.len() as u64).to_le_bytes());
        context.update(&encoded);
        let mut commitment = [0_u8; 32];
        commitment.copy_from_slice(context.finish().as_ref());
        Ok(commitment)
    }
}

/// Stable rejection category for an input event that did no platform work.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputRejection {
    /// Current machine policy no longer permits the resource.
    Policy,
    /// The event did not match a live, authorized input resource.
    Grant,
    /// The registered agent does not implement the requested input operation.
    Unsupported,
    /// The input desktop changed after the resource was authorized.
    StaleDesktop,
    /// The resource-local replay or ordering rules rejected the sequence.
    Replay,
    /// The event violated a bounded protocol invariant.
    InvalidEvent,
}

/// Stable platform-failure category for an authorized input event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputFailure {
    /// Windows User Interface Privilege Isolation blocked injection.
    Uipi,
    /// Input injection failed for another platform reason.
    Platform,
}

/// Result of processing one resource-bound input event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputAckOutcome {
    /// The operation was accepted exactly once.
    Applied,
    /// The operation was rejected before platform injection.
    Rejected {
        /// Coarse rejection category with no input-payload detail.
        reason: InputRejection,
    },
    /// The operation was authorized but platform injection failed.
    Failed {
        /// Coarse platform failure with no native error string.
        reason: InputFailure,
    },
}

/// Payload-free acknowledgment for one input sequence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InputAck {
    /// Exact transport token copied from the input event.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Registration that processed the event.
    pub registration_id: [u8; 16],
    /// Registration generation that processed the event.
    pub registration_epoch: u64,
    /// Product session named by the event.
    pub session_id: SessionId,
    /// Input resource named by the event.
    pub resource_id: [u8; 16],
    /// Execute grant that established the input resource.
    pub start_grant_id: [u8; 32],
    /// Resource-local input sequence being acknowledged.
    pub sequence: u64,
    /// Commitment of the exact event envelope being acknowledged.
    pub event_commitment: [u8; 32],
    /// Structured result. It deliberately contains no input payload or free text.
    pub outcome: InputAckOutcome,
}

/// Product operation executed by an interactive-session agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentCommand {
    /// Start screen capture for one display.
    StartCapture {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
        /// Platform display identifier.
        display_id: u32,
    },
    /// Stop a capture resource.
    StopCapture {
        /// Resource created by `start_capture`.
        resource_id: [u8; 16],
    },
    /// Start input injection.
    StartInput {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
        /// Exact pointer and/or keyboard permissions enabled for this resource.
        input_scopes: PermissionScopes,
    },
    /// Stop input injection and release held input state.
    StopInput {
        /// Resource created by `start_input`.
        resource_id: [u8; 16],
    },
    /// Start audio processing.
    StartAudio {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
        /// Requested audio direction.
        direction: AudioDirection,
    },
    /// Stop audio processing.
    StopAudio {
        /// Resource created by `start_audio`.
        resource_id: [u8; 16],
    },
    /// Start clipboard synchronization.
    StartClipboard {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
    },
    /// Stop clipboard synchronization.
    StopClipboard {
        /// Resource created by `start_clipboard`.
        resource_id: [u8; 16],
    },
    /// Start file transfer handling.
    StartFile {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
        /// Requested transfer direction.
        direction: FileDirection,
    },
    /// Stop file transfer handling.
    StopFile {
        /// Resource created by `start_file`.
        resource_id: [u8; 16],
    },
    /// Start rendering a remote display.
    StartRender {
        /// Resource identity used for idempotency and cleanup.
        resource_id: [u8; 16],
        /// Platform display identifier.
        display_id: u32,
    },
    /// Stop rendering.
    StopRender {
        /// Resource created by `start_render`.
        resource_id: [u8; 16],
    },
}

impl AgentCommand {
    /// Return a stable, domain-separated SHA-256 digest for grant binding.
    pub fn digest(&self) -> [u8; 32] {
        let mut context = DigestContext::new(&SHA256);
        context.update(b"mrd-agent-command-v1\0");

        let (kind, resource_id, display_id, direction) = match self {
            Self::StartCapture {
                resource_id,
                display_id,
            } => (
                b"start_capture".as_slice(),
                resource_id,
                Some(*display_id),
                None,
            ),
            Self::StopCapture { resource_id } => {
                (b"stop_capture".as_slice(), resource_id, None, None)
            }
            Self::StartInput { resource_id, .. } => {
                (b"start_input".as_slice(), resource_id, None, None)
            }
            Self::StopInput { resource_id } => (b"stop_input".as_slice(), resource_id, None, None),
            Self::StartAudio {
                resource_id,
                direction,
            } => (
                b"start_audio".as_slice(),
                resource_id,
                None,
                Some(match direction {
                    AudioDirection::Listen => 1,
                    AudioDirection::Talk => 2,
                    AudioDirection::Duplex => 3,
                }),
            ),
            Self::StopAudio { resource_id } => (b"stop_audio".as_slice(), resource_id, None, None),
            Self::StartClipboard { resource_id } => {
                (b"start_clipboard".as_slice(), resource_id, None, None)
            }
            Self::StopClipboard { resource_id } => {
                (b"stop_clipboard".as_slice(), resource_id, None, None)
            }
            Self::StartFile {
                resource_id,
                direction,
            } => (
                b"start_file".as_slice(),
                resource_id,
                None,
                Some(match direction {
                    FileDirection::Download => 1,
                    FileDirection::Upload => 2,
                    FileDirection::Bidirectional => 3,
                }),
            ),
            Self::StopFile { resource_id } => (b"stop_file".as_slice(), resource_id, None, None),
            Self::StartRender {
                resource_id,
                display_id,
            } => (
                b"start_render".as_slice(),
                resource_id,
                Some(*display_id),
                None,
            ),
            Self::StopRender { resource_id } => {
                (b"stop_render".as_slice(), resource_id, None, None)
            }
        };

        context.update(&(kind.len() as u16).to_le_bytes());
        context.update(kind);
        context.update(resource_id);
        if let Some(display_id) = display_id {
            context.update(&display_id.to_le_bytes());
        }
        if let Some(direction) = direction {
            context.update(&[direction]);
        }
        if let Self::StartInput { input_scopes, .. } = self {
            let encoded_scopes = serde_json::to_vec(input_scopes)
                .expect("input permission scopes are infallibly serializable");
            context.update(&(encoded_scopes.len() as u64).to_le_bytes());
            context.update(&encoded_scopes);
        }

        let digest = context.finish();
        let mut output = [0_u8; 32];
        output.copy_from_slice(digest.as_ref());
        output
    }

    /// Alias that makes the relationship to `ExecuteGrantClaims.command_digest` explicit.
    pub fn command_digest(&self) -> [u8; 32] {
        self.digest()
    }

    /// Desktop capability required to execute this command family.
    pub fn required_capability(&self) -> AgentCapability {
        match self {
            Self::StartCapture { .. } | Self::StopCapture { .. } => AgentCapability::Capture,
            Self::StartInput { .. } | Self::StopInput { .. } => AgentCapability::Input,
            Self::StartAudio { .. } | Self::StopAudio { .. } => AgentCapability::Audio,
            Self::StartClipboard { .. } | Self::StopClipboard { .. } => AgentCapability::Clipboard,
            Self::StartFile { .. } | Self::StopFile { .. } => AgentCapability::File,
            Self::StartRender { .. } | Self::StopRender { .. } => AgentCapability::Render,
        }
    }

    /// Permission scopes that must be present before this command can start work.
    pub fn required_scopes(&self) -> PermissionScopes {
        let mut scopes = PermissionScopes::new();
        match self {
            Self::StartCapture { .. } | Self::StartRender { .. } => {
                scopes.insert(PermissionScope::ScreenView);
            }
            Self::StartInput { input_scopes, .. } => scopes.extend(input_scopes.iter().copied()),
            Self::StartAudio { direction, .. } => match direction {
                AudioDirection::Listen => {
                    scopes.insert(PermissionScope::AudioListen);
                }
                AudioDirection::Talk => {
                    scopes.insert(PermissionScope::AudioTalk);
                }
                AudioDirection::Duplex => {
                    scopes.insert(PermissionScope::AudioListen);
                    scopes.insert(PermissionScope::AudioTalk);
                }
            },
            Self::StartClipboard { .. } => {
                scopes.insert(PermissionScope::ClipboardRead);
                scopes.insert(PermissionScope::ClipboardWrite);
            }
            Self::StartFile { direction, .. } => match direction {
                FileDirection::Download => {
                    scopes.insert(PermissionScope::FileRead);
                }
                FileDirection::Upload => {
                    scopes.insert(PermissionScope::FileWrite);
                }
                FileDirection::Bidirectional => {
                    scopes.insert(PermissionScope::FileRead);
                    scopes.insert(PermissionScope::FileWrite);
                }
            },
            Self::StopCapture { .. }
            | Self::StopInput { .. }
            | Self::StopAudio { .. }
            | Self::StopClipboard { .. }
            | Self::StopFile { .. }
            | Self::StopRender { .. } => {}
        }
        scopes
    }

    /// Whether the command only tears down an already-bound resource.
    pub fn is_cleanup(&self) -> bool {
        matches!(
            self,
            Self::StopCapture { .. }
                | Self::StopInput { .. }
                | Self::StopAudio { .. }
                | Self::StopClipboard { .. }
                | Self::StopFile { .. }
                | Self::StopRender { .. }
        )
    }

    /// Resource identity bound by the command digest.
    pub fn resource_id(&self) -> &[u8; 16] {
        match self {
            Self::StartCapture { resource_id, .. }
            | Self::StopCapture { resource_id }
            | Self::StartInput { resource_id, .. }
            | Self::StopInput { resource_id }
            | Self::StartAudio { resource_id, .. }
            | Self::StopAudio { resource_id }
            | Self::StartClipboard { resource_id }
            | Self::StopClipboard { resource_id }
            | Self::StartFile { resource_id, .. }
            | Self::StopFile { resource_id }
            | Self::StartRender { resource_id, .. }
            | Self::StopRender { resource_id } => resource_id,
        }
    }
}

/// A command plus its signed authorization grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecuteCommand {
    /// Service-assigned transport correlation token.
    ///
    /// This field is deliberately excluded from [`Self::command_digest`], so a
    /// retry attempt does not change the signed authorization command.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Per-command idempotency identifier.
    pub command_id: [u8; 16],
    /// Authorization proof bound to this command.
    pub grant: ExecuteGrant,
    /// Product operation to execute.
    pub command: AgentCommand,
}

impl ExecuteCommand {
    /// Digest that must match the grant claim.
    pub fn command_digest(&self) -> [u8; 32] {
        let mut context = DigestContext::new(&SHA256);
        context.update(b"mrd-agent-execute-command-v1\0");
        context.update(&self.command_id);
        context.update(&self.command.digest());
        let digest = context.finish();
        let mut output = [0_u8; 32];
        output.copy_from_slice(digest.as_ref());
        output
    }

    /// Permission scopes required by the command.
    pub fn required_scopes(&self) -> PermissionScopes {
        self.command.required_scopes()
    }

    /// Desktop capability required by the enclosed command family.
    pub fn required_capability(&self) -> AgentCapability {
        self.command.required_capability()
    }

    /// Whether this command is teardown-only cleanup.
    pub fn is_cleanup(&self) -> bool {
        self.command.is_cleanup()
    }
}

/// Context common to lifecycle messages emitted by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentEventContext {
    /// Active registration.
    pub registration_id: [u8; 16],
    /// Registration generation.
    pub registration_epoch: u64,
    /// Interactive session from which the event originated.
    pub windows_session_id: u32,
    /// Current desktop generation.
    pub desktop_epoch: u64,
    /// Monotonic event sequence for this registration.
    pub sequence: u64,
    /// Observation time.
    pub observed_at_ms: u64,
}

/// Kind of interactive desktop currently active in a Windows session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopKind {
    /// Normal user desktop.
    Default,
    /// Secure desktop, such as a UAC consent surface.
    Secure,
    /// Logon or lock-screen desktop.
    Winlogon,
    /// Desktop kind is not recognized by this protocol version.
    Unknown,
}

/// Notification that the agent's input desktop changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopChanged {
    /// Common event binding.
    pub context: AgentEventContext,
    /// Desktop generation superseded by this event.
    pub previous_desktop_epoch: u64,
    /// Newly active desktop.
    pub desktop: DesktopKind,
}

/// Notification that the interactive session locked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Locked {
    /// Common event binding.
    pub context: AgentEventContext,
}

/// Notification that the interactive session unlocked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Unlocked {
    /// Common event binding.
    pub context: AgentEventContext,
}

/// Reason the agent began a graceful shutdown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoppingReason {
    /// Machine service requested shutdown.
    ServiceRequest,
    /// Interactive user is logging off.
    UserLogoff,
    /// Windows session is ending.
    SessionEnding,
    /// Agent is being replaced during an upgrade.
    Upgrade,
    /// Agent cannot continue safely.
    FatalError,
}

/// Graceful agent shutdown notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentStopping {
    /// Common event binding.
    pub context: AgentEventContext,
    /// Shutdown reason.
    pub reason: StoppingReason,
}

/// Unexpected agent termination notification.
///
/// This message can be synthesized by a platform supervisor when the terminated
/// process can no longer emit its own event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentCrashed {
    /// Common event binding.
    pub context: AgentEventContext,
    /// Platform exit code, when available.
    pub exit_code: Option<i32>,
}

/// Liveness signal from a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentHeartbeat {
    /// Common event binding.
    pub context: AgentEventContext,
}

/// Codec carried by an encoded media access unit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaCodec {
    /// H.264/AVC video.
    H264,
    /// H.265/HEVC video.
    Hevc,
    /// AV1 video.
    Av1,
}

/// One grant-bound encoded media access unit emitted by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaAccessUnit {
    /// Lifecycle context binding this unit to the registered agent.
    pub context: AgentEventContext,
    /// Authorized capture/render resource.
    pub resource_id: [u8; 16],
    /// Monotonic sequence within the resource.
    pub sequence: u64,
    /// Presentation timestamp in microseconds.
    pub timestamp_us: u64,
    /// Encoded video codec.
    pub codec: MediaCodec,
    /// Whether this unit is an intra-coded keyframe.
    pub is_keyframe: bool,
    /// Encoded payload bytes.
    pub payload: Vec<u8>,
}

impl MediaAccessUnit {
    /// Return whether identifiers, sequences, and payload bounds are valid.
    pub fn is_valid(&self) -> bool {
        self.resource_id != [0; 16]
            && self.sequence > 0
            && self.context.sequence > 0
            && !self.payload.is_empty()
            && self.payload.len() <= AGENT_IPC_MAX_MEDIA_ACCESS_UNIT_BYTES
    }
}

/// Outcome of an agent command.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    /// Command completed successfully.
    Completed,
    /// Command was rejected before product work began.
    Rejected,
    /// Command began but failed.
    Failed,
    /// Cleanup target was already absent.
    AlreadyStopped,
}

/// Completion notification for a service command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    /// Exact transport token copied from the execute command.
    #[serde(deserialize_with = "request_token::deserialize")]
    pub request_token: u64,
    /// Registration that processed the command.
    pub registration_id: [u8; 16],
    /// Command being completed.
    pub command_id: [u8; 16],
    /// Final outcome.
    pub outcome: CommandOutcome,
    /// Completion time.
    pub completed_at_ms: u64,
}

/// Reason the machine service requests agent shutdown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Machine service itself is shutting down.
    ServiceShutdown,
    /// Interactive Windows session is ending.
    SessionEnding,
    /// Agent is superseded by a new registration.
    Replaced,
    /// Product is being upgraded.
    Upgrade,
    /// Policy no longer permits the agent to run.
    PolicyChange,
}

/// Graceful shutdown request from the machine service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StopAgent {
    /// Idempotency identifier for the stop request.
    pub request_id: [u8; 16],
    /// Absolute shutdown deadline.
    pub deadline_ms: u64,
    /// Shutdown reason.
    pub reason: StopReason,
}

/// Messages emitted by the interactive-session agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentToService {
    /// Begin authenticated registration.
    AgentRegister(AgentRegister),
    /// Complete the challenge-response handshake.
    AgentRegistered(AgentRegistered),
    /// Report current product capabilities.
    AgentCapabilitySnapshot(AgentCapabilitySnapshot),
    /// Report an interactive consent decision.
    ConsentResult(ConsentResult),
    /// Report a desktop transition.
    DesktopChanged(DesktopChanged),
    /// Report session lock.
    Locked(Locked),
    /// Report session unlock.
    Unlocked(Unlocked),
    /// Report graceful shutdown.
    AgentStopping(AgentStopping),
    /// Report or synthesize an unexpected termination.
    AgentCrashed(AgentCrashed),
    /// Report liveness.
    AgentHeartbeat(AgentHeartbeat),
    /// Report command completion.
    CommandResult(CommandResult),
    /// Acknowledge one resource-bound input event.
    InputAck(InputAck),
    /// Deliver one grant-bound encoded media access unit.
    MediaAccessUnit(MediaAccessUnit),
}

/// Messages emitted by the machine service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServiceToAgent {
    /// Challenge an agent registration.
    AgentChallenge(AgentChallenge),
    /// Ask the interactive user for consent.
    ConsentRequest(ConsentRequest),
    /// Withdraw one consent prompt already delivered on this exact connection.
    CancelConsent(CancelConsent),
    /// Execute one grant-bearing product command.
    Execute(Box<ExecuteCommand>),
    /// Deliver one event to a previously authorized input resource.
    InputEvent(InputEventEnvelope),
    /// Request graceful agent shutdown.
    StopAgent(StopAgent),
}

/// Signature verification boundary for registration proofs.
pub trait RegistrationProofVerifier {
    /// Verify `signature` over the canonical registration transcript.
    fn verify(&self, agent_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool;
}

/// Immutable identity established by a completed registration handshake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisteredAgentIdentity {
    /// Agent process instance.
    pub agent_instance_id: [u8; 16],
    /// Process identifier supplied at connection time.
    pub process_id: u32,
    /// Process creation time supplied at connection time.
    pub process_creation_time: u64,
    /// Hash of the interactive logon SID.
    pub logon_sid_hash: [u8; 32],
    /// Bound interactive Windows session.
    pub windows_session_id: u32,
    /// Pre-trusted agent signing key identity.
    pub agent_key_id: [u8; 32],
    /// Service registration identity.
    pub registration_id: [u8; 16],
    /// Service registration generation.
    pub registration_epoch: u64,
    /// Negotiated protocol major.
    pub protocol_major: u16,
    /// Negotiated protocol minor.
    pub protocol_minor: u16,
}

/// Registration protocol failures.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RegistrationError {
    /// A register message contains zero sentinel identity or nonce fields.
    #[error("agent registration identity has an invalid shape")]
    InvalidRegistrationShape,
    /// This connection has already submitted an immutable registration identity.
    #[error("duplicate agent registration")]
    DuplicateRegistration,
    /// A challenge was issued before an agent registered.
    #[error("agent registration is required before issuing a challenge")]
    RegistrationRequired,
    /// This connection already has a live challenge.
    #[error("registration challenge already issued")]
    ChallengeAlreadyIssued,
    /// No challenge is available for completion.
    #[error("registration challenge is required")]
    ChallengeRequired,
    /// A previous completion attempt already consumed the challenge.
    #[error("registration challenge has already been consumed")]
    ChallengeConsumed,
    /// Challenge identity fields do not match the immutable registration.
    #[error("registration challenge does not match agent identity")]
    ChallengeIdentityMismatch,
    /// The challenge expiry is not later than its issue time.
    #[error("registration challenge has an invalid time window")]
    InvalidChallengeWindow,
    /// A challenge contains zero sentinel identity or nonce fields.
    #[error("agent registration challenge has an invalid shape")]
    InvalidChallengeShape,
    /// A challenge remains valid beyond the protocol safety bound.
    #[error("agent registration challenge lifetime exceeds the maximum")]
    ChallengeLifetimeExceeded,
    /// Completion was attempted before the challenge became valid.
    #[error("registration challenge is not active yet")]
    ChallengeNotYetValid,
    /// Completion occurred at or after the exclusive challenge expiry.
    #[error("registration challenge expired")]
    ChallengeExpired,
    /// Proof identifiers do not match the one-shot challenge.
    #[error("registration proof does not match the challenge")]
    ProofMismatch,
    /// The proof signature timestamp lies outside the challenge interval.
    #[error("registration proof timestamp is outside the challenge window")]
    ProofTimestampOutsideWindow,
    /// The proof claims to have been signed after the verifier's current time.
    #[error("registration proof timestamp is in the future")]
    ProofTimestampInFuture,
    /// The proof selected an unsupported protocol version.
    #[error("unsupported registered protocol version {major}.{minor}")]
    UnsupportedProtocol {
        /// Selected major version.
        major: u16,
        /// Selected minor version.
        minor: u16,
    },
    /// The trusted verifier rejected the signature.
    #[error("invalid registration proof signature")]
    InvalidSignature,
    /// This connection has already completed registration.
    #[error("agent is already registered")]
    AlreadyRegistered,
}

#[derive(Debug, Clone)]
enum RegistrationPhase {
    Empty,
    AwaitingChallenge(AgentRegister),
    ChallengeIssued {
        register: AgentRegister,
        challenge: AgentChallenge,
    },
    ChallengeConsumed(AgentRegister),
    Registered(RegisteredAgentIdentity),
}

/// Per-connection registration state machine.
#[derive(Debug, Clone)]
pub struct AgentProtocolState {
    phase: RegistrationPhase,
}

impl Default for AgentProtocolState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProtocolState {
    /// Create an unregistered protocol state.
    pub fn new() -> Self {
        Self {
            phase: RegistrationPhase::Empty,
        }
    }

    /// Pin the immutable identity supplied by the first register message.
    pub fn accept_register(&mut self, register: AgentRegister) -> Result<(), RegistrationError> {
        if register.agent_instance_id.iter().all(|byte| *byte == 0)
            || register.process_id == 0
            || register.process_creation_time == 0
            || register.logon_sid_hash.iter().all(|byte| *byte == 0)
            || register.windows_session_id == 0
            || register.agent_key_id.iter().all(|byte| *byte == 0)
            || register.agent_nonce.iter().all(|byte| *byte == 0)
        {
            return Err(RegistrationError::InvalidRegistrationShape);
        }

        match self.phase {
            RegistrationPhase::Empty => {
                self.phase = RegistrationPhase::AwaitingChallenge(register);
                Ok(())
            }
            _ => Err(RegistrationError::DuplicateRegistration),
        }
    }

    /// Issue one challenge after verifying it targets the pinned identity.
    pub fn issue_challenge(&mut self, challenge: AgentChallenge) -> Result<(), RegistrationError> {
        let register = match &self.phase {
            RegistrationPhase::Empty => return Err(RegistrationError::RegistrationRequired),
            RegistrationPhase::AwaitingChallenge(register) => register,
            RegistrationPhase::ChallengeIssued { .. } => {
                return Err(RegistrationError::ChallengeAlreadyIssued)
            }
            RegistrationPhase::ChallengeConsumed(_) => {
                return Err(RegistrationError::ChallengeConsumed)
            }
            RegistrationPhase::Registered(_) => return Err(RegistrationError::AlreadyRegistered),
        };

        if challenge.expires_at_ms <= challenge.issued_at_ms {
            return Err(RegistrationError::InvalidChallengeWindow);
        }
        if challenge.registration_id.iter().all(|byte| *byte == 0)
            || challenge.registration_epoch == 0
            || challenge.challenge_id.iter().all(|byte| *byte == 0)
            || challenge.challenge_nonce.iter().all(|byte| *byte == 0)
            || challenge.issued_at_ms == 0
        {
            return Err(RegistrationError::InvalidChallengeShape);
        }
        if challenge
            .expires_at_ms
            .saturating_sub(challenge.issued_at_ms)
            > AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS
        {
            return Err(RegistrationError::ChallengeLifetimeExceeded);
        }
        if challenge.expected_agent_instance_id != register.agent_instance_id
            || challenge.expected_process_id != register.process_id
            || challenge.expected_process_creation_time != register.process_creation_time
            || challenge.expected_logon_sid_hash != register.logon_sid_hash
            || challenge.expected_windows_session_id != register.windows_session_id
        {
            return Err(RegistrationError::ChallengeIdentityMismatch);
        }

        let register = register.clone();
        self.phase = RegistrationPhase::ChallengeIssued {
            register,
            challenge,
        };
        Ok(())
    }

    /// Consume the outstanding challenge and establish a registered identity.
    ///
    /// A challenge is consumed even when validation fails. A failed proof must
    /// reconnect and start a new handshake; it cannot be retried as an oracle.
    pub fn complete_registration<V: RegistrationProofVerifier + ?Sized>(
        &mut self,
        proof: AgentRegistered,
        now_ms: u64,
        verifier: &V,
    ) -> Result<RegisteredAgentIdentity, RegistrationError> {
        let previous = std::mem::replace(&mut self.phase, RegistrationPhase::Empty);
        let (register, challenge) = match previous {
            RegistrationPhase::Empty => {
                self.phase = RegistrationPhase::Empty;
                return Err(RegistrationError::ChallengeRequired);
            }
            RegistrationPhase::AwaitingChallenge(register) => {
                self.phase = RegistrationPhase::AwaitingChallenge(register);
                return Err(RegistrationError::ChallengeRequired);
            }
            RegistrationPhase::ChallengeIssued {
                register,
                challenge,
            } => (register, challenge),
            RegistrationPhase::ChallengeConsumed(register) => {
                self.phase = RegistrationPhase::ChallengeConsumed(register);
                return Err(RegistrationError::ChallengeConsumed);
            }
            RegistrationPhase::Registered(identity) => {
                self.phase = RegistrationPhase::Registered(identity);
                return Err(RegistrationError::AlreadyRegistered);
            }
        };

        self.phase = RegistrationPhase::ChallengeConsumed(register.clone());

        if now_ms < challenge.issued_at_ms {
            return Err(RegistrationError::ChallengeNotYetValid);
        }
        if now_ms >= challenge.expires_at_ms {
            return Err(RegistrationError::ChallengeExpired);
        }
        if proof.registration_id != challenge.registration_id
            || proof.registration_epoch != challenge.registration_epoch
            || proof.challenge_id != challenge.challenge_id
            || proof.agent_instance_id != register.agent_instance_id
        {
            return Err(RegistrationError::ProofMismatch);
        }
        if proof.signed_at_ms < challenge.issued_at_ms
            || proof.signed_at_ms >= challenge.expires_at_ms
        {
            return Err(RegistrationError::ProofTimestampOutsideWindow);
        }
        if proof.signed_at_ms > now_ms {
            return Err(RegistrationError::ProofTimestampInFuture);
        }
        if proof.accepted_protocol_major != AGENT_IPC_PROTOCOL_MAJOR
            || proof.accepted_protocol_minor > AGENT_IPC_PROTOCOL_MINOR
        {
            return Err(RegistrationError::UnsupportedProtocol {
                major: proof.accepted_protocol_major,
                minor: proof.accepted_protocol_minor,
            });
        }
        if proof.signature.iter().all(|byte| *byte == 0) {
            return Err(RegistrationError::InvalidSignature);
        }

        let signing_bytes = registration_proof_signing_bytes(&register, &challenge, &proof);
        if !verifier.verify(&register.agent_key_id, &signing_bytes, &proof.signature) {
            return Err(RegistrationError::InvalidSignature);
        }

        let identity = RegisteredAgentIdentity {
            agent_instance_id: register.agent_instance_id,
            process_id: register.process_id,
            process_creation_time: register.process_creation_time,
            logon_sid_hash: register.logon_sid_hash,
            windows_session_id: register.windows_session_id,
            agent_key_id: register.agent_key_id,
            registration_id: challenge.registration_id,
            registration_epoch: challenge.registration_epoch,
            protocol_major: proof.accepted_protocol_major,
            protocol_minor: proof.accepted_protocol_minor,
        };
        self.phase = RegistrationPhase::Registered(identity.clone());
        Ok(identity)
    }

    /// Whether this connection completed registration successfully.
    pub fn is_registered(&self) -> bool {
        matches!(self.phase, RegistrationPhase::Registered(_))
    }

    /// Return the immutable registered identity, if established.
    pub fn registered_identity(&self) -> Option<&RegisteredAgentIdentity> {
        match &self.phase {
            RegistrationPhase::Registered(identity) => Some(identity),
            _ => None,
        }
    }
}

/// Canonical, domain-separated bytes covered by an [`AgentRegistered`] signature.
pub fn registration_proof_signing_bytes(
    register: &AgentRegister,
    challenge: &AgentChallenge,
    proof: &AgentRegistered,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(384);
    bytes.extend_from_slice(b"mrd-agent-registration-v1\0");

    bytes.extend_from_slice(&register.agent_instance_id);
    bytes.extend_from_slice(&register.process_id.to_le_bytes());
    bytes.extend_from_slice(&register.process_creation_time.to_le_bytes());
    bytes.extend_from_slice(&register.logon_sid_hash);
    bytes.extend_from_slice(&register.windows_session_id.to_le_bytes());
    bytes.extend_from_slice(&register.agent_key_id);
    bytes.extend_from_slice(&register.agent_nonce);

    bytes.extend_from_slice(&challenge.registration_id);
    bytes.extend_from_slice(&challenge.registration_epoch.to_le_bytes());
    bytes.extend_from_slice(&challenge.challenge_id);
    bytes.extend_from_slice(&challenge.challenge_nonce);
    bytes.extend_from_slice(&challenge.expected_agent_instance_id);
    bytes.extend_from_slice(&challenge.expected_process_id.to_le_bytes());
    bytes.extend_from_slice(&challenge.expected_process_creation_time.to_le_bytes());
    bytes.extend_from_slice(&challenge.expected_logon_sid_hash);
    bytes.extend_from_slice(&challenge.expected_windows_session_id.to_le_bytes());
    bytes.extend_from_slice(&challenge.issued_at_ms.to_le_bytes());
    bytes.extend_from_slice(&challenge.expires_at_ms.to_le_bytes());

    bytes.extend_from_slice(&proof.registration_id);
    bytes.extend_from_slice(&proof.registration_epoch.to_le_bytes());
    bytes.extend_from_slice(&proof.challenge_id);
    bytes.extend_from_slice(&proof.agent_instance_id);
    bytes.extend_from_slice(&proof.accepted_protocol_major.to_le_bytes());
    bytes.extend_from_slice(&proof.accepted_protocol_minor.to_le_bytes());
    bytes.extend_from_slice(&proof.signed_at_ms.to_le_bytes());
    bytes
}
