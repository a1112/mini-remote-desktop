use axum::http::{header::AUTHORIZATION, HeaderMap};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_proto::BackendRole;
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;
use zeroize::Zeroizing;

const CONTEXT: &[u8] = b"MRD_REALTIME_PRESENCE_QUERY_V1\0";
const MAX_DEVICES: usize = 128;

pub(crate) struct PresenceAuthorization(hmac::Key);

#[derive(Debug, Error)]
#[error("private presence authorization configuration is invalid")]
pub struct PresenceAuthorizationError;

impl PresenceAuthorization {
    pub(crate) fn from_secret(encoded: &str) -> Result<Self, PresenceAuthorizationError> {
        let decoded =
            Zeroizing::new(decode_canonical_32(encoded).ok_or(PresenceAuthorizationError)?);
        Ok(Self(hmac::Key::new(hmac::HMAC_SHA256, &decoded)))
    }

    pub(crate) fn authorize(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some(encoded) = value.strip_prefix("Bearer ") else {
            return false;
        };
        let Some(tag) = decode_canonical_32(encoded) else {
            return false;
        };
        hmac::verify(&self.0, CONTEXT, &tag).is_ok()
    }
}

fn decode_canonical_32(encoded: &str) -> Option<Vec<u8>> {
    if encoded.len() != 43
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    (decoded.len() == 32 && URL_SAFE_NO_PAD.encode(&decoded) == encoded).then_some(decoded)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresenceQuery {
    device_ids: Vec<String>,
}

impl PresenceQuery {
    pub(crate) fn unique_device_ids(self) -> Option<Vec<String>> {
        if self.device_ids.len() > MAX_DEVICES
            || self.device_ids.iter().any(|id| {
                id.is_empty()
                    || id.len() > 128
                    || !id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
            })
        {
            return None;
        }
        let mut seen = HashSet::with_capacity(self.device_ids.len());
        Some(
            self.device_ids
                .into_iter()
                .filter(|id| seen.insert(id.clone()))
                .collect(),
        )
    }
}

#[derive(Serialize)]
pub(crate) struct PresenceSnapshot {
    pub version: u8,
    pub sampled_at_ms: u64,
    pub devices: Vec<DevicePresenceSnapshot>,
}

#[derive(Serialize)]
pub(crate) struct DevicePresenceSnapshot {
    pub device_id: String,
    pub online: bool,
    pub last_seen_ms: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct IdentitySnapshot {
    pub version: u8,
    pub sampled_at_ms: u64,
    pub identities: Vec<RegisteredIdentitySnapshot>,
}

#[derive(Serialize)]
pub(crate) struct RegisteredIdentitySnapshot {
    pub device_id: String,
    pub device_key_id: String,
    pub role: BackendRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_requires_canonical_exactly_32_bytes_and_error_never_contains_it() {
        for secret in [
            "",
            "invalid",
            &URL_SAFE_NO_PAD.encode([1; 31]),
            &URL_SAFE_NO_PAD.encode([1; 33]),
            &(URL_SAFE_NO_PAD.encode([1; 32]) + "="),
            &"+".repeat(43),
        ] {
            let error = PresenceAuthorization::from_secret(secret).err().unwrap();
            assert_eq!(
                error.to_string(),
                "private presence authorization configuration is invalid"
            );
        }
        assert!(PresenceAuthorization::from_secret(&URL_SAFE_NO_PAD.encode([1; 32])).is_ok());
    }
}
