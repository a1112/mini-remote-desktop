use crate::{SecretBytes, SecretProtector, StoreError};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) const STORE_FORMAT_VERSION: u32 = 2;
const STORE_KEY_NAME: &str = "store_integrity_key_v1";
const STORE_KEY_PURPOSE: &[u8] = b"MRD_STORE_INTEGRITY_KEY_V1";
const MANIFEST_DOMAIN: &[u8] = b"MRD_STORE_MANIFEST_V2";

#[derive(Clone)]
pub(crate) struct StoreMeta {
    pub(crate) store_id: Vec<u8>,
    pub(crate) generation: u64,
    pub(crate) schema_commitment: Vec<u8>,
    pub(crate) identity_initialized: bool,
    pub(crate) identity_commitment: Vec<u8>,
    pub(crate) trust_count: u64,
    pub(crate) trust_commitment: Vec<u8>,
    pub(crate) audit_initialized: bool,
    pub(crate) audit_commitment: Vec<u8>,
    manifest_seal: Vec<u8>,
}

pub(crate) fn bootstrap_store(
    connection: &Connection,
    protector: &dyn SecretProtector,
) -> Result<(), StoreError> {
    ensure_empty_security_state(connection)?;
    let mut store_id = vec![0_u8; 16];
    let mut raw_store_key = vec![0_u8; 32];
    SystemRandom::new()
        .fill(&mut store_id)
        .and_then(|_| SystemRandom::new().fill(&mut raw_store_key))
        .map_err(|_| StoreError::SecretProtection("store key generation failed".to_owned()))?;
    let store_key = SecretBytes::new(raw_store_key);
    let protected_store_key = protector
        .protect(
            &purpose_with_store_id(STORE_KEY_PURPOSE, &store_id),
            store_key.as_ref(),
        )
        .map_err(StoreError::SecretProtection)?;
    connection.execute(
        "INSERT INTO store_secrets(name, protected_blob) VALUES (?1, ?2)",
        params![STORE_KEY_NAME, protected_store_key],
    )?;

    let audit_commitment =
        crate::audit_store::initialize_new_audit(connection, protector, &store_id)?;
    let schema_commitment = crate::migrations::schema_commitment(connection)?;
    let identity_commitment = crate::identity_store::uninitialized_identity_commitment();
    let (trust_count, trust_commitment) = crate::trust_store::trust_commitment(connection)?;
    let mut meta = StoreMeta {
        store_id,
        generation: 1,
        schema_commitment,
        identity_initialized: false,
        identity_commitment,
        trust_count,
        trust_commitment,
        audit_initialized: true,
        audit_commitment,
        manifest_seal: Vec::new(),
    };
    meta.manifest_seal = manifest_seal(store_key.as_ref(), &meta);
    connection.execute(
        "INSERT INTO store_meta(
           singleton, format_version, store_id, generation, schema_commitment, identity_initialized,
           identity_commitment, trust_count, trust_commitment, audit_initialized,
           audit_commitment, manifest_seal
         ) VALUES (1, ?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, 1, ?8, ?9)",
        params![
            STORE_FORMAT_VERSION,
            meta.store_id,
            meta.generation,
            meta.schema_commitment,
            meta.identity_commitment,
            meta.trust_count,
            meta.trust_commitment,
            meta.audit_commitment,
            meta.manifest_seal
        ],
    )?;
    Ok(())
}

pub(crate) fn load_verified_meta(
    connection: &Connection,
    protector: &dyn SecretProtector,
) -> Result<(StoreMeta, SecretBytes), StoreError> {
    let meta = query_meta(connection)?.ok_or(StoreError::StoreIntegrity)?;
    if meta.store_id.len() != 16
        || meta.generation == 0
        || meta.schema_commitment.len() != 32
        || meta.identity_commitment.len() != 32
        || meta.trust_commitment.len() != 32
        || meta.audit_commitment.len() != 32
        || meta.manifest_seal.len() != 32
    {
        return Err(StoreError::StoreIntegrity);
    }
    let protected: Option<Vec<u8>> = connection
        .query_row(
            "SELECT protected_blob FROM store_secrets WHERE name = ?1",
            [STORE_KEY_NAME],
            |row| row.get(0),
        )
        .optional()?;
    let protected = protected.ok_or(StoreError::StoreIntegrity)?;
    let store_key = protector
        .unprotect(
            &purpose_with_store_id(STORE_KEY_PURPOSE, &meta.store_id),
            &protected,
        )
        .map_err(StoreError::SecretProtection)?;
    if store_key.len() != 32
        || hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, store_key.as_ref()),
            &manifest_bytes(&meta),
            &meta.manifest_seal,
        )
        .is_err()
    {
        return Err(StoreError::StoreIntegrity);
    }
    Ok((meta, store_key))
}

