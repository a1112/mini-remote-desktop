//! Execute-grant wire contracts and authorization checks.

use crate::protocol::{
    AgentCommand, DesktopKind, ExecuteCommand, PeerBinding, AGENT_IPC_MAX_IDENTIFIER_BYTES,
};
use mrd_proto::SessionId;
use mrd_session::PermissionScopes;
use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

/// Domain separator used when an execute grant is signed.
pub const AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT: &[u8] = b"mrd-agent-ipc/execute-grant/v1\0";

/// Longest interval for which an execute grant may be valid.
///
/// Grants authorize one command digest and are deliberately short lived. Cleanup
/// may be performed after expiry, but an excessively long or malformed grant is
/// never accepted for either execution purpose.
pub const AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS: u64 = 5 * 60 * 1_000;

/// The component for which a signed grant was issued.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum GrantAudience {
    /// The interactive-session agent command executor.
    SessionAgent,
}

/// Immutable authorization claims attached to an agent command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecuteGrantClaims {
    /// Unique replay-detection identifier for this grant.
    pub grant_id: [u8; 32],
    /// Registration to which this grant is pinned.
    pub registration_id: [u8; 16],
    /// Monotonic epoch of the pinned registration.
    pub registration_epoch: u64,
    /// Product session authorized by the grant.
    pub session_id: SessionId,
    /// Remote peer identity authorized by the grant.
    pub peer: PeerBinding,
    /// Maximum permission scopes available to the command.
    pub scopes: PermissionScopes,
    /// Local policy revision under which authorization was decided.
    pub policy_revision: u64,
    /// Windows interactive-session identifier to which execution is pinned.
    pub windows_session_id: u32,
    /// Desktop generation to which execution is pinned.
    pub desktop_epoch: u64,
    /// Interactive desktop kind to which execution is pinned.
    pub desktop_kind: DesktopKind,
    /// Time at which the issuer created the grant.
    pub issued_at_ms: u64,
    /// Inclusive beginning of the validity interval.
    pub not_before_ms: u64,
    /// Exclusive end of the validity interval.
    pub expires_at_ms: u64,
    /// Digest of the exact command envelope authorized by the grant.
    pub command_digest: [u8; 32],
    /// Intended verifier of the grant.
    pub audience: GrantAudience,
}

/// A signed execute grant carried by a service-to-agent command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecuteGrant {
    /// Signed authorization claims.
    pub claims: ExecuteGrantClaims,
    /// Identifier of the preconfigured trusted issuer key.
    pub issuer_key_id: [u8; 32],
    /// Fixed-size issuer signature.
    #[serde(
        serialize_with = "serialize_ed25519_signature",
        deserialize_with = "deserialize_ed25519_signature"
    )]
    pub signature: [u8; 64],
}

impl ExecuteGrant {
    /// Produces the canonical, domain-separated byte string covered by the signature.
    ///
    /// `ExecuteGrantClaims` consists solely of structs, scalar values and ordered
    /// sets, so serde's struct-field order plus `BTreeSet` ordering makes this JSON
    /// representation deterministic for protocol version 1. The issuer key id is
    /// also covered, preventing key-id substitution in the signed envelope.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let claims = serde_json::to_vec(&self.claims)
            .expect("execute grant claims contain only infallibly serializable values");
        let mut signing_bytes = Vec::with_capacity(
            AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT.len()
                + self.issuer_key_id.len()
                + std::mem::size_of::<u64>()
                + claims.len(),
        );
        signing_bytes.extend_from_slice(AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT);
        signing_bytes.extend_from_slice(&self.issuer_key_id);
        signing_bytes.extend_from_slice(&(claims.len() as u64).to_le_bytes());
        signing_bytes.extend_from_slice(&claims);
        signing_bytes
    }
}

/// Signature-verification boundary for execute grants.
///
/// Implementations obtain the public key from a local trust store by key id. No
/// key material capable of signing is ever carried by the wire protocol.
pub trait ExecuteGrantVerifier {
    /// Verifies an issuer signature over canonical grant bytes.
    fn verify(&self, issuer_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool;
}

/// Why the command is being authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPurpose {
    /// Start or mutate a product resource; the grant must be currently valid.
    Start,
    /// Release a resource already created by the same bound command.
    ///
    /// Cleanup deliberately remains available after expiry so a clock boundary
    /// cannot strand input, capture, or media resources.
    Cleanup,
}

