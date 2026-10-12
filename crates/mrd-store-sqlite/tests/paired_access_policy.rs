use mrd_identity::DeviceIdentity;
use mrd_store_sqlite::{
    AeadSecretProtector, AuditDraft, AuditQuery, PairedAccessEnrollment,
    PairedAccessPasswordBinding, PersistentStore, SecretBytes, SecretProtector, StoreError,
    TrustState,
};
use ring::rand::SystemRandom;
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const POLICY_ACTION: &str = "paired_access.policy.v1";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "mrd-paired-policy-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn open(&self) -> PersistentStore {
        PersistentStore::open(&self.0, protector()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

fn protector() -> Arc<AeadSecretProtector> {
    Arc::new(AeadSecretProtector::from_key([91; 32]).unwrap())
}

fn audit(action: &str) -> AuditDraft {
    AuditDraft {
        timestamp_ms: 7,
        action: action.to_owned(),
        outcome: "allowed".to_owned(),
        session_id: None,
        actor_device_id: None,
        peer_device_id: None,
        transport_kind: None,
        reason_code: None,
        details: Default::default(),
    }
}

fn audits(store: &PersistentStore) -> Vec<mrd_store_sqlite::AuditRecord> {
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

#[test]
fn generic_audit_cannot_create_an_authoritative_policy_commitment() {
    let fixture = Fixture::new();
    let store = fixture.open();
    for action in [POLICY_ACTION, "paired_access.policy.v2"] {
        assert!(
            matches!(
                store.append_audit(audit(action)),
                Err(StoreError::InvalidAuditEvent)
            ),
            "ordinary audit append must reject the reserved policy namespace"
        );
    }
    assert!(audits(&store).is_empty());
}

#[test]
fn generic_trust_audit_cannot_forge_policy_authority_or_mutate_trust() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    assert!(matches!(
        store.insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            audit(POLICY_ACTION),
        ),
        Err(StoreError::InvalidAuditEvent)
    ));
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(audits(&store).is_empty());
}

fn scopes() -> Vec<String> {
    ["screen.view", "input.pointer", "input.keyboard"]
        .map(str::to_owned)
        .to_vec()
}

fn enrollment(peer: &DeviceIdentity) -> PairedAccessEnrollment {
    PairedAccessEnrollment {
        peer_key_id: peer.key_id().to_owned(),
        public_key: peer.public_key().to_vec(),
        epoch: 1,
        owner_user_sid_hash: [7; 32],
        expected_trust_revision: None,
        expected_policy_revision: 0,
        scopes: scopes(),
        password_binding: password_binding(3, 2),
    }
}

fn enroll(store: &PersistentStore, peer: &DeviceIdentity) -> mrd_store_sqlite::PairedAccessPolicy {
    store
        .enroll_paired_access_guarded(enrollment(peer), 7, || Ok(()))
        .unwrap()
}

fn blob(connection: &Connection, peer: &DeviceIdentity) -> Vec<u8> {
    connection
        .query_row(
            "SELECT protected_blob FROM store_secrets WHERE name = ?1",
            [format!("paired_access_policy_v1:{}", peer.key_id())],
            |row| row.get(0),
        )
        .unwrap()
}

fn password_binding(
    temporary_generation: u64,
    target_auth_version: u64,
) -> PairedAccessPasswordBinding {
    PairedAccessPasswordBinding {
        temporary_generation,
        target_auth_version,
    }
}

