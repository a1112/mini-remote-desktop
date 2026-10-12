use crate::{
    audit_store, integrity, trust_store, PersistentStore, SecretBytes, SecretProtector, StoreError,
    TrustState,
};
use ring::digest;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const POLICY_ACTION: &str = "paired_access.policy.v1";
const POLICY_ACTION_PREFIX: &str = "paired_access.policy.";
const POLICY_NAME_PREFIX: &str = "paired_access_policy_";
const POLICY_NAME_V1: &str = "paired_access_policy_v1:";
const POLICY_PURPOSE: &[u8] = b"MRD_PAIRED_ACCESS_POLICY_V1";
const MAX_POLICY_BYTES: usize = 16 * 1024;
const POLICY_SCOPES: &[&str] = &["input.keyboard", "input.pointer", "screen.view"];

/// Target password versions observed during explicit paired enrollment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedAccessPasswordBinding {
    /// Temporary password refresh generation, positive and SQLite-compatible.
    pub temporary_generation: u64,
    /// Target device credential authentication version, positive and SQLite-compatible.
    pub target_auth_version: u64,
}

/// Authenticated policy bound to an exact pinned Ed25519 peer and local owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedAccessPolicy {
    /// SHA-256 key identifier of the pinned peer public key.
    pub peer_key_id: String,
    /// Exact Ed25519 public key, never a credential or signing seed.
    pub public_key: Vec<u8>,
    /// Peer identity epoch authenticated by the signed intent.
    pub epoch: u64,
    /// Stable TokenUser SID fingerprint observed by the service at enrollment;
    /// current logon/session/desktop binding is verified separately per grant.
    pub owner_user_sid_hash: [u8; 32],
    /// Trust revision to which this policy is bound.
    pub trust_revision: u64,
    /// Canonical subset of screen, pointer and keyboard permissions.
    pub scopes: Vec<String>,
    /// Monotonic policy revision, including disable tombstones.
    pub revision: u64,
    /// Explicit enablement; trust mismatch also disables list projections.
    pub enabled: bool,
    /// Legacy policies remain manageable without an invented password binding.
    /// Automatic grants require Some and exact current target password versions.
    #[serde(default)]
    pub password_binding: Option<PairedAccessPasswordBinding>,
}

/// Enrollment metadata from an exact verified pending session and local actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedAccessEnrollment {
    /// Peer identity and owner fields must come from verified service state.
    pub peer_key_id: String,
    pub public_key: Vec<u8>,
    pub epoch: u64,
    pub owner_user_sid_hash: [u8; 32],
    /// None only for a previously unpinned key.
    pub expected_trust_revision: Option<u64>,
    /// Zero only when no policy, including tombstone, exists.
    pub expected_policy_revision: u64,
    pub scopes: Vec<String>,
    /// Required exact target password versions from verified service state.
    pub password_binding: PairedAccessPasswordBinding,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyEnvelope {
    format_version: u32,
    policy: PairedAccessPolicy,
}

struct PolicyCommitment {
    revision: u64,
    blob_sha256: String,
    enabled: bool,
}