/// Trusted local state against which an untrusted grant is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// Active agent registration identifier.
    pub registration_id: [u8; 16],
    /// Active agent registration epoch.
    pub registration_epoch: u64,
    /// Active product session.
    pub session_id: SessionId,
    /// Authenticated remote peer.
    pub peer: PeerBinding,
    /// Active local-policy revision.
    pub policy_revision: u64,
    /// Current Windows interactive-session identifier.
    pub windows_session_id: u32,
    /// Current desktop generation.
    pub desktop_epoch: u64,
    /// Current trusted interactive desktop kind.
    pub desktop_kind: DesktopKind,
    /// Trusted current wall-clock time.
    pub now_ms: u64,
    /// Sole issuer key id trusted for this command path.
    pub expected_issuer_key_id: [u8; 32],
}

/// Reason an execute grant failed authorization.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum GrantValidationError {
    /// The configured or presented issuer key id is the all-zero sentinel.
    #[error("execute grant issuer key id is invalid")]
    InvalidIssuerKeyId,
    /// The envelope names a key other than the configured issuer.
    #[error("execute grant issuer does not match the configured issuer")]
    IssuerMismatch,
    /// The cryptographic signature is invalid.
    #[error("execute grant signature is invalid")]
    InvalidSignature,
    /// The grant is intended for a different verifier.
    #[error("execute grant audience is not the session agent")]
    AudienceMismatch,
    /// The unique grant identifier is the all-zero sentinel.
    #[error("execute grant id is invalid")]
    InvalidGrantId,
    /// The registration identifier is the all-zero sentinel.
    #[error("execute grant registration id is invalid")]
    InvalidRegistrationId,
    /// Registration epochs begin at one.
    #[error("execute grant registration epoch is invalid")]
    InvalidRegistrationEpoch,
    /// The product session identifier is empty.
    #[error("execute grant session id is invalid")]
    InvalidSessionId,
    /// The peer device identifier is empty.
    #[error("execute grant peer device id is invalid")]
    InvalidPeerDeviceId,
    /// The peer key identifier is the all-zero sentinel.
    #[error("execute grant peer key id is invalid")]
    InvalidPeerKeyId,
    /// Policy revisions begin at one.
    #[error("execute grant policy revision is invalid")]
    InvalidPolicyRevision,
    /// Interactive Windows session identifiers cannot be session zero.
    #[error("execute grant Windows session id is invalid")]
    InvalidWindowsSessionId,
    /// Desktop epochs begin at one.
    #[error("execute grant desktop epoch is invalid")]
    InvalidDesktopEpoch,
    /// The command digest is the all-zero sentinel.
    #[error("execute grant command digest is invalid")]
    InvalidCommandDigest,
    /// The command id is the all-zero sentinel.
    #[error("execute command id is invalid")]
    InvalidCommandId,
    /// The resource id is the all-zero sentinel.
    #[error("execute command resource id is invalid")]
    InvalidResourceId,
    /// The grant belongs to a different registration.
    #[error("execute grant registration does not match")]
    RegistrationMismatch,
    /// The grant belongs to a stale or future registration epoch.
    #[error("execute grant registration epoch does not match")]
    RegistrationEpochMismatch,
    /// The grant belongs to a different product session.
    #[error("execute grant session does not match")]
    SessionMismatch,
    /// The grant names a different peer device.
    #[error("execute grant peer device does not match")]
    PeerDeviceMismatch,
    /// The grant names a different authenticated peer key.
    #[error("execute grant peer key does not match")]
    PeerKeyMismatch,
    /// The policy changed after the grant was issued.
    #[error("execute grant policy revision does not match")]
    PolicyRevisionMismatch,
    /// The grant targets another Windows interactive session.
    #[error("execute grant Windows session does not match")]
    WindowsSessionMismatch,
    /// The desktop changed after the grant was issued.
    #[error("execute grant desktop epoch does not match")]
    DesktopEpochMismatch,
    /// The signed desktop kind does not match current trusted state.
    #[error("execute grant desktop kind does not match")]
    DesktopKindMismatch,
    /// Ordinary session agents must not start work on non-default desktops.
    #[error("ordinary session agent cannot start commands on this desktop")]
    UnsupportedDesktop,
    /// The grant is attached to a command other than the one it signed.
    #[error("execute grant command digest does not match")]
    CommandMismatch,
    /// The grant lacks at least one scope required by the command.
    #[error("execute grant does not contain every required scope")]
    InsufficientScopes,
    /// No concrete scope was supplied for authorization.
    #[error("the command authorization context has no required scopes")]
    EmptyRequiredScopes,
    /// The signed timestamps do not describe a valid half-open interval.
    #[error("execute grant has an invalid validity interval")]
    InvalidTimeWindow,
    /// The signed validity interval exceeds the protocol safety limit.
    #[error("execute grant validity interval exceeds the maximum lifetime")]
    LifetimeExceeded,
    /// Trusted current time is before the inclusive start of the interval.
    #[error("execute grant is not valid yet")]
    NotYetValid,
    /// Trusted current time reached or passed the exclusive end of the interval.
    #[error("execute grant has expired")]
    Expired,
}

