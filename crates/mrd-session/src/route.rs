#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RouteKind {
    LanQuic,
    WebRtcDirect,
    WebRtcRelay,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RouteState {
    #[default]
    Idle,
    Establishing(RouteKind),
    Active(RouteKind),
    Failed {
        kind: RouteKind,
        reason: String,
    },
}