impl PersistentStore {
    /// Enrolls or explicitly updates policy in the same sealed transaction as
    /// its trust ceiling and typed audit commitment. The local actor guard is
    /// checked after waiting for the write lock and before any mutation. It
    /// must not reenter this store. Existing ceilings are never expanded.
    pub fn enroll_paired_access_guarded<F>(
        &self,
        enrollment: PairedAccessEnrollment,
        timestamp_ms: u64,
        guard: F,
    ) -> Result<PairedAccessPolicy, StoreError>
    where
        F: FnOnce() -> Result<(), StoreError>,
    {
        trust_store::validate_pinned_identity(
            &enrollment.peer_key_id,
            &enrollment.public_key,
            enrollment.epoch,
            1,
        )?;
        let scopes = canonical_scopes(&enrollment.scopes)?;
        if enrollment.owner_user_sid_hash == [0; 32]
            || !sqlite_counter(enrollment.epoch)
            || !valid_password_binding(&enrollment.password_binding)
            || enrollment.expected_policy_revision > i64::MAX as u64
            || enrollment
                .expected_trust_revision
                .is_some_and(|revision| !sqlite_counter(revision))
            || timestamp_ms > i64::MAX as u64
        {
            return Err(rejected("invalid paired enrollment"));
        }
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        if meta.format_version != integrity::STORE_FORMAT_VERSION {
            return Err(StoreError::StoreIntegrity);
        }
        let policies = load_verified_policies(&transaction, self.protector.as_ref(), &meta)?;
        let current = policies.get(&enrollment.peer_key_id);
        if current.map_or(0, |policy| policy.revision) != enrollment.expected_policy_revision {
            return Err(rejected("paired policy revision mismatch"));
        }
        if current.is_some_and(|policy| {
            policy.owner_user_sid_hash != enrollment.owner_user_sid_hash
                || policy.public_key != enrollment.public_key
                || policy.epoch != enrollment.epoch
        }) {
            return Err(rejected("paired policy identity or owner mismatch"));
        }
        let trust = trust_store::query_record(&transaction, &enrollment.peer_key_id)?;
        let new_trust = trust.is_none();
        let trust_revision = match trust {
            Some(trust) => {
                if trust.state != TrustState::Trusted
                    || trust.public_key != enrollment.public_key
                    || trust.epoch != enrollment.epoch
                    || Some(trust.revision) != enrollment.expected_trust_revision
                {
                    return Err(rejected("paired trust is inactive or changed"));
                }
                let ceiling = permission_ceiling(&transaction, &enrollment.peer_key_id)?;
                if scopes.iter().any(|scope| !ceiling.contains(scope)) {
                    return Err(rejected("paired policy exceeds trust ceiling"));
                }
                trust.revision
            }
            None => {
                if enrollment.expected_trust_revision.is_some() || current.is_some() {
                    return Err(rejected("paired trust revision mismatch"));
                }
                1
            }
        };
        let revision = next_revision(enrollment.expected_policy_revision)?;
        guard()?;
        if new_trust {
            trust_store::insert_record(
                &transaction,
                &enrollment.peer_key_id,
                &enrollment.public_key,
                enrollment.epoch,
                TrustState::Trusted,
            )?;
            for scope in &scopes {
                transaction.execute(
                    "INSERT INTO trust_permissions(peer_key_id, permission_scope) VALUES (?1, ?2)",
                    params![enrollment.peer_key_id, scope],
                )?;
            }
        }
        let policy = PairedAccessPolicy {
            peer_key_id: enrollment.peer_key_id,
            public_key: enrollment.public_key,
            epoch: enrollment.epoch,
            owner_user_sid_hash: enrollment.owner_user_sid_hash,
            trust_revision,
            scopes,
            revision,
            enabled: true,
            password_binding: Some(enrollment.password_binding),
        };
        persist_policy(
            &transaction,
            self.protector.as_ref(),
            &mut meta,
            store_key.as_ref(),
            &policy,
            timestamp_ms,
        )?;
        // Verify the newly protected bytes and their commitment before commit.
        self.verify_store_snapshot_connection(&transaction)?;
        transaction.commit()?;
        Ok(policy)
    }

