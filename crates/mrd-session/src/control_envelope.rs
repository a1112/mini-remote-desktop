#![allow(missing_docs)]

use crate::PermissionScope;
use mrd_proto::{DeviceId, SessionId};
use serde::{
    de::{Error as DeError, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use thiserror::Error;

pub const CONTROL_ENVELOPE_VERSION: u16 = 2;
pub const CONTROL_ENVELOPE_SIGNATURE_CONTEXT: &str = "MRD_LAN_CONTROL_ENVELOPE_V2";
pub const CONTROL_ENVELOPE_MAX_EVENT_BYTES: usize = 64;
pub const CONTROL_ENVELOPE_MAX_WIRE_BYTES: usize = 4_096;
pub const CONTROL_ENVELOPE_MAX_LIFETIME_MS: u64 = 2_000;
pub const CONTROL_ENVELOPE_MAX_CLOCK_SKEW_MS: u64 = 2_000;
const CONTROL_ENVELOPE_MAX_ID_BYTES: usize = 256;
const CONTROL_ENVELOPE_MAX_KEY_ID_BYTES: usize = 256;
const CONTROL_SEQUENCE_WINDOW_MAX_WIDTH: usize = 4_096;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ControlEnvelopeError {
    #[error("unsupported control-envelope protocol version")]
    UnsupportedVersion,
    #[error("control envelope is missing a required binding")]
    MissingBinding,
    #[error("control envelope peer bindings are invalid")]
    InvalidPeerBinding,
    #[error("control envelope grant identifier is invalid")]
    InvalidGrantId,
    #[error("control envelope scope is not an input scope")]
    InvalidScope,
    #[error("control envelope sequence must be non-zero and below u64::MAX")]
    InvalidSequence,
    #[error("control envelope event identifier must be non-zero")]
    InvalidEventId,
    #[error("control envelope policy revision must be non-zero")]
    InvalidPolicyRevision,
    #[error("control envelope issuance time is invalid")]
    InvalidIssuedAt,
    #[error("control envelope validity interval is invalid")]
    InvalidLifetime,
    #[error("control envelope issuance time is too far in the future")]
    NotYetValid,
    #[error("control envelope has expired")]
    Expired,
    #[error("control envelope event bytes are invalid")]
    InvalidEventBytes,
    #[error("control envelope canonical encoding failed")]
    Encoding,
    #[error("control envelope wire frame exceeds its pre-parse bound")]
    WireTooLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControlEnvelopeV2 {
    pub protocol_version: u16,
    #[serde(deserialize_with = "deserialize_session_id")]
    pub session_id: SessionId,
    pub grant_id: [u8; 32],
    #[serde(deserialize_with = "deserialize_device_id")]
    pub source_device_id: DeviceId,
    #[serde(deserialize_with = "deserialize_device_id")]
    pub target_device_id: DeviceId,
    #[serde(deserialize_with = "deserialize_key_id")]
    pub source_key_id: String,
    #[serde(deserialize_with = "deserialize_key_id")]
    pub target_key_id: String,
    pub scope: PermissionScope,
    pub sequence: u64,
    pub event_id: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub policy_revision: u64,
    #[serde(deserialize_with = "deserialize_event_bytes")]
    pub authenticated_event_bytes: Vec<u8>,
}

impl ControlEnvelopeV2 {
    pub fn validate_shape(&self, now_ms: u64) -> Result<(), ControlEnvelopeError> {
        if self.protocol_version != CONTROL_ENVELOPE_VERSION {
            return Err(ControlEnvelopeError::UnsupportedVersion);
        }
        if !bounded_required(&self.session_id.0, CONTROL_ENVELOPE_MAX_ID_BYTES)
            || !bounded_required(&self.source_device_id.0, CONTROL_ENVELOPE_MAX_ID_BYTES)
            || !bounded_required(&self.target_device_id.0, CONTROL_ENVELOPE_MAX_ID_BYTES)
            || !bounded_required(&self.source_key_id, CONTROL_ENVELOPE_MAX_KEY_ID_BYTES)
            || !bounded_required(&self.target_key_id, CONTROL_ENVELOPE_MAX_KEY_ID_BYTES)
        {
            return Err(ControlEnvelopeError::MissingBinding);
        }
        if self.source_device_id == self.target_device_id
            || self.source_key_id == self.target_key_id
        {
            return Err(ControlEnvelopeError::InvalidPeerBinding);
        }
        if self.grant_id.iter().all(|byte| *byte == 0) {
            return Err(ControlEnvelopeError::InvalidGrantId);
        }
        if !matches!(
            self.scope,
            PermissionScope::InputPointer | PermissionScope::InputKeyboard
        ) {
            return Err(ControlEnvelopeError::InvalidScope);
        }
        if self.sequence == 0 || self.sequence == u64::MAX {
            return Err(ControlEnvelopeError::InvalidSequence);
        }
        if self.event_id == 0 {
            return Err(ControlEnvelopeError::InvalidEventId);
        }
        if self.policy_revision == 0 {
            return Err(ControlEnvelopeError::InvalidPolicyRevision);
        }
        if self.issued_at_ms == 0 {
            return Err(ControlEnvelopeError::InvalidIssuedAt);
        }
        if self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms.saturating_sub(self.issued_at_ms)
                > CONTROL_ENVELOPE_MAX_LIFETIME_MS
        {
            return Err(ControlEnvelopeError::InvalidLifetime);
        }
        if self.issued_at_ms > now_ms.saturating_add(CONTROL_ENVELOPE_MAX_CLOCK_SKEW_MS) {
            return Err(ControlEnvelopeError::NotYetValid);
        }
        if self.expires_at_ms < now_ms {
            return Err(ControlEnvelopeError::Expired);
        }
        if self.authenticated_event_bytes.is_empty()
            || self.authenticated_event_bytes.len() > CONTROL_ENVELOPE_MAX_EVENT_BYTES
        {
            return Err(ControlEnvelopeError::InvalidEventBytes);
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, ControlEnvelopeError> {
        #[derive(Serialize)]
        struct Commitment<'a> {
            schema_version: u16,
            kind: &'static str,
            payload: &'a ControlEnvelopeV2,
        }

        serde_json::to_vec(&Commitment {
            schema_version: CONTROL_ENVELOPE_VERSION,
            kind: "control_envelope",
            payload: self,
        })
        .map_err(|_| ControlEnvelopeError::Encoding)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedControlEnvelopeV2 {
    pub payload: ControlEnvelopeV2,
    pub public_key: [u8; 32],
    #[serde(
        serialize_with = "serialize_ed25519_signature",
        deserialize_with = "deserialize_ed25519_signature"
    )]
    pub signature: [u8; 64],
}

impl SignedControlEnvelopeV2 {
    pub fn validate_shape(&self, now_ms: u64) -> Result<(), ControlEnvelopeError> {
        self.payload.validate_shape(now_ms)
    }

    pub fn decode_bounded_json(bytes: &[u8], now_ms: u64) -> Result<Self, ControlEnvelopeError> {
        if bytes.is_empty() || bytes.len() > CONTROL_ENVELOPE_MAX_WIRE_BYTES {
            return Err(ControlEnvelopeError::WireTooLarge);
        }
        let envelope =
            serde_json::from_slice::<Self>(bytes).map_err(|_| ControlEnvelopeError::Encoding)?;
        envelope.validate_shape(now_ms)?;
        Ok(envelope)
    }
}

fn serialize_ed25519_signature<S>(signature: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_seq(signature.iter())
}

fn deserialize_ed25519_signature<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
where
    D: Deserializer<'de>,
{
    struct SignatureVisitor;

    impl<'de> Visitor<'de> for SignatureVisitor {
        type Value = [u8; 64];

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("exactly 64 Ed25519 signature bytes")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut signature = [0_u8; 64];
            for (index, byte) in signature.iter_mut().enumerate() {
                *byte = sequence.next_element()?.ok_or_else(|| {
                    A::Error::invalid_length(index, &"exactly 64 signature bytes")
                })?;
            }
            if sequence.next_element::<u8>()?.is_some() {
                return Err(A::Error::invalid_length(65, &"exactly 64 signature bytes"));
            }
            Ok(signature)
        }
    }

    deserializer.deserialize_tuple(64, SignatureVisitor)
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<SessionId, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, CONTROL_ENVELOPE_MAX_ID_BYTES>(deserializer).map(SessionId)
}

fn deserialize_device_id<'de, D>(deserializer: D) -> Result<DeviceId, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, CONTROL_ENVELOPE_MAX_ID_BYTES>(deserializer).map(DeviceId)
}

fn deserialize_key_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, CONTROL_ENVELOPE_MAX_KEY_ID_BYTES>(deserializer)
}

fn deserialize_bounded_string<'de, D, const MAX: usize>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoundedStringVisitor<const MAX: usize>;

    impl<const MAX: usize> Visitor<'_> for BoundedStringVisitor<MAX> {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a non-empty string of at most {MAX} bytes")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            if bounded_required(value, MAX) {
                Ok(value.to_owned())
            } else {
                Err(E::invalid_value(serde::de::Unexpected::Str(value), &self))
            }
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            if bounded_required(&value, MAX) {
                Ok(value)
            } else {
                Err(E::custom("string exceeds authenticated control bound"))
            }
        }
    }

    deserializer.deserialize_str(BoundedStringVisitor::<MAX>)
}

