//! Service-owned signing of exact, short-lived session-agent execute grants.

use super::AgentBinding;
use ed25519_dalek::{Signer, SigningKey};
use mrd_agent_ipc::{
    derive_execute_grant_issuer_key_id, validate_paired_authority_claims, AgentCapability,
    AgentCommand, DesktopKind, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, GrantAudience,
    PairedSessionAuthority, PairedSessionAuthorityClaims, PeerBinding,
    AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS,
};
use mrd_proto::SessionId;
use mrd_session::PermissionScopes;
use thiserror::Error;

/// Immutable authorization facts used to issue one command-bound grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteGrantTemplate {
    registration_id: [u8; 16],
    registration_epoch: u64,
    session_id: SessionId,
    peer: PeerBinding,
    scopes: PermissionScopes,
    policy_revision: u64,
    windows_session_id: u32,
    desktop_epoch: u64,
    desktop_kind: DesktopKind,
    issued_at_ms: u64,
    not_before_ms: u64,
    expires_at_ms: u64,
    required_capability: AgentCapability,
}

impl ExecuteGrantTemplate {
    /// Bind trusted authorization facts to one exact registered Agent generation.
    #[allow(clippy::too_many_arguments)]
    pub fn for_binding(
        binding: &AgentBinding,
        session_id: SessionId,
        peer: PeerBinding,
        scopes: PermissionScopes,
        policy_revision: u64,
        desktop_kind: DesktopKind,
        issued_at_ms: u64,
        not_before_ms: u64,
        expires_at_ms: u64,
    ) -> Result<Self, ExecuteGrantIssueError> {
        if *binding.registration_id() == [0; 16]
            || binding.registration_epoch() == 0
            || binding.windows_session_id() == 0
            || binding.desktop_epoch() == 0
            || session_id.0.is_empty()
            || peer.device_id.0.is_empty()
            || peer.key_id == [0; 32]
            || policy_revision == 0
        {
            return Err(ExecuteGrantIssueError::InvalidAuthority);
        }
        validate_window(issued_at_ms, not_before_ms, expires_at_ms)?;
        Ok(Self {
            registration_id: *binding.registration_id(),
            registration_epoch: binding.registration_epoch(),
            session_id,
            peer,
            scopes,
            policy_revision,
            windows_session_id: binding.windows_session_id(),
            desktop_epoch: binding.desktop_epoch(),
            desktop_kind,
            issued_at_ms,
            not_before_ms,
            expires_at_ms,
            required_capability: binding.required_capability(),
        })
    }

    /// Return a copy with a different scope set, useful for policy derivation.
    pub fn with_scopes(mut self, scopes: PermissionScopes) -> Self {
        self.scopes = scopes;
        self
    }

    /// Logical product session authorized by this template.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Whether the immutable claims still match the exact persisted binding.
    pub fn matches_binding(&self, binding: &AgentBinding) -> bool {
        self.registration_id == *binding.registration_id()
            && self.registration_epoch == binding.registration_epoch()
            && self.windows_session_id == binding.windows_session_id()
            && self.desktop_epoch == binding.desktop_epoch()
            && self.required_capability == binding.required_capability()
    }
}

/// Execute-grant issuance failures detected before signing.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ExecuteGrantIssueError {
    /// Issuer seed or trusted authority facts contain sentinel values.
    #[error("execute grant authority is invalid")]
    InvalidAuthority,
    /// The validity interval is empty, inverted, or too long.
    #[error("execute grant validity window is invalid")]
    InvalidWindow,
    /// Command/grant identity uses a zero sentinel.
    #[error("execute command identity is invalid")]
    InvalidCommandIdentity,
    /// Command family does not match the exact capability binding.
    #[error("execute command capability does not match the agent binding")]
    CapabilityMismatch,
    /// Required command permission is absent from the local authority.
    #[error("execute grant is missing a required permission scope")]
    MissingScope,
}

/// Dedicated Ed25519 issuer whose public half is pinned in Agent bootstrap.
pub struct ExecuteGrantIssuer {
    signing_key: SigningKey,
    public_key: [u8; 32],
    key_id: [u8; 32],
}

impl std::fmt::Debug for ExecuteGrantIssuer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecuteGrantIssuer")
            .field("signing_key", &"REDACTED")
            .field("public_key", &self.public_key)
            .field("key_id", &self.key_id)
            .finish()
    }
}

impl ExecuteGrantIssuer {
    /// Sign an independently bounded paired authority with the bootstrap-pinned issuer.
    pub fn issue_paired_authority(
        &self,
        claims: PairedSessionAuthorityClaims,
    ) -> Result<PairedSessionAuthority, ExecuteGrantIssueError> {
        validate_paired_authority_claims(&claims)
            .map_err(|_| ExecuteGrantIssueError::InvalidAuthority)?;
        let mut authority = PairedSessionAuthority {
            claims,
            issuer_key_id: self.key_id,
            signature: [0; 64],
        };
        authority.signature = self.signing_key.sign(&authority.signing_bytes()).to_bytes();
        Ok(authority)
    }
    /// Construct an issuer from protected 32-byte seed material.
    pub fn from_seed(seed: [u8; 32]) -> Option<Self> {
        if seed == [0; 32] {
            return None;
        }
        let signing_key = SigningKey::from_bytes(&seed);
        let public_key = signing_key.verifying_key().to_bytes();
        let key_id = derive_execute_grant_issuer_key_id(&public_key);
        Some(Self {
            signing_key,
            public_key,
            key_id,
        })
    }

