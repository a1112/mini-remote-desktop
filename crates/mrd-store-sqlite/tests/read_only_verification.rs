use mrd_identity::DeviceIdentity;
use mrd_store_sqlite::{AeadSecretProtector, AuditDraft, PersistentStore, StoreError};
use ring::rand::SystemRandom;
use rusqlite::Connection;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

fn path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "mrd-readonly-{name}-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn protector() -> Arc<AeadSecretProtector> {
    Arc::new(AeadSecretProtector::from_key([85; 32]).unwrap())
}
fn files(path: &Path) -> [Option<Vec<u8>>; 2] {
    [
        std::fs::read(path).ok(),
        std::fs::read(format!("{}-wal", path.display())).ok(),
    ]
}
fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
fn frozen_v2(path: &Path) {
    let sql = include_str!("fixtures/sealed-v2.sql").replace("\r\n", "\n");
    let digest = ring::digest::digest(&ring::digest::SHA256, sql.as_bytes());
    assert_eq!(
        digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "b0ff71a2c45f41151a0527370679031c2e2a06dc35eeb43ee1d679ee59e13cc6"
    );
    Connection::open(path).unwrap().execute_batch(&sql).unwrap();
}
fn populated_v3(path: &Path) -> PersistentStore {
    let store = PersistentStore::open(path, protector()).unwrap();
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    store.save_identity(&identity).unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    store
        .insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            AuditDraft {
                timestamp_ms: 1,
                action: "trust.lan_paired".to_owned(),
                outcome: "allowed".to_owned(),
                session_id: None,
                actor_device_id: None,
                peer_device_id: None,
                transport_kind: Some("lan_quic".to_owned()),
                reason_code: None,
                details: BTreeMap::new(),
            },
        )
        .unwrap();
    store
}

#[test]
fn read_only_existing_v3_reports_only_verified_summary_without_database_or_wal_changes() {
    let path = path("v3");
    let store = populated_v3(&path);
    let before = files(&path);
    assert!(
        before[1].as_ref().is_some_and(|wal| wal.len() > 32),
        "fixture must require reading committed WAL"
    );
    let summary = PersistentStore::verify_existing_read_only(&path, protector()).unwrap();
    assert_eq!(summary.format_version, 3);
    assert!(summary.identity_initialized);
    assert_eq!(files(&path), before);
    assert_eq!(store.list_trusted_devices(false).unwrap().len(), 1);
    drop(store);
    cleanup(&path);
}

#[test]
fn read_only_existing_uninitialized_v3_is_verified_without_initializing_identity() {
    let path = path("uninitialized");
    let store = PersistentStore::open(&path, protector()).unwrap();
    let before = files(&path);
    let summary = PersistentStore::verify_existing_read_only(&path, protector()).unwrap();
    assert_eq!(summary.format_version, 3);
    assert!(!summary.identity_initialized);
    assert_eq!(files(&path), before);
    assert!(matches!(
        store.load_identity(),
        Err(StoreError::InvalidIdentity)
    ));
    drop(store);
    cleanup(&path);
}

#[test]
fn read_only_genuine_sealed_v2_is_authenticated_without_migration() {
    let path = path("v2");
    frozen_v2(&path);
    let before = files(&path);
    let summary = PersistentStore::verify_existing_read_only(&path, protector()).unwrap();
    assert_eq!(summary.format_version, 2);
    assert!(summary.identity_initialized);
    assert_eq!(files(&path), before);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='trust_permissions'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    drop(connection);
    cleanup(&path);
}

#[test]
fn read_only_missing_store_never_creates_database_or_sidecars() {
    let path = path("missing");
    assert_eq!(files(&path), [None, None]);
    assert!(PersistentStore::verify_existing_read_only(&path, protector()).is_err());
    assert_eq!(files(&path), [None, None]);
    assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());
}

#[test]
fn read_only_empty_existing_store_is_not_bootstrapped() {
    let path = path("empty");
    std::fs::write(&path, []).unwrap();
    let before = files(&path);
    assert!(PersistentStore::verify_existing_read_only(&path, protector()).is_err());
    assert_eq!(files(&path), before);
    cleanup(&path);
}

#[test]
fn read_only_future_store_is_rejected_without_changes() {
    let path = path("future");
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "user_version", 999_u32)
        .unwrap();
    drop(connection);
    let before = files(&path);
    assert!(matches!(
        PersistentStore::verify_existing_read_only(&path, protector()),
        Err(StoreError::UnsupportedSchema(999))
    ));
    assert_eq!(files(&path), before);
    cleanup(&path);
}

#[test]
fn read_only_authentication_detects_policy_and_audit_tampering_in_committed_wal() {
    for sql in [
        "UPDATE trust_permissions SET permission_scope='input.pointer'",
        "UPDATE audit_events SET outcome='denied'",
    ] {
        let path = path("tampered-wal");
        let store = populated_v3(&path);
        let connection = Connection::open(&path).unwrap();
        connection.execute(sql, []).unwrap();
        let before = files(&path);
        assert!(PersistentStore::verify_existing_read_only(&path, protector()).is_err());
        assert_eq!(files(&path), before);
        drop(connection);
        drop(store);
        cleanup(&path);
    }
}

#[test]
fn read_only_wrong_protector_and_tampered_v2_fail_without_changes() {
    let path = path("wrong-key");
    frozen_v2(&path);
    let before = files(&path);
    assert!(PersistentStore::verify_existing_read_only(
        &path,
        Arc::new(AeadSecretProtector::from_key([84; 32]).unwrap())
    )
    .is_err());
    assert_eq!(files(&path), before);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE trusted_devices SET state='suspended' WHERE state='trusted'",
            [],
        )
        .unwrap();
    drop(connection);
    let before = files(&path);
    assert!(PersistentStore::verify_existing_read_only(&path, protector()).is_err());
    assert_eq!(files(&path), before);
    cleanup(&path);
}
