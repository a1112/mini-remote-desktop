//! Shared UDP heartbeat protocol DTOs.

use serde::{Deserialize, Serialize};

/// Heartbeat message sent by a local device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatMessage {
    /// Device identifier.
    pub device_id: String,
    /// Device role, usually `agent` or `controller`.
    pub device_type: String,
    /// Human-readable device name.
    pub device_name: String,
    /// Protocol version.
    pub protocol_version: u32,
    /// Client timestamp in milliseconds since Unix epoch.
    pub timestamp_ms: u64,
    /// Supported transport identifiers.
    #[serde(default)]
    pub transports: Vec<String>,
}

/// Heartbeat response sent by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    /// Server timestamp in milliseconds since Unix epoch.
    pub server_timestamp_ms: u64,
    /// Number of currently online devices.
    pub online_count: usize,
    /// Whether the client should re-register.
    #[serde(default)]
    pub reregister: bool,
}