fn deserialize_event_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct EventBytesVisitor;

    impl<'de> Visitor<'de> for EventBytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "1 to {CONTROL_ENVELOPE_MAX_EVENT_BYTES} authenticated event bytes"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut bytes = Vec::with_capacity(
                sequence
                    .size_hint()
                    .unwrap_or(0)
                    .min(CONTROL_ENVELOPE_MAX_EVENT_BYTES),
            );
            while let Some(byte) = sequence.next_element::<u8>()? {
                if bytes.len() == CONTROL_ENVELOPE_MAX_EVENT_BYTES {
                    return Err(A::Error::custom(
                        "authenticated event bytes exceed protocol bound",
                    ));
                }
                bytes.push(byte);
            }
            if bytes.is_empty() {
                return Err(A::Error::custom(
                    "authenticated event bytes cannot be empty",
                ));
            }
            Ok(bytes)
        }

        fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            if value.is_empty() || value.len() > CONTROL_ENVELOPE_MAX_EVENT_BYTES {
                return Err(E::custom(
                    "authenticated event bytes violate protocol bound",
                ));
            }
            Ok(value.to_vec())
        }

        fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            if value.is_empty() || value.len() > CONTROL_ENVELOPE_MAX_EVENT_BYTES {
                return Err(E::custom(
                    "authenticated event bytes violate protocol bound",
                ));
            }
            Ok(value)
        }
    }

    deserializer.deserialize_seq(EventBytesVisitor)
}