/// Proof that a grant passed every authorization check for one command context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedGrant {
    claims: ExecuteGrantClaims,
}

/// A product command whose signed grant and trusted bindings were validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedCommand {
    command_id: [u8; 16],
    command: AgentCommand,
    grant: AuthorizedGrant,
}

impl AuthorizedCommand {
    /// Return the idempotency identifier covered by the grant's command digest.
    pub fn command_id(&self) -> &[u8; 16] {
        &self.command_id
    }

    /// Return the exact command that passed authorization.
    pub fn command(&self) -> &AgentCommand {
        &self.command
    }

    /// Return the unique grant identifier for replay tracking.
    pub fn grant_id(&self) -> &[u8; 32] {
        self.grant.grant_id()
    }

    /// Return the validated grant proof.
    pub fn grant(&self) -> &AuthorizedGrant {
        &self.grant
    }
}

/// Validate an execute command without trusting command-derived caller input.
///
/// The command digest, required scopes, and start/cleanup purpose are derived
/// inside this function. Callers provide only authenticated connection state.
pub fn validate_execute_command<V>(
    execute: &ExecuteCommand,
    context: &ExecutionContext,
    verifier: &V,
) -> Result<AuthorizedCommand, GrantValidationError>
where
    V: ExecuteGrantVerifier + ?Sized,
{
    if execute.command_id.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidCommandId);
    }
    if execute.command.resource_id().iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidResourceId);
    }

    let purpose = if execute.command.is_cleanup() {
        ExecutionPurpose::Cleanup
    } else {
        ExecutionPurpose::Start
    };
    if purpose == ExecutionPurpose::Start && context.desktop_kind != DesktopKind::Default {
        return Err(GrantValidationError::UnsupportedDesktop);
    }
    let required_scopes = execute.command.required_scopes();
    let grant = validate_execute_grant(
        &execute.grant,
        context,
        execute.command_digest(),
        &required_scopes,
        purpose,
        verifier,
    )?;

    Ok(AuthorizedCommand {
        command_id: execute.command_id,
        command: execute.command.clone(),
        grant,
    })
}

impl AuthorizedGrant {
    /// Returns the unique grant identifier.
    pub fn grant_id(&self) -> &[u8; 32] {
        &self.claims.grant_id
    }

    /// Returns the validated claims for downstream replay protection and audit.
    pub fn claims(&self) -> &ExecuteGrantClaims {
        &self.claims
    }

    /// Returns the scopes authorized by the validated grant.
    pub fn scopes(&self) -> &PermissionScopes {
        &self.claims.scopes
    }
}