fn seal_snapshot(connection: &Connection) -> (u64, Vec<u8>) {
    connection
        .query_row(
            "SELECT generation, manifest_seal FROM store_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

enum PolicyBindingRewrite {
    LegacyMissing,
    Invalid(PairedAccessPasswordBinding),
}

/// Uses real purpose-bound AEAD and normal typed audit/seal writes. Only the
/// fixture policy serializer changes, to model old or invalid authenticated
/// bytes without introducing a production API that can enroll unbound policy.
struct PolicyBindingRewriteProtector {
    inner: AeadSecretProtector,
    rewrite: PolicyBindingRewrite,
    rewrites: AtomicUsize,
}

impl SecretProtector for PolicyBindingRewriteProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        if !purpose.starts_with(b"MRD_PAIRED_ACCESS_POLICY_V1") {
            return self.inner.protect(purpose, plaintext);
        }
        let mut envelope: serde_json::Value = serde_json::from_slice(plaintext).unwrap();
        let policy = envelope["policy"].as_object_mut().unwrap();
        assert!(policy.remove("password_binding").is_some());
        if let PolicyBindingRewrite::Invalid(binding) = &self.rewrite {
            policy.insert(
                "password_binding".to_owned(),
                serde_json::json!({
                    "temporary_generation": binding.temporary_generation,
                    "target_auth_version": binding.target_auth_version,
                }),
            );
        }
        self.rewrites.fetch_add(1, Ordering::SeqCst);
        let rewritten = SecretBytes::new(serde_json::to_vec(&envelope).unwrap());
        self.inner.protect(purpose, rewritten.as_ref())
    }

    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        self.inner.unprotect(purpose, protected)
    }
}

fn rewriting_protector(rewrite: PolicyBindingRewrite) -> Arc<PolicyBindingRewriteProtector> {
    Arc::new(PolicyBindingRewriteProtector {
        inner: AeadSecretProtector::from_key([91; 32]).unwrap(),
        rewrite,
        rewrites: AtomicUsize::new(0),
    })
}

#[test]
fn password_binding_persists_exact_values_across_reopen() {
    let fixture = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = fixture.open();
    let policy = enroll(&store, &peer);
    assert_eq!(policy.password_binding, Some(password_binding(3, 2)));
    drop(store);
    let reopened = fixture.open();
    assert_eq!(
        reopened.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy.clone())
    );
    assert_eq!(reopened.list_paired_access_policies().unwrap(), [policy]);
    assert_eq!(
        PersistentStore::verify_existing_read_only(&fixture.0, protector())
            .unwrap()
            .format_version,
        3
    );
}

#[test]
fn password_binding_explicit_repair_advances_revision_without_changing_trust() {
    let fixture = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = fixture.open();
    let original = enroll(&store, &peer);
    let trust = store.trust_record(peer.key_id()).unwrap().unwrap();
    let ceiling = store.trust_permission_ceiling(peer.key_id()).unwrap();
    let connection = Connection::open(&fixture.0).unwrap();
    let previous_blob = blob(&connection, &peer);
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(trust.revision);
    request.expected_policy_revision = original.revision;
    request.password_binding = password_binding(4, 3);
    let updated = store
        .enroll_paired_access_guarded(request, 8, || Ok(()))
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.password_binding, Some(password_binding(4, 3)));
    assert_eq!(store.trust_record(peer.key_id()).unwrap().unwrap(), trust);
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        ceiling
    );
    assert_eq!(audits(&store).len(), 2);
    assert_ne!(blob(&connection, &peer), previous_blob);
    drop(connection);
    drop(store);
    assert_eq!(
        fixture.open().paired_access_policy(peer.key_id()).unwrap(),
        Some(updated)
    );
}

#[test]
fn password_binding_accepts_maximum_sqlite_counters_without_truncation() {
    let fixture = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = fixture.open();
    let mut request = enrollment(&peer);
    request.password_binding = password_binding(i64::MAX as u64, i64::MAX as u64);
    let policy = store
        .enroll_paired_access_guarded(request, 7, || Ok(()))
        .unwrap();
    assert_eq!(
        policy.password_binding,
        Some(password_binding(i64::MAX as u64, i64::MAX as u64))
    );
    drop(store);
    assert_eq!(
        fixture.open().paired_access_policy(peer.key_id()).unwrap(),
        Some(policy)
    );
}

