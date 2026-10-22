use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_proto::WanPermissionScopeV3;
use realtime_server::{BackendTokenVerifier, JwtBackendTokenVerifier};
use ring::hmac;
use serde_json::{json, Value};

const SECRET: &[u8] = b"guest-signaling-test-key-at-least-32-bytes";
const NOW: u64 = 1_800_000_000_000;

fn verifier() -> JwtBackendTokenVerifier {
    JwtBackendTokenVerifier::new(SECRET, "rdesk-backend".into(), "rdesk-signaling".into()).unwrap()
}

fn claims() -> Value {
    json!({
        "sub":"browser_0123456789abcdef0123456789abcdef",
        "device_id":"browser_0123456789abcdef0123456789abcdef",
        "device_key_id":"ab".repeat(32), "role":"Controller",
        "token_type":"guest_browser_signaling","authority_kind":"temporary_password",
        "iss":"rdesk-backend","aud":"rdesk-signaling",
        "iat":NOW/1000,"exp":NOW/1000+300,"user_id":null,"tenant_id":"tenant-target",
        "session_id":"session-guest-1","target_device_id":"123456789012",
        "allowed_scopes":["input.keyboard","input.pointer","screen.view"],
        "temporary_access_generation":1,"target_auth_version":2
    })
}

fn sign_raw(header: &str, payload: &str) -> String {
    let encoded = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, SECRET),
        encoded.as_bytes(),
    );
    format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

fn sign(value: &Value) -> String {
    sign_raw(r#"{"alg":"HS256","typ":"JWT"}"#, &value.to_string())
}

#[test]
fn temporary_password_guest_is_a_session_bound_browser_controller_without_an_account() {
    let verified = verifier()
        .verify(&sign(&claims()), NOW + 1)
        .expect("explicit temporary-password authority must authenticate without a synthetic user");
    assert_eq!(
        verified.device_id,
        DeviceId("browser_0123456789abcdef0123456789abcdef".into())
    );
    assert_eq!(verified.role, BackendRole::Controller);
    assert_eq!(verified.expires_at_ms, NOW + 300_000);
    let restriction = verified
        .browser
        .expect("guest must retain browser-only routing and lifetime constraints");
    assert_eq!(
        restriction.authority,
        realtime_server::auth::BrowserSignalingAuthority::TemporaryPassword {
            temporary_access_generation: 1,
            target_auth_version: 2
        }
    );
    assert_eq!(restriction.tenant_id, "tenant-target");
    assert_eq!(restriction.session_id, SessionId("session-guest-1".into()));
    assert_eq!(
        restriction.target_device_id,
        DeviceId("123456789012".into())
    );
    assert_eq!(
        restriction.allowed_scopes,
        vec![
            WanPermissionScopeV3::InputKeyboard,
            WanPermissionScopeV3::InputPointer,
            WanPermissionScopeV3::ScreenView
        ]
    );
}

#[test]
fn guest_authority_requires_exact_null_user_tenant_generation_and_auth_version_fields() {
    let base = claims();
    for (field, value) in [
        ("authority_kind", json!("account")),
        ("authority_kind", json!(null)),
        ("user_id", json!("user-1")),
        ("tenant_id", json!(null)),
        ("tenant_id", json!("")),
        ("temporary_access_generation", json!(0)),
        ("temporary_access_generation", json!(-1)),
        ("temporary_access_generation", json!(true)),
        ("temporary_access_generation", json!(9223372036854775808u64)),
        ("target_auth_version", json!(0)),
        ("target_auth_version", json!(-1)),
        ("target_auth_version", json!(9223372036854775808u64)),
        ("unexpected", json!("ignored")),
    ] {
        let mut altered = base.clone();
        altered[field] = value;
        assert!(
            verifier().verify(&sign(&altered), NOW).is_err(),
            "accepted guest {field}"
        );
    }
    for field in [
        "authority_kind",
        "user_id",
        "tenant_id",
        "temporary_access_generation",
        "target_auth_version",
        "session_id",
        "target_device_id",
        "allowed_scopes",
    ] {
        let mut altered = base.clone();
        altered.as_object_mut().unwrap().remove(field);
        assert!(
            verifier().verify(&sign(&altered), NOW).is_err(),
            "accepted missing guest {field}"
        );
    }
}

#[test]
fn guest_signaling_rejects_machine_account_http_roles_key_and_scope_substitution() {
    let base = claims();
    for (field, value) in [
        ("role", json!("Agent")),
        ("role", json!("Peer")),
        ("token_type", json!("signaling")),
        ("token_type", json!("browser_signaling")),
        ("token_type", json!("guest_browser_http")),
        ("aud", json!("rdesk-guest-browser-http")),
        ("iss", json!("other-issuer")),
        ("sub", json!("other-browser")),
        ("device_id", json!("123456789012")),
        ("device_key_id", json!("AB".repeat(32))),
        ("target_device_id", base["device_id"].clone()),
        ("session_id", json!("")),
        ("allowed_scopes", json!(["input.keyboard"])),
        ("allowed_scopes", json!(["screen.view", "input.pointer"])),
        ("allowed_scopes", json!(["screen.view", "screen.view"])),
        ("allowed_scopes", json!(["file.write", "screen.view"])),
        ("exp", json!(NOW / 1000 + 601)),
    ] {
        let mut altered = base.clone();
        altered[field] = value;
        assert!(
            verifier().verify(&sign(&altered), NOW).is_err(),
            "accepted guest substitution {field}"
        );
    }
    for token_type in ["signaling", "browser_signaling"] {
        let mut stripped = base.clone();
        stripped["token_type"] = json!(token_type);
        for field in [
            "authority_kind",
            "temporary_access_generation",
            "target_auth_version",
        ] {
            stripped.as_object_mut().unwrap().remove(field);
        }
        assert!(
            verifier().verify(&sign(&stripped), NOW).is_err(),
            "guest cannot become an account or machine principal"
        );
    }
}

#[test]
fn guest_claims_reject_duplicate_fields_even_when_the_last_value_is_valid() {
    let base = claims().to_string();
    for prefix in [
        r#""user_id":"synthetic-user","#,
        r#""authority_kind":"account","#,
        r#""temporary_access_generation":0,"#,
        r#""target_auth_version":0,"#,
        r#""target_device_id":"other-target","#,
        r#""token_type":"signaling","#,
    ] {
        let ambiguous = format!("{{{prefix}{}", &base[1..]);
        assert!(
            verifier()
                .verify(&sign_raw(r#"{"alg":"HS256","typ":"JWT"}"#, &ambiguous), NOW)
                .is_err(),
            "accepted duplicate guest field"
        );
    }
    assert!(verifier()
        .verify(
            &sign_raw(r#"{"alg":"none","alg":"HS256","typ":"JWT"}"#, &base),
            NOW
        )
        .is_err());
}

#[test]
fn guest_lifetime_and_database_generation_bounds_are_exact() {
    let mut boundary = claims();
    boundary["exp"] = json!(NOW / 1000 + 600);
    boundary["temporary_access_generation"] = json!(i64::MAX);
    boundary["target_auth_version"] = json!(i64::MAX);
    assert!(
        verifier().verify(&sign(&boundary), NOW).is_ok(),
        "approved ten-minute/i64 bounds must be accepted"
    );
    assert!(
        verifier().verify(&sign(&boundary), NOW + 600_000).is_err(),
        "expiry must be enforced without JWT skew extension"
    );
    boundary["allowed_scopes"] = json!(["screen.view"]);
    assert!(
        verifier().verify(&sign(&boundary), NOW + 1).is_ok(),
        "view-only guest authority is supported"
    );
}
