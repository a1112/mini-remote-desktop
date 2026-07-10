#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RouteKind {
    LanQuic,
    WebRtcDirect,
    WebRtcRelay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RouteState {
    Idle,
    Establishing(RouteKind),
    Active(RouteKind),
    Failed { kind: RouteKind, reason: String },
}

impl Default for RouteState {
    fn default() -> Self { Self::Idle }
}
