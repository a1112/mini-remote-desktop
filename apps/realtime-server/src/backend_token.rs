//! Verification of backend-issued credentials bound to one signaling identity and role.
use crate::{BackendTokenError, BackendTokenVerifier, VerifiedBackendToken};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_proto::{BackendRole, DeviceId};
use ring::hmac;
use serde::Deserialize;
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_TOKEN_BYTES: usize = 8_192;
const MAX_LIFETIME_SECONDS: u64 = 3_600;
const CLOCK_SKEW_SECONDS: u64 = 60;
const DEFAULT_AUDIENCE: &str = "rdesk-signaling";

/// Only the backend and the realtime sidecar possess this HS256 verification key.
/// Its Debug representation deliberately omits signing material.
pub struct JwtBackendTokenVerifier {
    key: hmac::Key,
    issuer: String,
    audience: String,
}

impl std::fmt::Debug for JwtBackendTokenVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JwtBackendTokenVerifier")
            .field("key", &"[REDACTED]")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid realtime-server environment variable: {0}")]
pub struct BackendTokenConfigError(&'static str);

impl JwtBackendTokenVerifier {
    pub fn new(
        secret: &[u8],
        issuer: String,
        audience: String,
    ) -> Result<Self, BackendTokenConfigError> {
        if secret.len() < 32 {
            return Err(BackendTokenConfigError("MRD_REALTIME_JWT_SECRET"));
        }
        if !valid_identifier_config(&issuer) {
            return Err(BackendTokenConfigError("MRD_REALTIME_JWT_ISSUER"));
        }
        if !valid_identifier_config(&audience) {
            return Err(BackendTokenConfigError("MRD_REALTIME_JWT_AUDIENCE"));
        }
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, secret),
            issuer,
            audience,
        })
    }

    pub fn from_env() -> Result<Self, BackendTokenConfigError> {
        let secret = Zeroizing::new(required_env("MRD_REALTIME_JWT_SECRET")?);
        let issuer = required_env("MRD_REALTIME_JWT_ISSUER")?;
        let audience = match std::env::var("MRD_REALTIME_JWT_AUDIENCE") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => DEFAULT_AUDIENCE.into(),
            Err(_) => return Err(BackendTokenConfigError("MRD_REALTIME_JWT_AUDIENCE")),
        };
        Self::new(secret.as_bytes(), issuer, audience)
    }

    fn verify_inner(&self, token: &str, now_ms: u64) -> Option<VerifiedBackendToken> {
        if token.len() > MAX_TOKEN_BYTES {
            return None;
        }
        let mut parts = token.split('.');
        let header = parts.next()?;
        let payload = parts.next()?;
        let signature = parts.next()?;
        if parts.next().is_some() || header.is_empty() || payload.is_empty() || signature.is_empty()
        {
            return None;
        }
        let header: Header = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).ok()?).ok()?;
        if header.alg != "HS256" || header.typ != "JWT" {
            return None;
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let signed_bytes = token.as_bytes().get(..token.rfind('.')?)?;
        hmac::verify(&self.key, signed_bytes, &signature).ok()?;
        let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
        let expires_at_ms = claims.exp.checked_mul(1_000)?;
        if claims.token_type != "signaling"
            || claims.iss != self.issuer
            || claims.aud != self.audience
            || claims.sub != claims.device_id
            || !valid_device_id(&claims.device_id)
            || !valid_key_id(&claims.device_key_id)
            || now_ms >= expires_at_ms
            || claims.iat > (now_ms / 1_000).saturating_add(CLOCK_SKEW_SECONDS)
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > MAX_LIFETIME_SECONDS
        {
            return None;
        }
        Some(VerifiedBackendToken {
            device_id: DeviceId(claims.device_id),
            device_key_id: claims.device_key_id,
            role: claims.role,
            expires_at_ms,
        })
    }
}

impl BackendTokenVerifier for JwtBackendTokenVerifier {
    fn verify(&self, token: &str, now_ms: u64) -> Result<VerifiedBackendToken, BackendTokenError> {
        self.verify_inner(token, now_ms)
            .ok_or(BackendTokenError::Invalid)
    }
}

fn required_env(name: &'static str) -> Result<String, BackendTokenConfigError> {
    std::env::var(name).map_err(|_| BackendTokenConfigError(name))
}

fn valid_identifier_config(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

fn valid_device_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_key_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// Deserializing directly into structs rejects duplicate fields. Unknown header
// extensions and claims are also rejected instead of silently ignoring semantics.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: String,
    device_id: String,
    device_key_id: String,
    role: BackendRole,
    token_type: String,
    iss: String,
    aud: String,
    iat: u64,
    exp: u64,
}
