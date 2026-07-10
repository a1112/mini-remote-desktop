use crate::{integrity, PersistentStore, SecretBytes, SecretProtector, StoreError};
use ring::{
    digest, hmac,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const AUDIT_KEY_NAME: &str = "audit_hmac_key_v1";
const AUDIT_KEY_PURPOSE: &[u8] = b"MRD_AUDIT_HMAC_KEY_V1";
const AUDIT_COMMITMENT_DOMAIN: &[u8] = b"MRD_AUDIT_COMMITMENT_V2";
const MAX_AUDIT_QUERY_LIMIT: u32 = 1_000;
const MAX_ACTION_BYTES: usize = 128;
const MAX_OUTCOME_BYTES: usize = 32;
const MAX_CORRELATION_ID_BYTES: usize = 256;
const MAX_TRANSPORT_KIND_BYTES: usize = 64;
const MAX_REASON_CODE_BYTES: usize = 128;
const MAX_DETAIL_ENTRIES: usize = 32;
const MAX_DETAIL_KEY_BYTES: usize = 64;
const MAX_DETAIL_VALUE_BYTES: usize = 512;
const MAX_DETAILS_JSON_BYTES: usize = 16 * 1024;

/// Canonical, redacted audit event supplied by authoritative service transitions.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditDraft {
    /// Monotonic-clock-independent event timestamp supplied by the service.
    pub timestamp_ms: u64,
    /// Stable action identifier such as `pair.approved`.
    pub action: String,
    /// Stable outcome such as `allowed` or `denied`.
    pub outcome: String,
    /// Optional session correlation identifier.
    pub session_id: Option<String>,
    /// Optional local actor device identifier.
    pub actor_device_id: Option<String>,
    /// Optional peer device identifier.
    pub peer_device_id: Option<String>,
    /// Optional selected transport identifier.
    pub transport_kind: Option<String>,
    /// Optional stable reason code.
    pub reason_code: Option<String>,
    /// Redacted, sorted metadata. Secret or content fields are forbidden by callers.
    pub details: BTreeMap<String, String>,
}

/// A persisted audit event with integrity metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Monotonic sequence number.
    pub sequence: u64,
    /// Canonical event.
    pub draft: AuditDraft,
    /// HMAC of this record and the previous HMAC.
    pub event_hash: Vec<u8>,
}

/// Exact-match filters and a bounded cursor for durable audit reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    /// Exclusive event-sequence cursor. When absent, the latest matching page is returned.
    pub after_sequence: Option<u64>,
    /// Maximum number of records to return. Values above 1,000 are clamped.
    pub limit: u32,
    /// Optional exact session identifier filter.
    pub session_id: Option<String>,
    /// Optional exact action filter.
    pub action: Option<String>,
    /// Optional exact outcome filter.
    pub outcome: Option<String>,
    /// Optional exact peer device identifier filter.
    pub peer_device_id: Option<String>,
}

impl std::fmt::Debug for AuditDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuditDraft")
            .field("timestamp_ms", &self.timestamp_ms)
            .field("action", &self.action)
            .field("outcome", &self.outcome)
            .field("session_id", &self.session_id)
            .field("actor_device_id", &self.actor_device_id)
            .field("peer_device_id", &self.peer_device_id)
            .field("transport_kind", &self.transport_kind)
            .field("reason_code", &self.reason_code)
            .field("detail_keys", &self.details.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl std::fmt::Debug for AuditRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuditRecord")
            .field("sequence", &self.sequence)
            .field("draft", &self.draft)
            .field("event_hash", &"REDACTED")
            .finish()
    }
}

impl PersistentStore {
    /// Appends a typed audit event only after verifying the sealed store and audit chain.
    pub fn append_audit(&self, draft: AuditDraft) -> Result<AuditRecord, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let record = append_audit_in_transaction(
            &transaction,
            self.protector.as_ref(),
            &meta.store_id,
            draft,
        )?;
        meta.audit_commitment = audit_commitment(&transaction)?;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Queries a verified audit snapshot with exact filters and a bounded result size.
    pub fn query_audit(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        self.verify_store_snapshot_connection(&transaction)?;
        let records = query_audit_records(&transaction, query)?;
        transaction.commit()?;
        Ok(records)
    }

    /// Verifies the sealed store, sequence continuity, HMAC linkage, and chain head.
    pub fn verify_audit_chain(&self) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        self.verify_store_snapshot_connection(&transaction)?;
        transaction.commit()?;
        Ok(())
    }
}

