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
    use mrd_proto::{BackendRole, DeviceId};
    use mrd_signal_proto::{RegisterRequest, SignalMessage};

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
}
