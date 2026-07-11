//! Bounded, versioned framing for the private agent control channel.

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Current protocol major version.
pub const AGENT_IPC_PROTOCOL_MAJOR: u16 = 1;
/// Current protocol minor version.
///
/// Minor version 1 adds mandatory nonzero request tokens to every correlated
/// consent, execute, and input request/response pair. Minor version 2 adds
/// exact-request consent cancellation cleanup.
pub const AGENT_IPC_PROTOCOL_MINOR: u16 = 2;
/// Minimum negotiated minor version that supports correlated request tokens.
pub const AGENT_IPC_CORRELATED_REQUESTS_PROTOCOL_MINOR: u16 = 1;
/// Minimum negotiated minor version that supports consent cancellation cleanup.
pub const AGENT_IPC_CONSENT_CANCEL_PROTOCOL_MINOR: u16 = 2;
/// Maximum JSON payload carried by one control frame.
pub const AGENT_IPC_MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Bytes in the length-and-version frame header.
pub const AGENT_IPC_FRAME_HEADER_BYTES: usize = 8;

/// A decoded frame together with its negotiated wire version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame<T> {
    /// Protocol major read from the frame header.
    pub protocol_major: u16,
    /// Protocol minor read from the frame header.
    pub protocol_minor: u16,
    /// Deserialized control-plane message.
    pub message: T,
}

