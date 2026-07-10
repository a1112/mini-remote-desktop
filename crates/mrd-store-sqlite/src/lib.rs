//! SQLite-backed machine identity, peer trust, and tamper-evident audit storage.

mod audit_store;
mod identity_store;
mod integrity;
mod migrations;
mod trust_store;

pub use audit_store::{AuditDraft, AuditRecord};
pub use trust_store::{TrustRecord, TrustState};

use rusqlite::Connection;
use std::{
    fmt,
    ops::Deref,
    path::Path,
    sync::{Arc, Mutex},
};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Protects machine secrets before they enter persistent storage.
pub trait SecretProtector: Send + Sync {
    /// Encrypts or OS-protects a secret for a fixed purpose.
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String>;
    /// Unprotects a previously protected secret for the same purpose.
    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String>;
}

/// Secret plaintext that is zeroed when it leaves scope, including error paths.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    /// Wraps plaintext returned by a platform secret protector.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl Deref for SecretBytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes(REDACTED)")
    }
}

/// Persistent storage failures. Secret bytes are never included in messages.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite operation failed.
    #[error("database operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    /// Secret protection or authentication failed.
    #[error("secret protection failed: {0}")]
    SecretProtection(String),
    /// Stored identity is absent.
    #[error("machine identity is missing")]
    MissingIdentity,
    /// Stored identity metadata does not match the protected key.
    #[error("stored machine identity is invalid")]
    InvalidIdentity,
    /// A machine identity was already initialized and cannot be overwritten.
    #[error("machine identity is already initialized")]
    IdentityAlreadyInitialized,
    /// A trust state transition violated revision or terminal-state rules.
    #[error("trust transition rejected: {0}")]
    TrustTransition(String),
    /// Audit chain verification failed at a sequence.
    #[error("audit integrity failed at sequence {sequence}")]
    AuditIntegrity { sequence: u64 },
    /// The sealed store manifest or one of its committed sub-states is invalid.
    #[error("persistent store integrity verification failed")]
    StoreIntegrity,
    /// Database was created by a newer incompatible schema.
    #[error("unsupported database schema version {0}")]
    UnsupportedSchema(u32),
}

/// Transactional store sharing one protected SQLite connection.
pub struct PersistentStore {
    connection: Mutex<Connection>,
    protector: Arc<dyn SecretProtector>,
}

impl PersistentStore {
    /// Opens the database and applies idempotent migrations.
    pub fn open(
        path: impl AsRef<Path>,
        protector: Arc<dyn SecretProtector>,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let is_new = match std::fs::symlink_metadata(path) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => return Err(StoreError::StoreIntegrity),
        };
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let observed_version = migrations::schema_version(&connection)?;
        if observed_version > integrity::STORE_FORMAT_VERSION {
            return Err(StoreError::UnsupportedSchema(observed_version));
        }
        if observed_version != 0 && observed_version != integrity::STORE_FORMAT_VERSION {
            return Err(StoreError::StoreIntegrity);
        }
        migrations::configure(&connection)?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version = migrations::schema_version(&transaction)?;
        if version == 0 {
            if !is_new {
                return Err(StoreError::StoreIntegrity);
            }
            migrations::create_schema(&transaction)?;
            integrity::bootstrap_store(&transaction, protector.as_ref())?;
        } else if version == integrity::STORE_FORMAT_VERSION {
            migrations::validate_schema(&transaction)?;
        } else if version > integrity::STORE_FORMAT_VERSION {
            return Err(StoreError::UnsupportedSchema(version));
        } else {
            return Err(StoreError::StoreIntegrity);
        }
        verify_store_snapshot_connection(&transaction, protector.as_ref())?;
        transaction.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
            protector,
        })
    }

    fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn verify_store_snapshot_connection(
        &self,
        connection: &Connection,
    ) -> Result<(integrity::StoreMeta, SecretBytes), StoreError> {
        verify_store_snapshot_connection(connection, self.protector.as_ref())
    }
}

fn verify_store_snapshot_connection(
    connection: &Connection,
    protector: &dyn SecretProtector,
) -> Result<(integrity::StoreMeta, SecretBytes), StoreError> {
    let (meta, store_key) = integrity::load_verified_meta(connection, protector)?;
    if migrations::schema_commitment(connection)? != meta.schema_commitment {
        return Err(StoreError::StoreIntegrity);
    }
    identity_store::verify_identity_snapshot(connection, protector, &meta)?;
    trust_store::verify_trust_snapshot(connection, &meta)?;
    audit_store::verify_audit_snapshot(connection, protector, &meta)?;
    Ok((meta, store_key))
}

#[cfg(test)]
mod tests {
    use super::SecretBytes;
    use zeroize::ZeroizeOnDrop;

    fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}

    #[test]
    fn secret_bytes_have_a_compiler_resistant_zeroize_drop_contract() {
        assert_zeroize_on_drop::<SecretBytes>();
    }
}