    /// Reads policy authority only from the authenticated blob and latest typed
    /// commitment, and only for the same currently trusted identity/revision.
    /// Disabled policies are returned as tombstones with enabled=false. Callers
    /// must additionally bind the observed active owner, requested scopes and
    /// exact current password versions. Legacy password_binding=None cannot
    /// authorize automatic access, even when enabled=true.
    pub fn paired_access_policy(
        &self,
        peer_key_id: &str,
    ) -> Result<Option<PairedAccessPolicy>, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let (meta, _) = self.verify_store_snapshot_connection(&transaction)?;
        let policies = load_verified_policies(&transaction, self.protector.as_ref(), &meta)?;
        let policy = policies.get(peer_key_id).cloned();
        let policy = match policy {
            Some(policy) if trust_matches(&transaction, &policy)? => Some(policy),
            _ => None,
        };
        transaction.commit()?;
        Ok(policy)
    }

    /// Returns authenticated management projections, including tombstones and
    /// policies invalidated by trust changes. This list is not grant authority.
    /// The service must restrict projections to the matching local owner.
    pub fn list_paired_access_policies(&self) -> Result<Vec<PairedAccessPolicy>, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let (meta, _) = self.verify_store_snapshot_connection(&transaction)?;
        let mut policies: Vec<_> =
            load_verified_policies(&transaction, self.protector.as_ref(), &meta)?
                .into_values()
                .collect();
        for policy in &mut policies {
            policy.enabled &= trust_matches(&transaction, policy)?;
        }
        transaction.commit()?;
        Ok(policies)
    }

    /// Persists a revision-checked disable tombstone and typed commitment. The
    /// guard must verify the local actor and exact stored policy owner after
    /// the write lock is obtained. Trust state is not revived or modified.
    pub fn revoke_paired_access_guarded<F>(
        &self,
        peer_key_id: &str,
        expected_revision: u64,
        timestamp_ms: u64,
        guard: F,
    ) -> Result<PairedAccessPolicy, StoreError>
    where
        F: FnOnce() -> Result<(), StoreError>,
    {
        if !sqlite_counter(expected_revision) || timestamp_ms > i64::MAX as u64 {
            return Err(rejected("invalid paired revoke revision"));
        }
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut meta, store_key) = self.verify_store_snapshot_connection(&transaction)?;
        let mut policies = load_verified_policies(&transaction, self.protector.as_ref(), &meta)?;
        let mut policy = policies
            .remove(peer_key_id)
            .ok_or_else(|| rejected("paired policy not found"))?;
        if policy.revision != expected_revision {
            return Err(rejected("paired policy revision mismatch"));
        }
        policy.revision = next_revision(policy.revision)?;
        policy.enabled = false;
        guard()?;
        persist_policy(
            &transaction,
            self.protector.as_ref(),
            &mut meta,
            store_key.as_ref(),
            &policy,
            timestamp_ms,
        )?;
        self.verify_store_snapshot_connection(&transaction)?;
        transaction.commit()?;
        Ok(policy)
    }
}

fn persist_policy(
    connection: &Connection,
    protector: &dyn SecretProtector,
    meta: &mut integrity::StoreMeta,
    store_key: &[u8],
    policy: &PairedAccessPolicy,
    timestamp_ms: u64,
) -> Result<(), StoreError> {
    let plaintext = SecretBytes::new(
        serde_json::to_vec(&PolicyEnvelope {
            format_version: 1,
            policy: policy.clone(),
        })
        .map_err(|_| StoreError::StoreIntegrity)?,
    );
    let protected = protector
        .protect(
            &policy_purpose(&meta.store_id, &policy.peer_key_id),
            plaintext.as_ref(),
        )
        .map_err(|_| StoreError::SecretProtection("paired policy protection failed".to_owned()))?;
    if protected.is_empty() || protected.len() > MAX_POLICY_BYTES {
        return Err(StoreError::StoreIntegrity);
    }
    connection.execute(
        "INSERT INTO store_secrets(name, protected_blob) VALUES (?1, ?2)
        ON CONFLICT(name) DO UPDATE SET protected_blob = excluded.protected_blob",
        params![format!("{POLICY_NAME_V1}{}", policy.peer_key_id), protected],
    )?;
    audit_store::append_paired_policy_audit(
        connection,
        protector,
        &meta.store_id,
        policy,
        &protected,
        timestamp_ms,
    )?;
    let (count, commitment) = trust_store::trust_commitment(connection)?;
    meta.trust_count = count;
    meta.trust_commitment = commitment;
    meta.audit_commitment = audit_store::audit_commitment(connection)?;
    integrity::write_meta(connection, store_key, meta)
}

