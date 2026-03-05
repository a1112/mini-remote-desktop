use thiserror::Error;

pub const PROTOCOL_VERSION: u8 = 2;
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
    ClipboardSet = 7,
    ClipboardGet = 8,
    FileControl = 9,
    FileChunk = 10,
    AudioControl = 11,
    FileMount = 12,
    AudioRouteControl = 13,
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
            7 => Ok(Self::ClipboardSet),
            8 => Ok(Self::ClipboardGet),
            9 => Ok(Self::FileControl),
            10 => Ok(Self::FileChunk),
            11 => Ok(Self::AudioControl),
            12 => Ok(Self::FileMount),
            13 => Ok(Self::AudioRouteControl),
            _ => Err(ProtoError::UnknownEventType(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u8,
        pressed: bool,
    },
    MouseWheel {
        delta: i32,
    },
    Key {
        key: u32,
        pressed: bool,
    },
    GamepadAxis {
        gamepad: u8,
        axis: u8,
        value: i16,
    },
    GamepadButton {
        gamepad: u8,
        button: u8,
        pressed: bool,
    },
    ClipboardSet {
        mime: u8,
        bytes: Vec<u8>,
    },
    ClipboardGet {},
    FileControl {
        op: u8,
        transfer_id: u64,
        arg0: u64,
        arg1: u64,
    },
    FileChunk {
        transfer_id: u64,
        chunk_idx: u32,
        total_chunks: u32,
        sha256_16: [u8; 16],
        payload: Vec<u8>,
    },
    AudioControl {
        op: u8,
        codec: u8,
        sample_rate: u32,
        channels: u8,
        frame_ms: u16,
    },
    AudioRouteControl {
        mode: u8,
        scope: u8,
        target_pid: u32,
        follow_children: bool,
    },
    FileMount {
        op: u8,
        mount_id: u64,
        flags: u32,
        path: String,
    },
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
            Self::ClipboardSet { .. } => EventType::ClipboardSet,
            Self::ClipboardGet { .. } => EventType::ClipboardGet,
            Self::FileControl { .. } => EventType::FileControl,
            Self::FileChunk { .. } => EventType::FileChunk,
            Self::AudioControl { .. } => EventType::AudioControl,
            Self::FileMount { .. } => EventType::FileMount,
            Self::AudioRouteControl { .. } => EventType::AudioRouteControl,
        }
    }

    pub fn channel_class(&self) -> ChannelClass {
        match self {
            Self::MouseMove { .. } | Self::MouseWheel { .. } | Self::GamepadAxis { .. } => {
                ChannelClass::Realtime
            }
            Self::MouseButton { .. }
            | Self::Key { .. }
            | Self::GamepadButton { .. }
            | Self::ClipboardSet { .. }
            | Self::ClipboardGet { .. }
            | Self::FileControl { .. }
            | Self::FileChunk { .. }
            | Self::AudioControl { .. }
            | Self::FileMount { .. }
            | Self::AudioRouteControl { .. } => ChannelClass::Reliable,
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
        ControlEvent::ClipboardSet { mime, bytes } => {
            let mut v = Vec::with_capacity(1 + 4 + bytes.len());
            v.push(*mime);
            v.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            v.extend_from_slice(bytes);
            v
        }
        ControlEvent::ClipboardGet {} => Vec::new(),
        ControlEvent::FileControl {
            op,
            transfer_id,
            arg0,
            arg1,
        } => {
            let mut v = Vec::with_capacity(1 + 8 + 8 + 8);
            v.push(*op);
            v.extend_from_slice(&transfer_id.to_be_bytes());
            v.extend_from_slice(&arg0.to_be_bytes());
            v.extend_from_slice(&arg1.to_be_bytes());
            v
        }
        ControlEvent::FileChunk {
            transfer_id,
            chunk_idx,
            total_chunks,
            sha256_16,
            payload,
        } => {
            let mut v = Vec::with_capacity(8 + 4 + 4 + 16 + 4 + payload.len());
            v.extend_from_slice(&transfer_id.to_be_bytes());
            v.extend_from_slice(&chunk_idx.to_be_bytes());
            v.extend_from_slice(&total_chunks.to_be_bytes());
            v.extend_from_slice(sha256_16);
            v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            v.extend_from_slice(payload);
            v
        }
        ControlEvent::AudioControl {
            op,
            codec,
            sample_rate,
            channels,
            frame_ms,
        } => {
            let mut v = Vec::with_capacity(1 + 1 + 4 + 1 + 2);
            v.push(*op);
            v.push(*codec);
            v.extend_from_slice(&sample_rate.to_be_bytes());
            v.push(*channels);
            v.extend_from_slice(&frame_ms.to_be_bytes());
            v
        }
        ControlEvent::FileMount {
            op,
            mount_id,
            flags,
            path,
        } => {
            let path_bytes = path.as_bytes();
            let mut v = Vec::with_capacity(1 + 8 + 4 + 2 + path_bytes.len());
            v.push(*op);
            v.extend_from_slice(&mount_id.to_be_bytes());
            v.extend_from_slice(&flags.to_be_bytes());
            v.extend_from_slice(&(path_bytes.len() as u16).to_be_bytes());
            v.extend_from_slice(path_bytes);
            v
        }
        ControlEvent::AudioRouteControl {
            mode,
            scope,
            target_pid,
            follow_children,
        } => {
            let mut v = Vec::with_capacity(1 + 1 + 4 + 1);
            v.push(*mode);
            v.push(*scope);
            v.extend_from_slice(&target_pid.to_be_bytes());
            v.push(u8::from(*follow_children));
            v
        }
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
        EventType::ClipboardSet => {
            if payload.len() < 5 {
                return Err(ProtoError::InvalidEventPayloadLength {
                    event_type: typ,
                    expected: 5,
                    actual: payload.len(),
                });
            }
            let mime = payload[0];
            let len = u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
            if payload.len() != 5 + len {
                return Err(ProtoError::PayloadLengthMismatch {
                    declared: 5 + len,
                    actual: payload.len(),
                });
            }
            Ok(ControlEvent::ClipboardSet {
                mime,
                bytes: payload[5..].to_vec(),
            })
        }
        EventType::ClipboardGet => {
            expect_payload_len(typ, payload, 0)?;
            Ok(ControlEvent::ClipboardGet {})
        }
        EventType::FileControl => {
            expect_payload_len(typ, payload, 25)?;
            Ok(ControlEvent::FileControl {
                op: payload[0],
                transfer_id: u64::from_be_bytes([
                    payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
                    payload[7], payload[8],
                ]),
                arg0: u64::from_be_bytes([
                    payload[9],
                    payload[10],
                    payload[11],
                    payload[12],
                    payload[13],
                    payload[14],
                    payload[15],
                    payload[16],
                ]),
                arg1: u64::from_be_bytes([
                    payload[17],
                    payload[18],
                    payload[19],
                    payload[20],
                    payload[21],
                    payload[22],
                    payload[23],
                    payload[24],
                ]),
            })
        }
        EventType::FileChunk => {
            if payload.len() < 36 {
                return Err(ProtoError::InvalidEventPayloadLength {
                    event_type: typ,
                    expected: 36,
                    actual: payload.len(),
                });
            }
            let transfer_id = u64::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
                payload[7],
            ]);
            let chunk_idx = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
            let total_chunks =
                u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]);
            let mut sha256_16 = [0_u8; 16];
            sha256_16.copy_from_slice(&payload[16..32]);
            let declared =
                u32::from_be_bytes([payload[32], payload[33], payload[34], payload[35]]) as usize;
            if payload.len() != 36 + declared {
                return Err(ProtoError::PayloadLengthMismatch {
                    declared: 36 + declared,
                    actual: payload.len(),
                });
            }
            Ok(ControlEvent::FileChunk {
                transfer_id,
                chunk_idx,
                total_chunks,
                sha256_16,
                payload: payload[36..].to_vec(),
            })
        }
        EventType::AudioControl => {
            expect_payload_len(typ, payload, 9)?;
            Ok(ControlEvent::AudioControl {
                op: payload[0],
                codec: payload[1],
                sample_rate: u32::from_be_bytes([payload[2], payload[3], payload[4], payload[5]]),
                channels: payload[6],
                frame_ms: u16::from_be_bytes([payload[7], payload[8]]),
            })
        }
        EventType::FileMount => {
            if payload.len() < 15 {
                return Err(ProtoError::InvalidEventPayloadLength {
                    event_type: typ,
                    expected: 15,
                    actual: payload.len(),
                });
            }
            let op = payload[0];
            let mount_id = u64::from_be_bytes([
                payload[1], payload[2], payload[3], payload[4], payload[5], payload[6], payload[7],
                payload[8],
            ]);
            let flags = u32::from_be_bytes([payload[9], payload[10], payload[11], payload[12]]);
            let declared_len = u16::from_be_bytes([payload[13], payload[14]]) as usize;
            if payload.len() != 15 + declared_len {
                return Err(ProtoError::PayloadLengthMismatch {
                    declared: 15 + declared_len,
                    actual: payload.len(),
                });
            }
            let path = String::from_utf8_lossy(&payload[15..]).to_string();
            Ok(ControlEvent::FileMount {
                op,
                mount_id,
                flags,
                path,
            })
        }
        EventType::AudioRouteControl => {
            expect_payload_len(typ, payload, 7)?;
            Ok(ControlEvent::AudioRouteControl {
                mode: payload[0],
                scope: payload[1],
                target_pid: u32::from_be_bytes([payload[2], payload[3], payload[4], payload[5]]),
                follow_children: payload[6] != 0,
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

    #[test]
    fn encode_decode_roundtrip_clipboard_set() {
        let frame = Frame {
            flags: 0,
            seq: 7,
            ts_us: 42,
            event: ControlEvent::ClipboardSet {
                mime: 1,
                bytes: b"hello".to_vec(),
            },
        };
        let bytes = frame.encode();
        let decoded = Frame::decode(&bytes).expect("decode clipboard set");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn encode_decode_roundtrip_file_chunk() {
        let frame = Frame {
            flags: 3,
            seq: 99,
            ts_us: 1_001,
            event: ControlEvent::FileChunk {
                transfer_id: 1234,
                chunk_idx: 2,
                total_chunks: 9,
                sha256_16: [0xAB; 16],
                payload: vec![1, 2, 3, 4, 5],
            },
        };
        let bytes = frame.encode();
        let decoded = Frame::decode(&bytes).expect("decode file chunk");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn encode_decode_roundtrip_file_mount() {
        let frame = Frame {
            flags: 1,
            seq: 123,
            ts_us: 9_999,
            event: ControlEvent::FileMount {
                op: 1,
                mount_id: 77,
                flags: 0b101,
                path: "C:/Users/Public".to_string(),
            },
        };
        let bytes = frame.encode();
        let decoded = Frame::decode(&bytes).expect("decode file mount");
        assert_eq!(decoded, frame);
    }
}
