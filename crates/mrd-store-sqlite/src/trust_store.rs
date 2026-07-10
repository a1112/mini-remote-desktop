use crate::{
    audit_store::{self, AuditDraft, AuditRecord},
    integrity, PersistentStore, StoreError,
};
use ring::digest;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

const TRUST_COMMITMENT_DOMAIN: &[u8] = b"MRD_TRUST_COMMITMENT_V2";

/// Durable peer trust state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    Trusted,
    Suspended,
    Revoked,
}

/// Pinned trust record keyed by the peer public-key identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRecord {
    /// Stable key identifier derived from the pinned public key.
    pub peer_key_id: String,
    /// Pinned Ed25519 public key bytes.
    pub public_key: Vec<u8>,
    /// Monotonic peer-key epoch.
    pub epoch: u64,
    /// Current trust state.
    pub state: TrustState,
    /// Optimistic-concurrency revision.
    pub revision: u64,
    /// Last durable state-change timestamp in Unix milliseconds.
    pub updated_at_ms: u64,
}

/// Stable reasons why an audited trust transition was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustTransitionRejection {
    /// No pinned peer exists for the requested key identifier.
    NotFound,
    /// The caller's optimistic-concurrency revision is stale.
    RevisionMismatch,
    /// A revoked key identifier cannot transition again.
    RevokedTerminal,
}

/// A trust mutation and its audit record committed by the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTrustTransition {
    /// Updated durable trust state.
    pub record: TrustRecord,
    /// Durable audit evidence for the mutation.
    pub audit: AuditRecord,
}

/// Result of a revision-checked transition whose success or denial was audited atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditedTrustTransition {
    /// Trust state changed and the accompanying audit committed.
    Applied(Box<AppliedTrustTransition>),
    /// Trust state was unchanged and a denial audit committed.
    Rejected {
        /// Stable denial classification.
        rejection: TrustTransitionRejection,
        /// Sequence of the durable denial audit.
        audit_sequence: u64,
    },
}

impl AuditedTrustTransition {
    /// Extracts an applied transition, returning `None` for a durable denial.
    pub fn into_applied(self) -> Option<AppliedTrustTransition> {
        match self {
            Self::Applied(applied) => Some(*applied),
            Self::Rejected { .. } => None,
        }
    }
}

impl TrustTransitionRejection {
    fn reason_code(self) -> &'static str {
        match self {
            Self::NotFound => "trust_peer_not_found",
            Self::RevisionMismatch => "trust_revision_mismatch",
            Self::RevokedTerminal => "trust_revoked_terminal",
        }
    }

    fn legacy_message(self) -> &'static str {
        match self {
            Self::NotFound => "peer key is not pinned",
            Self::RevisionMismatch => "trust revision mismatch",
            Self::RevokedTerminal => "revoked peer key is terminal",
        }
    }
}

impl TrustState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Suspended => "suspended",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "trusted" => Some(Self::Trusted),
            "suspended" => Some(Self::Suspended),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

struct StoredTrustRecord {
    record: TrustRecord,
    updated_at: u64,
}