pub(crate) fn append_audit_in_transaction(
    connection: &Connection,
    protector: &dyn SecretProtector,
    store_id: &[u8],
    draft: AuditDraft,
) -> Result<AuditRecord, StoreError> {
    validate_draft(&draft)?;
    let key = load_audit_key(connection, protector, store_id)?;
    let (sequence, previous_hash): (u64, Vec<u8>) = connection.query_row(
        "SELECT next_sequence, head_hash FROM audit_head WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let details_json = serde_json::to_string(&draft.details)
        .map_err(|_| StoreError::AuditIntegrity { sequence })?;
    let event_hash = audit_hash(key.as_ref(), sequence, &previous_hash, &draft)?;
    connection.execute(
        "INSERT INTO audit_events(
           sequence, timestamp_ms, action, outcome, session_id, actor_device_id,
           peer_device_id, transport_kind, reason_code, details_json, previous_hash, event_hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            sequence,
            draft.timestamp_ms,
            draft.action,
            draft.outcome,
            draft.session_id,
            draft.actor_device_id,
            draft.peer_device_id,
            draft.transport_kind,
            draft.reason_code,
            details_json,
            previous_hash,
            event_hash
        ],
    )?;
    let next_sequence = sequence
        .checked_add(1)
        .ok_or(StoreError::AuditIntegrity { sequence })?;
    let seal = head_seal_for(key.as_ref(), next_sequence, &event_hash);
    connection.execute(
        "UPDATE audit_head SET next_sequence = ?1, head_hash = ?2, head_seal = ?3
         WHERE singleton = 1",
        params![next_sequence, event_hash, seal],
    )?;
    Ok(AuditRecord {
        sequence,
        draft,
        event_hash,
    })
}

pub(crate) fn initialize_new_audit(
    connection: &Connection,
    protector: &dyn SecretProtector,
    store_id: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let mut raw_key = vec![0_u8; 32];
    SystemRandom::new()
        .fill(&mut raw_key)
        .map_err(|_| StoreError::SecretProtection("audit key generation failed".to_owned()))?;
    let key = SecretBytes::new(raw_key);
    let protected = protector
        .protect(
            &integrity::purpose_with_store_id(AUDIT_KEY_PURPOSE, store_id),
            key.as_ref(),
        )
        .map_err(StoreError::SecretProtection)?;
    let head_seal = head_seal_for(key.as_ref(), 1, &[]);
    connection.execute(
        "INSERT INTO store_secrets(name, protected_blob) VALUES (?1, ?2)",
        params![AUDIT_KEY_NAME, protected],
    )?;
    connection.execute(
        "INSERT INTO audit_head(singleton, next_sequence, head_hash, head_seal)
         VALUES (1, 1, x'', ?1)",
        [head_seal],
    )?;
    audit_commitment(connection)
}

pub(crate) fn verify_audit_snapshot(
    connection: &Connection,
    protector: &dyn SecretProtector,
    meta: &integrity::StoreMeta,
) -> Result<(), StoreError> {
    if !meta.audit_initialized || audit_commitment(connection)? != meta.audit_commitment {
        return Err(StoreError::AuditIntegrity { sequence: 0 });
    }
    let key = load_audit_key(connection, protector, &meta.store_id)?;
    verify_chain_connection(connection, key.as_ref())
}

fn load_audit_key(
    connection: &Connection,
    protector: &dyn SecretProtector,
    store_id: &[u8],
) -> Result<SecretBytes, StoreError> {
    let protected: Option<Vec<u8>> = connection
        .query_row(
            "SELECT protected_blob FROM store_secrets WHERE name = ?1",
            [AUDIT_KEY_NAME],
            |row| row.get(0),
        )
        .optional()?;
    let protected = protected.ok_or(StoreError::StoreIntegrity)?;
    let key = protector
        .unprotect(
            &integrity::purpose_with_store_id(AUDIT_KEY_PURPOSE, store_id),
            &protected,
        )
        .map_err(StoreError::SecretProtection)?;
    if key.len() != 32 {
        return Err(StoreError::AuditIntegrity { sequence: 0 });
    }
    Ok(key)
}

pub(crate) fn audit_commitment(connection: &Connection) -> Result<Vec<u8>, StoreError> {
    let protected: Option<Vec<u8>> = connection
        .query_row(
            "SELECT protected_blob FROM store_secrets WHERE name = ?1",
            [AUDIT_KEY_NAME],
            |row| row.get(0),
        )
        .optional()?;
    let protected = protected.ok_or(StoreError::StoreIntegrity)?;
    let head: Option<(u64, Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT next_sequence, head_hash, head_seal FROM audit_head WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (next_sequence, head_hash, head_seal) = head.ok_or(StoreError::StoreIntegrity)?;
    let event_count: u64 =
        connection.query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))?;
    let mut bytes = AUDIT_COMMITMENT_DOMAIN.to_vec();
    integrity::append_field(&mut bytes, &protected);
    bytes.extend_from_slice(&event_count.to_be_bytes());
    bytes.extend_from_slice(&next_sequence.to_be_bytes());
    integrity::append_field(&mut bytes, &head_hash);
    integrity::append_field(&mut bytes, &head_seal);
    Ok(digest::digest(&digest::SHA256, &bytes).as_ref().to_vec())
}

