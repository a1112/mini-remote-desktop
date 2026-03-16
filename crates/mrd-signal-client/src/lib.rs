use mrd_signal_proto::SignalMessage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignalClientError {
    #[error("serialize signal message failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn encode_message(message: &SignalMessage) -> Result<String, SignalClientError> {
    serde_json::to_string(message).map_err(Into::into)
}

pub fn decode_message(raw: &str) -> Result<SignalMessage, SignalClientError> {
    serde_json::from_str(raw).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{decode_message, encode_message};
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
}