pub(crate) fn verify_policy_snapshot(
    connection: &Connection,
    protector: &dyn SecretProtector,
    meta: &integrity::StoreMeta,
) -> Result<(), StoreError> {
    load_verified_policies(connection, protector, meta).map(|_| ())
}

fn load_verified_policies(
    connection: &Connection,
    protector: &dyn SecretProtector,
    meta: &integrity::StoreMeta,
) -> Result<BTreeMap<String, PairedAccessPolicy>, StoreError> {
    // This is called only after the entire audit chain has been authenticated.
    // Audit data alone never grants permission: both directions of the exact
    // latest commitment/blob relation must hold, including disable tombstones.
    let commitments = policy_commitments(connection)?;
    let mut statement = connection.prepare(
        "SELECT name, protected_blob FROM store_secrets
        WHERE substr(name, 1, ?1) = ?2 ORDER BY name",
    )?;
    let mut rows = statement.query(params![POLICY_NAME_PREFIX.len(), POLICY_NAME_PREFIX])?;
    let mut policies = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let key = name
            .strip_prefix(POLICY_NAME_V1)
            .ok_or(StoreError::StoreIntegrity)?;
        if !valid_key_id(key) {
            return Err(StoreError::StoreIntegrity);
        }
        let protected: Vec<u8> = row.get(1)?;
        let commitment = commitments.get(key).ok_or(StoreError::StoreIntegrity)?;
        if protected.is_empty()
            || protected.len() > MAX_POLICY_BYTES
            || sha256_hex(&protected) != commitment.blob_sha256
        {
            return Err(StoreError::StoreIntegrity);
        }
        let plaintext = protector
            .unprotect(&policy_purpose(&meta.store_id, key), &protected)
            .map_err(|_| StoreError::StoreIntegrity)?;
        if plaintext.len() > MAX_POLICY_BYTES {
            return Err(StoreError::StoreIntegrity);
        }
        let envelope: PolicyEnvelope =
            serde_json::from_slice(plaintext.as_ref()).map_err(|_| StoreError::StoreIntegrity)?;
        let policy = envelope.policy;
        if envelope.format_version != 1
            || policy.peer_key_id != key
            || policy.revision != commitment.revision
            || policy.enabled != commitment.enabled
        {
            return Err(StoreError::StoreIntegrity);
        }
        validate_stored_policy(&policy)?;
        policies.insert(key.to_owned(), policy);
    }
    if policies.len() != commitments.len()
        || (meta.format_version != integrity::STORE_FORMAT_VERSION && !policies.is_empty())
    {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(policies)
}

fn policy_commitments(
    connection: &Connection,
) -> Result<BTreeMap<String, PolicyCommitment>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT action, outcome, peer_device_id, details_json,
        session_id, actor_device_id, transport_kind, reason_code FROM audit_events
        WHERE substr(action, 1, ?1) = ?2 ORDER BY sequence",
    )?;
    let mut rows = statement.query(params![POLICY_ACTION_PREFIX.len(), POLICY_ACTION_PREFIX])?;
    let mut commitments: BTreeMap<String, PolicyCommitment> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let action: String = row.get(0)?;
        let outcome: String = row.get(1)?;
        let key: Option<String> = row.get(2)?;
        let details: BTreeMap<String, String> = serde_json::from_str(&row.get::<_, String>(3)?)
            .map_err(|_| StoreError::StoreIntegrity)?;
        let key = key
            .filter(|key| valid_key_id(key))
            .ok_or(StoreError::StoreIntegrity)?;
        let enabled = match outcome.as_str() {
            "enabled" => true,
            "disabled" => false,
            _ => return Err(StoreError::StoreIntegrity),
        };
        if action != POLICY_ACTION
            || details.len() != 2
            || (4..8).any(|column| {
                row.get::<_, Option<String>>(column)
                    .map_or(true, |field| field.is_some())
            })
        {
            return Err(StoreError::StoreIntegrity);
        }
        let revision_text = details.get("revision").ok_or(StoreError::StoreIntegrity)?;
        let revision: u64 = revision_text
            .parse()
            .map_err(|_| StoreError::StoreIntegrity)?;
        let previous = commitments
            .get(&key)
            .map_or(0, |commitment| commitment.revision);
        let hash = details
            .get("blob_sha256")
            .filter(|hash| valid_key_id(hash))
            .ok_or(StoreError::StoreIntegrity)?
            .clone();
        if !sqlite_counter(revision)
            || revision.to_string() != *revision_text
            || previous.checked_add(1) != Some(revision)
        {
            return Err(StoreError::StoreIntegrity);
        }
        commitments.insert(
            key,
            PolicyCommitment {
                revision,
                blob_sha256: hash,
                enabled,
            },
        );
    }
    Ok(commitments)
}

