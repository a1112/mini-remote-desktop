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

/// Requested or selected media stream profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_mbps: u32,
    pub codec: String,
}

/// Result of media profile negotiation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaProfileNegotiation {
    pub requested: MediaProfile,
    pub selected: MediaProfile,
    pub status: String,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downgrade_reason: Option<String>,
}

/// A native render surface attached to a media pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachedRenderSurface {
    pub surface_id: String,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_handle: Option<i64>,
}

/// Aggregated latency metrics for one media pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaStageMetrics {
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
}

/// Runtime state for a session media pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaPipelineSnapshot {
    pub session_id: SessionId,
    pub attached_surfaces: Vec<AttachedRenderSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_decoder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_renderer: Option<String>,
    pub queue_depth: u32,
    pub dropped_frames: u64,
    pub stage_metrics: Vec<MediaStageMetrics>,
}

/// A capture source that can be selected for a remote session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSource {
    pub id: String,
    pub platform: String,
    pub source_kind: String,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
    pub process_id: u32,
    pub app_name: Option<String>,
    pub bundle_identifier: Option<String>,
    pub preview_data_url: Option<String>,
    pub preview_width: Option<u32>,
    pub preview_height: Option<u32>,
}

/// Result of selecting a capture source on the remote peer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSourceSelection {
    pub session_id: SessionId,
    pub source: CaptureSource,
    pub status: String,
    pub reason: Option<String>,
}

/// Platform identifier used by structured capability snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPlatform {
    /// Microsoft Windows desktop.
    Windows,
    /// Apple macOS desktop.
    Macos,
    /// Linux desktop.
    Linux,
    /// Android client/host.
    Android,
    /// iOS client.
    Ios,
    /// Browser/web runtime.
    Web,
    /// Unknown or unsupported platform.
    Unknown,
}

/// Product capability domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDomain {
    /// Screen/window capture.
    Capture,
    /// Selectable capture sources.
    CaptureSource,
    /// Video encoding.
    Encode,
    /// Video decoding.
    Decode,
    /// Frame rendering.
    Render,
    /// Frame memory/interoperability path.
    Memory,
    /// Media or control transport.
    Transport,
    /// Keyboard/mouse/control-plane input.
    Control,
    /// Audio capture/playback/media path.
    Audio,
    /// Local service lifecycle features.
    Service,
    /// Pairing, consent, and encryption features.
    Security,
}

/// Runtime support state for one capability.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Product code exists but runtime validation has not proven usability.
    Supported,
    /// Runtime probe found required APIs, drivers, or permissions.
    Available,
    /// Lightweight validation succeeded.
    Usable,
    /// Usable fallback path below preferred parity.
    Degraded,
    /// Blocked by an OS permission.
    PermissionMissing,
    /// Driver/runtime library is missing.
    DriverMissing,
    /// Required hardware is absent.
    HardwareMissing,
    /// Matrix concept exists but no runner is wired.
    Unimplemented,
    /// Unsupported on this platform or product mode.
    Unsupported,
    /// Not yet probed or not recognized.
    Unknown,
}

/// Structured capability item shared by service, UI, and LAN discovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityItem {
    /// Stable capability id, for example `capture.dxgi`.
    pub id: String,
    /// Product domain for grouping and evaluation.
    pub domain: CapabilityDomain,
    /// Human-readable short label.
    pub label: String,
    /// Current support state.
    pub status: CapabilityStatus,
    /// Platform that produced the capability.
    pub platform: CapabilityPlatform,
    /// Short reason when the status is not plainly available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Optional diagnostic detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Capability ids required by this item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Capability ids that conflict with this item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts_with: Vec<String>,
    /// Capability ids this item depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Lower-parity fallback capability ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_ids: Vec<String>,
    /// Last probe timestamp in milliseconds since Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe_time_ms: Option<u64>,
}

/// Compatibility status for a cross-capability constraint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityConstraintStatus {
    /// Combination is allowed.
    Allow,
    /// Combination must not run.
    Block,
    /// Combination runs below preferred parity.
    Degrade,
    /// Combination needs a copy/conversion step.
    RequiresCopy,
    /// Combination requires runtime probe validation.
    RequiresProbe,
}

/// Rule describing whether multiple capabilities can be combined.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityConstraint {
    /// Stable constraint id.
    pub id: String,
    /// Capability ids or prefixes this rule applies to.
    pub applies_to: Vec<String>,
    /// Constraint result.
    pub status: CapabilityConstraintStatus,
    /// Deterministic explanation for UI and automation.
    pub reason: String,
    /// Fallback capability ids when applicable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_ids: Vec<String>,
}