    /// Bootstrap-pinned public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// SHA-256 identifier of the bootstrap-pinned public key.
    pub fn key_id(&self) -> [u8; 32] {
        self.key_id
    }

    /// Issue one command digest-bound grant without exposing signing material.
    pub fn issue(
        &self,
        command_id: [u8; 16],
        grant_id: [u8; 32],
        command: AgentCommand,
        template: ExecuteGrantTemplate,
    ) -> Result<ExecuteCommand, ExecuteGrantIssueError> {
        if command_id == [0; 16] || grant_id == [0; 32] {
            return Err(ExecuteGrantIssueError::InvalidCommandIdentity);
        }
        if command.required_capability() != template.required_capability {
            return Err(ExecuteGrantIssueError::CapabilityMismatch);
        }
        if !command.required_scopes().is_subset(&template.scopes) {
            return Err(ExecuteGrantIssueError::MissingScope);
        }
        validate_window(
            template.issued_at_ms,
            template.not_before_ms,
            template.expires_at_ms,
        )?;
        let claims = ExecuteGrantClaims {
            grant_id,
            registration_id: template.registration_id,
            registration_epoch: template.registration_epoch,
            session_id: template.session_id,
            peer: template.peer,
            scopes: template.scopes,
            policy_revision: template.policy_revision,
            windows_session_id: template.windows_session_id,
            desktop_epoch: template.desktop_epoch,
            desktop_kind: template.desktop_kind,
            issued_at_ms: template.issued_at_ms,
            not_before_ms: template.not_before_ms,
            expires_at_ms: template.expires_at_ms,
            command_digest: [0; 32],
            audience: GrantAudience::SessionAgent,
        };
        let grant = ExecuteGrant {
            claims,
            issuer_key_id: self.key_id,
            signature: [0; 64],
        };
        let mut execute = ExecuteCommand {
            request_token: 1,
            command_id,
            grant,
            command,
        };
        // The command identity is part of the semantic authorization envelope;
        // transport request tokens are intentionally outside this digest.
        execute.grant.claims.command_digest = execute.command_digest();
        execute.grant.signature = self
            .signing_key
            .sign(&execute.grant.signing_bytes())
            .to_bytes();
        Ok(execute)
    }
}

fn validate_window(
    issued_at_ms: u64,
    not_before_ms: u64,
    expires_at_ms: u64,
) -> Result<(), ExecuteGrantIssueError> {
    if issued_at_ms == 0
        || not_before_ms < issued_at_ms
        || expires_at_ms <= not_before_ms
        || expires_at_ms.saturating_sub(not_before_ms) > AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS
    {
        Err(ExecuteGrantIssueError::InvalidWindow)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod paired_tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};
    use mrd_proto::DeviceId;
    use mrd_session::PermissionScope;

    #[test]
    fn paired_issuer_uses_existing_pinned_key_and_independent_domain() {
        let issuer = ExecuteGrantIssuer::from_seed([19; 32]).unwrap();
        let claims = PairedSessionAuthorityClaims {
            authority_id: [1; 32],
            registration_id: [2; 16],
            registration_epoch: 3,
            session_id: SessionId("paired-session".into()),
            peer: PeerBinding {
                device_id: DeviceId("peer".into()),
                key_id: [4; 32],
            },
            scopes: [PermissionScope::ScreenView].into_iter().collect(),
            policy_revision: 5,
            windows_session_id: 1,
            desktop_epoch: 6,
            desktop_kind: DesktopKind::Default,
            issued_at_ms: 1000,
            not_before_ms: 1000,
            expires_at_ms: 2000,
            audience: GrantAudience::SessionAgent,
        };
        let authority = issuer.issue_paired_authority(claims.clone()).unwrap();
        assert_eq!(authority.issuer_key_id, issuer.key_id());
        let public = ed25519_dalek::VerifyingKey::from_bytes(&issuer.public_key()).unwrap();
        assert!(public
            .verify(
                &authority.signing_bytes(),
                &Signature::from_bytes(&authority.signature)
            )
            .is_ok());
        assert!(authority
            .signing_bytes()
            .starts_with(mrd_agent_ipc::AGENT_PAIRED_AUTHORITY_SIGNATURE_CONTEXT));
        for change in 0..3 {
            let mut invalid = claims.clone();
            match change {
                0 => invalid.desktop_kind = DesktopKind::Secure,
                1 => invalid.expires_at_ms = 301001,
                _ => {
                    invalid.scopes.insert(PermissionScope::ClipboardRead);
                }
            }
            assert_eq!(
                issuer.issue_paired_authority(invalid),
                Err(ExecuteGrantIssueError::InvalidAuthority)
            );
        }
    }
}