impl PersistentStore {
    /// Inserts a newly approved pinned peer. Existing key IDs are never overwritten.
    pub fn insert_trusted_device(
        &self,
        peer_key_id: &str,
        public_key: &[u8],
        epoch: u64,
        state: TrustState,
    ) -> Result<TrustRecord, StoreError> {
        validate_pinned_identity(peer_key_id, public_key, epoch, 1)?;
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let record = insert_record(&transaction, peer_key_id, public_key, epoch, state)?;
        let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
        meta.trust_count = trust_count;
        meta.trust_commitment = trust_commitment;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Inserts pinned trust and its approval audit in one sealed transaction.
    pub fn insert_trusted_device_with_audit(
        &self,
        peer_key_id: &str,
        public_key: &[u8],
        epoch: u64,
        state: TrustState,
        mut audit: AuditDraft,
    ) -> Result<(TrustRecord, AuditRecord), StoreError> {
        validate_pinned_identity(peer_key_id, public_key, epoch, 1)?;
        audit_store::validate_draft(&audit)?;
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let record = insert_record(&transaction, peer_key_id, public_key, epoch, state)?;
        audit.outcome = "allowed".to_owned();
        audit.reason_code = None;
        let audit = audit_store::append_audit_in_transaction(
            &transaction,
            self.protector.as_ref(),
            &meta.store_id,
            audit,
        )?;
        let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
        meta.trust_count = trust_count;
        meta.trust_commitment = trust_commitment;
        meta.audit_commitment = audit_store::audit_commitment(&transaction)?;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok((record, audit))
    }

    /// Applies a revision-checked transition. Revocation is terminal for a key ID.
    pub fn transition_trust(
        &self,
        peer_key_id: &str,
        expected_revision: u64,
        next: TrustState,
    ) -> Result<TrustRecord, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let record = match transition_record(&transaction, peer_key_id, expected_revision, next)? {
            Ok(record) => record,
            Err(rejection) => {
                return Err(StoreError::TrustTransition(
                    rejection.legacy_message().to_owned(),
                ));
            }
        };
        let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
        meta.trust_count = trust_count;
        meta.trust_commitment = trust_commitment;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Applies or denies a trust transition and commits the matching audit atomically.
    pub fn transition_trust_with_audit(
        &self,
        peer_key_id: &str,
        expected_revision: u64,
        next: TrustState,
        mut audit: AuditDraft,
    ) -> Result<AuditedTrustTransition, StoreError> {
        audit_store::validate_draft(&audit)?;
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let transition = transition_record(&transaction, peer_key_id, expected_revision, next)?;
        let result = match transition {
            Ok(record) => {
                audit.outcome = "allowed".to_owned();
                audit.reason_code = None;
                let audit = audit_store::append_audit_in_transaction(
                    &transaction,
                    self.protector.as_ref(),
                    &meta.store_id,
                    audit,
                )?;
                let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
                meta.trust_count = trust_count;
                meta.trust_commitment = trust_commitment;
                AuditedTrustTransition::Applied(Box::new(AppliedTrustTransition { record, audit }))
            }
            Err(rejection) => {
                audit.outcome = "denied".to_owned();
                audit.reason_code = Some(rejection.reason_code().to_owned());
                let audit = audit_store::append_audit_in_transaction(
                    &transaction,
                    self.protector.as_ref(),
                    &meta.store_id,
                    audit,
                )?;
                AuditedTrustTransition::Rejected {
                    rejection,
                    audit_sequence: audit.sequence,
                }
            }
        };
        meta.audit_commitment = audit_store::audit_commitment(&transaction)?;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Returns the pinned peer record after verifying the sealed trust snapshot.
    pub fn trust_record(&self, peer_key_id: &str) -> Result<Option<TrustRecord>, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        self.verify_store_snapshot_connection(&transaction)?;
        let record = query_record(&transaction, peer_key_id)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Lists the verified pinned trust snapshot in stable key-ID order.
    pub fn list_trusted_devices(
        &self,
        include_revoked: bool,
    ) -> Result<Vec<TrustRecord>, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        self.verify_store_snapshot_connection(&transaction)?;
        let records = query_all_records(&transaction)?
            .into_iter()
            .map(|stored| stored.record)
            .filter(|record| include_revoked || record.state != TrustState::Revoked)
            .collect();
        transaction.commit()?;
        Ok(records)
    }
}

pub(crate) fn verify_trust_snapshot(
    connection: &Connection,
    meta: &integrity::StoreMeta,
) -> Result<(), StoreError> {
    let (count, commitment) = trust_commitment(connection)?;
    if count != meta.trust_count || commitment != meta.trust_commitment {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(())
}

pub(crate) fn trust_commitment(connection: &Connection) -> Result<(u64, Vec<u8>), StoreError> {
    let records = query_all_records(connection)?;
    let count = records.len() as u64;
    let mut bytes = TRUST_COMMITMENT_DOMAIN.to_vec();
    bytes.extend_from_slice(&count.to_be_bytes());
    for stored in records {
        let record = stored.record;
        integrity::append_field(&mut bytes, record.peer_key_id.as_bytes());
        integrity::append_field(&mut bytes, &record.public_key);
        bytes.extend_from_slice(&record.epoch.to_be_bytes());
        integrity::append_field(&mut bytes, record.state.as_str().as_bytes());
        bytes.extend_from_slice(&record.revision.to_be_bytes());
        bytes.extend_from_slice(&stored.updated_at.to_be_bytes());
    }
    Ok((
        count,
        digest::digest(&digest::SHA256, &bytes).as_ref().to_vec(),
    ))
}

fn insert_record(
    connection: &Connection,
    peer_key_id: &str,
    public_key: &[u8],
    epoch: u64,
    state: TrustState,
) -> Result<TrustRecord, StoreError> {
    let updated_at: u64 = connection.query_row(
        "SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER)",
        [],
        |row| row.get(0),
    )?;
    connection
        .execute(
            "INSERT INTO trusted_devices(peer_key_id, public_key, epoch, state, revision, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            params![peer_key_id, public_key, epoch, state.as_str(), updated_at],
        )
        .map_err(map_constraint)?;
    Ok(TrustRecord {
        peer_key_id: peer_key_id.to_owned(),
        public_key: public_key.to_vec(),
        epoch,
        state,
        revision: 1,
        updated_at_ms: updated_at,
    })
}

fn transition_record(
    connection: &Connection,
    peer_key_id: &str,
    expected_revision: u64,
    next: TrustState,
) -> Result<Result<TrustRecord, TrustTransitionRejection>, StoreError> {
    let Some(current) = query_record(connection, peer_key_id)? else {
        return Ok(Err(TrustTransitionRejection::NotFound));
    };
    if current.revision != expected_revision {
        return Ok(Err(TrustTransitionRejection::RevisionMismatch));
    }
    if current.state == TrustState::Revoked {
        return Ok(Err(TrustTransitionRejection::RevokedTerminal));
    }
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| StoreError::TrustTransition("trust revision exhausted".to_owned()))?;
    let updated_at_ms: u64 = connection.query_row(
        "SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER)",
        [],
        |row| row.get(0),
    )?;
    let changed = connection.execute(
        "UPDATE trusted_devices SET state = ?1, revision = ?2,
                updated_at = ?3
         WHERE peer_key_id = ?4 AND revision = ?5",
        params![
            next.as_str(),
            revision,
            updated_at_ms,
            peer_key_id,
            expected_revision
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(Ok(TrustRecord {
        state: next,
        revision,
        updated_at_ms,
        ..current
    }))
}

fn query_all_records(connection: &Connection) -> Result<Vec<StoredTrustRecord>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT peer_key_id, public_key, epoch, state, revision, updated_at
         FROM trusted_devices ORDER BY peer_key_id",
    )?;
    let mut rows = statement.query([])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        let peer_key_id: String = row.get(0)?;
        let public_key: Vec<u8> = row.get(1)?;
        let epoch: u64 = row.get(2)?;
        let raw_state: String = row.get(3)?;
        let revision: u64 = row.get(4)?;
        let updated_at: u64 = row.get(5)?;
        validate_pinned_identity(&peer_key_id, &public_key, epoch, revision)?;
        let state = TrustState::parse(&raw_state).ok_or(StoreError::StoreIntegrity)?;
        records.push(StoredTrustRecord {
            record: TrustRecord {
                peer_key_id,
                public_key,
                epoch,
                state,
                revision,
                updated_at_ms: updated_at,
            },
            updated_at,
        });
    }
    Ok(records)
}

