use mrd_identity::DeviceIdentity;
use mrd_store_sqlite::{
    AuditDraft, AuditQuery, AuditedTrustTransition, PersistentStore, SecretBytes, SecretProtector,
    StoreError, TrustState, TrustTransitionRejection,
};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{params, Connection};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

struct SensitiveBuffer(Vec<u8>);

impl Drop for SensitiveBuffer {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

struct TestSecretProtector {
    key: [u8; 32],
}

impl TestSecretProtector {
    fn new(key: [u8; 32]) -> Self {
        Self { key }
    }
}

impl SecretProtector for TestSecretProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
            .map_err(|_| "invalid test protector key".to_owned())?;
        let key = aead::LessSafeKey::new(unbound);
        let mut nonce_bytes = [0_u8; 12];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| "test protector nonce generation failed".to_owned())?;
        let mut ciphertext = SensitiveBuffer(plaintext.to_vec());
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::from(purpose),
            &mut ciphertext.0,
        )
        .map_err(|_| "test protector encryption failed".to_owned())?;
        let mut protected = nonce_bytes.to_vec();
        protected.extend_from_slice(&ciphertext.0);
        Ok(protected)
    }

    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        if protected.len() < 12 + aead::AES_256_GCM.tag_len() {
            return Err("protected secret is truncated".to_owned());
        }
        let mut nonce_bytes = [0_u8; 12];
        nonce_bytes.copy_from_slice(&protected[..12]);
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
            .map_err(|_| "invalid test protector key".to_owned())?;
        let key = aead::LessSafeKey::new(unbound);
        let mut plaintext = SensitiveBuffer(protected[12..].to_vec());
        let plaintext_len = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce_bytes),
                aead::Aad::from(purpose),
                &mut plaintext.0,
            )
            .map_err(|_| "protected secret authentication failed".to_owned())?
            .len();
        plaintext.0.truncate(plaintext_len);
        Ok(SecretBytes::new(std::mem::take(&mut plaintext.0)))
    }
}

struct FailNthAuditUnprotectProtector {
    inner: TestSecretProtector,
    audit_unprotect_calls: AtomicUsize,
    fail_on_call: AtomicUsize,
}

impl FailNthAuditUnprotectProtector {
    fn new(key: [u8; 32]) -> Self {
        Self {
            inner: TestSecretProtector::new(key),
            audit_unprotect_calls: AtomicUsize::new(0),
            fail_on_call: AtomicUsize::new(0),
        }
    }

    fn fail_on_audit_unprotect(&self, call: usize) {
        self.audit_unprotect_calls.store(0, Ordering::Release);
        self.fail_on_call.store(call, Ordering::Release);
    }

    fn disable_failure(&self) {
        self.fail_on_call.store(0, Ordering::Release);
    }
}

impl SecretProtector for FailNthAuditUnprotectProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        self.inner.protect(purpose, plaintext)
    }

    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        if purpose.starts_with(b"MRD_AUDIT_HMAC_KEY_V1") {
            let call = self.audit_unprotect_calls.fetch_add(1, Ordering::AcqRel) + 1;
            if call == self.fail_on_call.load(Ordering::Acquire) {
                return Err("injected audit-key unprotect failure".to_owned());
            }
        }
        self.inner.unprotect(purpose, protected)
    }
}

fn temp_db(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mrd-store-{name}-{}-{unique}.sqlite",
        std::process::id()
    ))
}

fn protector() -> Arc<TestSecretProtector> {
    Arc::new(TestSecretProtector::new([0x5a; 32]))
}

fn audit(timestamp_ms: u64, action: &str, peer: &str) -> AuditDraft {
    AuditDraft {
        timestamp_ms,
        action: action.to_owned(),
        outcome: "allowed".to_owned(),
        session_id: None,
        actor_device_id: Some("local".to_owned()),
        peer_device_id: Some(peer.to_owned()),
        transport_kind: None,
        reason_code: None,
        details: BTreeMap::new(),
    }
}

