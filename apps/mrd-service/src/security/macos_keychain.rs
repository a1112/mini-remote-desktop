//! Logged-in-user service identity: Keychain-held wrapping key and sealed SQLite state.
use mrd_store_sqlite::{SecretBytes, SecretProtector};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use security_framework::{os::macos::keychain::SecKeychain, passwords::get_generic_password};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    sync::Arc,
};
use zeroize::Zeroizing;

const SERVICE: &str = "com.a1112.rdesk.service.identity";
const ACCOUNT: &str = "local-session-wrapping-key-v1";
const MAGIC: &[u8] = b"MRDKEY01";

pub fn protected_product_data_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    let parent = PathBuf::from(home).join("Library/Application Support");
    let path = parent.join("mini-remote-desktop");
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(
            "service data directory must be a user-owned, non-symlink directory with mode 0700"
                .into(),
        );
    }
    Ok(path)
}

pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    let bytes = match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => bytes,
        Err(error) if error.code() == -25300 => {
            if protected_product_data_dir()?
                .join("security-state-v2.sqlite3")
                .exists()
            {
                return Err("service identity exists but its Keychain key is missing; restore the key before restarting".into());
            }
            let mut key = Zeroizing::new([0_u8; 32]);
            SystemRandom::new()
                .fill(key.as_mut())
                .map_err(|_| "Keychain key entropy unavailable")?;
            let keychain =
                SecKeychain::default().map_err(|error| format!("open login Keychain: {error}"))?;
            // Add-only: a racing startup must read the winning key, never overwrite it.
            match keychain.add_generic_password(SERVICE, ACCOUNT, key.as_ref()) {
                Ok(_) => {}
                Err(error) if error.code() == -25299 => {}
                Err(error) => return Err(format!("create service Keychain item: {error}")),
            }
            get_generic_password(SERVICE, ACCOUNT)
                .map_err(|error| format!("read service Keychain item: {error}"))?
        }
        Err(error) => {
            return Err(format!(
                "unlock or authorize the login Keychain for Rdesk Service: {error}"
            ))
        }
    };
    let bytes = Zeroizing::new(bytes);
    if bytes.len() != 32 {
        return Err("service Keychain key has an invalid length".into());
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&bytes);
    Ok(Arc::new(KeychainProtector { key }))
}

struct KeychainProtector {
    key: Zeroizing<[u8; 32]>,
}
impl KeychainProtector {
    fn cipher(&self) -> Result<aead::LessSafeKey, String> {
        aead::UnboundKey::new(&aead::AES_256_GCM, self.key.as_ref())
            .map(aead::LessSafeKey::new)
            .map_err(|_| "invalid wrapping key".into())
    }
}
impl SecretProtector for KeychainProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "nonce entropy unavailable")?;
        let mut body = Zeroizing::new(plaintext.to_vec());
        self.cipher()?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(purpose),
                &mut *body,
            )
            .map_err(|_| "secret sealing failed")?;
        Ok([MAGIC, &nonce, body.as_slice()].concat())
    }
    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        if protected.len() < MAGIC.len() + 12 + aead::AES_256_GCM.tag_len()
            || !protected.starts_with(MAGIC)
        {
            return Err("invalid sealed service secret".into());
        }
        let nonce = protected[8..20].try_into().map_err(|_| "invalid nonce")?;
        let mut body = Zeroizing::new(protected[20..].to_vec());
        let plaintext = self
            .cipher()?
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(purpose),
                &mut body,
            )
            .map_err(|_| "service secret authentication failed")?;
        Ok(SecretBytes::new(plaintext.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_envelope_binds_purpose_key_and_ciphertext() {
        let protector = KeychainProtector {
            key: Zeroizing::new([42; 32]),
        };
        let sealed = protector.protect(b"identity", b"private seed").unwrap();
        assert!(protector.unprotect(b"identity", &sealed).is_ok());
        assert!(protector.unprotect(b"different purpose", &sealed).is_err());
        assert!(KeychainProtector {
            key: Zeroizing::new([43; 32])
        }
        .unprotect(b"identity", &sealed)
        .is_err());
        for length in 0..36 {
            assert!(protector.unprotect(b"identity", &sealed[..length]).is_err());
        }
        let mut corrupt = sealed.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(protector.unprotect(b"identity", &corrupt).is_err());
        assert_ne!(
            sealed,
            protector.protect(b"identity", b"private seed").unwrap()
        );
    }
}
