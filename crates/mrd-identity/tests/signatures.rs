use mrd_identity::{DeviceIdentity, IdentityError, PermissionScope, SessionIntent, SignedGrantPayload};
use ring::rand::SystemRandom;
use std::collections::BTreeSet;

fn intent() -> SessionIntent {
    SessionIntent {
        session_id: "session-1".into(),
        controller_key_id: "controller".into(),
        target_key_id: "target".into(),
        requested_scopes: BTreeSet::from([PermissionScope::ScreenView]),
    }
}

#[test]
fn signed_intent_rejects_target_or_scope_tampering() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let mut signed = identity.sign_intent(intent()).unwrap();
    signed.payload.requested_scopes.insert(PermissionScope::FileWrite);
    assert_eq!(signed.verify(), Err(IdentityError::InvalidSignature));
}

#[test]
fn signed_grant_is_bound_to_both_peer_keys() {
    let controller = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let target = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let other = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let payload = SignedGrantPayload {
        session_id: "session-1".into(),
        controller_public_key: controller.public_key().to_vec(),
        target_public_key: target.public_key().to_vec(),
        nonce: [9; 16],
    };
    let signed = controller.sign_grant(payload).unwrap();
    assert!(signed.verify_for(controller.public_key(), target.public_key()).is_ok());
    assert!(signed.verify_for(other.public_key(), target.public_key()).is_err());
}
