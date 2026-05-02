// mrd-ipc: Local IPC protocol between Rdesk and mrd-service
//
// Defines stable request/response DTOs for local communication.
// This crate must remain independent of Tauri types to maintain
// a clean boundary between UI shell and service.

#![warn(missing_docs)]

pub mod client;
pub mod transport;

use mrd_proto::{DeviceId, SessionId};
use serde::{Deserialize, Serialize};

// === Shell / Lifecycle DTOs (Phase 2) ===
// Defined first to avoid forward references

/// Reason for opening the UI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenUiReason {
    /// User clicked tray menu
    TrayOpen,
    /// Incoming session request
    SessionIncoming,
    /// User action (e.g., from diagnostics)
    UserRequest,
    /// Opening diagnostics/debugging view
    Diagnostics,
}

/// Result of UI open operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiOpenStatus {
    /// Focused existing UI window
    FocusedExisting,
    /// Spawned new UI process
    SpawnedNew,
    /// UI unavailable (e.g., not configured)
    Unavailable,
}

/// Reason for UI detachment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiDetachReason {
    /// User closed UI window normally
    UserClose,
    /// User explicitly quit UI
    UserQuit,
    /// UI crashed
    Crash,
    /// Connection lost
    ConnectionLost,
}

/// Service shutdown mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownMode {
    /// Graceful shutdown - finish active sessions if possible
    Graceful,
    /// Force shutdown - terminate immediately
    Force,
    /// Shutdown after sessions end
    AfterSessions,
}

/// Shell/service status snapshot
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellStatusSnapshot {
    pub service_pid: u32,
    pub ui_pid: Option<u32>,
    pub tray_available: bool,
    pub autostart_enabled: Option<bool>,
    pub active_session_count: usize,
    pub last_error: Option<String>,
}

// === Core IPC Types ===

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
    /// Get the current LAN peer discovery snapshot.
    LanDiscoverySnapshot,
    /// Send an immediate LAN discovery probe and return the current snapshot.
    RefreshLanDiscovery,
    /// List all active sessions
    ListSessions,
    /// Start a new session as controller
    StartSession {
        session_id: SessionId,
        target_device_id: DeviceId,
        transport_kind: String, // "quic" or "webrtc"
    },
    /// Start a LAN P2P session as controller and ask the discovered peer to accept it.
    StartLanRemoteSession {
        session_id: SessionId,
        target_device_id: DeviceId,
        transport_kind: String, // "quic" or "webrtc"
    },
    /// Accept an incoming session as agent
    AcceptSession {
        session_id: SessionId,
        source_device_id: DeviceId,
    },
    /// Start sending media (controller role)
    StartSender { session_id: SessionId },
    /// Start receiving media (agent role)
    StartReceiver { session_id: SessionId },
    /// Stop a session
    StopSession { session_id: SessionId },
    /// Mark a session as failed and retain its failure reason.
    FailSession {
        session_id: SessionId,
        reason: String,
    },
    /// Recover a failed or closed session back to its role-appropriate startup state.
    RecoverSession { session_id: SessionId },
    /// Get current session runtime snapshot
    SessionRuntimeSnapshot { session_id: SessionId },
    /// Get aggregated runtime snapshot
    RuntimeSnapshot,
    /// Get probe snapshot data
    ProbeSnapshot { session_id: SessionId },
    /// Stream probe events
    StreamProbeEvents,
    /// Health check for service
    ServiceHealth,

    // === Shell / Lifecycle Commands (Phase 2) ===
    /// Request to open/focus the UI
    OpenUi { reason: OpenUiReason },
    /// Request to focus an existing UI window
    FocusUi,
    /// Notify service that UI has attached
    UiAttached {
        pid: u32,
        executable_path: Option<String>,
    },
    /// Notify service that UI is detaching
    UiDetached { pid: u32, reason: UiDetachReason },
    /// Get current shell/service status
    GetShellStatus,
    /// Set autostart enabled state
    SetAutostart { enabled: bool },
    /// Get autostart status
    GetAutostartStatus,
    /// Request service shutdown
    ShutdownService { mode: ShutdownMode },
}

