//! First-start enrollment proved by the protected installation's Ed25519 key.
//!
//! The signer receives canonical bytes and keeps its private key with its owner.
//! Neither server error bodies nor enrollment credentials enter diagnostics.

use crate::{DeviceRegistrationRequest, DeviceRegistrationResponse};
use ring::digest;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub const SIGNATURE_CONTEXT: &str = "MRD_DEVICE_SELF_ENROLLMENT_V1";
const MAX_REGISTRATION_BYTES: usize = 8192;
const MAX_CHALLENGE_BYTES: usize = 4096;
const MAX_CHALLENGE_TTL_MS: u64 = 120_000;

/// Stable, non-sensitive supervisor diagnostics. Backend bodies are discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfEnrollmentError {
    InvalidRequest,
    ConnectionFailed,
    ProtocolUnavailable,
    Rejected,
    MachineAlreadyRegistered,
    RateLimited,
    ChallengeExpired,
    InvalidChallenge,
    InvalidResponse,
    SigningFailed,
}

impl SelfEnrollmentError {
    pub const fn status_code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "public_auto_enrollment_invalid_request",
            Self::ConnectionFailed => "public_auto_enrollment_connection_failed",
            Self::ProtocolUnavailable => "public_auto_enrollment_unavailable",
            Self::Rejected => "public_auto_enrollment_rejected",
            Self::MachineAlreadyRegistered => "public_auto_enrollment_identity_conflict",
            Self::RateLimited => "public_auto_enrollment_rate_limited",
            Self::ChallengeExpired => "public_auto_enrollment_challenge_expired",
            Self::InvalidChallenge => "public_auto_enrollment_invalid_challenge",
            Self::InvalidResponse => "public_auto_enrollment_invalid_response",
            Self::SigningFailed => "public_auto_enrollment_signing_failed",
        }
    }
}

#[derive(Serialize)]
struct ChallengeRequest<'a> {
    protocol_version: u8,
    key_id: &'a str,
    public_key: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeResponse {
    protocol_version: u8,
    challenge_id: String,
    nonce: String,
    api_url: String,
    expires_at_ms: u64,
}

#[derive(Serialize)]
struct ClaimRequest<'a> {
    protocol_version: u8,
    challenge_id: &'a str,
    nonce: &'a str,
    key_id: &'a str,
    public_key: &'a str,
    registration_json: &'a str,
    signature: &'a str,
}

/// Obtain the same server-managed device identity on retries after a lost reply.
/// The caller signs with `SIGNATURE_CONTEXT`; it never exports the private key.
pub async fn self_register<F>(
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    key_id: &str,
    public_key: &[u8],
    signer: F,
) -> Result<DeviceRegistrationResponse, SelfEnrollmentError>
where
    F: FnOnce(&[u8]) -> Result<Vec<u8>, SelfEnrollmentError>,
{
    let challenge_endpoint = crate::registration_endpoint(api_base, "self-enrollment-challenge")
        .map_err(|_| SelfEnrollmentError::InvalidRequest)?;
    let claim_endpoint = crate::registration_endpoint(api_base, "self-register")
        .map_err(|_| SelfEnrollmentError::InvalidRequest)?;
    if public_key.len() != 32 || !lower_hex(key_id, 64) || sha256_hex(public_key) != key_id {
        return Err(SelfEnrollmentError::InvalidRequest);
    }
    let registration_json = Zeroizing::new(
        serde_json::to_string(payload).map_err(|_| SelfEnrollmentError::InvalidRequest)?,
    );
    if registration_json.len() > MAX_REGISTRATION_BYTES {
        return Err(SelfEnrollmentError::InvalidRequest);
    }
    let public_key = hex_encode(public_key);
    let client = crate::registration_client().map_err(|_| SelfEnrollmentError::ConnectionFailed)?;
    let response = client
        .post(challenge_endpoint)
        .json(&ChallengeRequest {
            protocol_version: 1,
            key_id,
            public_key: &public_key,
        })
        .send()
        .await
        .map_err(|_| SelfEnrollmentError::ConnectionFailed)?;
    check_response_status(response.status())?;
    let body = bounded_challenge_body(response).await?;
    let challenge: ChallengeResponse =
        serde_json::from_slice(&body).map_err(|_| SelfEnrollmentError::InvalidChallenge)?;
    validate_challenge(&challenge, api_base, unix_ms()?)?;
    let signing_bytes = canonical_bytes(&challenge, key_id, &registration_json);
    let signature = signer(&signing_bytes)?;
    if signature.len() != 64 {
        return Err(SelfEnrollmentError::SigningFailed);
    }
    // Signing must not let a challenge expire before its claim is transmitted.
    validate_challenge(&challenge, api_base, unix_ms()?)?;
    let response = client
        .post(claim_endpoint)
        .json(&ClaimRequest {
            protocol_version: 1,
            challenge_id: &challenge.challenge_id,
            nonce: &challenge.nonce,
            key_id,
            public_key: &public_key,
            registration_json: &registration_json,
            signature: &hex_encode(&signature),
        })
        .send()
        .await
        .map_err(|_| SelfEnrollmentError::ConnectionFailed)?;
    check_response_status(response.status())?;
    let registration = crate::parse_registration(response, true)
        .await
        .map_err(|_| SelfEnrollmentError::InvalidResponse)?;
    if registration.device_id.len() != 10
        || !registration
            .device_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(SelfEnrollmentError::InvalidResponse);
    }
    Ok(registration)
}

