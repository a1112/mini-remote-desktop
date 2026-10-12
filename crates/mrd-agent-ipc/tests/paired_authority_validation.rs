use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use mrd_agent_ipc::*;
use mrd_proto::{DeviceId, SessionId};
use mrd_session::PermissionScope;

struct Pinned(SigningKey);
impl ExecuteGrantVerifier for Pinned {
    fn verify(&self, issuer: &[u8; 32], bytes: &[u8], signature: &[u8; 64]) -> bool {
        *issuer == derive_execute_grant_issuer_key_id(&self.0.verifying_key().to_bytes())
            && self
                .0
                .verifying_key()
                .verify(bytes, &Signature::from_bytes(signature))
                .is_ok()
    }
}
fn fixture() -> (
    Pinned,
    PairedSessionAuthority,
    PairedSessionAuthorityContext,
) {
    let verifier = Pinned(SigningKey::from_bytes(&[19; 32]));
    let issuer = derive_execute_grant_issuer_key_id(&verifier.0.verifying_key().to_bytes());
    let mut authority = PairedSessionAuthority {
        claims: PairedSessionAuthorityClaims {
            authority_id: [1; 32],
            registration_id: [2; 16],
            registration_epoch: 3,
            session_id: SessionId("paired-session".into()),
            peer: PeerBinding {
                device_id: DeviceId("paired-peer".into()),
                key_id: [4; 32],
            },
            scopes: [PermissionScope::ScreenView, PermissionScope::InputKeyboard]
                .into_iter()
                .collect(),
            policy_revision: 5,
            windows_session_id: 1,
            desktop_epoch: 6,
            desktop_kind: DesktopKind::Default,
            issued_at_ms: 1000,
            not_before_ms: 1000,
            expires_at_ms: 301000,
            audience: GrantAudience::SessionAgent,
        },
        issuer_key_id: issuer,
        signature: [0; 64],
    };
    authority.signature = verifier.0.sign(&authority.signing_bytes()).to_bytes();
    let context = PairedSessionAuthorityContext {
        registration_id: [2; 16],
        registration_epoch: 3,
        windows_session_id: 1,
        desktop_epoch: 6,
        desktop_kind: DesktopKind::Default,
        now_ms: 1500,
        expected_issuer_key_id: issuer,
    };
    (verifier, authority, context)
}
#[test]
fn paired_signature_covers_every_authority_binding() {
    let (verifier, authority, context) = fixture();
    assert_eq!(
        validate_paired_session_authority(&authority, &context, &verifier),
        Ok(())
    );
    for change in 0..14 {
        let mut altered = authority.clone();
        match change {
            0 => altered.claims.authority_id[0] ^= 1,
            1 => altered.claims.registration_id[0] ^= 1,
            2 => altered.claims.registration_epoch += 1,
            3 => altered.claims.session_id.0.push('x'),
            4 => altered.claims.peer.device_id.0.push('x'),
            5 => altered.claims.peer.key_id[0] ^= 1,
            6 => {
                altered.claims.scopes.insert(PermissionScope::InputPointer);
            }
            7 => altered.claims.policy_revision += 1,
            8 => altered.claims.windows_session_id += 1,
            9 => altered.claims.desktop_epoch += 1,
            10 => altered.claims.issued_at_ms += 1,
            11 => altered.claims.not_before_ms += 1,
            12 => altered.claims.expires_at_ms -= 1,
            _ => altered.issuer_key_id[0] ^= 1,
        }
        assert!(
            validate_paired_session_authority(&altered, &context, &verifier).is_err(),
            "alteration {change}"
        );
    }
}
#[test]
fn paired_authority_requires_exact_live_registration_default_desktop_and_time() {
    let (verifier, authority, context) = fixture();
    for change in 0..8 {
        let mut stale = context;
        match change {
            0 => stale.registration_id[0] ^= 1,
            1 => stale.registration_epoch += 1,
            2 => stale.windows_session_id += 1,
            3 => stale.desktop_epoch += 1,
            4 => stale.desktop_kind = DesktopKind::Unknown,
            5 => stale.desktop_kind = DesktopKind::Secure,
            6 => stale.now_ms = 999,
            _ => stale.now_ms = 301000,
        }
        assert!(validate_paired_session_authority(&authority, &stale, &verifier).is_err());
    }
}
#[test]
fn malformed_or_expanded_scope_authority_is_invalid() {
    let (_, authority, _) = fixture();
    for change in 0..9 {
        let mut claims = authority.claims.clone();
        match change {
            0 => claims.authority_id = [0; 32],
            1 => claims.policy_revision = 0,
            2 => claims.desktop_kind = DesktopKind::Winlogon,
            3 => {
                claims.scopes.insert(PermissionScope::SecureDesktopControl);
            }
            4 => {
                claims.scopes.clear();
            }
            5 => claims.expires_at_ms += 1,
            6 => claims.not_before_ms = 999,
            7 => claims.expires_at_ms = 1000,
            _ => claims.peer.key_id = [0; 32],
        }
        assert!(validate_paired_authority_claims(&claims).is_err());
    }
}
#[test]
fn unknown_paired_wire_fields_and_audience_fail_closed() {
    let (_, authority, _) = fixture();
    let mut wire = serde_json::to_value(authority).unwrap();
    wire["claims"]["audience"] = serde_json::json!("untrusted_ui");
    assert!(serde_json::from_value::<PairedSessionAuthority>(wire.clone()).is_err());
    wire["claims"]["audience"] = serde_json::json!("session_agent");
    wire["claims"]["unrecognized"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PairedSessionAuthority>(wire).is_err());
}

#[test]
fn execute_signature_domain_cannot_install_paired_authority() {
    let (verifier, mut authority, context) = fixture();
    let paired = authority.signing_bytes();
    let mut other = AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT.to_vec();
    other.extend_from_slice(&paired[AGENT_PAIRED_AUTHORITY_SIGNATURE_CONTEXT.len()..]);
    authority.signature = verifier.0.sign(&other).to_bytes();
    assert_eq!(
        validate_paired_session_authority(&authority, &context, &verifier),
        Err(PairedSessionAuthorityValidationError::InvalidSignature)
    );
}
