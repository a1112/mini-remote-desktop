use mrd_signal_proto::{SignalEnvelope, SignalMessage, SignalProtocolError};
use thiserror::Error;

mod issued_time;
mod strict_value;
pub use issued_time::{wait_until_message_issued, MAX_FUTURE_MESSAGE_WAIT_MS};

#[derive(Debug, Error)]
pub enum SignalClientError {
    #[error("serialize signal message failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    Protocol(#[from] SignalProtocolError),
    #[error("signal message exceeds the bounded wire size")]
    MessageTooLarge,
}

pub const MAX_SIGNAL_MESSAGE_BYTES: usize = 512 * 1_024;

pub fn encode_message(message: &SignalMessage) -> Result<String, SignalClientError> {
    serde_json::to_string(message).map_err(Into::into)
}

pub fn decode_message(raw: &str) -> Result<SignalMessage, SignalClientError> {
    serde_json::from_str(raw).map_err(Into::into)
}

/// Encode one mandatory-version authenticated signaling envelope.
pub fn encode_authenticated_message(
    envelope: &SignalEnvelope,
) -> Result<String, SignalClientError> {
    envelope.validate_version()?;
    let encoded = serde_json::to_string(envelope)?;
    if encoded.len() > MAX_SIGNAL_MESSAGE_BYTES {
        return Err(SignalClientError::MessageTooLarge);
    }
    Ok(encoded)
}

