use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackendRole {
    Controller,
    Agent,
    Peer,
}

impl BackendRole {
    /// Authenticated full clients can act as either endpoint. Registration
    /// must still bind the exact declared role to its backend credential.
    pub fn has_capability(&self, required: BackendRole) -> bool {
        self == &required
            || (matches!(self, Self::Peer) && matches!(required, Self::Controller | Self::Agent))
    }
}

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("invalid identifier")]
    InvalidIdentifier,
}
