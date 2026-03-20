// mrd-ipc: Local IPC protocol between Rdesk and mrd-service
//
// Defines stable request/response DTOs for local communication.
// This crate must remain independent of Tauri types to maintain
// a clean boundary between UI shell and service.

#![warn(missing_docs)]

pub mod client;

use serde::{Deserialize, Serialize};
use mrd_proto::{SessionId, DeviceId};

/// IPC request from Rdesk to mrd-service
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum IpcRequest {
    /// Register local device with the service
    RegisterDevice {
        device_id: DeviceId,
        device_name: String,
    },
    /// List available devices
    ListDevices,
    /// Start a new session as controller
    StartSession {
        session_id: SessionId,
        target_device_id: DeviceId,
        transport_kind: String,  // "quic" or "webrtc"
    },
    /// Accept an incoming session as agent
    AcceptSession {
        session_id: SessionId,
        source_device_id: DeviceId,
    },
    /// Start sending media (controller role)
    StartSender {
        session_id: SessionId,
    },
    /// Start receiving media (agent role)
    StartReceiver {
        session_id: SessionId,
    },
    /// Stop a session
    StopSession {
        session_id: SessionId,
    },
    /// Get current session runtime snapshot
    SessionRuntimeSnapshot {
        session_id: SessionId,
    },
    /// Stream probe events
    StreamProbeEvents,
}

/// IPC response from mrd-service to Rdesk
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum IpcResponse {
    /// Device registration successful
    DeviceRegistered {
        device_id: DeviceId,
    },
    /// List of available devices
    DeviceList {
        devices: Vec<DeviceInfo>,
    },
    /// Session started successfully
    SessionStarted {
        session_id: SessionId,
    },
    /// Session accepted successfully
    SessionAccepted {
        session_id: SessionId,
    },
    /// Sender started
    SenderStarted {
        session_id: SessionId,
    },
    /// Receiver started
    ReceiverStarted {
        session_id: SessionId,
    },
    /// Session stopped
    SessionStopped {
        session_id: SessionId,
    },
    /// Session runtime snapshot
    SessionSnapshot {
        snapshot: SessionRuntimeSnapshot,
    },
    /// Probe event data
    ProbeEvent {
        event: Vec<u8>,  // Serialized probe event
    },
    /// Error response
    Error {
        code: String,
        message: String,
    },
}

/// Device information DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_id: DeviceId,
    pub device_name: String,
    pub is_online: bool,
}

/// Session runtime snapshot DTO (stable IPC contract)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeSnapshot {
    pub session_id: SessionId,
    pub role: String,  // "controller" or "agent"
    pub state: String,  // "created", "listening", "connecting", "connected", "streaming", "failed", "closed"
    pub transport_kind: String,  // "quic" or "webrtc"
    pub local_bootstrap: Option<SessionBootstrap>,
    pub remote_bootstrap: Option<SessionBootstrap>,
}

/// Session bootstrap metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBootstrap {
    pub listen_addr: Option<String>,
    pub server_name: Option<String>,
    pub cert_der: Option<String>,  // Base64-encoded DER certificate
}
