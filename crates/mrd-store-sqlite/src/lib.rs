//! SQLite-backed machine identity, peer trust, and tamper-evident audit storage.

mod audit_store;
mod identity_store;
mod integrity;
mod migrations;
mod secret_protection;
mod trust_store;

pub use audit_store::{AuditDraft, AuditQuery, AuditRecord};
pub use secret_protection::AeadSecretProtector;
pub use trust_store::{
    AppliedTrustTransition, AuditedTrustTransition, TrustRecord, TrustState,
    TrustTransitionRejection,
};

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
    /// A caller supplied an audit event outside the durable redaction contract.
    #[error("invalid audit event")]
    InvalidAuditEvent,
    /// A caller supplied an audit query outside the bounded query contract.
    #[error("invalid audit query")]
    InvalidAuditQuery,
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
        let observed_version = if !is_new && observed_version == 0 {
            // A concurrent opener can see the file before its creator commits
            // the sealed schema. Read only until that commit: this opener must
            // never bootstrap an existing empty or damaged database itself.
            wait_for_original_store_birth(&connection)?
        } else {
            observed_version
        };
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

fn wait_for_original_store_birth(connection: &Connection) -> Result<u32, StoreError> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StoreError::StoreIntegrity);
        }
        connection.busy_timeout(remaining.min(Duration::from_millis(50)))?;
        match migrations::schema_version(connection) {
            Ok(version) if version != 0 => {
                connection.busy_timeout(Duration::from_secs(5))?;
                return Ok(version);
            }
            Ok(_) => {}
            Err(StoreError::Database(rusqlite::Error::SqliteFailure(error, _)))
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) => {}
            Err(error) => return Err(error),
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(10)),
        );
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
    use super::*;
    use zeroize::ZeroizeOnDrop;

    fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}

    #[test]
    fn secret_bytes_have_a_compiler_resistant_zeroize_drop_contract() {
        assert_zeroize_on_drop::<SecretBytes>();
    }

    fn pending_birth_path() -> std::path::PathBuf {
        pending_birth_path_with_timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
    }

    fn pending_birth_path_with_timestamp(timestamp: u128) -> std::path::PathBuf {
        static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mrd-pending-birth-{}-{}-{}.sqlite",
            std::process::id(),
            timestamp,
            sequence
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        path
    }

    #[test]
    fn concurrent_birth_fixtures_are_independent_with_a_frozen_clock() {
        let workers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| pending_birth_path_with_timestamp(0)))
            .collect();
        // Reap every worker and remove its file even when the regression causes
        // create_new to panic, before reporting the allocation failure.
        let paths: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .collect();
        let distinct: std::collections::HashSet<_> = paths.iter().collect();
        for path in &distinct {
            assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
            std::fs::remove_file(path).unwrap();
        }
        assert_eq!(
            paths.len(),
            8,
            "every concurrent fixture must allocate a file"
        );
        assert_eq!(distinct.len(), 8, "fixtures must never share a file");
    }

    #[test]
    fn opener_waits_for_original_creator_without_initializing_an_existing_file() {
        use std::{sync::mpsc, time::Duration};
        let path = pending_birth_path();
        let protector: Arc<dyn SecretProtector> =
            Arc::new(AeadSecretProtector::from_key([73; 32]).unwrap());
        let (sender, receiver) = mpsc::channel();
        let pending_path = path.clone();
        let pending_protector = protector.clone();
        let pending = std::thread::spawn(move || {
            sender
                .send(PersistentStore::open(pending_path, pending_protector).map(drop))
                .unwrap();
        });
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        // The original file creator completes the same sealed transaction used
        // by open. The observer has no permission to bootstrap this file itself.
        let mut connection = Connection::open(&path).unwrap();
        migrations::configure(&connection).unwrap();
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        migrations::create_schema(&transaction).unwrap();
        integrity::bootstrap_store(&transaction, protector.as_ref()).unwrap();
        transaction.commit().unwrap();
        assert!(receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .is_ok());
        pending.join().unwrap();
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn existing_empty_file_is_never_bootstrapped_by_a_waiting_opener() {
        let path = pending_birth_path();
        let protector: Arc<dyn SecretProtector> =
            Arc::new(AeadSecretProtector::from_key([74; 32]).unwrap());
        assert!(matches!(
            PersistentStore::open(&path, protector),
            Err(StoreError::StoreIntegrity)
        ));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        std::fs::remove_file(path).unwrap();
    }
}
