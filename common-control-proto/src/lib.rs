use thiserror::Error;

pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelClass {
    Realtime,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventType {
    MouseMove = 1,
    MouseButton = 2,
    MouseWheel = 3,
    Key = 4,
    GamepadAxis = 5,
    GamepadButton = 6,
}

impl TryFrom<u8> for EventType {
    type Error = ProtoError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::MouseMove),
            2 => Ok(Self::MouseButton),
            3 => Ok(Self::MouseWheel),
            4 => Ok(Self::Key),
            5 => Ok(Self::GamepadAxis),
            6 => Ok(Self::GamepadButton),
            _ => Err(ProtoError::UnknownEventType(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: u8, pressed: bool },
    MouseWheel { delta: i32 },
    Key { key: u32, pressed: bool },
    GamepadAxis { gamepad: u8, axis: u8, value: i16 },
    GamepadButton { gamepad: u8, button: u8, pressed: bool },
}

impl ControlEvent {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::MouseMove { .. } => EventType::MouseMove,
            Self::MouseButton { .. } => EventType::MouseButton,
            Self::MouseWheel { .. } => EventType::MouseWheel,
            Self::Key { .. } => EventType::Key,
            Self::GamepadAxis { .. } => EventType::GamepadAxis,
            Self::GamepadButton { .. } => EventType::GamepadButton,
        }
    }

    pub fn channel_class(&self) -> ChannelClass {
        match self {
            Self::MouseMove { .. } | Self::MouseWheel { .. } | Self::GamepadAxis { .. } => {
                ChannelClass::Realtime
            }
            Self::MouseButton { .. } | Self::Key { .. } | Self::GamepadButton { .. } => {
                ChannelClass::Reliable
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub flags: u8,
    pub seq: u32,
    pub ts_us: u64,
    pub event: ControlEvent,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let payload = encode_event_payload(&self.event);
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        out.push(PROTOCOL_VERSION);
        out.push(self.event.event_type() as u8);
        out.push(self.flags);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.ts_us.to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.len() < HEADER_LEN {
            return Err(ProtoError::FrameTooShort {
                actual: buf.len(),
                min: HEADER_LEN,
            });
        }
        let ver = buf[0];
        if ver != PROTOCOL_VERSION {
            return Err(ProtoError::UnsupportedVersion(ver));
        }
        let typ = EventType::try_from(buf[1])?;
        let flags = buf[2];
        let payload_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
        let seq = u32::from_be_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let ts_us = u64::from_be_bytes([
            buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15], buf[16],
        ]);
        if buf.len() != HEADER_LEN + payload_len {
            return Err(ProtoError::PayloadLengthMismatch {
                declared: payload_len,
                actual: buf.len().saturating_sub(HEADER_LEN),
            });
        }
        let event = decode_event_payload(typ, &buf[HEADER_LEN..])?;
        Ok(Self {
            flags,
            seq,
            ts_us,
            event,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("frame too short: actual={actual}, min={min}")]
    FrameTooShort { actual: usize, min: usize },
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("unknown event type: {0}")]
    UnknownEventType(u8),
    #[error("payload length mismatch: declared={declared}, actual={actual}")]
    PayloadLengthMismatch { declared: usize, actual: usize },
    #[error("invalid payload length for {event_type:?}: expected={expected}, actual={actual}")]
    InvalidEventPayloadLength {
        event_type: EventType,
        expected: usize,
        actual: usize,
    },
}

fn encode_event_payload(event: &ControlEvent) -> Vec<u8> {
    match event {
        ControlEvent::MouseMove { x, y } => {
            let mut v = Vec::with_capacity(8);
            v.extend_from_slice(&x.to_be_bytes());
            v.extend_from_slice(&y.to_be_bytes());
            v
        }
        ControlEvent::MouseButton { button, pressed } => vec![*button, u8::from(*pressed)],
        ControlEvent::MouseWheel { delta } => delta.to_be_bytes().to_vec(),
        ControlEvent::Key { key, pressed } => {
            let mut v = Vec::with_capacity(5);
            v.extend_from_slice(&key.to_be_bytes());
            v.push(u8::from(*pressed));
            v
        }
        ControlEvent::GamepadAxis {
            gamepad,
            axis,
            value,
        } => {
            let mut v = Vec::with_capacity(4);
            v.push(*gamepad);
            v.push(*axis);
            v.extend_from_slice(&value.to_be_bytes());
            v
        }
        ControlEvent::GamepadButton {
            gamepad,
            button,
            pressed,
        } => vec![*gamepad, *button, u8::from(*pressed)],
    }
}

fn decode_event_payload(typ: EventType, payload: &[u8]) -> Result<ControlEvent, ProtoError> {
    match typ {
        EventType::MouseMove => {
            expect_payload_len(typ, payload, 8)?;
            Ok(ControlEvent::MouseMove {
                x: i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                y: i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
            })
        }
        EventType::MouseButton => {
            expect_payload_len(typ, payload, 2)?;
            Ok(ControlEvent::MouseButton {
                button: payload[0],
                pressed: payload[1] != 0,
            })
        }
        EventType::MouseWheel => {
            expect_payload_len(typ, payload, 4)?;
            Ok(ControlEvent::MouseWheel {
                delta: i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
            })
        }
        EventType::Key => {
            expect_payload_len(typ, payload, 5)?;
            Ok(ControlEvent::Key {
                key: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                pressed: payload[4] != 0,
            })
        }
        EventType::GamepadAxis => {
            expect_payload_len(typ, payload, 4)?;
            Ok(ControlEvent::GamepadAxis {
                gamepad: payload[0],
                axis: payload[1],
                value: i16::from_be_bytes([payload[2], payload[3]]),
            })
        }
        EventType::GamepadButton => {
            expect_payload_len(typ, payload, 3)?;
            Ok(ControlEvent::GamepadButton {
                gamepad: payload[0],
                button: payload[1],
                pressed: payload[2] != 0,
            })
        }
    }
}

fn expect_payload_len(
    event_type: EventType,
    payload: &[u8],
    expected: usize,
) -> Result<(), ProtoError> {
    if payload.len() != expected {
        return Err(ProtoError::InvalidEventPayloadLength {
            event_type,
            expected,
            actual: payload.len(),
        });
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct SeqTracker {
    last_seq: Option<u32>,
}

impl SeqTracker {
    pub fn validate_and_update(&mut self, seq: u32) -> bool {
        let is_valid = match self.last_seq {
            None => true,
            Some(prev) => seq > prev,
        };
        if is_valid {
            self.last_seq = Some(seq);
        }
        is_valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip_mouse_move() {
        let frame = Frame {
            flags: 0xA5,
            seq: 42,
            ts_us: 1_234_567,
            event: ControlEvent::MouseMove { x: -120, y: 88 },
        };
        let bytes = frame.encode();
        let decoded = Frame::decode(&bytes).expect("decode should succeed");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn reject_unknown_event_type() {
        let mut bytes = Frame {
            flags: 0,
            seq: 1,
            ts_us: 2,
            event: ControlEvent::MouseWheel { delta: 1 },
        }
        .encode();
        bytes[1] = 0xFE;
        let err = Frame::decode(&bytes).expect_err("should reject unknown type");
        assert_eq!(err, ProtoError::UnknownEventType(0xFE));
    }

    #[test]
    fn seq_tracker_requires_monotonic_increase() {
        let mut tracker = SeqTracker::default();
        assert!(tracker.validate_and_update(10));
        assert!(!tracker.validate_and_update(10));
        assert!(!tracker.validate_and_update(9));
        assert!(tracker.validate_and_update(11));
    }
}