fn query_record(
    connection: &Connection,
    peer_key_id: &str,
) -> Result<Option<TrustRecord>, StoreError> {
    let raw = connection
        .query_row(
            "SELECT peer_key_id, public_key, epoch, state, revision, updated_at
             FROM trusted_devices WHERE peer_key_id = ?1",
            [peer_key_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, u64>(5)?,
                ))
            },
        )
        .optional()?;
    raw.map(
        |(peer_key_id, public_key, epoch, state, revision, updated_at_ms)| {
            validate_pinned_identity(&peer_key_id, &public_key, epoch, revision)?;
            let state = TrustState::parse(&state).ok_or(StoreError::StoreIntegrity)?;
            Ok(TrustRecord {
                peer_key_id,
                public_key,
                epoch,
                state,
                revision,
                updated_at_ms,
            })
        },
    )
    .transpose()
}

fn validate_pinned_identity(
    peer_key_id: &str,
    public_key: &[u8],
    epoch: u64,
    revision: u64,
) -> Result<(), StoreError> {
    if public_key.len() != 32 || epoch == 0 || revision == 0 || key_id(public_key) != peer_key_id {
        return Err(StoreError::TrustTransition(
            "invalid pinned peer identity".to_owned(),
        ));
    }
    Ok(())
}

fn key_id(public_key: &[u8]) -> String {
    digest::digest(&digest::SHA256, public_key)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn map_constraint(error: rusqlite::Error) -> StoreError {
    match error {
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::TrustTransition("peer key is already pinned".to_owned())
        }
        other => StoreError::Database(other),
    }
}
