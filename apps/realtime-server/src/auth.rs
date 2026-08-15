use crate::ConnectionId;
use mrd_proto::{BackendRole, DeviceId};
use mrd_signal_proto::{
    AuthenticatedRegister, ServerChallenge, SignalProtocolError, SignalReplayGuard,
    VerifiedSignalMetadata,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBackendToken {
    pub device_id: DeviceId,
    pub device_key_id: String,
    pub role: BackendRole,
    pub expires_at_ms: u64,
}

pub trait BackendTokenVerifier: Send + Sync {
    fn verify(&self, token: &str, now_ms: u64) -> Result<VerifiedBackendToken, BackendTokenError>;
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum BackendTokenError {
    #[error("backend device token is invalid")]
    Invalid,
    #[error("backend device token verification is unavailable")]
    Unavailable,
}

#[derive(Debug, Default)]
pub struct RejectAllBackendTokens;

impl BackendTokenVerifier for RejectAllBackendTokens {
    fn verify(
        &self,
        _token: &str,
        _now_ms: u64,
    ) -> Result<VerifiedBackendToken, BackendTokenError> {
        Err(BackendTokenError::Unavailable)
    }
}

pub trait ChallengeSource: Send + Sync {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), AuthError>;
}

#[derive(Debug, Default)]
pub struct SystemChallengeSource;

impl ChallengeSource for SystemChallengeSource {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), AuthError> {
        SystemRandom::new()
            .fill(bytes)
            .map_err(|_| AuthError::EntropyUnavailable)
    }
}

#[derive(Debug, Clone)]
struct PendingChallenge {
    challenge: ServerChallenge,
}

pub struct Authenticator {
    server_device_id: DeviceId,
    challenge_ttl_ms: u64,
    token_verifier: Arc<dyn BackendTokenVerifier>,
    challenge_source: Arc<dyn ChallengeSource>,
    pending: HashMap<ConnectionId, PendingChallenge>,
}

impl std::fmt::Debug for Authenticator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Authenticator")
            .field("server_device_id", &self.server_device_id)
            .field("challenge_ttl_ms", &self.challenge_ttl_ms)
            .field("pending_count", &self.pending.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedRegistration {
    pub token: VerifiedBackendToken,
    pub metadata: VerifiedSignalMetadata,
}

impl Authenticator {
    pub fn new(
        server_device_id: DeviceId,
        challenge_ttl_ms: u64,
        token_verifier: Arc<dyn BackendTokenVerifier>,
        challenge_source: Arc<dyn ChallengeSource>,
    ) -> Self {
        Self {
            server_device_id,
            challenge_ttl_ms,
            token_verifier,
            challenge_source,
            pending: HashMap::new(),
        }
    }

    pub fn issue(
        &mut self,
        connection_id: ConnectionId,
        now_ms: u64,
    ) -> Result<ServerChallenge, AuthError> {
        let mut challenge_id = [0_u8; 16];
        let mut challenge_nonce = [0_u8; 32];
        self.challenge_source.fill(&mut challenge_id)?;
        self.challenge_source.fill(&mut challenge_nonce)?;
        if challenge_id == [0; 16] || challenge_nonce == [0; 32] {
            return Err(AuthError::EntropyUnavailable);
        }
        let challenge = ServerChallenge {
            challenge_id,
            challenge_nonce,
            issued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(self.challenge_ttl_ms),
        };
        self.pending.insert(
            connection_id,
            PendingChallenge {
                challenge: challenge.clone(),
            },
        );
        Ok(challenge)
    }

    pub fn authenticate(
        &mut self,
        connection_id: ConnectionId,
        register: &AuthenticatedRegister,
        now_ms: u64,
        replay: &mut SignalReplayGuard,
    ) -> Result<AuthenticatedRegistration, AuthError> {
        let pending = self
            .pending
            .remove(&connection_id)
            .ok_or(AuthError::ChallengeMissing)?;
        if now_ms >= pending.challenge.expires_at_ms {
            return Err(AuthError::ChallengeExpired);
        }
        if register.payload.challenge_id != pending.challenge.challenge_id
            || register.payload.challenge_nonce != pending.challenge.challenge_nonce
        {
            return Err(AuthError::ChallengeMismatch);
        }
        register.verify_for(&self.server_device_id, now_ms, replay)?;
        let token = self
            .token_verifier
            .verify(&register.payload.backend_device_token, now_ms)
            .map_err(AuthError::BackendToken)?;
        if now_ms >= token.expires_at_ms {
            return Err(AuthError::TokenExpired);
        }
        let claims = &register.payload.claims;
        if token.device_id != claims.issuer_device_id
            || token.device_key_id != claims.issuer_key_id
            || token.role != register.payload.role
        {
            return Err(AuthError::TokenBindingMismatch);
        }
        Ok(AuthenticatedRegistration {
            token,
            metadata: VerifiedSignalMetadata {
                issuer_device_id: claims.issuer_device_id.clone(),
                issuer_key_id: claims.issuer_key_id.clone(),
                intended_peer_device_id: claims.intended_peer_device_id.clone(),
                counter: claims.counter,
                nonce: claims.nonce,
            },
        })
    }

    pub fn remove_connection(&mut self, connection_id: ConnectionId) {
        self.pending.remove(&connection_id);
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("registration challenge is missing")]
    ChallengeMissing,
    #[error("registration challenge expired")]
    ChallengeExpired,
    #[error("registration challenge does not match")]
    ChallengeMismatch,
    #[error("backend token failed: {0}")]
    BackendToken(BackendTokenError),
    #[error("backend token expired")]
    TokenExpired,
    #[error("backend token is not bound to the signed device identity")]
    TokenBindingMismatch,
    #[error("challenge entropy is unavailable")]
    EntropyUnavailable,
    #[error(transparent)]
    Protocol(#[from] SignalProtocolError),
}