/// IPC response from mrd-service to Rdesk
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum IpcResponse {
    /// Device registration successful
    DeviceRegistered { device_id: DeviceId },
    /// List of available devices
    DeviceList { devices: Vec<DeviceInfo> },
    /// LAN peer discovery snapshot.
    LanDiscoverySnapshot {
        /// Current discovery state.
        snapshot: LanDiscoverySnapshot,
    },
    /// List of active sessions
    SessionList { sessions: Vec<SessionInfo> },
    /// Session started successfully
    SessionStarted { session_id: SessionId },
    /// Session accepted successfully
    SessionAccepted { session_id: SessionId },
    /// Sender started
    SenderStarted { session_id: SessionId },
    /// Receiver started
    ReceiverStarted { session_id: SessionId },
    /// Session stopped
    SessionStopped { session_id: SessionId },
    /// Session failed
    SessionFailed { session_id: SessionId },
    /// Session recovered
    SessionRecovered { session_id: SessionId },
    /// Session runtime snapshot
    SessionSnapshot { snapshot: SessionRuntimeSnapshot },
    /// Aggregated runtime snapshot
    RuntimeSnapshot { snapshot: RuntimeSnapshot },
    /// Probe snapshot data
    ProbeSnapshot { snapshot: ProbeSnapshot },
    /// Probe event data
    ProbeEvent {
        event: Vec<u8>, // Serialized probe event
    },
    /// Service health status
    ServiceHealth { status: ServiceStatus },

    // === Shell / Lifecycle Responses (Phase 2) ===
    /// Result of UI open request
    UiOpenResult {
        status: UiOpenStatus,
        pid: Option<u32>,
    },
    /// Shell/service status snapshot
    ShellStatus { status: ShellStatusSnapshot },
    /// Autostart status
    AutostartStatus { enabled: bool, supported: bool },
    /// Generic acknowledgment
    Ack,
    /// Error response
    Error { code: String, message: String },
}

/// Device information DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_id: DeviceId,
    pub device_name: String,
    pub is_online: bool,
}

/// Discovered LAN peer information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanPeerInfo {
    /// Remote device id.
    pub device_id: DeviceId,
    /// Remote display name.
    pub device_name: String,
    /// Device role/type advertised by the peer.
    pub device_type: String,
    /// Peer IP address string.
    pub ip: String,
    /// UDP port used by the LAN discovery control plane.
    pub discovery_port: u16,
    /// Direct LAN control endpoint as `ip:port`.
    pub p2p_control_addr: String,
    /// Supported media/session transports, for example `webrtc` or `quic`.
    pub transports: Vec<String>,
    /// Protocol version advertised by the peer.
    pub protocol_version: u32,
    /// Milliseconds since this peer was last observed.
    pub age_ms: u64,
    /// Whether this peer was discovered through the local P2P LAN path.
    pub p2p_available: bool,
}

/// LAN discovery state exposed over IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanDiscoverySnapshot {
    /// Whether LAN discovery is enabled in this service process.
    pub enabled: bool,
    /// Whether the UDP discovery task is currently running.
    pub running: bool,
    /// Local UDP discovery port.
    pub discovery_port: u16,
    /// Local discovery instance id.
    pub instance_id: String,
    /// Last successful announce/probe timestamp in milliseconds since Unix epoch.
    pub last_probe_ms: Option<u64>,
    /// Currently known LAN peers.
    pub peers: Vec<LanPeerInfo>,
}

/// Session information DTO (for list responses)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub role: String,  // "controller" or "agent"
    pub state: String, // "created", "listening", "connecting", "connected", "streaming", "failed", "closed"
    pub transport_kind: String,
    pub last_error: Option<String>,
    /// Whether the media sender is currently marked active.
    pub sender_active: bool,
    /// Whether the media receiver is currently marked active.
    pub receiver_active: bool,
}

/// Session runtime snapshot DTO (stable IPC contract)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeSnapshot {
    pub session_id: SessionId,
    pub role: String,           // "controller" or "agent"
    pub state: String, // "created", "listening", "connecting", "connected", "streaming", "failed", "closed"
    pub transport_kind: String, // "quic" or "webrtc"
    pub local_bootstrap: Option<SessionBootstrap>,
    pub remote_bootstrap: Option<SessionBootstrap>,
    pub last_error: Option<String>,
    /// Media pipeline state
    pub sender_active: bool,
    pub receiver_active: bool,
}

/// Session bootstrap metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBootstrap {
    pub listen_addr: Option<String>,
    pub server_name: Option<String>,
    pub cert_der: Option<String>, // Base64-encoded DER certificate
}

/// Aggregated runtime snapshot DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub sessions: Vec<SessionRuntimeSnapshot>,
    pub device_id: Option<DeviceId>,
    pub is_registered: bool,
}

/// Probe snapshot DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeSnapshot {
    pub session_id: SessionId,
    pub frames_received: u64,
    pub frames_decoded: u64,
    pub frames_dropped: u64,
    pub current_fps: Option<f32>,
    pub bitrate_mbps: Option<f32>,
    pub media_probe_valid: bool,
    pub media_probe_format: Option<String>,
    pub media_probe_width: Option<u32>,
    pub media_probe_height: Option<u32>,
    pub last_media_sequence: Option<u64>,
    pub last_media_timestamp_us: Option<u64>,
    pub last_media_payload_hash: Option<String>,
    pub last_error: Option<String>,
}

/// Service status DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub running: bool,
    pub healthy: bool,
    pub pid: Option<u32>,
}
