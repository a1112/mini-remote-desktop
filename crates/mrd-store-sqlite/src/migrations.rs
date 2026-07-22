use crate::{
    integrity::{append_field, STORE_FORMAT_VERSION},
    StoreError,
};
use ring::digest;
use rusqlite::Connection;

pub(crate) fn schema_version(connection: &Connection) -> Result<u32, StoreError> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(StoreError::Database)
}

pub(crate) fn configure(connection: &Connection) -> Result<(), StoreError> {
    for attempt in 0..=500 {
        match connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;",
        ) {
            Ok(()) => return Ok(()),
            Err(error) if is_busy(&error) && attempt < 500 => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(StoreError::Database(error)),
        }
    }
    Err(StoreError::StoreIntegrity)
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

pub(crate) fn create_schema(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           version INTEGER PRIMARY KEY
         );
         CREATE TABLE IF NOT EXISTS store_meta (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           format_version INTEGER NOT NULL CHECK (format_version = 2),
           store_id BLOB NOT NULL CHECK (length(store_id) = 16),
           generation INTEGER NOT NULL CHECK (generation > 0),
           schema_commitment BLOB NOT NULL CHECK (length(schema_commitment) = 32),
           identity_initialized INTEGER NOT NULL CHECK (identity_initialized IN (0, 1)),
           identity_commitment BLOB NOT NULL CHECK (length(identity_commitment) = 32),
           trust_count INTEGER NOT NULL CHECK (trust_count >= 0),
           trust_commitment BLOB NOT NULL CHECK (length(trust_commitment) = 32),
           audit_initialized INTEGER NOT NULL CHECK (audit_initialized = 1),
           audit_commitment BLOB NOT NULL CHECK (length(audit_commitment) = 32),
           manifest_seal BLOB NOT NULL CHECK (length(manifest_seal) = 32)
         );
         CREATE TABLE IF NOT EXISTS machine_identity (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           key_id TEXT NOT NULL UNIQUE,
           epoch INTEGER NOT NULL CHECK (epoch > 0),
           public_key BLOB NOT NULL UNIQUE,
           protected_pkcs8 BLOB NOT NULL,
           created_at_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS trusted_devices (
           peer_key_id TEXT PRIMARY KEY,
           public_key BLOB NOT NULL UNIQUE,
           epoch INTEGER NOT NULL CHECK (epoch > 0),
           state TEXT NOT NULL CHECK (state IN ('trusted', 'suspended', 'revoked')),
           revision INTEGER NOT NULL CHECK (revision > 0),
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS store_secrets (
           name TEXT PRIMARY KEY,
           protected_blob BLOB NOT NULL
         );
         CREATE TABLE IF NOT EXISTS audit_head (
           singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
           next_sequence INTEGER NOT NULL CHECK (next_sequence > 0),
           head_hash BLOB NOT NULL,
           head_seal BLOB NOT NULL
         );
         CREATE TABLE IF NOT EXISTS audit_events (
           sequence INTEGER PRIMARY KEY,
           timestamp_ms INTEGER NOT NULL,
           action TEXT NOT NULL,
           outcome TEXT NOT NULL,
           session_id TEXT,
           actor_device_id TEXT,
           peer_device_id TEXT,
           transport_kind TEXT,
           reason_code TEXT,
           details_json TEXT NOT NULL,
           previous_hash BLOB NOT NULL,
           event_hash BLOB NOT NULL
         );
         INSERT INTO schema_migrations(version) VALUES (2);",
    )?;
    connection.pragma_update(None, "user_version", STORE_FORMAT_VERSION)?;
    Ok(())
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), StoreError> {
    connection
        .query_row(
            "SELECT singleton, format_version, store_id, generation, schema_commitment,
                    identity_initialized, identity_commitment, trust_count, trust_commitment,
                    audit_initialized, audit_commitment, manifest_seal FROM store_meta LIMIT 1",
            [],
            |_| Ok(()),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => StoreError::StoreIntegrity,
            other => StoreError::Database(other),
        })?;
    let migration_count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE version = 2",
        [],
        |row| row.get(0),
    )?;
    if migration_count != 1 {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(())
}

pub(crate) fn schema_commitment(connection: &Connection) -> Result<Vec<u8>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT type, name, tbl_name, COALESCE(sql, '')
         FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%'
         ORDER BY type, name, tbl_name",
    )?;
    let mut rows = statement.query([])?;
    let mut bytes = b"MRD_SQLITE_SCHEMA_COMMITMENT_V2".to_vec();
    let mut count = 0_u64;
    while let Some(row) = rows.next()? {
        let object_type: String = row.get(0)?;
        let name: String = row.get(1)?;
        let table_name: String = row.get(2)?;
        let sql: String = row.get(3)?;
        append_field(&mut bytes, object_type.as_bytes());
        append_field(&mut bytes, name.as_bytes());
        append_field(&mut bytes, table_name.as_bytes());
        append_field(&mut bytes, sql.as_bytes());
        count = count.checked_add(1).ok_or(StoreError::StoreIntegrity)?;
    }
    bytes.extend_from_slice(&count.to_be_bytes());
    Ok(digest::digest(&digest::SHA256, &bytes).as_ref().to_vec())
}
