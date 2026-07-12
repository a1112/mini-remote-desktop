//! Private machine-service/session-agent IPC contracts.

#![warn(missing_docs)]

mod bootstrap;
mod framing;
mod grant;
mod protocol;

pub use bootstrap::*;

pub use framing::{
    decode_frame, encode_frame, read_frame, write_frame, DecodedFrame, FrameError,
    AGENT_IPC_CONSENT_CANCEL_PROTOCOL_MINOR, AGENT_IPC_CORRELATED_REQUESTS_PROTOCOL_MINOR,
    AGENT_IPC_FRAME_HEADER_BYTES, AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR,
    AGENT_IPC_PROTOCOL_MINOR, AGENT_IPC_RENDER_ACCESS_UNIT_PROTOCOL_MINOR,
    AGENT_IPC_RENDER_SURFACE_PROTOCOL_MINOR,
};
pub use grant::{
    authorize_input_resource, validate_execute_command, validate_input_event, AuthorizedCommand,
    AuthorizedGrant, AuthorizedInputResource, ExecuteGrant, ExecuteGrantClaims,
    ExecuteGrantVerifier, ExecutionContext, GrantAudience, GrantValidationError,
    InputResourceAuthorizationError, ValidatedInputEvent, AGENT_EXECUTE_GRANT_MAX_LIFETIME_MS,
    AGENT_EXECUTE_GRANT_SIGNATURE_CONTEXT,
};
pub use protocol::*;