fn verify_chain_connection(connection: &Connection, key: &[u8]) -> Result<(), StoreError> {
    let head: Option<(u64, Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT next_sequence, head_hash, head_seal FROM audit_head WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (next_sequence, head_hash, head_seal) =
        head.ok_or(StoreError::AuditIntegrity { sequence: 0 })?;
    if hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, key),
        &head_seal_bytes(next_sequence, &head_hash),
        &head_seal,
    )
    .is_err()
    {
        return Err(StoreError::AuditIntegrity {
            sequence: next_sequence.saturating_sub(1),
        });
    }
    let mut statement = connection.prepare(
        "SELECT sequence, timestamp_ms, action, outcome, session_id, actor_device_id,
                peer_device_id, transport_kind, reason_code, details_json, previous_hash, event_hash
         FROM audit_events ORDER BY sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut expected_sequence = 1_u64;
    let mut previous_hash = Vec::new();
    while let Some(row) = rows.next()? {
        let sequence: u64 = row.get(0)?;
        let details_json: String = row.get(9)?;
        let details: BTreeMap<String, String> = serde_json::from_str(&details_json)
            .map_err(|_| StoreError::AuditIntegrity { sequence })?;
        let draft = AuditDraft {
            timestamp_ms: row.get(1)?,
            action: row.get(2)?,
            outcome: row.get(3)?,
            session_id: row.get(4)?,
            actor_device_id: row.get(5)?,
            peer_device_id: row.get(6)?,
            transport_kind: row.get(7)?,
            reason_code: row.get(8)?,
            details,
        };
        let stored_previous: Vec<u8> = row.get(10)?;
        let stored_hash: Vec<u8> = row.get(11)?;
        if sequence != expected_sequence
            || stored_previous != previous_hash
            || hmac::verify(
                &hmac::Key::new(hmac::HMAC_SHA256, key),
                &audit_hash_bytes(sequence, &previous_hash, &draft)?,
                &stored_hash,
            )
            .is_err()
        {
            return Err(StoreError::AuditIntegrity { sequence });
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(StoreError::AuditIntegrity { sequence })?;
        previous_hash = stored_hash;
    }
    if expected_sequence != next_sequence || previous_hash != head_hash {
        return Err(StoreError::AuditIntegrity {
            sequence: expected_sequence.saturating_sub(1),
        });
    }
    Ok(())
}

fn query_audit_records(
    connection: &Connection,
    query: &AuditQuery,
) -> Result<Vec<AuditRecord>, StoreError> {
    validate_query(query)?;
    let limit = query.limit.min(MAX_AUDIT_QUERY_LIMIT);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let after_sequence = match query.after_sequence {
        Some(sequence) if sequence > i64::MAX as u64 => return Ok(Vec::new()),
        Some(sequence) => Some(sequence as i64),
        None => None,
    };
    let direction = if after_sequence.is_some() {
        "ASC"
    } else {
        "DESC"
    };
    let sql = format!(
        "SELECT sequence, timestamp_ms, action, outcome, session_id, actor_device_id,
                peer_device_id, transport_kind, reason_code, details_json, previous_hash, event_hash
         FROM audit_events
         WHERE (?1 IS NULL OR sequence > ?1)
           AND (?2 IS NULL OR session_id = ?2)
           AND (?3 IS NULL OR action = ?3)
           AND (?4 IS NULL OR outcome = ?4)
           AND (?5 IS NULL OR peer_device_id = ?5)
         ORDER BY sequence {direction}
         LIMIT ?6"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params![
        after_sequence,
        query.session_id.as_deref(),
        query.action.as_deref(),
        query.outcome.as_deref(),
        query.peer_device_id.as_deref(),
        i64::from(limit),
    ])?;
    let mut records = Vec::with_capacity(limit as usize);
    while let Some(row) = rows.next()? {
        let sequence: u64 = row.get(0)?;
        let details_json: String = row.get(9)?;
        let details: BTreeMap<String, String> = serde_json::from_str(&details_json)
            .map_err(|_| StoreError::AuditIntegrity { sequence })?;
        let draft = AuditDraft {
            timestamp_ms: row.get(1)?,
            action: row.get(2)?,
            outcome: row.get(3)?,
            session_id: row.get(4)?,
            actor_device_id: row.get(5)?,
            peer_device_id: row.get(6)?,
            transport_kind: row.get(7)?,
            reason_code: row.get(8)?,
            details,
        };
        records.push(AuditRecord {
            sequence,
            draft,
            event_hash: row.get(11)?,
        });
    }
    if after_sequence.is_none() {
        records.reverse();
    }
    Ok(records)
}