fn validate_stored_policy(policy: &PairedAccessPolicy) -> Result<(), StoreError> {
    trust_store::validate_pinned_identity(
        &policy.peer_key_id,
        &policy.public_key,
        policy.epoch,
        policy.trust_revision,
    )
    .map_err(|_| StoreError::StoreIntegrity)?;
    if policy.owner_user_sid_hash == [0; 32]
        || !sqlite_counter(policy.epoch)
        || !sqlite_counter(policy.trust_revision)
        || !sqlite_counter(policy.revision)
        || policy
            .password_binding
            .as_ref()
            .is_some_and(|binding| !valid_password_binding(binding))
        || canonical_scopes(&policy.scopes).map_err(|_| StoreError::StoreIntegrity)?
            != policy.scopes
    {
        return Err(StoreError::StoreIntegrity);
    }
    Ok(())
}

fn valid_password_binding(binding: &PairedAccessPasswordBinding) -> bool {
    sqlite_counter(binding.temporary_generation) && sqlite_counter(binding.target_auth_version)
}

fn canonical_scopes(scopes: &[String]) -> Result<Vec<String>, StoreError> {
    if scopes.is_empty()
        || scopes.len() > POLICY_SCOPES.len()
        || scopes
            .iter()
            .any(|scope| !POLICY_SCOPES.contains(&scope.as_str()))
    {
        return Err(rejected("invalid paired permission scope"));
    }
    let mut scopes = scopes.to_vec();
    scopes.sort_unstable();
    if scopes.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(rejected("duplicate paired permission scope"));
    }
    Ok(scopes)
}

fn permission_ceiling(connection: &Connection, key: &str) -> Result<Vec<String>, StoreError> {
    Ok(connection.prepare("SELECT permission_scope FROM trust_permissions WHERE peer_key_id = ?1 ORDER BY permission_scope")?
        .query_map([key], |row| row.get(0))?.collect::<Result<Vec<_>, _>>()?)
}

fn trust_matches(connection: &Connection, policy: &PairedAccessPolicy) -> Result<bool, StoreError> {
    let Some(trust) = trust_store::query_record(connection, &policy.peer_key_id)? else {
        return Ok(false);
    };
    if trust.state != TrustState::Trusted
        || trust.public_key != policy.public_key
        || trust.epoch != policy.epoch
        || trust.revision != policy.trust_revision
    {
        return Ok(false);
    }
    let ceiling = permission_ceiling(connection, &policy.peer_key_id)?;
    Ok(policy.scopes.iter().all(|scope| ceiling.contains(scope)))
}

fn policy_purpose(store_id: &[u8], key: &str) -> Vec<u8> {
    let mut purpose = integrity::purpose_with_store_id(POLICY_PURPOSE, store_id);
    integrity::append_field(&mut purpose, key.as_bytes());
    purpose
}

fn valid_key_id(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sqlite_counter(value: u64) -> bool {
    value > 0 && value <= i64::MAX as u64
}

fn next_revision(current: u64) -> Result<u64, StoreError> {
    current
        .checked_add(1)
        .filter(|revision| sqlite_counter(*revision))
        .ok_or_else(|| rejected("paired policy revision exhausted"))
}

fn rejected(reason: &str) -> StoreError {
    StoreError::TrustTransition(reason.to_owned())
}