/// Validates a signed grant against trusted registration and command state.
///
/// The validity interval is half open: `[not_before_ms, expires_at_ms)`. Cleanup
/// ignores only the final expiry comparison; it still verifies the signature,
/// every identity and policy binding, the command digest, scopes, and the shape
/// and maximum lifetime of the signed interval.
fn validate_execute_grant<V>(
    grant: &ExecuteGrant,
    context: &ExecutionContext,
    command_digest: [u8; 32],
    required_scopes: &PermissionScopes,
    purpose: ExecutionPurpose,
    verifier: &V,
) -> Result<AuthorizedGrant, GrantValidationError>
where
    V: ExecuteGrantVerifier + ?Sized,
{
    if grant.issuer_key_id.iter().all(|byte| *byte == 0)
        || context.expected_issuer_key_id.iter().all(|byte| *byte == 0)
    {
        return Err(GrantValidationError::InvalidIssuerKeyId);
    }
    if grant.issuer_key_id != context.expected_issuer_key_id {
        return Err(GrantValidationError::IssuerMismatch);
    }

    if grant.signature.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidSignature);
    }
    if !verifier.verify(
        &grant.issuer_key_id,
        &grant.signing_bytes(),
        &grant.signature,
    ) {
        return Err(GrantValidationError::InvalidSignature);
    }

    let claims = &grant.claims;
    if claims.audience != GrantAudience::SessionAgent {
        return Err(GrantValidationError::AudienceMismatch);
    }
    if claims.grant_id.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidGrantId);
    }
    if claims.registration_id.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidRegistrationId);
    }
    if claims.registration_epoch == 0 {
        return Err(GrantValidationError::InvalidRegistrationEpoch);
    }
    if claims.session_id.0.is_empty() || claims.session_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
    {
        return Err(GrantValidationError::InvalidSessionId);
    }
    if claims.peer.device_id.0.is_empty()
        || claims.peer.device_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
    {
        return Err(GrantValidationError::InvalidPeerDeviceId);
    }
    if claims.peer.key_id.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidPeerKeyId);
    }
    if claims.policy_revision == 0 {
        return Err(GrantValidationError::InvalidPolicyRevision);
    }
    if claims.windows_session_id == 0 {
        return Err(GrantValidationError::InvalidWindowsSessionId);
    }
    if claims.desktop_epoch == 0 {
        return Err(GrantValidationError::InvalidDesktopEpoch);
    }
    if claims.command_digest.iter().all(|byte| *byte == 0) {
        return Err(GrantValidationError::InvalidCommandDigest);
    }
    if claims.registration_id != context.registration_id {
        return Err(GrantValidationError::RegistrationMismatch);
    }
    if claims.registration_epoch != context.registration_epoch {
        return Err(GrantValidationError::RegistrationEpochMismatch);
    }
    if claims.session_id != context.session_id {
        return Err(GrantValidationError::SessionMismatch);
    }
    if claims.peer.device_id != context.peer.device_id {
        return Err(GrantValidationError::PeerDeviceMismatch);
    }
    if claims.peer.key_id != context.peer.key_id {
        return Err(GrantValidationError::PeerKeyMismatch);
    }
    if claims.policy_revision != context.policy_revision {
        return Err(GrantValidationError::PolicyRevisionMismatch);
    }
    if claims.windows_session_id != context.windows_session_id {
        return Err(GrantValidationError::WindowsSessionMismatch);
    }
    if claims.desktop_epoch != context.desktop_epoch {
        return Err(GrantValidationError::DesktopEpochMismatch);
    }
    if claims.desktop_kind != context.desktop_kind {
        return Err(GrantValidationError::DesktopKindMismatch);
    }
    if claims.command_digest != command_digest {
        return Err(GrantValidationError::CommandMismatch);
    }
    if purpose == ExecutionPurpose::Start && required_scopes.is_empty() {
        return Err(GrantValidationError::EmptyRequiredScopes);
    }
    if !claims.scopes.is_superset(required_scopes) {
        return Err(GrantValidationError::InsufficientScopes);
    }

    if claims.not_before_ms >= claims.expires_at_ms || claims.issued_at_ms >= claims.expires_at_ms {
        return Err(GrantValidationError::InvalidTimeWindow);
    }
    let issuance_lifetime_ms = claims
        .expires_at_ms
        .checked_sub(claims.issued_at_ms)
        .ok_or(GrantValidationError::InvalidTimeWindow)?;
    let validity_lifetime_ms = claims
        .expires_at_ms
        .checked_sub(claims.not_before_ms)
        .ok_or(GrantValidationError::InvalidTimeWindow)?;
    if issuance_lifetime_ms > AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS
        || validity_lifetime_ms > AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS
    {
        return Err(GrantValidationError::LifetimeExceeded);
    }
    if context.now_ms < claims.not_before_ms {
        return Err(GrantValidationError::NotYetValid);
    }
    if purpose == ExecutionPurpose::Start && context.now_ms >= claims.expires_at_ms {
        return Err(GrantValidationError::Expired);
    }

    Ok(AuthorizedGrant {
        claims: claims.clone(),
    })
}

fn serialize_ed25519_signature<S>(signature: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_seq(signature.iter())
}

fn deserialize_ed25519_signature<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
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
