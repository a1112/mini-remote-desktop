//! Independent service-signed authority for an explicitly paired peer.

use crate::{
    DesktopKind, ExecuteGrantVerifier, GrantAudience, PeerBinding, AGENT_IPC_MAX_IDENTIFIER_BYTES,
};
use mrd_proto::SessionId;
use mrd_session::{PermissionScope, PermissionScopes};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A paired authority never outlives five minutes or the current desktop.
pub const AGENT_PAIRED_AUTHORITY_MAX_LIFETIME_MS: u64 = 300_000;
/// Separate signature domain; an execute grant cannot install an authority.
pub const AGENT_PAIRED_AUTHORITY_SIGNATURE_CONTEXT: &[u8] =
    b"mrd-agent-ipc/paired-session-authority/v1\0";

/// Immutable claims decided by the service's verified durable pairing policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PairedSessionAuthorityClaims {
    /// Registration-lifetime unique replay identity.
    pub authority_id: [u8; 32],
    /// Exact authenticated Agent registration.
    pub registration_id: [u8; 16],
    /// Exact authenticated Agent generation.
    pub registration_epoch: u64,
    /// Exact product session.
    pub session_id: SessionId,
    /// Authenticated controller device and public-key identity.
    pub peer: PeerBinding,
    /// Screen and input scopes allowed for this connection.
    pub scopes: PermissionScopes,
    /// Policy revision used by this session's execute grants.
    pub policy_revision: u64,
    /// Interactive Windows session, never session zero.
    pub windows_session_id: u32,
    /// Current trusted desktop generation.
    pub desktop_epoch: u64,
    /// Must be the unlocked Default desktop.
    pub desktop_kind: DesktopKind,
    /// Issuance time in Unix milliseconds.
    pub issued_at_ms: u64,
    /// Inclusive validity beginning.
    pub not_before_ms: u64,
    /// Exclusive validity end.
    pub expires_at_ms: u64,
    /// Intended local verifier.
    pub audience: GrantAudience,
}

/// Signed, short-lived authority; it contains no persistent credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PairedSessionAuthority {
    /// Signed claims.
    pub claims: PairedSessionAuthorityClaims,
    /// Sole bootstrap-pinned service issuer.
    pub issuer_key_id: [u8; 32],
    /// Ed25519 signature over the separate canonical domain.
    #[serde(with = "crate::protocol::bytes_64")]
    pub signature: [u8; 64],
}

impl PairedSessionAuthority {
    /// Canonical signature bytes, including the issuer identifier.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let claims = serde_json::to_vec(&self.claims).expect("paired claims are serializable");
        let mut bytes = AGENT_PAIRED_AUTHORITY_SIGNATURE_CONTEXT.to_vec();
        bytes.extend_from_slice(&self.issuer_key_id);
        bytes.extend_from_slice(&(claims.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&claims);
        bytes
    }
}

/// OS-derived local state; peer/session/policy are subsequently pinned by installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedSessionAuthorityContext {
    /// Active registration identifier.
    pub registration_id: [u8; 16],
    /// Active registration generation.
    pub registration_epoch: u64,
    /// Active interactive Windows session.
    pub windows_session_id: u32,
    /// Active desktop generation.
    pub desktop_epoch: u64,
    /// Active desktop kind.
    pub desktop_kind: DesktopKind,
    /// Trusted current time.
    pub now_ms: u64,
    /// Bootstrap-pinned service issuer.
    pub expected_issuer_key_id: [u8; 32],
}

/// Static rejection reasons; no presented credential or signature is exposed.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum PairedSessionAuthorityValidationError {
    /// A required identifier, bounded scope, or Default desktop is invalid.
    #[error("paired authority claims are invalid")]
    InvalidClaims,
    /// The configured issuer does not match or verification fails.
    #[error("paired authority signature is invalid")]
    InvalidSignature,
    /// A registration, Windows session, or desktop generation differs.
    #[error("paired authority local binding does not match")]
    BindingMismatch,
    /// The validity interval is malformed, too long, future, or expired.
    #[error("paired authority validity window is invalid")]
    InvalidWindow,
}

