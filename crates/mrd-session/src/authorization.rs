#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthorizationState {
    Pending,
    Granted { policy_revision: u64 },
    Denied { reason: String },
    Revoked { reason: String },
}

impl Default for AuthorizationState {
    fn default() -> Self { Self::Pending }
}
