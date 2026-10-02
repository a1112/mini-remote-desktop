use super::{SignalingConfig, SignalingRuntimeError};
use futures_util::StreamExt;
use mrd_identity::DeviceIdentity;
use mrd_proto::BackendRole;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const MAX_CREDENTIAL_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Serialize)]
struct CredentialRequest<'a> {
    device_key_id: &'a str,
    role: BackendRole,
}

// Deliberately no Debug: this response contains a bearer credential.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialResponse {
    token: String,
    expires_at_ms: u64,
    device_id: String,
    device_key_id: String,
    role: BackendRole,
}

impl Drop for CredentialResponse {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

/// Obtain a fresh connection credential before the server's short-lived challenge
/// exists. Explicit configs without an endpoint use a pre-issued credential.
pub(super) async fn acquire_credential(
    config: &SignalingConfig,
    identity: &DeviceIdentity,
) -> Result<Zeroizing<String>, SignalingRuntimeError> {
    let Some(endpoint) = config.credential_endpoint() else {
        return Ok(Zeroizing::new(config.backend_device_token().to_owned()));
    };
    let operation = async {
        let cleartext_loopback = endpoint.scheme() == "http";
        let mut builder = reqwest::Client::builder()
            .https_only(!cleartext_loopback)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout());
        if cleartext_loopback {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|_| SignalingRuntimeError::CredentialsUnavailable)?;
        let authorization = Zeroizing::new(format!("Bearer {}", config.backend_device_token()));
        let mut header = HeaderValue::from_str(&authorization)
            .map_err(|_| SignalingRuntimeError::CredentialsInvalid)?;
        header.set_sensitive(true);
        let response = client
            .post(endpoint.clone())
            .header("X-Rdesk-Device-Authorization", header)
            .json(&CredentialRequest {
                device_key_id: identity.key_id(),
                role: config.role(),
            })
            .send()
            .await
            .map_err(|_| SignalingRuntimeError::CredentialsUnavailable)?;
        // Never consume or expose rejection bodies, which may echo credentials.
        if !response.status().is_success() {
            return Err(SignalingRuntimeError::CredentialsUnavailable);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_CREDENTIAL_RESPONSE_BYTES as u64)
        {
            return Err(SignalingRuntimeError::CredentialsInvalid);
        }
        let mut body = Zeroizing::new(Vec::new());
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| SignalingRuntimeError::CredentialsUnavailable)?;
            if body.len().saturating_add(chunk.len()) > MAX_CREDENTIAL_RESPONSE_BYTES {
                return Err(SignalingRuntimeError::CredentialsInvalid);
            }
            body.extend_from_slice(&chunk);
        }
        let mut credential: CredentialResponse =
            serde_json::from_slice(&body).map_err(|_| SignalingRuntimeError::CredentialsInvalid)?;
        if credential.device_id != config.device_id().0
            || credential.device_key_id != identity.key_id()
            || credential.role != config.role()
            || credential.expires_at_ms <= super::runtime::unix_time_ms().saturating_add(1_000)
            || credential.token.is_empty()
            || credential.token.len() > 4_096
            || credential.token.chars().any(char::is_control)
        {
            return Err(SignalingRuntimeError::CredentialsInvalid);
        }
        Ok(Zeroizing::new(std::mem::take(&mut credential.token)))
    };
    tokio::time::timeout(config.connect_timeout(), operation)
        .await
        .map_err(|_| SignalingRuntimeError::CredentialsTimeout)?
}
