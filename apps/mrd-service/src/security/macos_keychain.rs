use super::macos_user_storage::{authorize_existing_master_key, load_master_key};
use mrd_store_sqlite::{AeadSecretProtector, SecretProtector};
use std::sync::Arc;

/// Authorize this executable through the macOS dialog without starting the service.
/// Only an existing master key is read, validated and immediately erased from memory.
pub fn authorize_keychain_access() -> Result<(), String> {
    authorize_existing_master_key()
}

/// The master key stays in the login Keychain; persisted secrets are AEAD envelopes.
pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    let key = load_master_key()?;
    Ok(Arc::new(AeadSecretProtector::from_key(*key)?))
}
