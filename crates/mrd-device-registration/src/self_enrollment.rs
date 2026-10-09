//! Machine-key proof for first-time code allocation, independent of accounts.
use crate::{valid_device_token, DeviceRegistrationRequest, DeviceRegistrationResponse};
use mrd_identity::DeviceIdentity;
use ring::digest;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_REGISTRATION_BYTES: usize = 8192;
const MAX_CHALLENGE_TTL_MS: u64 = 120_000;
const INVALID_RESPONSE: &str = "public_self_enrollment_response_invalid";
const INVALID_CHALLENGE: &str = "public_self_enrollment_challenge_invalid";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    protocol_version: u8,
    challenge_id: String,
    nonce: String,
    api_url: String,
    expires_at_ms: u64,
}
impl Drop for Challenge {
    fn drop(&mut self) {
        self.nonce.zeroize();
    }
}
#[derive(Serialize)]
struct ChallengeRequest<'a> {
    protocol_version: u8,
    key_id: &'a str,
    public_key: &'a str,
}
#[derive(Serialize)]
struct RegistrationProof<'a> {
    protocol_version: u8,
    key_id: &'a str,
    public_key: &'a str,
    challenge_id: &'a str,
    nonce: &'a str,
    registration_json: &'a str,
    signature: &'a str,
}

/// Allocate or recover the code bound to this resident's durable machine key.
/// No account, administrator capability, proxy or redirect is accepted here.
pub async fn self_register_device(
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    identity: &DeviceIdentity,
) -> Result<DeviceRegistrationResponse, &'static str> {
    validate_origin(api_base)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "public_self_enrollment_transport")?;
    register_with_client(&client, api_base, payload, identity).await
}

fn validate_origin(api_base: &str) -> Result<String, &'static str> {
    const ERROR: &str = "public_self_enrollment_origin_invalid";
    if api_base.len() > 2048 || api_base.chars().any(char::is_whitespace) {
        return Err(ERROR);
    }
    let base = api_base.trim_end_matches('/');
    let parsed = reqwest::Url::parse(base).map_err(|_| ERROR)?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.as_str().trim_end_matches('/') != base
    {
        return Err(ERROR);
    }
    Ok(base.to_owned())
}
fn now_ms() -> Result<u64, &'static str> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| INVALID_CHALLENGE)?
            .as_millis(),
    )
    .map_err(|_| INVALID_CHALLENGE)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn challenge_is_valid(challenge: &Challenge, api: &str, now: u64) -> bool {
    challenge.protocol_version == 1
        && is_hex(&challenge.challenge_id, 32)
        && is_hex(&challenge.nonce, 64)
        && challenge.api_url == api
        && challenge.expires_at_ms > now
        && challenge.expires_at_ms - now <= MAX_CHALLENGE_TTL_MS
}
async fn read_json<T: DeserializeOwned>(
    mut response: reqwest::Response,
    invalid: &'static str,
) -> Result<T, &'static str> {
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err("public_self_enrollment_proof_rejected"),
        409 => return Err("public_self_enrollment_conflict"),
        429 => return Err("public_self_enrollment_rate_limited"),
        503 => return Err("public_self_enrollment_unavailable"),
        _ => return Err(INVALID_RESPONSE),
    }
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
        })
        || response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
    {
        return Err(invalid);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "public_self_enrollment_transport")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid)
}

async fn register_with_client(
    client: &reqwest::Client,
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    identity: &DeviceIdentity,
) -> Result<DeviceRegistrationResponse, &'static str> {
    let api = validate_origin(api_base)?;
    let registration_json = Zeroizing::new(
        serde_json::to_string(payload).map_err(|_| "public_self_enrollment_payload_invalid")?,
    );
    if registration_json.len() > MAX_REGISTRATION_BYTES
        || payload.motherboard_serial.trim().is_empty()
        || payload.hostname.trim().is_empty()
        || payload.os_version.trim().is_empty()
    {
        return Err("public_self_enrollment_payload_invalid");
    }
    let public_key = hex(identity.public_key());
    let challenge = client
        .post(format!("{api}/devices/self-enrollment-challenge"))
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&ChallengeRequest {
            protocol_version: 1,
            key_id: identity.key_id(),
            public_key: &public_key,
        })
        .send()
        .await
        .map_err(|_| "public_self_enrollment_transport")?;
    let challenge: Challenge = read_json(challenge, INVALID_CHALLENGE).await?;
    if !challenge_is_valid(&challenge, &api, now_ms()?) {
        return Err(INVALID_CHALLENGE);
    }
    let digest = hex(digest::digest(&digest::SHA256, registration_json.as_bytes()).as_ref());
    let canonical = Zeroizing::new(format!(
        "POST\n{api}/devices/self-register\n{}\n{}\n{}\n{digest}",
        challenge.challenge_id,
        challenge.nonce,
        identity.key_id()
    ));
    let signature_bytes = Zeroizing::new(
        identity
            .sign_context_bytes("MRD_DEVICE_SELF_ENROLLMENT_V1", canonical.as_bytes())
            .map_err(|_| "public_self_enrollment_proof_unavailable")?,
    );
    let signature = Zeroizing::new(hex(&signature_bytes));
    // Recheck immediately before transporting proof; a slow signer must not use an expired challenge.
    if !challenge_is_valid(&challenge, &api, now_ms()?) {
        return Err(INVALID_CHALLENGE);
    }
    let proof = RegistrationProof {
        protocol_version: 1,
        key_id: identity.key_id(),
        public_key: &public_key,
        challenge_id: &challenge.challenge_id,
        nonce: &challenge.nonce,
        registration_json: &registration_json,
        signature: &signature,
    };
    let response = client
        .post(format!("{api}/devices/self-register"))
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&proof)
        .send()
        .await
        .map_err(|_| "public_self_enrollment_transport")?;
    let registration: DeviceRegistrationResponse = read_json(response, INVALID_RESPONSE).await?;
    if !matches!(registration.device_id.len(), 9 | 10)
        || !registration.device_id.bytes().all(|b| b.is_ascii_digit())
        || registration.device_name.trim().is_empty()
        || registration.device_name.len() > 128
        || !valid_device_token(&registration.access_token)
        || registration
            .refresh_token
            .as_deref()
            .is_none_or(|token| !valid_device_token(token))
    {
        return Err(INVALID_RESPONSE);
    }
    Ok(registration)
}

#[cfg(test)]
mod tests;