/// Named performance profile used by static and runtime validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityProfile {
    /// Stable profile id.
    pub id: String,
    /// Target frame width.
    pub width: u32,
    /// Target frame height.
    pub height: u32,
    /// Target frame rate.
    pub fps: u32,
    /// Target bitrate in Mbps.
    pub bitrate_mbps: u32,
    /// Requested codec, for example `h264`.
    pub codec: String,
    /// Optional latency budget in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_budget_ms: Option<u32>,
    /// Optional minimum stable FPS ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_stable_fps_ratio: Option<f32>,
    /// Optional maximum frame drop ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_drop_ratio: Option<f32>,
    /// Capabilities required for static support.
    pub required_capabilities: Vec<String>,
}

/// Structured local or peer capability snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilitySnapshot {
    /// Schema version for forward-compatible readers.
    pub schema_version: u32,
    /// Platform that produced the snapshot.
    pub platform: CapabilityPlatform,
    /// Service or application version that produced the snapshot.
    pub service_version: String,
    /// Capability items.
    pub capabilities: Vec<CapabilityItem>,
    /// Cross-capability constraints.
    pub constraints: Vec<CapabilityConstraint>,
    /// Built-in performance profiles known by the producer.
    pub profiles: Vec<CapabilityProfile>,
    /// Snapshot timestamp in milliseconds since Unix epoch.
    pub updated_at_ms: u64,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_profile: Option<MediaProfile>,
    },
    /// Request a runtime media profile switch for an existing session.
    UpdateMediaProfile {
        session_id: SessionId,
        requested_profile: MediaProfile,
    },
    /// List selectable capture sources from the remote peer for a session.
    ListRemoteCaptureSources {
        session_id: SessionId,
        include_previews: bool,
        limit: Option<u32>,
    },
    /// Select one remote capture source for a session.
    SelectRemoteCaptureSource {
        session_id: SessionId,
        source_id: String,
    },
    /// Attach a native render surface to a session media pipeline.
    AttachRenderSurface {
        session_id: SessionId,
        surface_id: String,
        backend: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_handle: Option<i64>,
    },
    /// Detach a native render surface from a session media pipeline.
    DetachRenderSurface {
        session_id: SessionId,
        surface_id: String,
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
    /// Get structured local capability snapshot.
    CapabilitySnapshot,
    /// Get probe snapshot data
    ProbeSnapshot { session_id: SessionId },
    /// Get media pipeline snapshot data.
    MediaPipelineSnapshot { session_id: SessionId },
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
    /// Media profile switch completed.
    MediaProfileUpdated {
        session_id: SessionId,
        negotiation: MediaProfileNegotiation,
    },
    /// Selectable capture sources returned by the remote peer.
    CaptureSourceList {
        session_id: SessionId,
        sources: Vec<CaptureSource>,
    },
    /// Capture source selection result returned by the remote peer.
    CaptureSourceSelected {
        session_id: SessionId,
        selection: CaptureSourceSelection,
    },
    /// Native render surface attached.
    RenderSurfaceAttached {
        session_id: SessionId,
        surface_id: String,
    },
    /// Native render surface detached.
    RenderSurfaceDetached {
        session_id: SessionId,
        surface_id: String,
    },
    /// Session runtime snapshot
    SessionSnapshot { snapshot: SessionRuntimeSnapshot },
    /// Aggregated runtime snapshot
    RuntimeSnapshot { snapshot: RuntimeSnapshot },
    /// Structured local capability snapshot.
    CapabilitySnapshot {
        /// Current local capability snapshot.
        snapshot: CapabilitySnapshot,
    },
    /// Probe snapshot data
    ProbeSnapshot { snapshot: ProbeSnapshot },
    /// Media pipeline snapshot data.
    MediaPipelineSnapshot { snapshot: MediaPipelineSnapshot },
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
    /// Service build identifier advertised by the peer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_build_id: Option<String>,
    /// LAN media protocol version advertised by the peer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_protocol_version: Option<u32>,
    /// Structured media capabilities advertised by the peer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_capabilities: Vec<String>,
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
    pub media_probe_target_fps: Option<u32>,
    pub media_probe_target_bitrate_mbps: Option<u32>,
    pub media_probe_payload_bytes: Option<u32>,
    pub last_media_sequence: Option<u64>,
    pub last_media_timestamp_us: Option<u64>,
    pub last_media_payload_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_frame_data_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_frame_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_frame_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_frame_pixel_format: Option<String>,
    pub last_error: Option<String>,
}

/// Service status DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub running: bool,
    pub healthy: bool,
    pub pid: Option<u32>,
}
