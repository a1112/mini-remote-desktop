#![allow(missing_docs)]

use crate::permissions::{PermissionScope, PermissionScopes};
use mrd_proto::{DeviceId, SessionId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GrantError {
    #[error("grant has expired")]
    Expired,
    #[error("scope is not granted")]
    ScopeNotGranted,
    #[error("grant is not valid yet")]
    NotYetValid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGrant {
    pub session_id: SessionId,
    pub peer_id: DeviceId,
    pub scopes: PermissionScopes,
    pub windows_session_id: Option<u32>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub nonce: [u8; 16],
    pub policy_revision: u64,
    pub route_constraint: Option<String>,
    pub profile_constraint: Option<String>,
    pub transport_fingerprint: Option<[u8; 32]>,
    pub signature: Vec<u8>,
}

impl SessionGrant {
    pub fn new(
        session_id: SessionId,
        peer_id: DeviceId,
        scopes: PermissionScopes,
        issued_at: u64,
        expires_at: u64,
        nonce: [u8; 16],
    ) -> Self {
        Self {
            session_id,
            peer_id,
            scopes,
            windows_session_id: None,
            issued_at,
            expires_at,
            nonce,
            policy_revision: 0,
            route_constraint: None,
            profile_constraint: None,
            transport_fingerprint: None,
            signature: Vec::new(),
        }
    }

    pub fn authorize(&self, scope: PermissionScope, now: u64) -> Result<(), GrantError> {
        if now < self.issued_at {
            return Err(GrantError::NotYetValid);
        }
        if now > self.expires_at {
            return Err(GrantError::Expired);
        }
        if !self.scopes.contains(&scope) {
            return Err(GrantError::ScopeNotGranted);
        }
        Ok(())
    }
}
