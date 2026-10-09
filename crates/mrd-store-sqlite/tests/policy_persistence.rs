use mrd_identity::DeviceIdentity;
use mrd_store_sqlite::{
    AeadSecretProtector, AuditDraft, AuditQuery, PersistentStore, SecretBytes, SecretProtector,
    StoreError, TrustState,
};
use ring::rand::SystemRandom;
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
};

fn temp_db(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "mrd-policy-persistence-{name}-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn protector() -> Arc<AeadSecretProtector> {
    Arc::new(AeadSecretProtector::from_key([85; 32]).unwrap())
}

fn audit(peer: &str) -> AuditDraft {
    AuditDraft {
        timestamp_ms: 2,
        action: "lan.pairing.approved".to_owned(),
        outcome: "allowed".to_owned(),
        session_id: None,
        actor_device_id: Some("local".to_owned()),
        peer_device_id: Some(peer.to_owned()),
        transport_kind: Some("lan".to_owned()),
        reason_code: None,
        details: Default::default(),
    }
}

fn all_audits(store: &PersistentStore) -> Vec<mrd_store_sqlite::AuditRecord> {
    store
        .query_audit(&AuditQuery {
            after_sequence: None,
            limit: 100,
            session_id: None,
            action: None,
            outcome: None,
            peer_device_id: None,
        })
        .unwrap()
}

fn frozen_v2(path: &Path) {
    import_frozen_v2(path, include_str!("fixtures/sealed-v2.sql"));
}

fn import_frozen_v2(path: &Path, checkout_sql: &str) {
    // include_str! preserves Git's Windows checkout bytes. This frozen export
    // was sealed with literal LF inside sqlite_schema.sql; restore those exact
    // historical fixture bytes, never normalize a production database schema.
    let historical_sql = checkout_sql.replace("\r\n", "\n");
    let source_sha256 = ring::digest::digest(&ring::digest::SHA256, historical_sql.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        source_sha256, "b0ff71a2c45f41151a0527370679031c2e2a06dc35eeb43ee1d679ee59e13cc6",
        "frozen genuine v2 fixture bytes must stay identical to the historical LF export"
    );
    Connection::open(path)
        .unwrap()
        .execute_batch(&historical_sql)
        .unwrap();
}

