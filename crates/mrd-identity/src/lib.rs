#![allow(missing_docs)]

use ring::{digest, rand::SystemRandom, signature, signature::KeyPair};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use zeroize::Zeroize;

mod replay;
mod rotation;
mod sas;
mod unattended;

pub use replay::{ReplayError, ReplayWindow};
pub use rotation::{RotationError, RotationProof};
pub use sas::sas_code;
pub use unattended::UnattendedCredential;

pub use mrd_session::PermissionScope;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("key generation failed")]
    KeyGeneration,
    #[error("invalid private key")]
    InvalidPrivateKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("payload encoding failed")]
    PayloadEncoding,
    #[error("grant is not bound to the expected peer keys")]
    PeerBindingMismatch,
}

const CONTEXT_SIGNATURE_PREFIX: &[u8] = b"MRD_CONTEXT_SIGNATURE_V1";

#[derive(Clone)]
pub struct DeviceIdentity {
    private_pkcs8: Vec<u8>,
    public_key: Vec<u8>,
    key_id: String,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceIdentity")
            .field("private_pkcs8", &"REDACTED")
            .field("public_key", &self.public_key)
            .field("key_id", &self.key_id)
            .finish()
    }
}

impl Drop for DeviceIdentity {
    fn drop(&mut self) {
        self.private_pkcs8.zeroize();
    }
}

impl DeviceIdentity {
    pub fn generate(rng: &SystemRandom) -> Result<Self, IdentityError> {
        let pkcs8 = signature::Ed25519KeyPair::generate_pkcs8(rng)
            .map_err(|_| IdentityError::KeyGeneration)?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, IdentityError> {
        let pair = signature::Ed25519KeyPair::from_pkcs8(bytes)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        let public_key = pair.public_key().as_ref().to_vec();
        let key_id = public_key_id(&public_key);
        Ok(Self {
            private_pkcs8: bytes.to_vec(),
            public_key,
            key_id,
        })
    }

    pub fn private_pkcs8(&self) -> &[u8] {
        &self.private_pkcs8
    }
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn sign_intent(
        &self,
        payload: SessionIntent,
    ) -> Result<SignedSessionIntent, IdentityError> {
        let bytes = canonical_bytes("MRD_SESSION_INTENT_V1", &payload)?;
        let pair = signature::Ed25519KeyPair::from_pkcs8(&self.private_pkcs8)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        Ok(SignedSessionIntent {
            payload,
            public_key: self.public_key.clone(),
            signature: pair.sign(&bytes).as_ref().to_vec(),
        })
    }

    pub fn sign_grant(&self, payload: SignedGrantPayload) -> Result<SignedGrant, IdentityError> {
        let bytes = canonical_bytes("MRD_SESSION_GRANT_V1", &payload)?;
        let pair = signature::Ed25519KeyPair::from_pkcs8(&self.private_pkcs8)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        Ok(SignedGrant {
            payload,
            controller_public_key: self.public_key.clone(),
            signature: pair.sign(&bytes).as_ref().to_vec(),
        })
    }

    /// Signs already-canonical payload bytes under an explicit protocol context.
    pub fn sign_context_bytes(
        &self,
        context: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, IdentityError> {
        let bytes = contextual_bytes(context, payload)?;
        self.sign_domain_bytes(&bytes)
    }
}

/// Recomputes the stable SHA-256 key identifier for an Ed25519 public key.
pub fn public_key_id(public_key: &[u8]) -> String {
    hex_digest(public_key)
}

/// Verifies canonical payload bytes under the same explicit protocol context.
pub fn verify_context_bytes(
    public_key: &[u8],
    context: &str,
    payload: &[u8],
    signature_bytes: &[u8],
) -> Result<(), IdentityError> {
    let bytes = contextual_bytes(context, payload)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(&bytes, signature_bytes)
        .map_err(|_| IdentityError::InvalidSignature)
}

fn contextual_bytes(context: &str, payload: &[u8]) -> Result<Vec<u8>, IdentityError> {
    let context_len = u16::try_from(context.len()).map_err(|_| IdentityError::PayloadEncoding)?;
    let payload_len = u64::try_from(payload.len()).map_err(|_| IdentityError::PayloadEncoding)?;
    if context.is_empty() {
        return Err(IdentityError::PayloadEncoding);
    }

    let mut bytes =
        Vec::with_capacity(CONTEXT_SIGNATURE_PREFIX.len() + 2 + context.len() + 8 + payload.len());
    bytes.extend_from_slice(CONTEXT_SIGNATURE_PREFIX);
    bytes.extend_from_slice(&context_len.to_be_bytes());
    bytes.extend_from_slice(context.as_bytes());
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionIntent {
    pub session_id: String,
    pub controller_key_id: String,
    pub target_key_id: String,
    pub requested_scopes: BTreeSet<PermissionScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedSessionIntent {
    pub payload: SessionIntent,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl SignedSessionIntent {
    pub fn verify(&self) -> Result<(), IdentityError> {
        let bytes = canonical_bytes("MRD_SESSION_INTENT_V1", &self.payload)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, &self.public_key)
            .verify(&bytes, &self.signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedGrantPayload {
    pub session_id: String,
    pub controller_public_key: Vec<u8>,
    pub target_public_key: Vec<u8>,
    pub nonce: [u8; 16],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedGrant {
    pub payload: SignedGrantPayload,
    pub controller_public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl SignedGrant {
    pub fn verify_for(
        &self,
        controller_public_key: &[u8],
        target_public_key: &[u8],
    ) -> Result<(), IdentityError> {
        if self.controller_public_key != controller_public_key
            || self.payload.controller_public_key != controller_public_key
            || self.payload.target_public_key != target_public_key
        {
            return Err(IdentityError::PeerBindingMismatch);
        }
        let bytes = canonical_bytes("MRD_SESSION_GRANT_V1", &self.payload)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, controller_public_key)
            .verify(&bytes, &self.signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

pub(crate) fn canonical_bytes<T: Serialize>(
    domain: &str,
    payload: &T,
) -> Result<Vec<u8>, IdentityError> {
    #[derive(Serialize)]
    struct Envelope<'a, T> {
        version: u8,
        domain: &'a str,
        payload: &'a T,
    }
    serde_json::to_vec(&Envelope {
        version: 1,
        domain,
        payload,
    })
    .map_err(|_| IdentityError::PayloadEncoding)
}

impl DeviceIdentity {
    pub(crate) fn sign_domain_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, IdentityError> {
        let pair = signature::Ed25519KeyPair::from_pkcs8(&self.private_pkcs8)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        Ok(pair.sign(bytes).as_ref().to_vec())
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
