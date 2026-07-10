#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaState {
    Idle,
    Starting,
    Streaming,
    Stopped,
    Failed { reason: String },
}

impl Default for MediaState {
    fn default() -> Self { Self::Idle }
}