#[test]
fn sealed_v2_fixture_loader_restores_historical_lf_after_windows_crlf_checkout() {
    let path = temp_db("v2-crlf-checkout");
    let historical_lf = include_str!("fixtures/sealed-v2.sql").replace("\r\n", "\n");
    let crlf_checkout = historical_lf.replace('\n', "\r\n");
    assert!(crlf_checkout.contains("\r\n"));
    import_frozen_v2(&path, &crlf_checkout);
    let store = PersistentStore::open(&path, protector())
        .expect("fixture loader must restore the historically sealed LF schema bytes");
    store.verify_audit_chain().unwrap();
    assert_eq!(store.list_trusted_devices(true).unwrap().len(), 2);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn production_integrity_still_rejects_a_database_imported_with_changed_crlf_schema_bytes() {
    let path = temp_db("v2-raw-crlf-schema");
    let historical_lf = include_str!("fixtures/sealed-v2.sql").replace("\r\n", "\n");
    let changed_schema = historical_lf.replace('\n', "\r\n");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(&changed_schema).unwrap();
    let identity_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'machine_identity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        identity_schema.contains("\r\n"),
        "SQLite preserves the changed schema bytes"
    );
    assert!(matches!(
        PersistentStore::open(&path, protector()),
        Err(StoreError::StoreIntegrity)
    ));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn policy_and_audit_survive_restart_without_an_implicit_legacy_ceiling() {
    let path = temp_db("restart");
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let legacy = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = PersistentStore::open(&path, protector()).unwrap();
    let (record, event) = store
        .insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            audit(peer.key_id()),
        )
        .unwrap();
    assert_eq!(event.sequence, 1);
    assert_eq!(record.state, TrustState::Trusted);
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        ["screen.view"]
    );
    store
        .insert_trusted_device(legacy.key_id(), legacy.public_key(), 1, TrustState::Trusted)
        .unwrap();
    assert!(store
        .trust_permission_ceiling(legacy.key_id())
        .unwrap()
        .is_empty());
    assert!(store
        .trust_permission_ceiling("unknown")
        .unwrap()
        .is_empty());
    drop(store);
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(
        reopened.trust_permission_ceiling(peer.key_id()).unwrap(),
        ["screen.view"]
    );
    assert!(reopened
        .trust_permission_ceiling(legacy.key_id())
        .unwrap()
        .is_empty());
    assert_eq!(all_audits(&reopened).len(), 1);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn genuine_sealed_v2_migrates_preserving_identity_trust_and_audit_but_no_scopes() {
    let path = temp_db("v2-upgrade");
    frozen_v2(&path);
    let before = Connection::open(&path).unwrap();
    let identity: (String, Vec<u8>, Vec<u8>) = before
        .query_row(
            "SELECT key_id, public_key, protected_pkcs8 FROM machine_identity",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let previous_head: Vec<u8> = before
        .query_row("SELECT head_hash FROM audit_head", [], |row| row.get(0))
        .unwrap();
    drop(before);
    let store = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(store.load_identity().unwrap().key_id(), identity.0);
    assert_eq!(store.load_identity().unwrap().public_key(), identity.1);
    let records = store.list_trusted_devices(true).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records
            .iter()
            .filter(|record| record.state == TrustState::Revoked)
            .count(),
        1
    );
    for record in records {
        assert!(store
            .trust_permission_ceiling(&record.peer_key_id)
            .unwrap()
            .is_empty());
        assert_eq!(record.revision, 1);
    }
    assert_eq!(all_audits(&store).len(), 2);
    let after = Connection::open(&path).unwrap();
    assert_eq!(
        after
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        after
            .query_row("SELECT protected_pkcs8 FROM machine_identity", [], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .unwrap(),
        identity.2
    );
    assert_eq!(
        after
            .query_row("SELECT head_hash FROM audit_head", [], |row| row
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        previous_head
    );
    drop(after);
    drop(store);
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    reopened.verify_audit_chain().unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tampered_v2_is_rejected_before_any_schema_upgrade() {
    for sql in [
        "UPDATE trusted_devices SET revision = 2",
        "UPDATE audit_events SET details_json = '{}' WHERE sequence = 1",
        "UPDATE machine_identity SET public_key = x'01'",
        "UPDATE store_meta SET manifest_seal = zeroblob(32)",
        "CREATE TRIGGER unapproved_trigger AFTER UPDATE ON trusted_devices BEGIN SELECT 1; END",
        "DELETE FROM schema_migrations",
    ] {
        let path = temp_db("v2-tamper");
        frozen_v2(&path);
        let connection = Connection::open(&path).unwrap();
        // The fixture has empty details, so use a valid JSON change that really differs.
        let sql = if sql.starts_with("UPDATE audit_events") {
            "UPDATE audit_events SET action = 'tampered' WHERE sequence = 1"
        } else {
            sql
        };
        connection.execute_batch(sql).unwrap();
        assert!(
            PersistentStore::open(&path, protector()).is_err(),
            "tamper accepted: {sql}"
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'trust_permissions'",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            0
        );
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn permission_tampering_is_detected_in_process_and_on_reopen() {
    for sql in [
        "DELETE FROM trust_permissions",
        "UPDATE trust_permissions SET permission_scope = 'input.keyboard'",
        "INSERT INTO trust_permissions SELECT peer_key_id, 'input.pointer' FROM trusted_devices",
        "DROP TABLE trust_permissions",
    ] {
        let path = temp_db("scope-tamper");
        let store = PersistentStore::open(&path, protector()).unwrap();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        store
            .insert_trusted_device_with_policy_and_audit(
                peer.key_id(),
                peer.public_key(),
                1,
                &["screen.view".to_owned()],
                audit(peer.key_id()),
            )
            .unwrap();
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        assert!(
            store.trust_permission_ceiling(peer.key_id()).is_err(),
            "in-process tamper accepted: {sql}"
        );
        drop(store);
        assert!(
            PersistentStore::open(&path, protector()).is_err(),
            "reopen tamper accepted: {sql}"
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn malformed_or_duplicate_policy_leaves_trust_and_audit_unchanged() {
    let path = temp_db("invalid-policy");
    let store = PersistentStore::open(&path, protector()).unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    for scopes in [
        vec!["unknown.permission".to_owned()],
        vec!["Screen.View".to_owned()],
        vec!["screen.view ".to_owned()],
        vec!["screen.view\0".to_owned()],
        vec!["screen.view".to_owned(), "screen.view".to_owned()],
        vec!["screen.view".to_owned(); 19],
    ] {
        assert!(store
            .insert_trusted_device_with_policy_and_audit(
                peer.key_id(),
                peer.public_key(),
                1,
                &scopes,
                audit(peer.key_id())
            )
            .is_err());
        assert!(store.trust_record(peer.key_id()).unwrap().is_none());
        assert!(all_audits(&store).is_empty());
    }
    store
        .insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned(), "input.pointer".to_owned()],
            audit(peer.key_id()),
        )
        .unwrap();
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        ["input.pointer", "screen.view"]
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

struct FailAuditAppend {
    inner: AeadSecretProtector,
    calls: AtomicUsize,
    fail_on: AtomicUsize,
}

impl SecretProtector for FailAuditAppend {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        self.inner.protect(purpose, plaintext)
    }
    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        if purpose.starts_with(b"MRD_AUDIT_HMAC_KEY_V1") {
            let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
            if call == self.fail_on.load(Ordering::Acquire) {
                return Err("injected failure at audit append".to_owned());
            }
        }
        self.inner.unprotect(purpose, protected)
    }
}

#[test]
fn failed_final_verification_rolls_back_the_entire_v2_schema_migration() {
    let path = temp_db("migration-rollback");
    frozen_v2(&path);
    let before = Connection::open(&path).unwrap();
    let original_seal: Vec<u8> = before
        .query_row("SELECT manifest_seal FROM store_meta", [], |row| row.get(0))
        .unwrap();
    drop(before);
    let protector = Arc::new(FailAuditAppend {
        inner: AeadSecretProtector::from_key([85; 32]).unwrap(),
        calls: AtomicUsize::new(0),
        // v2 verification succeeds; the full v3 verification after DDL fails.
        fail_on: AtomicUsize::new(2),
    });
    assert!(matches!(
        PersistentStore::open(&path, protector.clone()),
        Err(StoreError::SecretProtection(_))
    ));
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('trust_permissions', 'store_meta_v2')", [], |row| row.get::<_, u32>(0)).unwrap(), 0);
    assert_eq!(
        connection
            .query_row("SELECT manifest_seal FROM store_meta", [], |row| row
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        original_seal
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    drop(connection);
    protector.fail_on.store(0, Ordering::Release);
    let reopened = PersistentStore::open(&path, protector).unwrap();
    reopened.verify_audit_chain().unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_openers_share_one_atomic_v2_upgrade_without_promoting_permissions() {
    const WORKERS: usize = 4;
    let path = temp_db("concurrent-v2-upgrade");
    frozen_v2(&path);
    let barrier = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let store = PersistentStore::open(path, protector())?;
                for record in store.list_trusted_devices(true)? {
                    assert!(store
                        .trust_permission_ceiling(&record.peer_key_id)?
                        .is_empty());
                }
                store.verify_audit_chain()
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    let store = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(store.list_trusted_devices(true).unwrap().len(), 2);
    assert_eq!(all_audits(&store).len(), 2);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn audit_failure_after_policy_insert_rolls_back_all_changes_and_seal() {
    let path = temp_db("policy-rollback");
    let protector = Arc::new(FailAuditAppend {
        inner: AeadSecretProtector::from_key([85; 32]).unwrap(),
        calls: AtomicUsize::new(0),
        fail_on: AtomicUsize::new(0),
    });
    let store = PersistentStore::open(&path, protector.clone()).unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    protector.calls.store(0, Ordering::Release);
    protector.fail_on.store(2, Ordering::Release);
    assert!(matches!(
        store.insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            audit(peer.key_id())
        ),
        Err(StoreError::SecretProtection(_))
    ));
    protector.fail_on.store(0, Ordering::Release);
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(store
        .trust_permission_ceiling(peer.key_id())
        .unwrap()
        .is_empty());
    assert!(all_audits(&store).is_empty());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM trust_permissions", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    drop(connection);
    drop(store);
    let reopened = PersistentStore::open(&path, protector).unwrap();
    reopened.verify_audit_chain().unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn duplicate_suspended_and_revoked_keys_cannot_be_overwritten_or_revived() {
    for state in [
        TrustState::Trusted,
        TrustState::Suspended,
        TrustState::Revoked,
    ] {
        let path = temp_db("no-overwrite");
        let store = PersistentStore::open(&path, protector()).unwrap();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        store
            .insert_trusted_device_with_policy_and_audit(
                peer.key_id(),
                peer.public_key(),
                1,
                &["screen.view".to_owned()],
                audit(peer.key_id()),
            )
            .unwrap();
        if state != TrustState::Trusted {
            store.transition_trust(peer.key_id(), 1, state).unwrap();
            assert!(store
                .trust_permission_ceiling(peer.key_id())
                .unwrap()
                .is_empty());
        }
        let before = store.trust_record(peer.key_id()).unwrap();
        assert!(store
            .insert_trusted_device_with_policy_and_audit(
                peer.key_id(),
                peer.public_key(),
                2,
                &["input.keyboard".to_owned()],
                audit(peer.key_id())
            )
            .is_err());
        assert_eq!(store.trust_record(peer.key_id()).unwrap(), before);
        assert_eq!(all_audits(&store).len(), 1);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn independent_concurrent_connections_create_only_one_policy_and_audit() {
    const WORKERS: usize = 4;
    let path = temp_db("policy-concurrent");
    drop(PersistentStore::open(&path, protector()).unwrap());
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let key_id = peer.key_id().to_owned();
    let public_key = peer.public_key().to_vec();
    let barrier = Arc::new(Barrier::new(WORKERS));
    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let path = path.clone();
            let key_id = key_id.clone();
            let public_key = public_key.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = PersistentStore::open(path, protector()).unwrap();
                barrier.wait();
                store.insert_trusted_device_with_policy_and_audit(
                    &key_id,
                    &public_key,
                    1,
                    &["screen.view".to_owned()],
                    audit(&key_id),
                )
            })
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let store = PersistentStore::open(&path, protector()).unwrap();
    assert_eq!(store.list_trusted_devices(true).unwrap().len(), 1);
    assert_eq!(
        store.trust_permission_ceiling(&key_id).unwrap(),
        ["screen.view"]
    );
    assert_eq!(all_audits(&store).len(), 1);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn guarded_insert_rejection_never_writes_trust_policy_or_audit() {
    let path = temp_db("guard-deny");
    let store = PersistentStore::open(&path, protector()).unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let result = store.insert_trusted_device_with_policy_and_audit_guarded(
        peer.key_id(),
        peer.public_key(),
        1,
        &["screen.view".to_owned()],
        audit(peer.key_id()),
        || {
            Err(StoreError::TrustTransition(
                "pairing candidate expired".to_owned(),
            ))
        },
    );
    assert!(matches!(result, Err(StoreError::TrustTransition(_))));
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(store
        .trust_permission_ceiling(peer.key_id())
        .unwrap()
        .is_empty());
    assert!(all_audits(&store).is_empty());
    drop(store);
    std::fs::remove_file(path).unwrap();
}

fn guarded_insert_after_busy_lock(allow_after_lock: bool) {
    use std::{
        sync::{atomic::AtomicBool, mpsc},
        time::Duration,
    };
    let path = temp_db("guard-busy");
    let store = Arc::new(PersistentStore::open(&path, protector()).unwrap());
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let key_id = peer.key_id().to_owned();
    let public_key = peer.public_key().to_vec();
    let blocker = Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let can_approve = Arc::new(AtomicBool::new(true));
    let late_check = can_approve.clone();
    let worker_store = store.clone();
    let worker_key_id = key_id.clone();
    let (started, started_rx) = mpsc::channel();
    let (guard_called, guard_called_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        worker_store.insert_trusted_device_with_policy_and_audit_guarded(
            &worker_key_id,
            &public_key,
            1,
            &["screen.view".to_owned()],
            audit(&worker_key_id),
            || {
                guard_called.send(()).unwrap();
                if late_check.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(StoreError::TrustTransition(
                        "candidate expired while waiting for writer".to_owned(),
                    ))
                }
            },
        )
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        guard_called_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    can_approve.store(allow_after_lock, Ordering::Release);
    blocker.execute_batch("ROLLBACK").unwrap();
    let result = worker.join().unwrap();
    assert_eq!(
        guard_called_rx.recv_timeout(Duration::from_secs(1)),
        Ok(()),
        "late guard must run once after acquiring the write lock"
    );
    if allow_after_lock {
        result.unwrap();
        assert_eq!(
            store.trust_permission_ceiling(&key_id).unwrap(),
            ["screen.view"]
        );
        assert_eq!(all_audits(&store).len(), 1);
    } else {
        assert!(matches!(result, Err(StoreError::TrustTransition(_))));
        assert!(store.trust_record(&key_id).unwrap().is_none());
        assert!(store.trust_permission_ceiling(&key_id).unwrap().is_empty());
        assert!(all_audits(&store).is_empty());
    }
    drop(blocker);
    drop(store);
    let reopened = PersistentStore::open(&path, protector()).unwrap();
    reopened.verify_audit_chain().unwrap();
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn guarded_insert_rechecks_expiry_after_waiting_for_a_busy_write_lock() {
    guarded_insert_after_busy_lock(false);
}

#[test]
fn guarded_insert_still_works_after_a_busy_write_lock_when_late_check_succeeds() {
    guarded_insert_after_busy_lock(true);
}