fn validate_query(query: &AuditQuery) -> Result<(), StoreError> {
    if !optional_is_bounded(query.session_id.as_deref(), MAX_CORRELATION_ID_BYTES)
        || !optional_is_bounded(query.action.as_deref(), MAX_ACTION_BYTES)
        || !optional_is_bounded(query.outcome.as_deref(), MAX_OUTCOME_BYTES)
        || !optional_is_bounded(query.peer_device_id.as_deref(), MAX_CORRELATION_ID_BYTES)
    {
        return Err(StoreError::InvalidAuditQuery);
    }
    Ok(())
}

pub(crate) fn validate_draft(draft: &AuditDraft) -> Result<(), StoreError> {
    if !is_nonempty_bounded(&draft.action, MAX_ACTION_BYTES)
        || !is_nonempty_bounded(&draft.outcome, MAX_OUTCOME_BYTES)
        || !optional_is_bounded(draft.session_id.as_deref(), MAX_CORRELATION_ID_BYTES)
        || !optional_is_bounded(draft.actor_device_id.as_deref(), MAX_CORRELATION_ID_BYTES)
        || !optional_is_bounded(draft.peer_device_id.as_deref(), MAX_CORRELATION_ID_BYTES)
        || !optional_is_bounded(draft.transport_kind.as_deref(), MAX_TRANSPORT_KIND_BYTES)
        || !optional_is_bounded(draft.reason_code.as_deref(), MAX_REASON_CODE_BYTES)
        || draft.details.len() > MAX_DETAIL_ENTRIES
    {
        return Err(StoreError::InvalidAuditEvent);
    }
    let forbidden = [
        "password",
        "secret",
        "private_key",
        "credential",
        "verifier",
        "proof",
        "token",
        "clipboard",
        "keystroke",
        "file_content",
        "media_payload",
    ];
    if draft.details.iter().any(|(key, value)| {
        let normalized_key = key.to_ascii_lowercase();
        !is_nonempty_bounded(key, MAX_DETAIL_KEY_BYTES)
            || value.len() > MAX_DETAIL_VALUE_BYTES
            || forbidden
                .iter()
                .any(|forbidden| normalized_key.contains(forbidden))
    }) || serde_json::to_vec(&draft.details)
        .map_or(true, |details| details.len() > MAX_DETAILS_JSON_BYTES)
    {
        return Err(StoreError::InvalidAuditEvent);
    }
    Ok(())
}

fn is_nonempty_bounded(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.contains('\0')
}

fn optional_is_bounded(value: Option<&str>, max_bytes: usize) -> bool {
    value.is_none_or(|value| value.len() <= max_bytes && !value.contains('\0'))
}

fn audit_hash(
    key: &[u8],
    sequence: u64,
    previous_hash: &[u8],
    draft: &AuditDraft,
) -> Result<Vec<u8>, StoreError> {
    Ok(hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, key),
        &audit_hash_bytes(sequence, previous_hash, draft)?,
    )
    .as_ref()
    .to_vec())
}

fn audit_hash_bytes(
    sequence: u64,
    previous_hash: &[u8],
    draft: &AuditDraft,
) -> Result<Vec<u8>, StoreError> {
    let mut bytes = b"MRD_AUDIT_CHAIN_V1".to_vec();
    bytes.extend_from_slice(&sequence.to_be_bytes());
    integrity::append_field(&mut bytes, previous_hash);
    let canonical =
        serde_json::to_vec(draft).map_err(|_| StoreError::AuditIntegrity { sequence })?;
    integrity::append_field(&mut bytes, &canonical);
    Ok(bytes)
}

fn head_seal_for(key: &[u8], next_sequence: u64, head_hash: &[u8]) -> Vec<u8> {
    hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, key),
        &head_seal_bytes(next_sequence, head_hash),
    )
    .as_ref()
    .to_vec()
}

fn head_seal_bytes(next_sequence: u64, head_hash: &[u8]) -> Vec<u8> {
    let mut bytes = b"MRD_AUDIT_HEAD_V1".to_vec();
    bytes.extend_from_slice(&next_sequence.to_be_bytes());
    integrity::append_field(&mut bytes, head_hash);
    bytes
}
