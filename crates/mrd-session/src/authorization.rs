#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthorizationState {
    #[default]
    Pending,
    Granted {
        policy_revision: u64,
    },
    Denied {
        reason: String,
    },
    Revoked {
        reason: String,
    },
}
