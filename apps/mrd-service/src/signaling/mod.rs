//! Service-owned authenticated realtime signaling runtime.

mod config;
mod event_mapper;
mod runtime;

pub use config::{SignalingConfig, SignalingConfigError};
pub use event_mapper::ServiceSignalingMapper;
pub use runtime::{
    spawn, spawn_from_env, InboundDisposition, SignalingConnectionState, SignalingRuntimeCore,
    SignalingRuntimeError, SignalingRuntimeSnapshot, SignalingStatus, SignalingTask,
};