fn bounded_required(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.contains('\0')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSequenceDecision {
    FirstSeen,
    ExactRetry,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ControlSequenceError {
    #[error("control sequence or event identifier is invalid")]
    Invalid,
    #[error("control sequence or event identifier was already used")]
    Duplicate,
    #[error("control sequence is outside the replay window")]
    OutOfWindow,
}

#[derive(Debug, Clone, Copy)]
struct ControlSequenceObservation {
    event_id: u64,
    commitment: [u8; 32],
}

#[derive(Debug)]
pub struct ControlSequenceWindow {
    width: usize,
    highest: Option<u64>,
    by_sequence: BTreeMap<u64, ControlSequenceObservation>,
    sequence_by_event: HashMap<u64, u64>,
}

impl ControlSequenceWindow {
    pub fn new(width: usize) -> Self {
        Self {
            width: width.clamp(1, CONTROL_SEQUENCE_WINDOW_MAX_WIDTH),
            highest: None,
            by_sequence: BTreeMap::new(),
            sequence_by_event: HashMap::new(),
        }
    }

    pub fn observe(
        &mut self,
        sequence: u64,
        event_id: u64,
        commitment: [u8; 32],
    ) -> Result<ControlSequenceDecision, ControlSequenceError> {
        if sequence == 0 || event_id == 0 {
            return Err(ControlSequenceError::Invalid);
        }
        if let Some(observed) = self.by_sequence.get(&sequence) {
            return if observed.event_id == event_id && observed.commitment == commitment {
                Ok(ControlSequenceDecision::ExactRetry)
            } else {
                Err(ControlSequenceError::Duplicate)
            };
        }
        if self.sequence_by_event.contains_key(&event_id) {
            return Err(ControlSequenceError::Duplicate);
        }
        if self
            .highest
            .is_some_and(|highest| sequence < replay_floor(highest, self.width))
        {
            return Err(ControlSequenceError::OutOfWindow);
        }

        self.highest = Some(
            self.highest
                .map_or(sequence, |highest| highest.max(sequence)),
        );
        self.by_sequence.insert(
            sequence,
            ControlSequenceObservation {
                event_id,
                commitment,
            },
        );
        self.sequence_by_event.insert(event_id, sequence);
        self.prune();
        Ok(ControlSequenceDecision::FirstSeen)
    }

    pub fn len(&self) -> usize {
        self.by_sequence.len()
    }

    pub fn capacity(&self) -> usize {
        self.width
    }

    pub fn is_empty(&self) -> bool {
        self.by_sequence.is_empty()
    }

    fn prune(&mut self) {
        let Some(highest) = self.highest else {
            return;
        };
        let floor = replay_floor(highest, self.width);
        let stale = self
            .by_sequence
            .range(..floor)
            .map(|(sequence, observation)| (*sequence, observation.event_id))
            .collect::<Vec<_>>();
        for (sequence, event_id) in stale {
            self.by_sequence.remove(&sequence);
            self.sequence_by_event.remove(&event_id);
        }
    }
}

fn replay_floor(highest: u64, width: usize) -> u64 {
    highest.saturating_sub((width as u64).saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PermissionScope;
    use mrd_proto::{DeviceId, SessionId};

    fn envelope() -> ControlEnvelopeV2 {
        ControlEnvelopeV2 {
            protocol_version: CONTROL_ENVELOPE_VERSION,
            session_id: SessionId("session-control-v2".to_string()),
            grant_id: [0x31; 32],
            source_device_id: DeviceId("controller-device".to_string()),
            target_device_id: DeviceId("target-device".to_string()),
            source_key_id: "controller-key-id".to_string(),
            target_key_id: "target-key-id".to_string(),
            scope: PermissionScope::InputPointer,
            sequence: 41,
            event_id: 91,
            issued_at_ms: 9_000,
            expires_at_ms: 11_000,
            policy_revision: 7,
            authenticated_event_bytes: vec![2, 1, 0, 8, 0, 0, 0, 1, 0, 0, 0, 2],
        }
    }

    #[test]
    fn signing_bytes_bind_every_security_field() {
        let original = envelope();
        let baseline = original.signing_bytes().expect("canonical envelope");
        let mut variants = Vec::new();

        let mut value = original.clone();
        value.protocol_version += 1;
        variants.push(value);
        let mut value = original.clone();
        value.session_id = SessionId("other-session".to_string());
        variants.push(value);
        let mut value = original.clone();
        value.grant_id[0] ^= 1;
        variants.push(value);
        let mut value = original.clone();
        value.source_device_id = DeviceId("forged-controller".to_string());
        variants.push(value);
        let mut value = original.clone();
        value.target_device_id = DeviceId("other-target".to_string());
        variants.push(value);
        let mut value = original.clone();
        value.source_key_id.push('x');
        variants.push(value);
        let mut value = original.clone();
        value.target_key_id.push('x');
        variants.push(value);
        let mut value = original.clone();
        value.scope = PermissionScope::InputKeyboard;
        variants.push(value);
        let mut value = original.clone();
        value.sequence += 1;
        variants.push(value);
        let mut value = original.clone();
        value.event_id += 1;
        variants.push(value);
        let mut value = original.clone();
        value.issued_at_ms += 1;
        variants.push(value);
        let mut value = original.clone();
        value.expires_at_ms += 1;
        variants.push(value);
        let mut value = original.clone();
        value.policy_revision += 1;
        variants.push(value);
        let mut value = original;
        value.authenticated_event_bytes[0] ^= 1;
        variants.push(value);

        for variant in variants {
            assert_ne!(
                variant.signing_bytes().expect("variant canonical bytes"),
                baseline
            );
        }
    }

    #[test]
    fn signing_bytes_match_the_v2_canonical_wire_golden_value() {
        let expected = concat!(
            "{\"schema_version\":2,\"kind\":\"control_envelope\",\"payload\":{",
            "\"protocol_version\":2,\"session_id\":\"session-control-v2\",",
            "\"grant_id\":[49,49,49,49,49,49,49,49,49,49,49,49,49,49,49,49,",
            "49,49,49,49,49,49,49,49,49,49,49,49,49,49,49,49],",
            "\"source_device_id\":\"controller-device\",\"target_device_id\":\"target-device\",",
            "\"source_key_id\":\"controller-key-id\",\"target_key_id\":\"target-key-id\",",
            "\"scope\":\"InputPointer\",\"sequence\":41,\"event_id\":91,",
            "\"issued_at_ms\":9000,\"expires_at_ms\":11000,\"policy_revision\":7,",
            "\"authenticated_event_bytes\":[2,1,0,8,0,0,0,1,0,0,0,2]}}"
        );

        assert_eq!(
            String::from_utf8(envelope().signing_bytes().expect("canonical envelope"))
                .expect("canonical UTF-8"),
            expected
        );
    }

    #[test]
    fn envelope_shape_rejects_downgrade_missing_binding_and_invalid_scope() {
        let mut value = envelope();
        value.protocol_version = 1;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::UnsupportedVersion)
        );

        let mut value = envelope();
        value.source_device_id.0.clear();
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::MissingBinding)
        );

        let mut value = envelope();
        value.source_key_id = value.target_key_id.clone();
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidPeerBinding)
        );

        let mut value = envelope();
        value.scope = PermissionScope::ScreenView;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidScope)
        );
    }

    #[test]
    fn envelope_shape_enforces_identifier_and_key_identifier_bounds() {
        let mut boundary = envelope();
        boundary.session_id = SessionId("s".repeat(CONTROL_ENVELOPE_MAX_ID_BYTES));
        boundary.source_device_id = DeviceId("c".repeat(CONTROL_ENVELOPE_MAX_ID_BYTES));
        boundary.target_device_id = DeviceId("t".repeat(CONTROL_ENVELOPE_MAX_ID_BYTES));
        boundary.source_key_id = "a".repeat(CONTROL_ENVELOPE_MAX_KEY_ID_BYTES);
        boundary.target_key_id = "b".repeat(CONTROL_ENVELOPE_MAX_KEY_ID_BYTES);
        assert_eq!(boundary.validate_shape(10_000), Ok(()));

        let mut oversized = boundary.clone();
        oversized.session_id.0.push('s');
        assert_eq!(
            oversized.validate_shape(10_000),
            Err(ControlEnvelopeError::MissingBinding)
        );

        let mut oversized = boundary.clone();
        oversized.source_device_id.0.push('c');
        assert_eq!(
            oversized.validate_shape(10_000),
            Err(ControlEnvelopeError::MissingBinding)
        );

        let mut oversized = boundary;
        oversized.target_key_id.push('b');
        assert_eq!(
            oversized.validate_shape(10_000),
            Err(ControlEnvelopeError::MissingBinding)
        );
    }

    #[test]
    fn envelope_shape_rejects_invalid_replay_lifetime_and_event_fields() {
        let mut value = envelope();
        value.grant_id = [0; 32];
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidGrantId)
        );

        let mut value = envelope();
        value.sequence = 0;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidSequence)
        );

        let mut value = envelope();
        value.sequence = u64::MAX;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidSequence)
        );

        let mut value = envelope();
        value.event_id = 0;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidEventId)
        );

        let mut value = envelope();
        value.policy_revision = 0;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidPolicyRevision)
        );

        let mut value = envelope();
        value.expires_at_ms = 9_999;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::Expired)
        );

        let mut value = envelope();
        value.issued_at_ms = 0;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidIssuedAt)
        );

        let mut value = envelope();
        value.expires_at_ms = value.issued_at_ms;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidLifetime)
        );

        let mut value = envelope();
        value.expires_at_ms = value
            .issued_at_ms
            .saturating_add(CONTROL_ENVELOPE_MAX_LIFETIME_MS + 1);
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidLifetime)
        );

        let mut value = envelope();
        value.issued_at_ms = 12_001;
        value.expires_at_ms = 13_000;
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::NotYetValid)
        );

        let mut value = envelope();
        value.authenticated_event_bytes.clear();
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidEventBytes)
        );

        let mut value = envelope();
        value.authenticated_event_bytes = vec![0; CONTROL_ENVELOPE_MAX_EVENT_BYTES + 1];
        assert_eq!(
            value.validate_shape(10_000),
            Err(ControlEnvelopeError::InvalidEventBytes)
        );
    }

    #[test]
    fn signed_shape_requires_ed25519_public_key_and_signature_lengths() {
        let signed = SignedControlEnvelopeV2 {
            payload: envelope(),
            public_key: [0x42; 32],
            signature: [0x24; 64],
        };
        assert_eq!(signed.validate_shape(10_000), Ok(()));
    }

    #[test]
    fn authenticated_wire_rejects_unknown_fields() {
        let signed = SignedControlEnvelopeV2 {
            payload: envelope(),
            public_key: [0x42; 32],
            signature: [0x24; 64],
        };
        let mut wire = serde_json::to_value(signed).expect("serialize signed envelope");
        wire.as_object_mut()
            .expect("signed envelope object")
            .insert("legacy_event".to_string(), serde_json::json!({"key": 65}));

        assert!(serde_json::from_value::<SignedControlEnvelopeV2>(wire).is_err());
    }

    #[test]
    fn wire_deserialization_rejects_oversized_bound_fields() {
        let mut oversized_id = serde_json::to_value(envelope()).expect("serialize envelope");
        oversized_id["session_id"] =
            serde_json::Value::String("s".repeat(CONTROL_ENVELOPE_MAX_ID_BYTES + 1));
        assert!(serde_json::from_value::<ControlEnvelopeV2>(oversized_id).is_err());

        let mut oversized_event = serde_json::to_value(envelope()).expect("serialize envelope");
        oversized_event["authenticated_event_bytes"] = serde_json::Value::Array(
            (0..=CONTROL_ENVELOPE_MAX_EVENT_BYTES)
                .map(|_| serde_json::Value::from(0))
                .collect(),
        );
        assert!(serde_json::from_value::<ControlEnvelopeV2>(oversized_event).is_err());

        let mut wrong_key = serde_json::to_value(SignedControlEnvelopeV2 {
            payload: envelope(),
            public_key: [0x42; 32],
            signature: [0x24; 64],
        })
        .expect("serialize signed envelope");
        wrong_key["public_key"] = serde_json::json!([1, 2, 3]);
        assert!(serde_json::from_value::<SignedControlEnvelopeV2>(wrong_key).is_err());
    }

    #[test]
    fn bounded_wire_decoder_rejects_before_parsing_oversized_json() {
        let signed = SignedControlEnvelopeV2 {
            payload: envelope(),
            public_key: [0x42; 32],
            signature: [0x24; 64],
        };
        let encoded = serde_json::to_vec(&signed).expect("serialize signed envelope");
        assert_eq!(
            SignedControlEnvelopeV2::decode_bounded_json(&encoded, 10_000)
                .expect("decode bounded envelope"),
            signed
        );

        let oversized_escaped_string = format!(
            "{{\"payload\":\"{}\"}}",
            "\\u0061".repeat(CONTROL_ENVELOPE_MAX_WIRE_BYTES)
        );
        assert_eq!(
            SignedControlEnvelopeV2::decode_bounded_json(
                oversized_escaped_string.as_bytes(),
                10_000,
            ),
            Err(ControlEnvelopeError::WireTooLarge)
        );
    }

    #[test]
    fn sequence_window_distinguishes_first_seen_exact_retry_and_conflicts() {
        let mut window = ControlSequenceWindow::new(8);
        let commitment = [0x11; 32];

        assert_eq!(
            window.observe(10, 20, commitment),
            Ok(ControlSequenceDecision::FirstSeen)
        );
        assert_eq!(
            window.observe(10, 20, commitment),
            Ok(ControlSequenceDecision::ExactRetry)
        );
        assert_eq!(
            window.observe(10, 21, commitment),
            Err(ControlSequenceError::Duplicate)
        );
        assert_eq!(
            window.observe(10, 20, [0x13; 32]),
            Err(ControlSequenceError::Duplicate)
        );
        assert_eq!(
            window.observe(11, 20, [0x12; 32]),
            Err(ControlSequenceError::Duplicate)
        );
        assert_eq!(window.len(), 1);
    }

    #[test]
    fn sequence_window_allows_fresh_reordering_but_rejects_out_of_window_values() {
        let mut window = ControlSequenceWindow::new(4);

        assert_eq!(
            window.observe(10, 100, [10; 32]),
            Ok(ControlSequenceDecision::FirstSeen)
        );
        assert_eq!(
            window.observe(12, 120, [12; 32]),
            Ok(ControlSequenceDecision::FirstSeen)
        );
        assert_eq!(
            window.observe(11, 110, [11; 32]),
            Ok(ControlSequenceDecision::FirstSeen)
        );
        assert_eq!(
            window.observe(8, 80, [8; 32]),
            Err(ControlSequenceError::OutOfWindow)
        );
    }

    #[test]
    fn sequence_window_prunes_state_to_its_bounded_capacity() {
        let mut window = ControlSequenceWindow::new(3);
        for sequence in 1_u64..=100 {
            assert_eq!(
                window.observe(sequence, sequence + 1_000, [sequence as u8; 32]),
                Ok(ControlSequenceDecision::FirstSeen)
            );
            assert!(window.len() <= 3);
        }

        assert_eq!(window.len(), 3);
        assert_eq!(
            window.observe(98, 1_098, [98; 32]),
            Ok(ControlSequenceDecision::ExactRetry)
        );
        assert_eq!(
            window.observe(97, 1_097, [97; 32]),
            Err(ControlSequenceError::OutOfWindow)
        );
    }

    #[test]
    fn sequence_window_clamps_capacity_and_handles_u64_max_without_wraparound() {
        let mut minimum = ControlSequenceWindow::new(0);
        assert_eq!(minimum.capacity(), 1);
        assert_eq!(
            minimum.observe(u64::MAX, 1, [1; 32]),
            Ok(ControlSequenceDecision::FirstSeen)
        );
        assert_eq!(
            minimum.observe(u64::MAX - 1, 2, [2; 32]),
            Err(ControlSequenceError::OutOfWindow)
        );

        let maximum = ControlSequenceWindow::new(usize::MAX);
        assert_eq!(maximum.capacity(), CONTROL_SEQUENCE_WINDOW_MAX_WIDTH);
        assert_eq!(CONTROL_ENVELOPE_MAX_EVENT_BYTES, 64);
    }
}