#[test]
fn trust_mutations_and_audits_commit_atomically_and_are_queryable() {
    let path = temp_db("audited-trust");
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let peer_key_id = peer.key_id().to_owned();
    let store = PersistentStore::open(&path, protector()).unwrap();

    let (approved, approval_audit) = store
        .insert_trusted_device_with_audit(
            &peer_key_id,
            peer.public_key(),
            1,
            TrustState::Trusted,
            audit(1, "trust.approved", &peer_key_id),
        )
        .unwrap();
    assert_eq!(approved.revision, 1);
    assert_eq!(approval_audit.sequence, 1);

    let stale = store
        .transition_trust_with_audit(
            &peer_key_id,
            99,
            TrustState::Suspended,
            audit(2, "trust.suspended", &peer_key_id),
        )
        .unwrap();
    assert_eq!(
        stale,
        AuditedTrustTransition::Rejected {
            rejection: TrustTransitionRejection::RevisionMismatch,
            audit_sequence: 2,
        }
    );
    assert_eq!(store.trust_record(&peer_key_id).unwrap(), Some(approved));

    let revoked = store
        .transition_trust_with_audit(
            &peer_key_id,
            1,
            TrustState::Revoked,
            audit(3, "trust.revoked", &peer_key_id),
        )
        .unwrap();
    let applied = revoked.into_applied().expect("revocation applies");
    assert_eq!(applied.record.state, TrustState::Revoked);
    assert_eq!(applied.record.revision, 2);
    assert_eq!(applied.audit.sequence, 3);

    let reactivation = store
        .transition_trust_with_audit(
            &peer_key_id,
            2,
            TrustState::Trusted,
            audit(4, "trust.reactivated", &peer_key_id),
        )
        .unwrap();
    assert_eq!(
        reactivation,
        AuditedTrustTransition::Rejected {
            rejection: TrustTransitionRejection::RevokedTerminal,
            audit_sequence: 4,
        }
    );

    assert_eq!(
        store.list_trusted_devices(true).unwrap(),
        vec![applied.record.clone()]
    );
    let events = store
        .query_audit(&AuditQuery {
            after_sequence: None,
            limit: 10,
            session_id: None,
            action: None,
            outcome: None,
            peer_device_id: None,
        })
        .unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| (
                event.sequence,
                event.draft.outcome.as_str(),
                event.draft.reason_code.as_deref(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, "allowed", None),
            (2, "denied", Some("trust_revision_mismatch")),
            (3, "allowed", None),
            (4, "denied", Some("trust_revoked_terminal")),
        ]
    );

    drop(store);
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(reopened.list_trusted_devices(true).unwrap().len(), 1);
    assert_eq!(
        reopened
            .query_audit(&AuditQuery {
                after_sequence: Some(1),
                limit: 10,
                session_id: None,
                action: None,
                outcome: None,
                peer_device_id: None,
            })
            .unwrap()
            .len(),
        3
    );
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn audit_append_failure_rolls_back_trust_and_manifest_together() {
    let path = temp_db("audited-trust-rollback");
    let protector = Arc::new(FailNthAuditUnprotectProtector::new([0x6b; 32]));
    let store = PersistentStore::open(&path, protector.clone()).unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();

    // Snapshot verification loads the audit key once; the atomic append loads it again.
    protector.fail_on_audit_unprotect(2);
    assert!(matches!(
        store.insert_trusted_device_with_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            TrustState::Trusted,
            audit(1, "trust.approved", peer.key_id()),
        ),
        Err(StoreError::SecretProtection(_))
    ));

    protector.disable_failure();
    assert_eq!(store.trust_record(peer.key_id()).unwrap(), None);
    assert!(store
        .query_audit(&AuditQuery {
            after_sequence: None,
            limit: 10,
            session_id: None,
            action: None,
            outcome: None,
            peer_device_id: None,
        })
        .unwrap()
        .is_empty());
    store.verify_audit_chain().unwrap();

    drop(store);
    let reopened = PersistentStore::open(&path, protector).unwrap();
    assert_eq!(reopened.trust_record(peer.key_id()).unwrap(), None);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn generated_identity_reloads_with_same_public_key_and_no_plaintext_secret() {
    let path = temp_db("identity");
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let expected_public = identity.public_key().to_vec();
    let private = identity.private_pkcs8().to_vec();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    for candidate in [
        &path,
        &path.with_extension("sqlite-wal"),
        &path.with_extension("sqlite-journal"),
    ] {
        if let Ok(bytes) = std::fs::read(candidate) {
            assert!(!bytes.windows(private.len()).any(|window| window == private));
        }
    }
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(
        reopened.load_identity().unwrap().public_key(),
        expected_public
    );
    assert_eq!(reopened.load_identity_epoch().unwrap(), 1);
    let replacement = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    assert!(matches!(
        reopened.save_identity(&replacement),
        Err(StoreError::IdentityAlreadyInitialized)
    ));
    let _ = std::fs::remove_file(path);
}