#[test]
fn password_binding_zero_or_overflow_never_mutates_new_or_existing_policy() {
    for binding in [
        password_binding(0, 2),
        password_binding(3, 0),
        password_binding(i64::MAX as u64 + 1, 2),
        password_binding(3, i64::MAX as u64 + 1),
        password_binding(u64::MAX, 2),
        password_binding(3, u64::MAX),
    ] {
        let fixture = Fixture::new();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let store = fixture.open();
        let connection = Connection::open(&fixture.0).unwrap();
        let empty_seal = seal_snapshot(&connection);
        let mut request = enrollment(&peer);
        request.password_binding = binding;
        let mut guard_called = false;
        let result = store.enroll_paired_access_guarded(request, 7, || {
            guard_called = true;
            Ok(())
        });
        assert!(matches!(result, Err(StoreError::TrustTransition(_))));
        assert!(!guard_called);
        assert!(store.trust_record(peer.key_id()).unwrap().is_none());
        assert!(store
            .trust_permission_ceiling(peer.key_id())
            .unwrap()
            .is_empty());
        assert!(store.list_paired_access_policies().unwrap().is_empty());
        assert!(audits(&store).is_empty());
        assert_eq!(seal_snapshot(&connection), empty_seal);

        let original = enroll(&store, &peer);
        let trust = store.trust_record(peer.key_id()).unwrap();
        let protected = blob(&connection, &peer);
        let before = seal_snapshot(&connection);
        let mut request = enrollment(&peer);
        request.expected_trust_revision = Some(original.trust_revision);
        request.expected_policy_revision = original.revision;
        request.password_binding = binding;
        assert!(matches!(
            store.enroll_paired_access_guarded(request, 8, || Ok(())),
            Err(StoreError::TrustTransition(_))
        ));
        assert_eq!(
            store.paired_access_policy(peer.key_id()).unwrap(),
            Some(original)
        );
        assert_eq!(store.trust_record(peer.key_id()).unwrap(), trust);
        assert_eq!(audits(&store).len(), 1);
        assert_eq!(blob(&connection, &peer), protected);
        assert_eq!(seal_snapshot(&connection), before);
    }
}

#[test]
fn password_binding_legacy_missing_field_remains_manageable_without_minting_default() {
    let fixture = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let legacy_writer = rewriting_protector(PolicyBindingRewrite::LegacyMissing);
    let store = PersistentStore::open(&fixture.0, legacy_writer.clone()).unwrap();
    enroll(&store, &peer);
    assert_eq!(legacy_writer.rewrites.load(Ordering::SeqCst), 1);
    drop(store);

    // Reopening with the ordinary protector proves compatibility is in the
    // authenticated store reader, not in the fixture's unprotect behavior.
    let store = fixture.open();
    let legacy = store.paired_access_policy(peer.key_id()).unwrap().unwrap();
    assert_eq!(legacy.password_binding, None);
    assert!(legacy.enabled);
    assert_eq!(store.list_paired_access_policies().unwrap(), [legacy]);
    assert_eq!(
        PersistentStore::verify_existing_read_only(&fixture.0, protector())
            .unwrap()
            .format_version,
        3
    );
    let revoked = store
        .revoke_paired_access_guarded(peer.key_id(), 1, 8, || Ok(()))
        .unwrap();
    assert!(!revoked.enabled);
    assert_eq!(revoked.revision, 2);
    assert_eq!(revoked.password_binding, None);
    drop(store);
    assert_eq!(
        fixture.open().paired_access_policy(peer.key_id()).unwrap(),
        Some(revoked)
    );
}

