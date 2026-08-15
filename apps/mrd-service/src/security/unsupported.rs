use mrd_store_sqlite::{SecretBytes, SecretProtector};
use std::sync::Arc;

/// Explicit non-Windows placeholder until native machine secret storage is implemented.
pub struct UnsupportedSecretProtector;

impl SecretProtector for UnsupportedSecretProtector {
    fn protect(&self, _purpose: &[u8], _plaintext: &[u8]) -> Result<Vec<u8>, String> {
        Err("machine secret protection is unsupported on this platform".to_owned())
    }

    fn unprotect(&self, _purpose: &[u8], _protected: &[u8]) -> Result<SecretBytes, String> {
        Err("machine secret protection is unsupported on this platform".to_owned())
    }
}

/// Returns an explicit unsupported error instead of a weak production fallback.
pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    Err("machine secret protection is unsupported on this platform".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_unsupported_platform_fails_closed() {
        assert!(platform_secret_protector().is_err());
    }
}