#[test]
fn trust_and_revocation_survive_reopen() {
    let path = temp_db("trust");
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let peer_key_id = peer.key_id().to_owned();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store
            .insert_trusted_device(&peer_key_id, peer.public_key(), 1, TrustState::Trusted)
            .unwrap();
    }
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        assert_eq!(
            store.trust_record(&peer_key_id).unwrap().unwrap().state,
            TrustState::Trusted
        );
        store
            .transition_trust(&peer_key_id, 1, TrustState::Revoked)
            .unwrap();
    }
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    let record = reopened.trust_record(&peer_key_id).unwrap().unwrap();
    assert_eq!((record.epoch, record.state), (1, TrustState::Revoked));
    assert!(matches!(
        reopened.transition_trust(&peer_key_id, 2, TrustState::Trusted),
        Err(StoreError::TrustTransition(_))
    ));
    let _ = std::fs::remove_file(path);
}

#[test]
fn audit_sequence_is_monotonic_across_restart_and_tampering_is_detected() {
    let path = temp_db("audit");
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        assert_eq!(
            store
                .append_audit(audit(1, "pair.approved", "peer-a"))
                .unwrap()
                .sequence,
            1
        );
    }
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        assert_eq!(
            store
                .append_audit(audit(2, "trust.revoked", "peer-a"))
                .unwrap()
                .sequence,
            2
        );
        store.verify_audit_chain().unwrap();
    }
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE audit_events SET details_json = ?1 WHERE sequence = 1",
            [r#"{"peer":"attacker"}"#],
        )
        .unwrap();
    assert!(matches!(
        reopened.verify_audit_chain(),
        Err(StoreError::AuditIntegrity { .. })
    ));
    assert!(matches!(
        reopened.append_audit(audit(3, "session.started", "peer-a")),
        Err(StoreError::AuditIntegrity { .. })
    ));
    drop(reopened);
    assert!(matches!(
        PersistentStore::open(&path, protector()),
        Err(StoreError::AuditIntegrity { .. })
    ));
    let _ = std::fs::remove_file(path);
}