fn validate_challenge(
    value: &ChallengeResponse,
    api_base: &str,
    now_ms: u64,
) -> Result<(), SelfEnrollmentError> {
    if value.protocol_version != 1
        || !lower_hex(&value.challenge_id, 32)
        || !lower_hex(&value.nonce, 64)
        || value.api_url != api_base.trim().trim_end_matches('/')
        || value.expires_at_ms > now_ms.saturating_add(MAX_CHALLENGE_TTL_MS)
    {
        return Err(SelfEnrollmentError::InvalidChallenge);
    }
    if value.expires_at_ms <= now_ms {
        return Err(SelfEnrollmentError::ChallengeExpired);
    }
    Ok(())
}

fn canonical_bytes(value: &ChallengeResponse, key_id: &str, registration_json: &str) -> Vec<u8> {
    format!(
        "POST\n{}/devices/self-register\n{}\n{}\n{}\n{}",
        value.api_url,
        value.challenge_id,
        value.nonce,
        key_id,
        sha256_hex(registration_json.as_bytes())
    )
    .into_bytes()
}

fn unix_ms() -> Result<u64, SelfEnrollmentError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or(SelfEnrollmentError::InvalidChallenge)
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(value: &[u8]) -> String {
    hex_encode(digest::digest(&digest::SHA256, value).as_ref())
}

fn check_response_status(status: reqwest::StatusCode) -> Result<(), SelfEnrollmentError> {
    match status.as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(SelfEnrollmentError::Rejected),
        404 | 405 | 503 => Err(SelfEnrollmentError::ProtocolUnavailable),
        409 => Err(SelfEnrollmentError::MachineAlreadyRegistered),
        410 => Err(SelfEnrollmentError::ChallengeExpired),
        429 => Err(SelfEnrollmentError::RateLimited),
        _ => Err(SelfEnrollmentError::InvalidResponse),
    }
}

