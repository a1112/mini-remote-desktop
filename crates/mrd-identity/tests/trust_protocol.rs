use mrd_identity::{
    sas_code, DeviceIdentity, ReplayError, ReplayWindow, RotationError, UnattendedCredential,
};
use ring::rand::SystemRandom;

#[test]
fn duplicate_nonce_and_counter_rollback_are_rejected() {
    let mut window = ReplayWindow::new(16);
    window.accept(10, [1; 16]).unwrap();
    assert_eq!(window.accept(10, [1; 16]), Err(ReplayError::DuplicateNonce));
    assert_eq!(window.accept(9, [2; 16]), Err(ReplayError::CounterRollback));
}

#[test]
fn sas_is_symmetric_and_transcript_bound() {
    let transcript = b"controller-target-transcript";
    assert_eq!(sas_code(transcript), sas_code(transcript));
    assert_ne!(sas_code(transcript), sas_code(b"tampered-transcript"));
}

#[test]
fn rotation_requires_old_key_and_increasing_epoch() {
    let rng = SystemRandom::new();
    let old = DeviceIdentity::generate(&rng).unwrap();
    let next = DeviceIdentity::generate(&rng).unwrap();
    let proof = old.sign_rotation(next.public_key().to_vec(), 2).unwrap();
    assert!(proof.verify(old.public_key(), 1, false).is_ok());
    assert_eq!(
        proof.verify(old.public_key(), 2, false),
        Err(RotationError::EpochNotIncreasing)
    );
    assert_eq!(
        proof.verify(old.public_key(), 1, true),
        Err(RotationError::OldKeyRevoked)
    );
}

#[test]
fn unattended_proof_is_transcript_bound_and_not_replayable() {
    let credential = UnattendedCredential::generate(&SystemRandom::new()).unwrap();
    let proof = credential.prove(b"session-transcript", [4; 16]);
    assert!(credential.verify(b"session-transcript", [4; 16], &proof));
    assert!(!credential.verify(b"tampered", [4; 16], &proof));
    assert!(!credential.verify(b"session-transcript", [5; 16], &proof));
}

#[test]
fn identity_debug_output_redacts_private_key_material() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let debug = format!("{identity:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains(&format!("{:?}", identity.private_pkcs8())));
}
