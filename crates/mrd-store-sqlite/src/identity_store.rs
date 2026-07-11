use crate::{integrity, PersistentStore, SecretProtector, StoreError};
use mrd_identity::{public_key_id, DeviceIdentity};
use ring::digest;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

const IDENTITY_PURPOSE: &[u8] = b"MRD_MACHINE_IDENTITY_PKCS8_V1";
const IDENTITY_COMMITMENT_DOMAIN: &[u8] = b"MRD_IDENTITY_COMMITMENT_V2";

struct StoredIdentity {
    key_id: String,
    epoch: u64,
    public_key: Vec<u8>,
    protected_pkcs8: Vec<u8>,
    created_at_ms: u64,
}

impl PersistentStore {
    /// Initializes the machine identity exactly once.
    pub fn save_identity(&self, identity: &DeviceIdentity) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        if meta.identity_initialized {
            return Err(StoreError::IdentityAlreadyInitialized);
        }
        if query_identity(&transaction)?.is_some() {
            return Err(StoreError::InvalidIdentity);
        }
        let protected = self
            .protector
            .protect(
                &integrity::purpose_with_store_id(IDENTITY_PURPOSE, &meta.store_id),
                identity.private_pkcs8(),
            )
            .map_err(StoreError::SecretProtection)?;
        transaction.execute(
            "INSERT INTO machine_identity(singleton, key_id, epoch, public_key, protected_pkcs8, created_at_ms)
             VALUES (1, ?1, 1, ?2, ?3, CAST(unixepoch('subsec') * 1000 AS INTEGER))",
            params![identity.key_id(), identity.public_key(), protected],
        )?;
        let stored = query_identity(&transaction)?.ok_or(StoreError::InvalidIdentity)?;
        validate_identity_metadata(&stored)?;
        meta.identity_initialized = true;
        meta.identity_commitment = identity_commitment(&stored);
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(())
    }

    /// Loads and verifies the protected machine identity.
    pub fn load_identity(&self) -> Result<DeviceIdentity, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let (meta, _) = self.verify_store_snapshot_connection(&transaction)?;
        let stored = query_identity(&transaction)?.ok_or(StoreError::InvalidIdentity)?;
        let identity = load_identity_from_row(self.protector.as_ref(), &meta.store_id, &stored)?;
        transaction.commit()?;
        Ok(identity)
    }

    /// Returns the sealed monotonic epoch for the current machine signing key.
    pub fn load_identity_epoch(&self) -> Result<u64, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        self.verify_store_snapshot_connection(&transaction)?;
        let stored = query_identity(&transaction)?.ok_or(StoreError::InvalidIdentity)?;
        validate_identity_metadata(&stored)?;
        let epoch = stored.epoch;
        transaction.commit()?;
        Ok(epoch)
    }

    /// Generates an identity only for a genuinely uninitialized sealed store.
    pub fn load_or_create_identity<F>(&self, create: F) -> Result<DeviceIdentity, StoreError>
    where
        F: FnOnce() -> Result<DeviceIdentity, StoreError>,
    {
        let initialized = {
            let mut connection = self.connection();
            let transaction = connection.transaction()?;
            let (meta, _) = self.verify_store_snapshot_connection(&transaction)?;
            transaction.commit()?;
            meta.identity_initialized
        };
        if initialized {
            return self.load_identity();
        }
        let identity = create()?;
        match self.save_identity(&identity) {
            Ok(()) => Ok(identity),
            Err(StoreError::IdentityAlreadyInitialized) => self.load_identity(),
            Err(error) => Err(error),
        }
    }
}

pub(crate) fn verify_identity_snapshot(
    connection: &Connection,
    protector: &dyn SecretProtector,
    meta: &integrity::StoreMeta,
) -> Result<(), StoreError> {
    let stored = query_identity(connection)?;
    match (meta.identity_initialized, stored) {
        (false, None) if meta.identity_commitment == uninitialized_identity_commitment() => Ok(()),
        (true, Some(stored)) => {
            validate_identity_metadata(&stored)?;
            if meta.identity_commitment != identity_commitment(&stored) {
                return Err(StoreError::StoreIntegrity);
            }
            load_identity_from_row(protector, &meta.store_id, &stored).map(|_| ())
        }
        _ => Err(StoreError::StoreIntegrity),
    }
}

pub(crate) fn uninitialized_identity_commitment() -> Vec<u8> {
    digest::digest(&digest::SHA256, IDENTITY_COMMITMENT_DOMAIN)
        .as_ref()
        .to_vec()
}

fn query_identity(connection: &Connection) -> Result<Option<StoredIdentity>, StoreError> {
    connection
        .query_row(
            "SELECT key_id, epoch, public_key, protected_pkcs8, created_at_ms
             FROM machine_identity WHERE singleton = 1",
            [],
            |row| {
                Ok(StoredIdentity {
                    key_id: row.get(0)?,
                    epoch: row.get(1)?,
                    public_key: row.get(2)?,
                    protected_pkcs8: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::Database)
}

fn validate_identity_metadata(stored: &StoredIdentity) -> Result<(), StoreError> {
    if stored.epoch == 0
        || stored.public_key.len() != 32
        || stored.protected_pkcs8.is_empty()
        || stored.created_at_ms == 0
        || public_key_id(&stored.public_key) != stored.key_id
    {
        return Err(StoreError::InvalidIdentity);
    }
    Ok(())
}

fn load_identity_from_row(
    protector: &dyn SecretProtector,
    store_id: &[u8],
    stored: &StoredIdentity,
) -> Result<DeviceIdentity, StoreError> {
    let plaintext = protector
        .unprotect(
            &integrity::purpose_with_store_id(IDENTITY_PURPOSE, store_id),
            &stored.protected_pkcs8,
        )
        .map_err(StoreError::SecretProtection)?;
    let identity =
        DeviceIdentity::from_pkcs8(plaintext.as_ref()).map_err(|_| StoreError::InvalidIdentity)?;
    if identity.key_id() != stored.key_id || identity.public_key() != stored.public_key.as_slice() {
        return Err(StoreError::InvalidIdentity);
    }
    Ok(identity)
}

fn identity_commitment(stored: &StoredIdentity) -> Vec<u8> {
    let mut bytes = IDENTITY_COMMITMENT_DOMAIN.to_vec();
    integrity::append_field(&mut bytes, stored.key_id.as_bytes());
    bytes.extend_from_slice(&stored.epoch.to_be_bytes());
    integrity::append_field(&mut bytes, &stored.public_key);
    integrity::append_field(&mut bytes, &stored.protected_pkcs8);
    bytes.extend_from_slice(&stored.created_at_ms.to_be_bytes());
    digest::digest(&digest::SHA256, &bytes).as_ref().to_vec()
}
