use mrd_store_sqlite::{AeadSecretProtector, SecretProtector};

#[test]
fn encrypted_secrets_survive_reopening_and_bind_their_purpose() {
    let key = [17_u8; 32];
    let first = AeadSecretProtector::from_key(key).unwrap();
    let sealed = first
        .protect(b"device-credential", b"private-token")
        .unwrap();
    assert!(!sealed.windows(13).any(|part| part == b"private-token"));
    let reopened = AeadSecretProtector::from_key(key).unwrap();
    assert_eq!(
        reopened
            .unprotect(b"device-credential", &sealed)
            .unwrap()
            .as_ref(),
        b"private-token"
    );
    assert!(reopened.unprotect(b"another-device", &sealed).is_err());
    assert!(AeadSecretProtector::from_key([18_u8; 32])
        .unwrap()
        .unprotect(b"device-credential", &sealed)
        .is_err());
}

#[test]
fn secret_envelopes_reject_tampering_truncation_and_unknown_versions() {
    let protector = AeadSecretProtector::from_key([19_u8; 32]).unwrap();
    let sealed = protector.protect(b"identity", b"secret").unwrap();
    for end in 0..sealed.len() {
        assert!(protector.unprotect(b"identity", &sealed[..end]).is_err());
    }
    for index in 0..sealed.len() {
        let mut changed = sealed.clone();
        changed[index] ^= 1;
        assert!(protector.unprotect(b"identity", &changed).is_err());
    }
    let repeated = protector.protect(b"identity", b"secret").unwrap();
    assert_ne!(sealed, repeated);
}
