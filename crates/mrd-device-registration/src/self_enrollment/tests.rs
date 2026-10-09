use super::*;
use ring::{digest, rand::SystemRandom};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn payload() -> DeviceRegistrationRequest {
    DeviceRegistrationRequest {
        motherboard_serial: "durable-machine-serial".into(),
        hostname: "Guest PC".into(),
        os_version: "Windows 11".into(),
        device_name: Some("未登录电脑".into()),
        cpu_info: None,
        total_memory_mb: None,
        gpu_info: None,
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

struct FakeHttps {
    base: String,
    client: reqwest::Client,
    requests: tokio::task::JoinHandle<Vec<Value>>,
}
async fn fake_https(
    challenge_change: Option<(&str, Value)>,
    registration_body: Value,
    first_status: &str,
    first_headers: &str,
) -> FakeHttps {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("https://{}/api/v1", listener.local_addr().unwrap());
    let generated = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let certificate = generated.cert.der().clone();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.clone()],
        rustls::pki_types::PrivateKeyDer::Pkcs8(generated.key_pair.serialize_der().into()),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_der(certificate.as_ref()).unwrap())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let api = base.clone();
    let change = challenge_change.map(|(key, value)| (key.to_owned(), value));
    let first_status = first_status.to_owned();
    let first_headers = first_headers.to_owned();
    let requests = tokio::spawn(async move {
        let mut requests: Vec<Value> = Vec::new();
        for hop in 0..2 {
            let Ok(Ok((socket, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            else {
                break;
            };
            let mut stream = acceptor.accept(socket).await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let size = stream.read(&mut buf).await.unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&buf[..size]);
                assert!(bytes.len() < 64 * 1024);
                if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&bytes[..split])
                        .unwrap()
                        .to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length: ")
                                .and_then(|n| n.parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            let split = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap();
            let headers = std::str::from_utf8(&bytes[..split])
                .unwrap()
                .to_ascii_lowercase();
            assert!(
                !headers.contains("authorization:")
                    && !headers.contains("cookie:")
                    && !headers.contains("?")
            );
            assert!(headers.starts_with(if hop == 0 {
                "post /api/v1/devices/self-enrollment-challenge http/1.1"
            } else {
                "post /api/v1/devices/self-register http/1.1"
            }));
            let request: Value = serde_json::from_slice(&bytes[split + 4..]).unwrap();
            assert_eq!(request["protocol_version"], 1);
            if hop == 1 {
                assert_eq!(request["challenge_id"], "a".repeat(32));
                assert_eq!(request["nonce"], "b".repeat(64));
                assert_eq!(request["key_id"], requests[0]["key_id"]);
                assert_eq!(request["public_key"], requests[0]["public_key"]);
                let registration_json = request["registration_json"].as_str().unwrap();
                assert_eq!(
                    registration_json,
                    serde_json::to_string(&payload()).unwrap()
                );
                let canonical = format!(
                    "POST\n{api}/devices/self-register\n{}\n{}\n{}\n{}",
                    request["challenge_id"].as_str().unwrap(),
                    request["nonce"].as_str().unwrap(),
                    request["key_id"].as_str().unwrap(),
                    hex(digest::digest(&digest::SHA256, registration_json.as_bytes()).as_ref())
                );
                let decode = |s: &str| {
                    (0..s.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                        .collect::<Vec<_>>()
                };
                mrd_identity::verify_context_bytes(
                    &decode(request["public_key"].as_str().unwrap()),
                    "MRD_DEVICE_SELF_ENROLLMENT_V1",
                    canonical.as_bytes(),
                    &decode(request["signature"].as_str().unwrap()),
                )
                .expect("actual Ed25519 proof must match exact transported JSON");
            }
            requests.push(request);
            let mut body = if hop == 0 {
                json!({"protocol_version":1,"challenge_id":"a".repeat(32),
                "nonce":"b".repeat(64),"api_url":api,"expires_at_ms":now_ms()+60_000})
            } else {
                registration_body.clone()
            };
            if hop == 0 {
                if let Some((key, value)) = &change {
                    body[key] = value.clone();
                }
            }
            let body = serde_json::to_vec(&body).unwrap();
            let status = if hop == 0 {
                first_status.as_str()
            } else {
                "200 OK"
            };
            let extra = if hop == 0 { first_headers.as_str() } else { "" };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
            let _ = stream.shutdown().await;
        }
        requests
    });
    FakeHttps {
        base,
        client,
        requests,
    }
}
fn registration(code: &str) -> Value {
    json!({"device_id":code,"device_name":"Guest PC", "access_token":"access.jwt.token","refresh_token":"refresh.jwt.token"})
}

#[tokio::test]
async fn first_time_machine_code_uses_two_https_requests_and_exact_context_signature() {
    let server = fake_https(None, registration("0123456789"), "200 OK", "").await;
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let result = register_with_client(&server.client, &server.base, &payload(), &identity).await;
    assert!(
        result.is_ok(),
        "first-time machine proof registration must succeed: {}",
        result.err().unwrap_or("")
    );
    let requests = server.requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["key_id"], identity.key_id());
}

#[tokio::test]
async fn machine_proof_recovers_an_existing_nine_digit_code_and_refresh_capability() {
    let server = fake_https(None, registration("753662296"), "200 OK", "").await;
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let result = register_with_client(&server.client, &server.base, &payload(), &identity)
        .await
        .unwrap();
    assert_eq!(result.device_id, "753662296");
    assert_eq!(result.refresh_token.as_deref(), Some("refresh.jwt.token"));
    assert_eq!(server.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_or_expired_challenge_never_sends_a_machine_proof() {
    for (field, value) in [
        ("protocol_version", json!(2)),
        ("protocol_version", json!(true)),
        ("challenge_id", json!("not-a-challenge")),
        ("nonce", json!("private-invalid-nonce")),
        ("api_url", json!("https://credential-sink.invalid/api/v1")),
        ("api_url", json!("https://127.0.0.1/api/v1?secret=token")),
        ("expires_at_ms", json!(now_ms() - 1)),
        ("expires_at_ms", json!(now_ms() + 180_000)),
        ("extra", json!("must reject unknown challenge fields")),
    ] {
        let server = fake_https(
            Some((field, value)),
            registration("0123456789"),
            "200 OK",
            "",
        )
        .await;
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        assert_eq!(
            register_with_client(&server.client, &server.base, &payload(), &identity)
                .await
                .unwrap_err(),
            "public_self_enrollment_challenge_invalid"
        );
        assert_eq!(server.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn challenge_redirects_and_error_bodies_are_fixed_codes_without_secret_diagnostics() {
    for (status, headers, code) in [
        (
            "302 Found",
            "Location: https://credential-sink.invalid/\r\n",
            "public_self_enrollment_response_invalid",
        ),
        (
            "401 Unauthorized",
            "",
            "public_self_enrollment_proof_rejected",
        ),
        ("409 Conflict", "", "public_self_enrollment_conflict"),
        (
            "429 Too Many Requests",
            "",
            "public_self_enrollment_rate_limited",
        ),
        (
            "503 Service Unavailable",
            "",
            "public_self_enrollment_unavailable",
        ),
    ] {
        let server = fake_https(
            Some(("nonce", json!("private-secret-token"))),
            registration("0123456789"),
            status,
            headers,
        )
        .await;
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        assert_eq!(
            register_with_client(&server.client, &server.base, &payload(), &identity)
                .await
                .unwrap_err(),
            code
        );
        assert_eq!(server.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn self_registration_requires_nonempty_refresh_and_valid_decimal_code() {
    for response in [
        json!({"device_id":"0123456789","device_name":"Guest PC","access_token":"access.jwt.token"}),
        json!({"device_id":"0123456789","device_name":"Guest PC","access_token":"access.jwt.token","refresh_token":""}),
        registration("browser_0123456789"),
        registration("lan-test"),
        registration("12345678"),
    ] {
        let server = fake_https(None, response, "200 OK", "").await;
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        assert_eq!(
            register_with_client(&server.client, &server.base, &payload(), &identity)
                .await
                .unwrap_err(),
            "public_self_enrollment_response_invalid"
        );
        assert_eq!(server.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn public_machine_registration_rejects_noncanonical_or_non_https_origin_without_network() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    for base in [
        "http://127.0.0.1/api/v1",
        "https://user:secret@server.invalid/api/v1",
        "https://server.invalid/api/v1?secret=token",
        "https://server.invalid/api/v1#fragment",
        " https://server.invalid/api/v1",
        "https://server.invalid/a/../api/v1",
    ] {
        assert_eq!(
            self_register_device(base, &payload(), &identity)
                .await
                .unwrap_err(),
            "public_self_enrollment_origin_invalid"
        );
    }
}
