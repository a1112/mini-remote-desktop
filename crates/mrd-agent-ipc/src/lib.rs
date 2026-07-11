//! Private machine-service/session-agent IPC contracts.

#![warn(missing_docs)]

mod framing;
mod grant;
mod protocol;

pub use framing::{
    decode_frame, encode_frame, read_frame, write_frame, DecodedFrame, FrameError,
    AGENT_IPC_FRAME_HEADER_BYTES, AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR,
    AGENT_IPC_PROTOCOL_MINOR,
};
pub use grant::{
    validate_execute_command, AuthorizedCommand, AuthorizedGrant, ExecuteGrant, ExecuteGrantClaims,
    ExecuteGrantVerifier, ExecutionContext, GrantAudience, GrantValidationError,
    AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS, AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT,
};
pub use protocol::*;