/// Decode only the authenticated protocol; legacy unversioned JSON is rejected.
pub fn decode_authenticated_message(raw: &str) -> Result<SignalEnvelope, SignalClientError> {
    if raw.len() > MAX_SIGNAL_MESSAGE_BYTES {
        return Err(SignalClientError::MessageTooLarge);
    }
    // Read generic JSON before the version gate, without collapsing duplicate
    // keys or deserializing a private authenticated payload prematurely.
    let value = serde_json::from_str::<strict_value::StrictValue>(raw)?.0;
    if let (Some(version), Some(message_type)) = (
        value.get("version").and_then(serde_json::Value::as_u64),
        value
            .get("message")
            .and_then(|message| message.get("type"))
            .and_then(serde_json::Value::as_str),
    ) {
        SignalEnvelope::validate_wire_version(version, message_type)?;
    }
    serde_json::from_value(value).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{
        decode_authenticated_message, decode_message, encode_authenticated_message, encode_message,
        SignalClientError,
    };
    use mrd_proto::{BackendRole, DeviceId, SessionId};
    use mrd_signal_proto::{RegisterRequest, SessionAccept, SessionRequest, SignalMessage};

    #[test]
    fn register_message_roundtrip() {
        let message = SignalMessage::Register(RegisterRequest {
            role: BackendRole::Controller,
            device_id: Some(DeviceId("controller-1".into())),
            name: "Rdesk".into(),
        });

        let encoded = encode_message(&message).expect("encode register message");
        let decoded = decode_message(&encoded).expect("decode register message");

        assert_eq!(decoded, message);
    }

    #[test]
    fn quic_session_messages_roundtrip() {
        let request = SignalMessage::SessionRequest(SessionRequest {
            session_id: SessionId("session-quic".into()),
            source_device_id: DeviceId("controller-1".into()),
            target_device_id: DeviceId("agent-1".into()),
            transport: "quic_quinn".into(),
            quic_listen_addr: Some("127.0.0.1:5000".into()),
            quic_server_name: Some("localhost".into()),
            quic_cert_der_b64: Some("AQID".into()),
        });
        let accept = SignalMessage::SessionAccept(SessionAccept {
            session_id: SessionId("session-quic".into()),
            transport: "quic_quinn".into(),
            quic_listen_addr: Some("127.0.0.1:6000".into()),
            quic_server_name: Some("localhost".into()),
            quic_cert_der_b64: Some("BAUG".into()),
        });

        let encoded_request = encode_message(&request).expect("encode quic request");
        let decoded_request = decode_message(&encoded_request).expect("decode quic request");
        assert_eq!(decoded_request, request);

        let encoded_accept = encode_message(&accept).expect("encode quic accept");
        let decoded_accept = decode_message(&encoded_accept).expect("decode quic accept");
        assert_eq!(decoded_accept, accept);
    }

    #[test]
    fn authenticated_decode_rejects_legacy_unversioned_message() {
        let legacy = encode_message(&SignalMessage::Register(RegisterRequest {
            role: BackendRole::Controller,
            device_id: Some(DeviceId("controller-1".into())),
            name: "Rdesk".into(),
        }))
        .unwrap();
        assert!(decode_authenticated_message(&legacy).is_err());
    }

    #[test]
    fn authenticated_encode_rejects_in_memory_wrong_version() {
        use mrd_signal_proto::{
            AuthenticatedSignalMessage, ProtocolReasonCode, SignalEnvelope, SignalErrorMessage,
            SIGNAL_PROTOCOL_VERSION,
        };
        let mut envelope = SignalEnvelope::new(AuthenticatedSignalMessage::ProtocolError(
            SignalErrorMessage {
                reason: ProtocolReasonCode::Malformed,
                correlation_id: None,
                detail: "invalid".into(),
            },
        ));
        envelope.version = SIGNAL_PROTOCOL_VERSION + 1;
        assert!(encode_authenticated_message(&envelope).is_err());
    }

    #[test]
    fn authenticated_wire_decode_preserves_unsupported_version() {
        use mrd_signal_proto::{ProtocolReasonCode, SignalProtocolError};

        for raw in [
            r#"{"version":2,"message":{"type":"session_intent","payload":{}}}"#,
            r#"{"version":3,"message":{"type":"protocol_error","payload":{"reason":"malformed","correlation_id":null,"detail":"request rejected"}}}"#,
        ] {
            let error = decode_authenticated_message(raw).unwrap_err();
            assert!(matches!(
                error,
                SignalClientError::Protocol(SignalProtocolError::UnsupportedVersion)
            ));
            let SignalClientError::Protocol(protocol) = error else {
                unreachable!()
            };
            assert_eq!(
                protocol.reason_code(),
                ProtocolReasonCode::UnsupportedVersion
            );
        }
    }

    #[test]
    fn authenticated_codec_roundtrips_versioned_envelope() {
        use mrd_signal_proto::{
            AuthenticatedSignalMessage, ProtocolReasonCode, SignalEnvelope, SignalErrorMessage,
        };
        let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::ProtocolError(
            SignalErrorMessage {
                reason: ProtocolReasonCode::RateLimited,
                correlation_id: Some([3; 16]),
                detail: "retry later".into(),
            },
        ));
        let encoded = encode_authenticated_message(&envelope).unwrap();
        assert_eq!(decode_authenticated_message(&encoded).unwrap(), envelope);
    }

    fn signed_register_wire() -> String {
        use mrd_identity::DeviceIdentity;
        use mrd_signal_proto::{
            AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, RegisterPayload,
            SignalEnvelope,
        };
        let identity = DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
        let signed = AuthenticatedRegister::sign(
            &identity,
            RegisterPayload {
                claims: AuthClaims {
                    issuer_device_id: DeviceId("controller-1".into()),
                    issuer_key_id: identity.key_id().into(),
                    intended_peer_device_id: DeviceId("signal-server".into()),
                    issued_at_ms: 1_000,
                    expires_at_ms: 2_000,
                    counter: 1,
                    nonce: [1; 16],
                },
                role: BackendRole::Controller,
                device_name: "Rdesk".into(),
                backend_device_token: "test-credential".into(),
                challenge_id: [7; 16],
                challenge_nonce: [8; 32],
            },
        )
        .unwrap();
        encode_authenticated_message(&SignalEnvelope::new(AuthenticatedSignalMessage::Register(
            signed,
        )))
        .unwrap()
    }

    #[test]
    fn authenticated_decode_rejects_duplicate_keys_at_every_envelope_depth() {
        let raw = signed_register_wire();
        assert!(decode_authenticated_message(&raw).is_ok());
        for (name, altered) in [
            (
                "version",
                raw.replacen(r#""version":2"#, r#""version":999,"version":2"#, 1),
            ),
            (
                "escaped version alias",
                raw.replacen(r#""version":2"#, r#""\u0076ersion":999,"version":2"#, 1),
            ),
            (
                "message type",
                raw.replacen(
                    r#""type":"register""#,
                    r#""type":"other","type":"register""#,
                    1,
                ),
            ),
            (
                "signed signal",
                raw.replacen(
                    r#""signer_public_key":"#,
                    r#""signer_public_key":[],"signer_public_key":"#,
                    1,
                ),
            ),
            (
                "payload",
                raw.replacen(
                    r#""device_name":"Rdesk""#,
                    r#""device_name":"other","device_name":"Rdesk""#,
                    1,
                ),
            ),
            (
                "claims",
                raw.replacen(r#""counter":1"#, r#""counter":999,"counter":1"#, 1),
            ),
            (
                "whole message",
                raw.replacen(r#""message":"#, r#""message":{},"message":"#, 1),
            ),
        ] {
            assert_ne!(altered, raw, "mutation did not apply: {name}");
            assert!(
                decode_authenticated_message(&altered).is_err(),
                "accepted ambiguous {name}"
            );
        }
    }

    #[test]
    fn authenticated_decode_rejects_unknown_message_enum_fields() {
        let raw = signed_register_wire();
        let altered = raw.replacen(
            r#""type":"register""#,
            r#""unknown":true,"type":"register""#,
            1,
        );
        assert_ne!(altered, raw);
        assert!(decode_authenticated_message(&altered).is_err());
    }

    #[test]
    fn authenticated_decode_keeps_version_precheck_before_sensitive_payload_deserialization() {
        use mrd_signal_proto::SignalProtocolError;
        for raw in [
            r#"{"version":999,"message":{"type":"register","payload":{"backend_device_token":{"sensitive":"not-a-string"}}}}"#,
            r#"{"version":4,"message":{"type":"session_intent_v3","payload":{"claims":"not-a-claims-object"}}}"#,
            r#"{"version":3,"message":{"type":"protocol_error","payload":{"reason":"not-a-valid-reason"}}}"#,
            r#"{"version":2,"message":{"type":"session_intent","payload":{"claims":"legacy-no-longer-accepted"}}}"#,
        ] {
            assert!(matches!(
                decode_authenticated_message(raw),
                Err(SignalClientError::Protocol(
                    SignalProtocolError::UnsupportedVersion
                ))
            ));
        }
    }
}
