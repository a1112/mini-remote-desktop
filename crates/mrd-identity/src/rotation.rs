#![allow(missing_docs)]

use crate::{canonical_bytes, DeviceIdentity, IdentityError};
use ring::signature;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RotationError {
    #[error("old key is revoked")]
    OldKeyRevoked,
    #[error("rotation epoch is not increasing")]
    EpochNotIncreasing,
    #[error("invalid rotation signature")]
    InvalidSignature,
    #[error("rotation payload encoding failed")]
    PayloadEncoding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationProof {
    pub new_public_key: Vec<u8>,
    pub epoch: u64,
    pub old_public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl DeviceIdentity {
    pub fn sign_rotation(&self, new_public_key: Vec<u8>, epoch: u64) -> Result<RotationProof, IdentityError> {
        let payload = RotationPayload { new_public_key: new_public_key.clone(), epoch, old_public_key: self.public_key().to_vec() };
        let bytes = canonical_bytes("MRD_KEY_ROTATION_V1", &payload).map_err(|_| IdentityError::PayloadEncoding)?;
        let signature = self.sign_domain_bytes(&bytes)?;
        Ok(RotationProof { new_public_key, epoch, old_public_key: self.public_key().to_vec(), signature })
    }
}

impl RotationProof {
    pub fn verify(&self, old_public_key: &[u8], current_epoch: u64, revoked: bool) -> Result<(), RotationError> {
        if revoked { return Err(RotationError::OldKeyRevoked); }
        if self.epoch <= current_epoch { return Err(RotationError::EpochNotIncreasing); }
        if self.old_public_key != old_public_key { return Err(RotationError::InvalidSignature); }
        let payload = RotationPayload { new_public_key: self.new_public_key.clone(), epoch: self.epoch, old_public_key: self.old_public_key.clone() };
        let bytes = canonical_bytes("MRD_KEY_ROTATION_V1", &payload).map_err(|_| RotationError::InvalidSignature)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, old_public_key).verify(&bytes, &self.signature).map_err(|_| RotationError::InvalidSignature)
    }
}

#[derive(Serialize)]
struct RotationPayload { new_public_key: Vec<u8>, epoch: u64, old_public_key: Vec<u8> }
