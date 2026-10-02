use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_identity::DeviceIdentity;
use mrd_proto::{BackendRole, DeviceId};
use mrd_signal_proto::{
    AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, RegisterPayload, SignalEnvelope,
};
use realtime_server::{
    BackendTokenError, BackendTokenVerifier, ConnectionId, CoreConfig, JwtBackendTokenVerifier,
    RealtimeCore,
};
use ring::{hmac, rand::SystemRandom};
use serde_json::{json, Value};
use std::sync::Arc;

const SECRET: &[u8] = b"test-backend-signing-secret-32-bytes-minimum";
const NOW: u64 = 1_800_000_000_000;

fn verifier() -> Arc<dyn BackendTokenVerifier> {
    Arc::new(
        JwtBackendTokenVerifier::new(SECRET, "rdesk-backend".into(), "rdesk-signaling".into())
            .unwrap(),
    )
}

fn claims(role: BackendRole, key: &str) -> Value {
    json!({
        "sub": "123456789012", "device_id": "123456789012", "device_key_id": key,
        "role": role, "token_type": "signaling", "iss": "rdesk-backend",
        "aud": "rdesk-signaling", "iat": NOW / 1000, "exp": NOW / 1000 + 300,
    })
}

fn signed_parts(header: &str, payload: &str, secret: &[u8]) -> String {
    let encoded = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, secret),
        encoded.as_bytes(),
    );
    format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

fn signed(payload: &Value) -> String {
    signed_parts(
        r#"{"alg":"HS256","typ":"JWT"}"#,
        &payload.to_string(),
        SECRET,
    )
}

#[test]
fn accepts_backend_signaling_credentials_for_both_roles() {
    for role in [BackendRole::Controller, BackendRole::Agent] {
        let key = "ab".repeat(32);
        let result = verifier()
            .verify(&signed(&claims(role.clone(), &key)), NOW + 1)
            .unwrap();
        assert_eq!(result.device_id, DeviceId("123456789012".into()));
        assert_eq!(result.device_key_id, key);
        assert_eq!(result.role, role);
        assert_eq!(result.expires_at_ms, NOW + 300_000);
    }
}

#[test]
fn accepts_existing_device_ids_and_exact_lifetime_and_clock_skew_boundaries() {
    let mut payload = claims(BackendRole::Agent, &"ab".repeat(32));
    payload["sub"] = json!("123456789012-dead");
    payload["device_id"] = payload["sub"].clone();
    payload["iat"] = json!(NOW / 1000 + 60);
    payload["exp"] = json!(NOW / 1000 + 3660);
    assert!(verifier().verify(&signed(&payload), NOW).is_ok());
    for device_id in ["", "bad/device", "bad device", "设备", &"a".repeat(65)] {
        payload["sub"] = json!(device_id);
        payload["device_id"] = json!(device_id);
        assert!(verifier().verify(&signed(&payload), NOW).is_err());
    }
}

#[test]
fn rejects_credentials_with_invalid_security_claims() {
    let base = claims(BackendRole::Agent, &"ab".repeat(32));
    for (field, value) in [
        ("iss", json!("another-issuer")),
        ("aud", json!("rdesk-device")),
        ("token_type", json!("device")),
        ("role", json!("device")),
        ("sub", json!("other-device")),
        ("device_id", json!("lan-device")),
        ("device_key_id", json!("AB".repeat(32))),
        ("device_key_id", json!("ab".repeat(31))),
        ("exp", json!(NOW / 1000)),
        ("exp", json!(NOW / 1000 - 1)),
        ("exp", json!(NOW / 1000 + 3601)),
        ("iat", json!(NOW / 1000 + 61)),
        ("iat", json!(-1)),
        ("exp", json!("tomorrow")),
    ] {
        let mut altered = base.clone();
        altered[field] = value;
        assert!(
            verifier().verify(&signed(&altered), NOW).is_err(),
            "accepted invalid {field}"
        );
    }
    for field in [
        "sub",
        "device_id",
        "device_key_id",
        "role",
        "token_type",
        "iss",
        "aud",
        "iat",
        "exp",
    ] {
        let mut altered = base.clone();
        altered.as_object_mut().unwrap().remove(field);
        assert!(
            verifier().verify(&signed(&altered), NOW).is_err(),
            "accepted missing {field}"
        );
    }
}