/// Check all static issuer-side constraints before signing.
pub fn validate_paired_authority_claims(
    claims: &PairedSessionAuthorityClaims,
) -> Result<(), PairedSessionAuthorityValidationError> {
    use PairedSessionAuthorityValidationError::*;
    if claims.authority_id == [0; 32]
        || claims.registration_id == [0; 16]
        || claims.registration_epoch == 0
        || claims.windows_session_id == 0
        || claims.desktop_epoch == 0
        || claims.desktop_kind != DesktopKind::Default
        || claims.policy_revision == 0
        || claims.peer.key_id == [0; 32]
        || claims.session_id.0.is_empty()
        || claims.session_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
        || claims.peer.device_id.0.is_empty()
        || claims.peer.device_id.0.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES
        || !claims.scopes.contains(&PermissionScope::ScreenView)
        || claims.scopes.iter().any(|scope| {
            !matches!(
                scope,
                PermissionScope::ScreenView
                    | PermissionScope::InputPointer
                    | PermissionScope::InputKeyboard
            )
        })
    {
        return Err(InvalidClaims);
    }
    if claims.issued_at_ms == 0
        || claims.not_before_ms < claims.issued_at_ms
        || claims.expires_at_ms <= claims.not_before_ms
        || claims.expires_at_ms.saturating_sub(claims.issued_at_ms)
            > AGENT_PAIRED_AUTHORITY_MAX_LIFETIME_MS
    {
        return Err(InvalidWindow);
    }
    Ok(())
}

/// Independently authenticate the authority against this live Agent and desktop.
pub fn validate_paired_session_authority(
    authority: &PairedSessionAuthority,
    context: &PairedSessionAuthorityContext,
    verifier: &dyn ExecuteGrantVerifier,
) -> Result<(), PairedSessionAuthorityValidationError> {
    use PairedSessionAuthorityValidationError::*;
    validate_paired_authority_claims(&authority.claims)?;
    if context.expected_issuer_key_id == [0; 32]
        || authority.issuer_key_id != context.expected_issuer_key_id
        || !verifier.verify(
            &authority.issuer_key_id,
            &authority.signing_bytes(),
            &authority.signature,
        )
    {
        return Err(InvalidSignature);
    }
    let claims = &authority.claims;
    if claims.registration_id != context.registration_id
        || claims.registration_epoch != context.registration_epoch
        || claims.windows_session_id != context.windows_session_id
        || claims.desktop_epoch != context.desktop_epoch
        || claims.desktop_kind != context.desktop_kind
        || context.desktop_kind != DesktopKind::Default
    {
        return Err(BindingMismatch);
    }
    if context.now_ms < claims.not_before_ms || context.now_ms >= claims.expires_at_ms {
        return Err(InvalidWindow);
    }
    Ok(())
}

/// Correlated install request, separate from any attended consent decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallPairedSessionAuthority {
    /// Nonzero service correlation token.
    pub request_token: u64,
    /// Fresh signed authority.
    pub authority: PairedSessionAuthority,
}

/// Exact connection-local revocation, accepted only from the authenticated service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RevokePairedSessionAuthority {
    /// Nonzero service correlation token.
    pub request_token: u64,
    /// Exact authority replay identity.
    pub authority_id: [u8; 32],
    /// Exact registration identifier.
    pub registration_id: [u8; 16],
    /// Exact registration generation.
    pub registration_epoch: u64,
    /// Exact product session.
    pub session_id: SessionId,
}

/// Only a positive correlated acknowledgement allows the service to proceed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairedSessionAuthorityOutcome {
    /// A fresh authority was independently validated and installed.
    Installed,
    /// The exact authority and all its resources were removed.
    Revoked,
    /// The request did not validate or could not safely change authority.
    Rejected,
}

/// Exact immutable correlation for install and revoke replies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PairedSessionAuthorityResult {
    /// Original service correlation token.
    pub request_token: u64,
    /// Exact authority identity.
    pub authority_id: [u8; 32],
    /// Exact registration identifier.
    pub registration_id: [u8; 16],
    /// Exact registration generation.
    pub registration_epoch: u64,
    /// Exact product session.
    pub session_id: SessionId,
    /// Independently verified outcome.
    pub outcome: PairedSessionAuthorityOutcome,
}
