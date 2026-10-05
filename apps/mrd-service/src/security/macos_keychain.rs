use super::macos_user_storage::load_master_key;
use mrd_store_sqlite::{AeadSecretProtector, SecretProtector};
use std::sync::Arc;

/// The master key stays in the login Keychain; persisted secrets are AEAD envelopes.
pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    let key = load_master_key()?;
    Ok(Arc::new(AeadSecretProtector::from_key(*key)?))
}