async fn bounded_challenge_body(
    mut response: reqwest::Response,
) -> Result<Zeroizing<Vec<u8>>, SelfEnrollmentError> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_CHALLENGE_BYTES as u64)
    {
        return Err(SelfEnrollmentError::InvalidChallenge);
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SelfEnrollmentError::InvalidChallenge)?
    {
        if body.len() + chunk.len() > MAX_CHALLENGE_BYTES {
            return Err(SelfEnrollmentError::InvalidChallenge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn payload() -> DeviceRegistrationRequest {
        DeviceRegistrationRequest {
            motherboard_serial: "windows-test-identity".into(),
            hostname: "办公电脑".into(),
            os_version: "Windows 11".into(),
            device_name: Some("办公电脑".into()),
            cpu_info: None,
            total_memory_mb: None,
            gpu_info: None,
        }
    }

    fn challenge(api_url: &str) -> ChallengeResponse {
        ChallengeResponse {
            protocol_version: 1,
            challenge_id: "11".repeat(16),
            nonce: "22".repeat(32),
            api_url: api_url.into(),
            expires_at_ms: unix_ms().unwrap() + 60_000,
        }
    }

    fn challenge_json(api_url: &str) -> String {
        let value = challenge(api_url);
        serde_json::json!({
            "protocol_version": value.protocol_version,
            "challenge_id": value.challenge_id,
            "nonce": value.nonce,
            "api_url": value.api_url,
            "expires_at_ms": value.expires_at_ms,
        })
        .to_string()
    }

    async fn fake_server<F>(responses: F) -> (String, tokio::task::JoinHandle<Vec<String>>)
    where
        F: FnOnce(&str) -> Vec<(String, String, String)>,
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let responses = responses(&base);
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body, extra_headers) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 1024];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0, "incomplete request");
                    bytes.extend_from_slice(&chunk[..count]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|value| value.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8(bytes).unwrap());
            }
            requests
        });
        (base, task)
    }

    fn http_body(request: &str) -> serde_json::Value {
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    #[test]
    fn matches_backend_fixed_ed25519_protocol_vector() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../apps/Rdesk-Server/tests/fixtures/device-self-enrollment-v1.json"
        ))
        .unwrap();
        let field = |name: &str| fixture[name].as_str().unwrap();
        let decode = |text: &str| {
            text.as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        let keypair =
            Ed25519KeyPair::from_seed_unchecked(&decode(field("test_only_seed_hex"))).unwrap();
        assert_eq!(
            hex_encode(keypair.public_key().as_ref()),
            field("public_key")
        );
        assert_eq!(sha256_hex(keypair.public_key().as_ref()), field("key_id"));
        let challenge = ChallengeResponse {
            protocol_version: 1,
            api_url: field("api_url").into(),
            challenge_id: field("challenge_id").into(),
            nonce: field("nonce").into(),
            expires_at_ms: 0,
        };
        for prefix in ["", "unicode_"] {
            let canonical = canonical_bytes(
                &challenge,
                field("key_id"),
                field(&format!("{prefix}registration_json")),
            );
            assert_eq!(canonical, field(&format!("{prefix}canonical")).as_bytes());
            // This is the DeviceIdentity.sign_context_bytes envelope, also checked
            // against the backend's independently generated RFC 8032 signature.
            let mut contextual = b"MRD_CONTEXT_SIGNATURE_V1".to_vec();
            contextual.extend_from_slice(&(SIGNATURE_CONTEXT.len() as u16).to_be_bytes());
            contextual.extend_from_slice(SIGNATURE_CONTEXT.as_bytes());
            contextual.extend_from_slice(&(canonical.len() as u64).to_be_bytes());
            contextual.extend_from_slice(&canonical);
            assert_eq!(
                hex_encode(&contextual),
                field(&format!("{prefix}contextual_bytes_hex"))
            );
            assert_eq!(
                hex_encode(keypair.sign(&contextual).as_ref()),
                field(&format!("{prefix}signature"))
            );
        }
    }

    #[tokio::test]
    async fn first_registration_binds_the_exact_utf8_payload_to_the_key_and_origin() {
        let public_key = [7u8; 32];
        let key_id = sha256_hex(&public_key);
        let body = r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"refresh.jwt.token"}"#;
        let (base, captured) = fake_server(|base| {
            vec![
                ("200 OK".into(), challenge_json(base), String::new()),
                ("200 OK".into(), body.into(), String::new()),
            ]
        })
        .await;
        let signed = Arc::new(Mutex::new(Vec::new()));
        let signed_copy = signed.clone();
        let result = self_register(
            &format!("{base}/"),
            &payload(),
            &key_id,
            &public_key,
            |bytes| {
                *signed_copy.lock().unwrap() = bytes.to_vec();
                Ok(vec![9; 64])
            },
        )
        .await
        .unwrap();
        assert_eq!(result.device_id, "0123456789");
        assert_eq!(result.refresh_token.as_deref(), Some("refresh.jwt.token"));
        let requests = captured.await.unwrap();
        assert!(
            requests[0].starts_with("POST /api/v1/devices/self-enrollment-challenge HTTP/1.1\r\n")
        );
        assert!(requests[1].starts_with("POST /api/v1/devices/self-register HTTP/1.1\r\n"));
        for request in &requests {
            let headers = request
                .split_once("\r\n\r\n")
                .unwrap()
                .0
                .to_ascii_lowercase();
            assert!(!headers.contains("authorization") && !headers.contains("enrollment:"));
            assert!(!headers.contains("windows-test-identity") && !headers.contains("?"));
        }
        let request = http_body(&requests[1]);
        assert_eq!(request["protocol_version"], 1);
        assert_eq!(request["key_id"], key_id);
        assert_eq!(request["public_key"], "07".repeat(32));
        assert_eq!(request["signature"], "09".repeat(64));
        let registration_json = request["registration_json"].as_str().unwrap();
        assert_eq!(
            registration_json,
            serde_json::to_string(&payload()).unwrap()
        );
        let expected = format!(
            "POST\n{base}/devices/self-register\n{}\n{}\n{key_id}\n{}",
            "11".repeat(16),
            "22".repeat(32),
            sha256_hex(registration_json.as_bytes()),
        );
        assert_eq!(*signed.lock().unwrap(), expected.as_bytes());
        assert!(!expected.ends_with('\n'));
        assert_eq!(expected.lines().count(), 6);
    }

    #[test]
    fn rejects_wrong_origin_expiry_version_and_hex_without_signing() {
        let now = unix_ms().unwrap();
        let base = "https://example.com/api/v1";
        let valid = challenge(base);
        assert_eq!(validate_challenge(&valid, base, now), Ok(()));
        let mut value = challenge(base);
        value.expires_at_ms = now + MAX_CHALLENGE_TTL_MS;
        assert_eq!(validate_challenge(&value, base, now), Ok(()));
        for mutation in 0..8 {
            let mut value = challenge(base);
            match mutation {
                0 => value.api_url = "https://other.example/api/v1".into(),
                1 => value.api_url = format!("{base}/"),
                2 => value.protocol_version = 2,
                3 => value.challenge_id = "AA".repeat(16),
                4 => value.nonce = "z".repeat(64),
                5 => value.nonce.push('0'),
                6 => value.expires_at_ms = now + MAX_CHALLENGE_TTL_MS + 1,
                7 => value.expires_at_ms = now,
                _ => unreachable!(),
            }
            assert!(
                validate_challenge(&value, base, now).is_err(),
                "mutation {mutation}"
            );
        }
        let mut value = challenge(base);
        value.expires_at_ms = now.saturating_sub(1);
        assert_eq!(
            validate_challenge(&value, base, now),
            Err(SelfEnrollmentError::ChallengeExpired)
        );
        let mut encoded = serde_json::to_value(
            serde_json::from_str::<serde_json::Value>(&challenge_json(base)).unwrap(),
        )
        .unwrap();
        encoded["access_token"] = "unexpected.secret.token".into();
        assert!(serde_json::from_value::<ChallengeResponse>(encoded).is_err());
    }

    #[tokio::test]
    async fn invalid_keys_payload_size_and_origins_fail_before_network() {
        let key = [7u8; 32];
        let key_id = sha256_hex(&key);
        for (base, supplied_id, supplied_key) in [
            ("http://example.com/api/v1", key_id.as_str(), key.as_slice()),
            (
                "https://user:secret@example.com/api/v1",
                key_id.as_str(),
                key.as_slice(),
            ),
            (
                "https://example.com/api/v1?token=secret",
                key_id.as_str(),
                key.as_slice(),
            ),
            (
                "https://example.com/api/v1#fragment",
                key_id.as_str(),
                key.as_slice(),
            ),
            ("https://127.0.0.1:9/api/v1", "bad", key.as_slice()),
            ("https://127.0.0.1:9/api/v1", key_id.as_str(), &[7u8; 31]),
            ("https://127.0.0.1:9/api/v1", key_id.as_str(), &[8u8; 32]),
        ] {
            assert_eq!(
                self_register(base, &payload(), supplied_id, supplied_key, |_| {
                    panic!("untrusted input must not reach signer")
                })
                .await
                .unwrap_err(),
                SelfEnrollmentError::InvalidRequest
            );
        }
        let mut huge = payload();
        huge.gpu_info = Some("a".repeat(MAX_REGISTRATION_BYTES));
        assert_eq!(
            self_register("https://127.0.0.1:9/api/v1", &huge, &key_id, &key, |_| {
                panic!("oversized input must not reach signer")
            })
            .await
            .unwrap_err(),
            SelfEnrollmentError::InvalidRequest
        );
    }

    #[tokio::test]
    async fn redirects_fail_without_following_or_exposing_error_bodies() {
        let key = [7u8; 32];
        for (status, expected) in [
            ("302 Found", SelfEnrollmentError::InvalidResponse),
            ("401 Unauthorized", SelfEnrollmentError::Rejected),
            ("403 Forbidden", SelfEnrollmentError::Rejected),
            ("404 Not Found", SelfEnrollmentError::ProtocolUnavailable),
            (
                "409 Conflict",
                SelfEnrollmentError::MachineAlreadyRegistered,
            ),
            ("410 Gone", SelfEnrollmentError::ChallengeExpired),
            ("429 Too Many Requests", SelfEnrollmentError::RateLimited),
            (
                "503 Service Unavailable",
                SelfEnrollmentError::ProtocolUnavailable,
            ),
        ] {
            let (base, captured) = fake_server(|_| {
                vec![(
                    status.into(),
                    "private.secret.token".into(),
                    "Location: https://example.com/credential-sink\r\n".into(),
                )]
            })
            .await;
            let error = self_register(&base, &payload(), &sha256_hex(&key), &key, |_| {
                panic!("failed challenge must not reach signer")
            })
            .await
            .unwrap_err();
            assert_eq!(error, expected);
            assert!(!format!("{error:?} {}", error.status_code()).contains("private.secret.token"));
            assert_eq!(captured.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn oversized_or_unknown_challenge_fields_never_reach_the_signer() {
        let key = [7u8; 32];
        for huge in [false, true] {
            let (base, captured) = fake_server(|base| {
                let body = if huge {
                    "x".repeat(MAX_CHALLENGE_BYTES + 1)
                } else {
                    let mut value: serde_json::Value =
                        serde_json::from_str(&challenge_json(base)).unwrap();
                    value["extra_secret"] = "private.secret.token".into();
                    value.to_string()
                };
                vec![("200 OK".into(), body, String::new())]
            })
            .await;
            assert_eq!(
                self_register(&base, &payload(), &sha256_hex(&key), &key, |_| {
                    panic!("malformed challenge must not reach signer")
                })
                .await
                .unwrap_err(),
                SelfEnrollmentError::InvalidChallenge
            );
            captured.await.unwrap();
        }
    }

    #[tokio::test]
    async fn claim_requires_a_ten_digit_code_and_durable_refresh_credential() {
        let key = [7u8; 32];
        for body in [
            r#"{"device_id":"123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"refresh.jwt.token"}"#,
            r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token"}"#,
            r#"{"device_id":"0123456789","device_name":"Office","access_token":"secret","refresh_token":"refresh.jwt.token"}"#,
        ] {
            let (base, captured) = fake_server(|base| {
                vec![
                    ("200 OK".into(), challenge_json(base), String::new()),
                    ("200 OK".into(), body.into(), String::new()),
                ]
            })
            .await;
            assert_eq!(
                self_register(&base, &payload(), &sha256_hex(&key), &key, |_| Ok(vec![
                    9;
                    64
                ]))
                .await
                .unwrap_err(),
                SelfEnrollmentError::InvalidResponse
            );
            assert_eq!(captured.await.unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn a_signer_failure_never_sends_a_claim() {
        let key = [7u8; 32];
        let (base, captured) =
            fake_server(|base| vec![("200 OK".into(), challenge_json(base), String::new())]).await;
        assert_eq!(
            self_register(&base, &payload(), &sha256_hex(&key), &key, |_| Err(
                SelfEnrollmentError::SigningFailed
            ))
            .await
            .unwrap_err(),
            SelfEnrollmentError::SigningFailed
        );
        assert_eq!(captured.await.unwrap().len(), 1);
    }
}