#[test]
fn password_binding_invalid_authenticated_blob_rolls_back_enrollment() {
    for binding in [
        password_binding(0, 2),
        password_binding(3, 0),
        password_binding(i64::MAX as u64 + 1, 2),
        password_binding(3, u64::MAX),
    ] {
        let fixture = Fixture::new();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let writer = rewriting_protector(PolicyBindingRewrite::Invalid(binding));
        let store = PersistentStore::open(&fixture.0, writer.clone()).unwrap();
        let connection = Connection::open(&fixture.0).unwrap();
        let before = seal_snapshot(&connection);
        assert!(matches!(
            store.enroll_paired_access_guarded(enrollment(&peer), 7, || Ok(())),
            Err(StoreError::StoreIntegrity)
        ));
        assert_eq!(writer.rewrites.load(Ordering::SeqCst), 1);
        assert!(store.trust_record(peer.key_id()).unwrap().is_none());
        assert!(store
            .trust_permission_ceiling(peer.key_id())
            .unwrap()
            .is_empty());
        assert!(store.list_paired_access_policies().unwrap().is_empty());
        assert!(audits(&store).is_empty());
        assert_eq!(seal_snapshot(&connection), before);
        drop(connection);
        drop(store);
        assert!(fixture
            .open()
            .list_paired_access_policies()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn new_pairing_persists_exact_policy_ceiling_audit_and_v3_across_reopen() {
    let fixture = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = fixture.open();
    let policy = enroll(&store, &peer);
    assert!(policy.enabled);
    assert_eq!(
        (policy.epoch, policy.trust_revision, policy.revision),
        (1, 1, 1)
    );
    assert_eq!(policy.public_key, peer.public_key());
    assert_eq!(policy.owner_user_sid_hash, [7; 32]);
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        policy.scopes
    );
    let records = audits(&store);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].draft.action, POLICY_ACTION);
    let connection = Connection::open(&fixture.0).unwrap();
    let protected = blob(&connection, &peer);
    let digest = ring::digest::digest(&ring::digest::SHA256, &protected)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(records[0].draft.details["blob_sha256"], digest);
    assert!(!protected
        .windows(b"screen.view".len())
        .any(|part| part == b"screen.view"));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        3
    );
    drop(connection);
    drop(store);
    let reopened = fixture.open();
    assert_eq!(
        reopened.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy.clone())
    );
    assert_eq!(reopened.list_paired_access_policies().unwrap(), [policy]);
    assert_eq!(
        PersistentStore::verify_existing_read_only(&fixture.0, protector())
            .unwrap()
            .format_version,
        3
    );
}

#[test]
fn old_trust_and_unknown_keys_never_implicitly_enable_paired_access() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    store
        .insert_trusted_device(peer.key_id(), peer.public_key(), 1, TrustState::Trusted)
        .unwrap();
    assert!(store.paired_access_policy(peer.key_id()).unwrap().is_none());
    assert!(store.paired_access_policy("unknown").unwrap().is_none());
    assert!(store.list_paired_access_policies().unwrap().is_empty());
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(1);
    assert!(store
        .enroll_paired_access_guarded(request, 7, || Ok(()))
        .is_err());
    assert!(audits(&store).is_empty());
}

#[test]
fn existing_ceiling_can_only_be_explicitly_enabled_for_a_subset() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    store
        .insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            audit("pair.approved"),
        )
        .unwrap();
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(1);
    assert!(store
        .enroll_paired_access_guarded(request.clone(), 8, || Ok(()))
        .is_err());
    assert!(store.paired_access_policy(peer.key_id()).unwrap().is_none());
    request.scopes = vec!["screen.view".to_owned()];
    let policy = store
        .enroll_paired_access_guarded(request, 8, || Ok(()))
        .unwrap();
    assert!(policy.enabled);
    assert_eq!(policy.scopes, ["screen.view"]);
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        ["screen.view"]
    );
}

#[test]
fn revisions_and_late_local_guard_are_atomic() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let result = store.enroll_paired_access_guarded(enrollment(&peer), 7, || {
        Err(StoreError::TrustTransition("local actor lost".to_owned()))
    });
    assert!(result.is_err());
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(store.list_paired_access_policies().unwrap().is_empty());
    assert!(audits(&store).is_empty());
    let policy = enroll(&store, &peer);
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(1);
    // An absent-policy expectation cannot overwrite an existing enrollment.
    assert!(store
        .enroll_paired_access_guarded(request.clone(), 8, || Ok(()))
        .is_err());
    request.expected_policy_revision = 1;
    request.expected_trust_revision = Some(2);
    assert!(store
        .enroll_paired_access_guarded(request.clone(), 8, || Ok(()))
        .is_err());
    request.expected_trust_revision = Some(1);
    assert!(store
        .enroll_paired_access_guarded(request, 8, || Err(StoreError::TrustTransition(
            "local actor lost".to_owned()
        )))
        .is_err());
    assert_eq!(
        store.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy)
    );
    assert_eq!(audits(&store).len(), 1);
}