/// Framing, transport, and bounded-JSON failures.
#[derive(Debug, Error)]
pub enum FrameError {
    /// The buffer ended before the complete fixed-size header.
    #[error("agent IPC frame header is incomplete: got {actual} bytes")]
    HeaderTooShort {
        /// Bytes that were available.
        actual: usize,
    },
    /// The sender uses a protocol major that this implementation cannot parse.
    #[error("unsupported agent IPC protocol major {received}; supported major is {supported}")]
    UnsupportedMajor {
        /// Major version received from the peer.
        received: u16,
        /// Major version supported locally.
        supported: u16,
    },
    /// Zero-length payloads are never valid agent messages.
    #[error("agent IPC frame payload is empty")]
    EmptyPayload,
    /// The declared or encoded payload exceeds the control-plane bound.
    #[error("agent IPC frame is too large: {declared} bytes exceeds {max}")]
    FrameTooLarge {
        /// Payload bytes declared or produced.
        declared: usize,
        /// Maximum accepted payload size.
        max: usize,
    },
    /// The in-memory frame has trailing bytes or a truncated payload.
    #[error("agent IPC frame length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch {
        /// Payload bytes declared in the header.
        declared: usize,
        /// Payload bytes actually present.
        actual: usize,
    },
    /// The message could not be encoded as bounded JSON.
    #[error("agent IPC message encoding failed")]
    Encode(#[source] serde_json::Error),
    /// The payload is not a valid message of the expected direction/type.
    #[error("agent IPC message decoding failed")]
    Decode(#[source] serde_json::Error),
    /// The underlying private stream failed.
    #[error("agent IPC transport failed")]
    Io(#[from] std::io::Error),
}

/// Serialize a message into the current length-delimited protocol frame.
pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    let payload = serde_json::to_vec(message).map_err(FrameError::Encode)?;
    validate_payload_len(payload.len())?;

    let payload_len = u32::try_from(payload.len()).map_err(|_| FrameError::FrameTooLarge {
        declared: payload.len(),
        max: AGENT_IPC_MAX_FRAME_BYTES,
    })?;
    let mut frame = Vec::with_capacity(AGENT_IPC_FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&AGENT_IPC_PROTOCOL_MAJOR.to_le_bytes());
    frame.extend_from_slice(&AGENT_IPC_PROTOCOL_MINOR.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode exactly one complete in-memory frame.
pub fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<DecodedFrame<T>, FrameError> {
    if frame.len() < AGENT_IPC_FRAME_HEADER_BYTES {
        return Err(FrameError::HeaderTooShort {
            actual: frame.len(),
        });
    }

    let (payload_len, protocol_major, protocol_minor) = parse_header(frame);
    validate_major(protocol_major)?;
    validate_payload_len(payload_len)?;

    let actual = frame.len() - AGENT_IPC_FRAME_HEADER_BYTES;
    if actual != payload_len {
        return Err(FrameError::LengthMismatch {
            declared: payload_len,
            actual,
        });
    }

    let message = serde_json::from_slice(&frame[AGENT_IPC_FRAME_HEADER_BYTES..])
        .map_err(FrameError::Decode)?;
    Ok(DecodedFrame {
        protocol_major,
        protocol_minor,
        message,
    })
}

/// Read and decode one frame from an asynchronous private stream.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<DecodedFrame<T>, FrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0_u8; AGENT_IPC_FRAME_HEADER_BYTES];
    reader.read_exact(&mut header).await?;
    let (payload_len, protocol_major, protocol_minor) = parse_header(&header);

    // Version and size are checked before any peer-controlled allocation.
    validate_major(protocol_major)?;
    validate_payload_len(payload_len)?;

    let mut payload = vec![0_u8; payload_len];
    reader.read_exact(&mut payload).await?;
    let message = serde_json::from_slice(&payload).map_err(FrameError::Decode)?;
    Ok(DecodedFrame {
        protocol_major,
        protocol_minor,
        message,
    })
}

/// Encode and write one frame to an asynchronous private stream.
pub async fn write_frame<W, T>(writer: &mut W, message: &T) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode_frame(message)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

fn parse_header(header: &[u8]) -> (usize, u16, u16) {
    let payload_len = u32::from_le_bytes(header[0..4].try_into().expect("fixed frame header"));
    let protocol_major = u16::from_le_bytes(header[4..6].try_into().expect("fixed frame header"));
    let protocol_minor = u16::from_le_bytes(header[6..8].try_into().expect("fixed frame header"));
    (payload_len as usize, protocol_major, protocol_minor)
}

fn validate_major(protocol_major: u16) -> Result<(), FrameError> {
    if protocol_major != AGENT_IPC_PROTOCOL_MAJOR {
        return Err(FrameError::UnsupportedMajor {
            received: protocol_major,
            supported: AGENT_IPC_PROTOCOL_MAJOR,
        });
    }
    Ok(())
}

fn validate_payload_len(payload_len: usize) -> Result<(), FrameError> {
    if payload_len == 0 {
        return Err(FrameError::EmptyPayload);
    }
    if payload_len > AGENT_IPC_MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge {
            declared: payload_len,
            max: AGENT_IPC_MAX_FRAME_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct Message {
        value: u64,
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut frame = encode_frame(&Message { value: 7 }).unwrap();
        frame.push(0);
        assert!(matches!(
            decode_frame::<Message>(&frame),
            Err(FrameError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn rejects_empty_payload_before_json_decode() {
        let mut frame = Vec::from(0_u32.to_le_bytes());
        frame.extend_from_slice(&AGENT_IPC_PROTOCOL_MAJOR.to_le_bytes());
        frame.extend_from_slice(&AGENT_IPC_PROTOCOL_MINOR.to_le_bytes());
        assert!(matches!(
            decode_frame::<Message>(&frame),
            Err(FrameError::EmptyPayload)
        ));
    }

    #[tokio::test]
    async fn stream_rejects_oversized_declaration_before_reading_a_body() {
        let (mut writer, mut reader) = tokio::io::duplex(AGENT_IPC_FRAME_HEADER_BYTES);
        let declared = (AGENT_IPC_MAX_FRAME_BYTES as u32) + 1;
        let mut header = Vec::from(declared.to_le_bytes());
        header.extend_from_slice(&AGENT_IPC_PROTOCOL_MAJOR.to_le_bytes());
        header.extend_from_slice(&AGENT_IPC_PROTOCOL_MINOR.to_le_bytes());
        writer.write_all(&header).await.unwrap();

        assert!(matches!(
            read_frame::<_, Message>(&mut reader).await,
            Err(FrameError::FrameTooLarge { .. })
        ));
    }
}
