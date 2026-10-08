use super::macos_user_storage::{load_master_key, load_master_key_for_user_start};
use mrd_store_sqlite::{AeadSecretProtector, SecretProtector};
use std::sync::Arc;

/// The master key stays in the login Keychain; persisted secrets are AEAD envelopes.
pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    let key = load_master_key()?;
    Ok(Arc::new(AeadSecretProtector::from_key(*key)?))
}

/// An explicit user start may show the native prompt and retain the authorized
/// key in this service process, even when the user permits a single read.
pub fn user_start_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    let key = load_master_key_for_user_start()?;
    Ok(Arc::new(AeadSecretProtector::from_key(*key)?))
}