#[test]
fn corrupt_identity_blob_fails_closed() {
    let path = temp_db("corrupt");
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE machine_identity SET protected_pkcs8 = x'00010203' WHERE singleton = 1",
            [],
        )
        .unwrap();
    let mut generator_called = false;
    let result = PersistentStore::open(&path, protector()).and_then(|reopened| {
        reopened.load_or_create_identity(|| {
            generator_called = true;
            DeviceIdentity::generate(&SystemRandom::new()).map_err(|_| StoreError::InvalidIdentity)
        })
    });
    assert!(result.is_err());
    assert!(
        !generator_called,
        "corrupt persisted identity must never be replaced automatically"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn deleted_initialized_identity_is_corruption_not_first_start() {
    let path = temp_db("deleted-identity");
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM machine_identity", [])
        .unwrap();
    let mut generator_called = false;
    let result = PersistentStore::open(&path, protector()).and_then(|reopened| {
        reopened.load_or_create_identity(|| {
            generator_called = true;
            DeviceIdentity::generate(&SystemRandom::new()).map_err(|_| StoreError::InvalidIdentity)
        })
    });
    assert!(result.is_err());
    assert!(!generator_called);
    let _ = std::fs::remove_file(path);
}

#[test]
fn audit_tail_and_head_tampering_are_detected() {
    for mode in ["tail", "head"] {
        let path = temp_db(mode);
        let store = PersistentStore::open(&path, protector()).unwrap();
        store
            .append_audit(audit(1, "pair.approved", "peer-a"))
            .unwrap();
        store
            .append_audit(audit(2, "session.started", "peer-a"))
            .unwrap();
        let connection = Connection::open(&path).unwrap();
        if mode == "tail" {
            connection
                .execute("DELETE FROM audit_events WHERE sequence = 2", [])
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE audit_head SET next_sequence = 99 WHERE singleton = 1",
                    [],
                )
                .unwrap();
        }
        assert!(matches!(
            store.verify_audit_chain(),
            Err(StoreError::AuditIntegrity { .. })
        ));
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn audit_rejects_secret_shaped_details() {
    let path = temp_db("audit-redaction");
    let store = PersistentStore::open(&path, protector()).unwrap();
    let mut draft = audit(1, "session.denied", "peer-a");
    draft
        .details
        .insert("password".to_owned(), "must-not-persist".to_owned());
    assert!(matches!(
        store.append_audit(draft),
        Err(StoreError::InvalidAuditEvent)
    ));
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes
        .windows(b"must-not-persist".len())
        .any(|window| window == b"must-not-persist"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn audit_debug_output_never_prints_detail_values() {
    let mut draft = audit(1, "session.denied", "peer-a");
    draft
        .details
        .insert("diagnostic_code".to_owned(), "private-value".to_owned());
    let debug = format!("{draft:?}");
    assert!(debug.contains("diagnostic_code"));
    assert!(!debug.contains("private-value"));
}

#[test]
fn future_schema_is_rejected_without_mutation() {
    let path = temp_db("future-schema");
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    assert!(matches!(
        PersistentStore::open(&path, protector()),
        Err(StoreError::UnsupportedSchema(99))
    ));
    let connection = Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 99);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalid_sqlite_header_is_rejected_without_fallback() {
    let path = temp_db("invalid-header");
    std::fs::write(&path, b"not a sqlite database").unwrap();
    assert!(matches!(
        PersistentStore::open(&path, protector()),
        Err(StoreError::Database(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"not a sqlite database");
    let _ = std::fs::remove_file(path);
}

#[test]
fn wrong_protector_and_public_metadata_tampering_fail_closed() {
    let path = temp_db("metadata");
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    let wrong = Arc::new(TestSecretProtector::new([0x33; 32]));
    assert!(matches!(
        PersistentStore::open(&path, wrong),
        Err(StoreError::SecretProtection(_))
    ));
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE machine_identity SET public_key = x'01' WHERE singleton = 1",
            [],
        )
        .unwrap();
    assert!(PersistentStore::open(&path, protector())
        .and_then(|store| store.load_identity())
        .is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn resetting_identity_initialization_metadata_cannot_replace_the_machine_key() {
    let path = temp_db("identity-reset");
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE store_meta SET identity_initialized = 0 WHERE singleton = 1",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM machine_identity WHERE singleton = 1", [])
        .unwrap();
    drop(connection);

    let mut generator_called = false;
    let result = PersistentStore::open(&path, protector()).and_then(|store| {
        store.load_or_create_identity(|| {
            generator_called = true;
            DeviceIdentity::generate(&SystemRandom::new()).map_err(|_| StoreError::InvalidIdentity)
        })
    });
    assert!(result.is_err());
    assert!(!generator_called);
    let _ = std::fs::remove_file(path);
}

#[test]
fn persisted_revocation_cannot_be_reactivated_by_editing_sqlite_rows() {
    let path = temp_db("trust-reactivation");
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let peer_key_id = peer.key_id().to_owned();
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store
            .insert_trusted_device(&peer_key_id, peer.public_key(), 1, TrustState::Trusted)
            .unwrap();
        store
            .transition_trust(&peer_key_id, 1, TrustState::Revoked)
            .unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE trusted_devices SET state = 'trusted', revision = 3 WHERE peer_key_id = ?1",
            [&peer_key_id],
        )
        .unwrap();

    let result = PersistentStore::open(&path, protector())
        .and_then(|store| store.trust_record(&peer_key_id).map(|_| store));
    assert!(result.is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn deleting_the_entire_audit_anchor_cannot_restart_the_sequence() {
    let path = temp_db("audit-reset");
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store
            .append_audit(audit(1, "pair.approved", "peer-a"))
            .unwrap();
    }
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "DELETE FROM store_secrets WHERE name = 'audit_hmac_key_v1'",
            [],
        )
        .unwrap();
    connection.execute("DELETE FROM audit_events", []).unwrap();
    connection
        .execute(
            "UPDATE audit_head SET next_sequence = 1, head_hash = x'', head_seal = x'' WHERE singleton = 1",
            [],
        )
        .unwrap();
    drop(connection);

    assert!(PersistentStore::open(&path, protector()).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_security_metadata_is_not_recreated_on_open() {
    let path = temp_db("missing-meta");
    {
        let store = PersistentStore::open(&path, protector()).unwrap();
        store
            .append_audit(audit(1, "store.created", "peer-a"))
            .unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM store_meta WHERE singleton = 1", [])
        .unwrap();

    assert!(PersistentStore::open(&path, protector()).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn deleted_trust_state_and_dropped_security_tables_are_not_repaired() {
    for mode in ["row", "table"] {
        let path = temp_db(&format!("trust-delete-{mode}"));
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let peer_key_id = peer.key_id().to_owned();
        {
            let store = PersistentStore::open(&path, protector()).unwrap();
            store
                .insert_trusted_device(&peer_key_id, peer.public_key(), 1, TrustState::Trusted)
                .unwrap();
            store
                .transition_trust(&peer_key_id, 1, TrustState::Revoked)
                .unwrap();
        }
        let connection = Connection::open(&path).unwrap();
        if mode == "row" {
            connection
                .execute(
                    "DELETE FROM trusted_devices WHERE peer_key_id = ?1",
                    [&peer_key_id],
                )
                .unwrap();
        } else {
            connection
                .execute("DROP TABLE trusted_devices", [])
                .unwrap();
        }
        drop(connection);

        assert!(PersistentStore::open(&path, protector()).is_err());
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn identity_rows_cannot_be_spliced_between_stores() {
    let source_path = temp_db("identity-source");
    let target_path = temp_db("identity-target");
    for path in [&source_path, &target_path] {
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let store = PersistentStore::open(path, protector()).unwrap();
        store.save_identity(&identity).unwrap();
    }
    let source_row: (String, u64, Vec<u8>, Vec<u8>, u64) = Connection::open(&source_path)
        .unwrap()
        .query_row(
            "SELECT key_id, epoch, public_key, protected_pkcs8, created_at_ms FROM machine_identity WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    let mut target = Connection::open(&target_path).unwrap();
    let transaction = target.transaction().unwrap();
    transaction
        .execute("DELETE FROM machine_identity", [])
        .unwrap();
    transaction
        .execute(
            "INSERT INTO machine_identity(singleton, key_id, epoch, public_key, protected_pkcs8, created_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                source_row.0,
                source_row.1,
                source_row.2,
                source_row.3,
                source_row.4
            ],
        )
        .unwrap();
    transaction.commit().unwrap();

    assert!(PersistentStore::open(&target_path, protector()).is_err());
    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(target_path);
}

#[test]
fn concurrent_openers_share_one_atomic_store_birth() {
    const OPENER_COUNT: usize = 8;
    let path = temp_db("concurrent-open");
    let barrier = Arc::new(Barrier::new(OPENER_COUNT));
    let handles = (0..OPENER_COUNT)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                PersistentStore::open(path, protector()).map(|store| {
                    store.verify_audit_chain().unwrap();
                })
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        let result = handle.join().unwrap();
        assert!(result.is_ok(), "concurrent open failed: {result:?}");
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn malicious_trust_trigger_cannot_be_laundered_into_the_sealed_manifest() {
    let path = temp_db("trigger-laundering");
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let attacker = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let attacker_key_hex = attacker
        .public_key()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = PersistentStore::open(&path, protector()).unwrap();
    store
        .insert_trusted_device(peer.key_id(), peer.public_key(), 1, TrustState::Trusted)
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER inject_trusted_peer AFTER UPDATE ON trusted_devices
             BEGIN
               INSERT INTO trusted_devices(peer_key_id, public_key, epoch, state, revision, updated_at)
               VALUES ('{}', x'{}', 1, 'trusted', 1, 0);
             END;",
            attacker.key_id(),
            attacker_key_hex
        ))
        .unwrap();

    let result = store.transition_trust(peer.key_id(), 1, TrustState::Suspended);
    assert!(result.is_err());
    let attacker_count: u64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM trusted_devices WHERE peer_key_id = ?1",
            [attacker.key_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attacker_count, 0);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn unsealed_legacy_schema_is_rejected_without_bootstrapping_or_mutation() {
    let path = temp_db("unsealed-v1");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("CREATE TABLE legacy_sentinel(value TEXT NOT NULL)", [])
        .unwrap();
    connection
        .execute("INSERT INTO legacy_sentinel(value) VALUES ('preserve')", [])
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);

    assert!(matches!(
        PersistentStore::open(&path, protector()),
        Err(StoreError::StoreIntegrity)
    ));
    let connection = Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let sentinel: String = connection
        .query_row("SELECT value FROM legacy_sentinel", [], |row| row.get(0))
        .unwrap();
    assert_eq!((version, sentinel.as_str()), (1, "preserve"));
    drop(connection);
    let _ = std::fs::remove_file(path);
}
