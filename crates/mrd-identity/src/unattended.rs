#![allow(missing_docs)]

use ring::{hmac, rand::SecureRandom};
use std::fmt;

pub struct UnattendedCredential {
    secret: [u8; 16],
}

impl fmt::Debug for UnattendedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UnattendedCredential(REDACTED)")
    }
}

impl UnattendedCredential {
    /// Construct access material obtained through an authenticated out-of-band enrollment.
    /// The secret is never exposed again by this type.
    pub fn from_secret(secret: [u8; 16]) -> Self {
        Self { secret }
    }

    pub fn generate(rng: &impl SecureRandom) -> Result<Self, ring::error::Unspecified> {
        let mut secret = [0; 16];
        rng.fill(&mut secret)?;
        Ok(Self { secret })
    }

    pub fn prove(&self, transcript: &[u8], nonce: [u8; 16]) -> Vec<u8> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        let mut input = b"MRD_UNATTENDED_PROOF_V1".to_vec();
        input.extend_from_slice(transcript);
        input.extend_from_slice(&nonce);
        hmac::sign(&key, &input).as_ref().to_vec()
    }

    pub fn verify(&self, transcript: &[u8], nonce: [u8; 16], proof: &[u8]) -> bool {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        let mut input = b"MRD_UNATTENDED_PROOF_V1".to_vec();
        input.extend_from_slice(transcript);
        input.extend_from_slice(&nonce);
        hmac::verify(&key, &input, proof).is_ok()
    }
}