#[test]
fn owner_is_required_and_cannot_be_changed_by_reenrollment() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let mut request = enrollment(&peer);
    request.owner_user_sid_hash = [0; 32];
    assert!(store
        .enroll_paired_access_guarded(request, 7, || Ok(()))
        .is_err());
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    let policy = enroll(&store, &peer);
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(1);
    request.expected_policy_revision = 1;
    request.owner_user_sid_hash = [8; 32];
    assert!(store
        .enroll_paired_access_guarded(request, 8, || Ok(()))
        .is_err());
    assert_eq!(
        store.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy)
    );
    assert_eq!(audits(&store).len(), 1);
}

#[test]
fn policy_revoke_is_a_durable_cas_tombstone_and_guard_failure_keeps_authority() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let policy = enroll(&store, &peer);
    assert!(store
        .revoke_paired_access_guarded(peer.key_id(), 1, 8, || Err(StoreError::TrustTransition(
            "local actor lost".to_owned()
        )))
        .is_err());
    assert_eq!(
        store.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy)
    );
    let revoked = store
        .revoke_paired_access_guarded(peer.key_id(), 1, 8, || Ok(()))
        .unwrap();
    assert!(!revoked.enabled);
    assert_eq!(revoked.revision, 2);
    assert!(store
        .revoke_paired_access_guarded(peer.key_id(), 1, 9, || Ok(()))
        .is_err());
    assert!(store
        .revoke_paired_access_guarded("unknown", 0, 9, || Ok(()))
        .is_err());
    drop(store);
    assert_eq!(
        fixture.open().paired_access_policy(peer.key_id()).unwrap(),
        Some(revoked)
    );
}