pub(crate) fn write_meta(
    connection: &Connection,
    store_key: &[u8],
    meta: &mut StoreMeta,
) -> Result<(), StoreError> {
    meta.generation = meta
        .generation
        .checked_add(1)
        .ok_or(StoreError::StoreIntegrity)?;
    meta.manifest_seal = manifest_seal(store_key, meta);
    let changed = connection.execute(
        "UPDATE store_meta SET
           generation = ?1, schema_commitment = ?2, identity_initialized = ?3,
           identity_commitment = ?4, trust_count = ?5, trust_commitment = ?6,
           audit_initialized = ?7, audit_commitment = ?8, manifest_seal = ?9
         WHERE singleton = 1 AND format_version = ?10 AND store_id = ?11",
        params![
            meta.generation,
            meta.schema_commitment,
            meta.identity_initialized,
            meta.identity_commitment,
            meta.trust_count,
            meta.trust_commitment,
            meta.audit_initialized,
            meta.audit_commitment,
            meta.manifest_seal,
            STORE_FORMAT_VERSION,
            meta.store_id
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(())
}

pub(crate) fn purpose_with_store_id(domain: &[u8], store_id: &[u8]) -> Vec<u8> {
    let mut purpose = domain.to_vec();
    append_field(&mut purpose, store_id);
    purpose
}

pub(crate) fn append_field(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_be_bytes());
    target.extend_from_slice(value);
}

fn query_meta(connection: &Connection) -> Result<Option<StoreMeta>, StoreError> {
    connection
        .query_row(
            "SELECT format_version, store_id, generation, schema_commitment,
                    identity_initialized, identity_commitment, trust_count,
                    trust_commitment, audit_initialized, audit_commitment, manifest_seal
             FROM store_meta WHERE singleton = 1",
            [],
            |row| {
                let format_version: u32 = row.get(0)?;
                if format_version != STORE_FORMAT_VERSION {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(StoreMeta {
                    store_id: row.get(1)?,
                    generation: row.get(2)?,
                    schema_commitment: row.get(3)?,
                    identity_initialized: row.get(4)?,
                    identity_commitment: row.get(5)?,
                    trust_count: row.get(6)?,
                    trust_commitment: row.get(7)?,
                    audit_initialized: row.get(8)?,
                    audit_commitment: row.get(9)?,
                    manifest_seal: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|error| match error {
            rusqlite::Error::InvalidQuery => StoreError::StoreIntegrity,
            other => StoreError::Database(other),
        })
}

fn manifest_seal(key: &[u8], meta: &StoreMeta) -> Vec<u8> {
    hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, key),
        &manifest_bytes(meta),
    )
    .as_ref()
    .to_vec()
}

fn manifest_bytes(meta: &StoreMeta) -> Vec<u8> {
    let mut bytes = MANIFEST_DOMAIN.to_vec();
    bytes.extend_from_slice(&STORE_FORMAT_VERSION.to_be_bytes());
    append_field(&mut bytes, &meta.store_id);
    bytes.extend_from_slice(&meta.generation.to_be_bytes());
    append_field(&mut bytes, &meta.schema_commitment);
    bytes.push(u8::from(meta.identity_initialized));
    append_field(&mut bytes, &meta.identity_commitment);
    bytes.extend_from_slice(&meta.trust_count.to_be_bytes());
    append_field(&mut bytes, &meta.trust_commitment);
    bytes.push(u8::from(meta.audit_initialized));
    append_field(&mut bytes, &meta.audit_commitment);
    bytes
}

fn ensure_empty_security_state(connection: &Connection) -> Result<(), StoreError> {
    for table in [
        "store_meta",
        "machine_identity",
        "trusted_devices",
        "store_secrets",
        "audit_head",
        "audit_events",
    ] {
        let count: u64 =
            connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        if count != 0 {
            return Err(StoreError::StoreIntegrity);
        }
    }
    Ok(())
}
