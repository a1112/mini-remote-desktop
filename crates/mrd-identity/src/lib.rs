#![allow(missing_docs)]

use ring::{digest, rand::SystemRandom, signature, signature::KeyPair};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub use mrd_session::PermissionScope as PermissionScope;

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

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    private_pkcs8: Vec<u8>,
    public_key: Vec<u8>,
    key_id: String,
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
        let key_id = hex_digest(&public_key);
        Ok(Self { private_pkcs8: bytes.to_vec(), public_key, key_id })
    }

    pub fn private_pkcs8(&self) -> &[u8] { &self.private_pkcs8 }
    pub fn public_key(&self) -> &[u8] { &self.public_key }
    pub fn key_id(&self) -> &str { &self.key_id }

    pub fn sign_intent(&self, payload: SessionIntent) -> Result<SignedSessionIntent, IdentityError> {
        let bytes = canonical_bytes("MRD_SESSION_INTENT_V1", &payload)?;
        let pair = signature::Ed25519KeyPair::from_pkcs8(&self.private_pkcs8)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        Ok(SignedSessionIntent { payload, public_key: self.public_key.clone(), signature: pair.sign(&bytes).as_ref().to_vec() })
    }

    pub fn sign_grant(&self, payload: SignedGrantPayload) -> Result<SignedGrant, IdentityError> {
        let bytes = canonical_bytes("MRD_SESSION_GRANT_V1", &payload)?;
        let pair = signature::Ed25519KeyPair::from_pkcs8(&self.private_pkcs8)
            .map_err(|_| IdentityError::InvalidPrivateKey)?;
        Ok(SignedGrant { payload, controller_public_key: self.public_key.clone(), signature: pair.sign(&bytes).as_ref().to_vec() })
    }
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
    pub fn verify_for(&self, controller_public_key: &[u8], target_public_key: &[u8]) -> Result<(), IdentityError> {
        if self.controller_public_key != controller_public_key ||
            self.payload.controller_public_key != controller_public_key ||
            self.payload.target_public_key != target_public_key {
            return Err(IdentityError::PeerBindingMismatch);
        }
        let bytes = canonical_bytes("MRD_SESSION_GRANT_V1", &self.payload)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, controller_public_key)
            .verify(&bytes, &self.signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

fn canonical_bytes<T: Serialize>(domain: &str, payload: &T) -> Result<Vec<u8>, IdentityError> {
    #[derive(Serialize)]
    struct Envelope<'a, T> { version: u8, domain: &'a str, payload: &'a T }
    serde_json::to_vec(&Envelope { version: 1, domain, payload }).map_err(|_| IdentityError::PayloadEncoding)
}

fn hex_digest(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}
