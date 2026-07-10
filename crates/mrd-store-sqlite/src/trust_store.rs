use crate::{integrity, PersistentStore, StoreError};
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
        let updated_at: u64 = transaction.query_row(
            "SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER)",
            [],
            |row| row.get(0),
        )?;
        transaction
            .execute(
                "INSERT INTO trusted_devices(peer_key_id, public_key, epoch, state, revision, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                params![peer_key_id, public_key, epoch, state.as_str(), updated_at],
            )
            .map_err(map_constraint)?;
        let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
        meta.trust_count = trust_count;
        meta.trust_commitment = trust_commitment;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(TrustRecord {
            peer_key_id: peer_key_id.to_owned(),
            public_key: public_key.to_vec(),
            epoch,
            state,
            revision: 1,
        })
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
        let current = query_record(&transaction, peer_key_id)?
            .ok_or_else(|| StoreError::TrustTransition("peer key is not pinned".to_owned()))?;
        if current.revision != expected_revision {
            return Err(StoreError::TrustTransition(
                "trust revision mismatch".to_owned(),
            ));
        }
        if current.state == TrustState::Revoked && next != TrustState::Revoked {
            return Err(StoreError::TrustTransition(
                "revoked peer key is terminal".to_owned(),
            ));
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| StoreError::TrustTransition("trust revision exhausted".to_owned()))?;
        transaction.execute(
            "UPDATE trusted_devices SET state = ?1, revision = ?2,
                    updated_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE peer_key_id = ?3",
            params![next.as_str(), revision, peer_key_id],
        )?;
        let (trust_count, trust_commitment) = trust_commitment(&transaction)?;
        meta.trust_count = trust_count;
        meta.trust_commitment = trust_commitment;
        integrity::write_meta(&transaction, store_key.as_ref(), &mut meta)?;
        transaction.commit()?;
        Ok(TrustRecord {
            state: next,
            revision,
            ..current
        })
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
            "SELECT peer_key_id, public_key, epoch, state, revision
             FROM trusted_devices WHERE peer_key_id = ?1",
            [peer_key_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                ))
            },
        )
        .optional()?;
    raw.map(|(peer_key_id, public_key, epoch, state, revision)| {
        validate_pinned_identity(&peer_key_id, &public_key, epoch, revision)?;
        let state = TrustState::parse(&state).ok_or(StoreError::StoreIntegrity)?;
        Ok(TrustRecord {
            peer_key_id,
            public_key,
            epoch,
            state,
            revision,
        })
    })
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