#[test]
fn rejects_tampering_algorithms_and_ambiguous_json() {
    let payload = claims(BackendRole::Agent, &"ab".repeat(32)).to_string();
    for header in [
        r#"{"alg":"none","typ":"JWT"}"#,
        r#"{"alg":"HS384","typ":"JWT"}"#,
        r#"{"alg":"HS256","alg":"none","typ":"JWT"}"#,
        r#"{"alg":"HS256","typ":"JWT","crit":["b64"],"b64":false}"#,
    ] {
        assert!(verifier()
            .verify(&signed_parts(header, &payload, SECRET), NOW)
            .is_err());
    }
    for duplicate in [
        r#""role":"Controller""#,
        r#""exp":1800000300"#,
        r#""device_id":"123456789012""#,
        r#""device_key_id":"ab""#,
    ] {
        let payload = format!("{{{duplicate},{}", &payload[1..]);
        assert!(verifier()
            .verify(
                &signed_parts(r#"{"alg":"HS256","typ":"JWT"}"#, &payload, SECRET),
                NOW
            )
            .is_err());
    }
    let token = signed_parts(
        r#"{"alg":"HS256","typ":"JWT"}"#,
        &payload,
        b"wrong-signing-secret-32-bytes-minimum",
    );
    assert!(verifier().verify(&token, NOW).is_err());
    for malformed in ["", "a.b", "a.b.c.d", "@@@.@@@.@@@"] {
        assert_eq!(
            verifier().verify(malformed, NOW),
            Err(BackendTokenError::Invalid)
        );
    }
    assert!(verifier()
        .verify(&format!("{}.a.a", "a".repeat(8192)), NOW)
        .is_err());
    let regular_device = json!({
        "sub": "internal-database-id", "device_id": "123456789012", "auth_version": 1,
        "role": "device", "token_type": "device", "iss": "rdesk-backend",
        "aud": "rdesk-device", "iat": NOW / 1000, "exp": NOW / 1000 + 300,
    });
    assert!(verifier().verify(&signed(&regular_device), NOW).is_err());
}

#[test]
fn rejects_invalid_config_and_redacts_signing_material() {
    for (secret, issuer, audience) in [
        (&b"short"[..], "rdesk-backend", "rdesk-signaling"),
        (SECRET, "", "rdesk-signaling"),
        (SECRET, "rdesk backend", "rdesk-signaling"),
        (SECRET, "rdesk-backend", ""),
        (SECRET, "rdesk-backend", "rdesk\nsignaling"),
    ] {
        let error =
            JwtBackendTokenVerifier::new(secret, issuer.into(), audience.into()).unwrap_err();
        assert!(error.to_string().contains("MRD_REALTIME_JWT_"));
        assert!(!error
            .to_string()
            .contains(std::str::from_utf8(SECRET).unwrap()));
    }
    let verifier =
        JwtBackendTokenVerifier::new(SECRET, "rdesk-backend".into(), "rdesk-signaling".into())
            .unwrap();
    let debug = format!("{verifier:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains(std::str::from_utf8(SECRET).unwrap()));
}

fn core() -> RealtimeCore {
    RealtimeCore::new(
        CoreConfig {
            server_device_id: DeviceId("signal-server".into()),
            challenge_ttl_ms: 10_000,
            presence_ttl_ms: 30_000,
            route_ttl_ms: 60_000,
            max_connections: 32,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        },
        verifier(),
    )
    .unwrap()
}

fn register(
    core: &mut RealtimeCore,
    jwt_claims: Value,
    identity: &DeviceIdentity,
    role: BackendRole,
) -> Result<Vec<realtime_server::Delivery>, realtime_server::RealtimeError> {
    let connection = ConnectionId::from_bytes([1; 16]).unwrap();
    let challenge = core.open_connection(connection, NOW).unwrap();
    let register = AuthenticatedRegister::sign(
        identity,
        RegisterPayload {
            claims: AuthClaims {
                issuer_device_id: DeviceId("123456789012".into()),
                issuer_key_id: identity.key_id().into(),
                intended_peer_device_id: DeviceId("signal-server".into()),
                issued_at_ms: NOW,
                expires_at_ms: NOW + 5000,
                counter: 1,
                nonce: [7; 16],
            },
            role,
            device_name: "test device".into(),
            backend_device_token: signed(&jwt_claims),
            challenge_id: challenge.challenge_id,
            challenge_nonce: challenge.challenge_nonce,
        },
    )
    .unwrap();
    core.handle(
        connection,
        SignalEnvelope::new(AuthenticatedSignalMessage::Register(register)),
        NOW + 1,
    )
}

#[test]
fn authenticates_signed_registration_with_real_backend_credential() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let mut core = core();
    let result = register(
        &mut core,
        claims(BackendRole::Agent, identity.key_id()),
        &identity,
        BackendRole::Agent,
    )
    .unwrap();
    assert!(matches!(
        result[0].envelope.message,
        AuthenticatedSignalMessage::Registered(_)
    ));
    assert!(core.is_present(&DeviceId("123456789012".into())));
}

#[test]
fn real_backend_credential_keeps_key_device_and_role_bound_to_signed_registration() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    for binding in ["device", "key", "role"] {
        let mut jwt_claims = claims(BackendRole::Agent, identity.key_id());
        match binding {
            "device" => {
                jwt_claims["sub"] = json!("999999999999");
                jwt_claims["device_id"] = json!("999999999999");
            }
            "key" => jwt_claims["device_key_id"] = json!("ab".repeat(32)),
            "role" => jwt_claims["role"] = json!("Controller"),
            _ => unreachable!(),
        }
        let mut core = core();
        let error = register(&mut core, jwt_claims, &identity, BackendRole::Agent).unwrap_err();
        assert!(
            matches!(
                error,
                realtime_server::RealtimeError::Auth(
                    realtime_server::auth::AuthError::TokenBindingMismatch
                )
            ),
            "{binding}: {error}"
        );
        assert_eq!(core.presence_count(), 0);
    }
}

#[test]
fn executable_fails_startup_when_signaling_verifier_config_is_missing_or_invalid() {
    for secret in [
        None,
        Some("short"),
        Some("sensitive-test-signing-key-32-bytes-minimum"),
    ] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_realtime-server"));
        command.env_clear().env("MRD_REALTIME_BIND", "127.0.0.1:0");
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        if let Some(secret) = secret {
            command.env("MRD_REALTIME_JWT_SECRET", secret);
            if secret == "short" {
                command.env("MRD_REALTIME_JWT_ISSUER", "rdesk-backend");
            }
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none()
            && started.elapsed() < std::time::Duration::from_secs(3)
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("realtime-server started without a valid JWT verifier configuration");
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("MRD_REALTIME_JWT_"), "{diagnostic}");
        assert!(!diagnostic.contains("sensitive-test-signing-key"));
    }
}