#[test]
fn trust_changes_disable_authority_without_corrupting_the_store_or_reviving_it() {
    for next in [
        TrustState::Suspended,
        TrustState::Revoked,
        TrustState::Trusted,
    ] {
        let fixture = Fixture::new();
        let store = fixture.open();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        enroll(&store, &peer);
        store.transition_trust(peer.key_id(), 1, next).unwrap();
        assert!(store.paired_access_policy(peer.key_id()).unwrap().is_none());
        let projection = store.list_paired_access_policies().unwrap();
        assert_eq!(projection.len(), 1);
        assert!(!projection[0].enabled);
        store.verify_audit_chain().unwrap();
        drop(store);
        let reopened = fixture.open();
        assert!(reopened
            .paired_access_policy(peer.key_id())
            .unwrap()
            .is_none());
        if next != TrustState::Trusted {
            let mut request = enrollment(&peer);
            request.expected_trust_revision = Some(2);
            request.expected_policy_revision = 1;
            assert!(reopened
                .enroll_paired_access_guarded(request, 9, || Ok(()))
                .is_err());
        }
        // Disabling is still possible after the trust record became inactive.
        assert!(
            !reopened
                .revoke_paired_access_guarded(peer.key_id(), 1, 9, || Ok(()))
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn invalid_scope_or_identity_never_creates_a_trust_record() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    for bad in [
        vec![],
        vec!["file.read".to_owned()],
        vec!["screen.view".to_owned(); 2],
    ] {
        let mut request = enrollment(&peer);
        request.scopes = bad;
        assert!(store
            .enroll_paired_access_guarded(request, 7, || Ok(()))
            .is_err());
    }
    for change in 0..3 {
        let mut request = enrollment(&peer);
        match change {
            0 => request.public_key[0] ^= 1,
            1 => request.epoch = 0,
            _ => request.epoch = u64::MAX,
        }
        assert!(store
            .enroll_paired_access_guarded(request, 7, || Ok(()))
            .is_err());
    }
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(audits(&store).is_empty());
}

#[test]
fn stale_or_deleted_or_tampered_policy_blob_fails_every_authenticated_read() {
    for mutation in 0..4 {
        let fixture = Fixture::new();
        let store = fixture.open();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        enroll(&store, &peer);
        let connection = Connection::open(&fixture.0).unwrap();
        let old = blob(&connection, &peer);
        store
            .revoke_paired_access_guarded(peer.key_id(), 1, 8, || Ok(()))
            .unwrap();
        let name = format!("paired_access_policy_v1:{}", peer.key_id());
        match mutation {
            0 => {
                connection
                    .execute(
                        "UPDATE store_secrets SET protected_blob = ?1 WHERE name = ?2",
                        rusqlite::params![old, name],
                    )
                    .unwrap();
            }
            1 => {
                connection
                    .execute("DELETE FROM store_secrets WHERE name = ?1", [&name])
                    .unwrap();
            }
            2 => {
                let mut current = blob(&connection, &peer);
                current[0] ^= 1;
                connection
                    .execute(
                        "UPDATE store_secrets SET protected_blob = ?1 WHERE name = ?2",
                        rusqlite::params![current, name],
                    )
                    .unwrap();
            }
            _ => {
                connection
                    .execute(
                        "UPDATE store_secrets SET name = ?1 WHERE name = ?2",
                        rusqlite::params![
                            format!("paired_access_policy_v1:{}", "f".repeat(64)),
                            name
                        ],
                    )
                    .unwrap();
            }
        }
        assert!(store.paired_access_policy(peer.key_id()).is_err());
        assert!(store.list_paired_access_policies().is_err());
        assert!(store.list_trusted_devices(true).is_err());
        assert!(store.verify_audit_chain().is_err());
        assert!(PersistentStore::verify_existing_read_only(&fixture.0, protector()).is_err());
        drop(connection);
        drop(store);
        assert!(PersistentStore::open(&fixture.0, protector()).is_err());
    }
}

#[test]
fn orphan_policy_blob_and_cross_store_ciphertext_are_rejected() {
    let a = Fixture::new();
    let b = Fixture::new();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store_a = a.open();
    let store_b = b.open();
    enroll(&store_a, &peer);
    enroll(&store_b, &peer);
    let connection_a = Connection::open(&a.0).unwrap();
    let connection_b = Connection::open(&b.0).unwrap();
    let foreign = blob(&connection_a, &peer);
    connection_b
        .execute(
            "UPDATE store_secrets SET protected_blob = ?1 WHERE name = ?2",
            rusqlite::params![
                foreign,
                format!("paired_access_policy_v1:{}", peer.key_id())
            ],
        )
        .unwrap();
    assert!(store_b.paired_access_policy(peer.key_id()).is_err());
    connection_a
        .execute(
            "INSERT INTO store_secrets(name, protected_blob) VALUES (?1, x'01')",
            [format!("paired_access_policy_v1:{}", "e".repeat(64))],
        )
        .unwrap();
    assert!(store_a.list_paired_access_policies().is_err());
}

#[test]
fn policy_audit_mutation_or_deletion_cannot_be_resealed_by_a_normal_write() {
    for sql in [
        "UPDATE audit_events SET details_json = '{}' WHERE action = 'paired_access.policy.v1'",
        "DELETE FROM audit_events WHERE action = 'paired_access.policy.v1'",
        "UPDATE audit_events SET action = 'ordinary.action' WHERE action = 'paired_access.policy.v1'",
    ] {
        let fixture = Fixture::new();
        let store = fixture.open();
        let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        enroll(&store, &peer);
        Connection::open(&fixture.0).unwrap().execute(sql, []).unwrap();
        assert!(store.paired_access_policy(peer.key_id()).is_err());
        assert!(store.append_audit(audit("ordinary.action")).is_err());
        drop(store);
        assert!(PersistentStore::open(&fixture.0, protector()).is_err());
    }
}

#[test]
fn concurrent_policy_updates_have_one_cas_winner_and_one_durable_commitment() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    enroll(&store, &peer);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = fixture.0.clone();
            let key = peer.key_id().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = PersistentStore::open(path, protector()).unwrap();
                barrier.wait();
                store.revoke_paired_access_guarded(&key, 1, 9, || Ok(()))
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    assert_eq!(audits(&store).len(), 2);
    assert_eq!(
        store
            .paired_access_policy(peer.key_id())
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}

#[test]
fn failed_policy_protection_rolls_back_new_trust_ceiling_audit_and_seal() {
    struct FailingPolicyProtector(AeadSecretProtector);
    impl SecretProtector for FailingPolicyProtector {
        fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
            if purpose.starts_with(b"MRD_PAIRED_ACCESS_POLICY_V1") {
                return Err("private fixture error must not escape".to_owned());
            }
            self.0.protect(purpose, plaintext)
        }
        fn unprotect(&self, purpose: &[u8], ciphertext: &[u8]) -> Result<SecretBytes, String> {
            self.0.unprotect(purpose, ciphertext)
        }
    }
    let fixture = Fixture::new();
    let store = PersistentStore::open(
        &fixture.0,
        Arc::new(FailingPolicyProtector(
            AeadSecretProtector::from_key([91; 32]).unwrap(),
        )),
    )
    .unwrap();
    let connection = Connection::open(&fixture.0).unwrap();
    let before: (u64, Vec<u8>) = connection
        .query_row(
            "SELECT generation, manifest_seal FROM store_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let error = store
        .enroll_paired_access_guarded(enrollment(&peer), 7, || Ok(()))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "secret protection failed: paired policy protection failed"
    );
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(store
        .trust_permission_ceiling(peer.key_id())
        .unwrap()
        .is_empty());
    assert!(store.list_paired_access_policies().unwrap().is_empty());
    assert!(audits(&store).is_empty());
    let after: (u64, Vec<u8>) = connection
        .query_row(
            "SELECT generation, manifest_seal FROM store_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn local_guard_is_rechecked_after_waiting_for_the_sqlite_write_lock() {
    use std::{
        sync::{atomic::AtomicBool, mpsc},
        time::Duration,
    };
    let fixture = Fixture::new();
    let store = Arc::new(fixture.open());
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let blocker = Connection::open(&fixture.0).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let actor_current = Arc::new(AtomicBool::new(true));
    let late_actor = actor_current.clone();
    let worker_store = store.clone();
    let request = enrollment(&peer);
    let (started, started_rx) = mpsc::channel();
    let (called, called_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        worker_store.enroll_paired_access_guarded(request, 7, || {
            called.send(()).unwrap();
            if late_actor.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(StoreError::TrustTransition(
                    "local actor changed".to_owned(),
                ))
            }
        })
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let prematurely_called = called_rx.recv_timeout(Duration::from_millis(200)).is_ok();
    actor_current.store(false, Ordering::SeqCst);
    blocker.execute_batch("ROLLBACK").unwrap();
    let result = worker.join().unwrap();
    assert!(
        !prematurely_called,
        "guard must not execute before the write lock"
    );
    assert!(matches!(result, Err(StoreError::TrustTransition(_))));
    assert!(store.trust_record(peer.key_id()).unwrap().is_none());
    assert!(store.list_paired_access_policies().unwrap().is_empty());
    assert!(audits(&store).is_empty());
}

#[test]
fn explicit_same_owner_repair_advances_policy_without_changing_trust_or_ceiling() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    enroll(&store, &peer);
    let trust = store.trust_record(peer.key_id()).unwrap().unwrap();
    let ceiling = store.trust_permission_ceiling(peer.key_id()).unwrap();
    store
        .revoke_paired_access_guarded(peer.key_id(), 1, 8, || Ok(()))
        .unwrap();
    store.append_audit(audit("session.closed")).unwrap();
    let mut request = enrollment(&peer);
    request.expected_trust_revision = Some(trust.revision);
    request.expected_policy_revision = 2;
    request.scopes = vec!["screen.view".to_owned()];
    let policy = store
        .enroll_paired_access_guarded(request, 9, || Ok(()))
        .unwrap();
    assert_eq!(policy.revision, 3);
    assert!(policy.enabled);
    assert_eq!(store.trust_record(peer.key_id()).unwrap().unwrap(), trust);
    assert_eq!(
        store.trust_permission_ceiling(peer.key_id()).unwrap(),
        ceiling
    );
    assert_eq!(
        store.paired_access_policy(peer.key_id()).unwrap(),
        Some(policy)
    );
    assert_eq!(audits(&store).len(), 4);
}
