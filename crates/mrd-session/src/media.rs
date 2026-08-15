#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaState {
    #[default]
    Idle,
    Starting,
    Streaming,
    Stopped,
    Failed {
        reason: String,
    },
}
