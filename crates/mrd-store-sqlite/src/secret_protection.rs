use crate::{SecretBytes, SecretProtector};
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use zeroize::{Zeroize, Zeroizing};

const HEADER: &[u8; 8] = b"MRDSEAL1";
const NONCE_LENGTH: usize = 12;

/// Authenticated secret envelopes for a key obtained from protected OS storage.
/// The adapter deliberately does not persist or generate its own master key.
pub struct AeadSecretProtector {
    key: Zeroizing<[u8; 32]>,
}

impl AeadSecretProtector {
    pub fn from_key(mut key: [u8; 32]) -> Result<Self, String> {
        let protected_key = Zeroizing::new(key);
        key.zeroize();
        Ok(Self { key: protected_key })
    }

    fn cipher(&self) -> Result<LessSafeKey, String> {
        UnboundKey::new(&aead::AES_256_GCM, self.key.as_ref())
            .map(LessSafeKey::new)
            .map_err(|_| "secret protection key is invalid".to_owned())
    }
}

impl SecretProtector for AeadSecretProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "secret protection entropy unavailable".to_owned())?;
        let mut encrypted = Zeroizing::new(plaintext.to_vec());
        self.cipher()?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(purpose),
                &mut *encrypted,
            )
            .map_err(|_| "secret protection failed".to_owned())?;
        let mut sealed = Vec::with_capacity(HEADER.len() + nonce.len() + encrypted.len());
        sealed.extend_from_slice(HEADER);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&encrypted);
        Ok(sealed)
    }

    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        let prefix_length = HEADER.len() + NONCE_LENGTH;
        if protected.len() < prefix_length + aead::AES_256_GCM.tag_len()
            || !protected.starts_with(HEADER)
        {
            return Err("secret envelope is invalid".to_owned());
        }
        let nonce: [u8; NONCE_LENGTH] = protected[HEADER.len()..prefix_length]
            .try_into()
            .map_err(|_| "secret envelope is invalid".to_owned())?;
        let mut encrypted = Zeroizing::new(protected[prefix_length..].to_vec());
        let plaintext_length = self
            .cipher()?
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(purpose),
                &mut encrypted,
            )
            .map_err(|_| "secret envelope authentication failed".to_owned())?
            .len();
        encrypted.truncate(plaintext_length);
        Ok(SecretBytes::new(std::mem::take(&mut *encrypted)))
    }
}
