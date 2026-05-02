// mrd-service application state
//
// This module defines the shared state owned by mrd-service.
// After the hard-cut migration, this becomes the single source
// of truth for all session orchestration, transport runtime,
// and media control.

use mrd_application::ports::SessionSnapshot;
use mrd_proto::{DeviceId, SessionId};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Session registry tracking all active sessions
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: std::collections::HashMap<SessionId, SessionSnapshot>,
}

impl SessionRegistry {
    pub fn insert(&mut self, session_id: SessionId, snapshot: SessionSnapshot) {
        self.sessions.insert(session_id, snapshot);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<&SessionSnapshot> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &SessionId) -> Option<&mut SessionSnapshot> {
        self.sessions.get_mut(session_id)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<SessionSnapshot> {
        self.sessions.remove(session_id)
    }

    pub fn list_all(&self) -> Vec<SessionSnapshot> {
        self.sessions.values().cloned().collect()
    }
}

/// Probe telemetry accumulated from LAN data-plane probe frames.
#[derive(Debug, Default)]
pub struct ProbeRegistry {
    probes: std::collections::HashMap<SessionId, SessionProbeStats>,
}

#[derive(Debug, Clone, Default)]
struct SessionProbeStats {
    frames_received: u64,
    frames_decoded: u64,
    frames_dropped: u64,
    bytes_received: u64,
    first_seen_ms: Option<u64>,
    last_seen_ms: Option<u64>,
    media_probe_valid: bool,
    media_probe_format: Option<String>,
    media_probe_width: Option<u32>,
    media_probe_height: Option<u32>,
    media_probe_target_fps: Option<u32>,
    media_probe_target_bitrate_mbps: Option<u32>,
    media_probe_payload_bytes: Option<u32>,
    last_media_sequence: Option<u64>,
    last_media_timestamp_us: Option<u64>,
    last_media_payload_hash: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaProbeFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub payload_bytes: u32,
    pub format: String,
    pub payload_hash: String,
}

impl ProbeRegistry {
    pub fn record_probe_frame(&mut self, session_id: &SessionId, bytes_received: u64, now_ms: u64) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = None;
    }

    pub fn record_media_probe_frame(
        &mut self,
        session_id: &SessionId,
        frame: MediaProbeFrameStats,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        if let Some(last_sequence) = stats.last_media_sequence {
            if frame.sequence > last_sequence.saturating_add(1) {
                stats.frames_dropped = stats
                    .frames_dropped
                    .saturating_add(frame.sequence.saturating_sub(last_sequence + 1));
            }
        }

        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(frame.bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.media_probe_valid = true;
        stats.media_probe_format = Some(frame.format);
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.payload_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        stats.last_error = None;
    }

    pub fn record_probe_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
        error: impl Into<String>,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = Some(error.into());
    }

    pub fn snapshot(&self, session_id: &SessionId) -> mrd_ipc::ProbeSnapshot {
        let Some(stats) = self.probes.get(session_id) else {
            return mrd_ipc::ProbeSnapshot {
                session_id: session_id.clone(),
                frames_received: 0,
                frames_decoded: 0,
                frames_dropped: 0,
                current_fps: None,
                bitrate_mbps: None,
                media_probe_valid: false,
                media_probe_format: None,
                media_probe_width: None,
                media_probe_height: None,
                media_probe_target_fps: None,
                media_probe_target_bitrate_mbps: None,
                media_probe_payload_bytes: None,
                last_media_sequence: None,
                last_media_timestamp_us: None,
                last_media_payload_hash: None,
                last_error: None,
            };
        };

        let elapsed_ms = match (stats.first_seen_ms, stats.last_seen_ms) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        };
        let current_fps = if elapsed_ms > 0 {
            Some((stats.frames_decoded as f32 * 1000.0) / elapsed_ms as f32)
        } else {
            Some(0.0)
        };
        let bitrate_mbps = if elapsed_ms > 0 {
            Some((stats.bytes_received as f32 * 8.0) / elapsed_ms as f32 / 1000.0)
        } else {
            Some(0.0)
        };

        mrd_ipc::ProbeSnapshot {
            session_id: session_id.clone(),
            frames_received: stats.frames_received,
            frames_decoded: stats.frames_decoded,
            frames_dropped: stats.frames_dropped,
            current_fps,
            bitrate_mbps,
            media_probe_valid: stats.media_probe_valid,
            media_probe_format: stats.media_probe_format.clone(),
            media_probe_width: stats.media_probe_width,
            media_probe_height: stats.media_probe_height,
            media_probe_target_fps: stats.media_probe_target_fps,
            media_probe_target_bitrate_mbps: stats.media_probe_target_bitrate_mbps,
            media_probe_payload_bytes: stats.media_probe_payload_bytes,
            last_media_sequence: stats.last_media_sequence,
            last_media_timestamp_us: stats.last_media_timestamp_us,
            last_media_payload_hash: stats.last_media_payload_hash.clone(),
            last_error: stats.last_error.clone(),
        }
    }
}

/// Shell state - tracks UI presence and service lifecycle
#[derive(Debug, Default)]
pub struct ShellState {
    /// UI process PID if attached
    pub ui_pid: Option<u32>,
    /// UI executable path for relaunch
    pub ui_executable_path: Option<String>,
    /// Tray availability (platform-dependent)
    pub tray_available: bool,
    /// Autostart enabled state (None if not supported)
    pub autostart_enabled: Option<bool>,
    /// Active session count (for tray display)
    pub active_session_count: usize,
    /// Last error message
    pub last_error: Option<String>,
}

/// Tray port - abstracts platform-specific tray implementation
pub type TrayPortRef = Arc<std::sync::Mutex<dyn crate::shell::TrayPort + Send + Sync>>;

/// Device registry
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    local_device: Option<(DeviceId, String)>, // (id, name)
}

impl DeviceRegistry {
    pub fn register(&mut self, device_id: DeviceId, device_name: String) {
        self.local_device = Some((device_id, device_name));
    }

    pub fn get_local_device(&self) -> Option<&(DeviceId, String)> {
        self.local_device.as_ref()
    }

    pub fn is_registered(&self) -> bool {
        self.local_device.is_some()
    }
}

/// Application state for mrd-service
///
/// This is the shared state that will be injected into IPC handlers.
/// After migration, it will own:
/// - RealtimeRuntime / signaling client
/// - WebrtcHost / WebrtcSessionCoordinator
/// - QuicHost / QuicSessionCoordinator
/// - Media senders/receivers
/// - Probe/telemetry state
/// - Shell/UI lifecycle state
/// - Tray port (Phase 4)
pub struct AppState {
    /// Session registry - single source of truth for all sessions
    pub sessions: Arc<Mutex<SessionRegistry>>,
    /// Device registry
    pub devices: Arc<Mutex<DeviceRegistry>>,
    /// Shell state - UI presence and service lifecycle
    pub shell: Arc<Mutex<ShellState>>,
    /// Tray port (Phase 4)
    pub tray: TrayPortRef,
    /// Peer-to-peer LAN discovery state.
    pub lan_discovery: Arc<crate::lan_discovery::LanDiscoveryState>,
    /// LAN probe telemetry keyed by session.
    pub probes: Arc<Mutex<ProbeRegistry>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_tray(Arc::new(std::sync::Mutex::new(
            crate::shell::NoOpTray::new(),
        )))
    }

    pub fn with_tray(tray: TrayPortRef) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionRegistry::default())),
            devices: Arc::new(Mutex::new(DeviceRegistry::default())),
            shell: Arc::new(Mutex::new(ShellState::default())),
            tray,
            lan_discovery: Arc::new(crate::lan_discovery::LanDiscoveryState::default()),
            probes: Arc::new(Mutex::new(ProbeRegistry::default())),
        }
    }

    /// Get a clone of the sessions Arc for injection into handlers
    pub fn sessions(&self) -> Arc<Mutex<SessionRegistry>> {
        self.sessions.clone()
    }

    /// Get a clone of the devices Arc for injection into handlers
    pub fn devices(&self) -> Arc<Mutex<DeviceRegistry>> {
        self.devices.clone()
    }

    /// Get a clone of the shell Arc for injection into handlers
    pub fn shell(&self) -> Arc<Mutex<ShellState>> {
        self.shell.clone()
    }

    /// Get a clone of the tray Arc for injection into handlers
    pub fn tray(&self) -> TrayPortRef {
        self.tray.clone()
    }

    /// Get a clone of the LAN discovery state.
    pub fn lan_discovery(&self) -> Arc<crate::lan_discovery::LanDiscoveryState> {
        self.lan_discovery.clone()
    }

    /// Get a clone of the probe telemetry registry.
    pub fn probes(&self) -> Arc<Mutex<ProbeRegistry>> {
        self.probes.clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_registry_tracks_sessions() {
        let mut registry = SessionRegistry::default();

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: "created".to_string(),
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        registry.insert(session_id.clone(), snapshot);

        let retrieved = registry.get(&session_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().transport, "quic");
    }

    #[test]
    fn device_registry_tracks_local_device() {
        let mut registry = DeviceRegistry::default();

        let device_id = DeviceId("test-device".to_string());
        registry.register(device_id.clone(), "Test Device".to_string());

        assert!(registry.is_registered());

        let retrieved = registry.get_local_device();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().0, device_id);
    }

    #[test]
    fn probe_registry_tracks_received_probe_frames() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("probe-session".to_string());

        registry.record_probe_frame(&session_id, 1200, 1_000);
        registry.record_probe_frame(&session_id, 1200, 1_250);

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 2);
        assert_eq!(snapshot.frames_decoded, 2);
        assert!(snapshot.current_fps.unwrap_or_default() > 0.0);
        assert!(snapshot.bitrate_mbps.unwrap_or_default() > 0.0);
    }

    #[test]
    fn probe_registry_exposes_valid_media_probe_metadata() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("media-probe-session".to_string());

        registry.record_media_probe_frame(
            &session_id,
            MediaProbeFrameStats {
                bytes_received: 2400,
                sequence: 7,
                timestamp_us: 123_456,
                width: 32,
                height: 18,
                target_fps: 144,
                target_bitrate_mbps: 64,
                payload_bytes: 2400,
                format: "rgba8_test_pattern".to_string(),
                payload_hash: "fnv1a64:abc123".to_string(),
            },
            2_000,
        );

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert!(snapshot.media_probe_valid);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("rgba8_test_pattern")
        );
        assert_eq!(snapshot.media_probe_width, Some(32));
        assert_eq!(snapshot.media_probe_height, Some(18));
        assert_eq!(snapshot.media_probe_target_fps, Some(144));
        assert_eq!(snapshot.media_probe_target_bitrate_mbps, Some(64));
        assert_eq!(snapshot.media_probe_payload_bytes, Some(2400));
        assert_eq!(snapshot.last_media_sequence, Some(7));
        assert_eq!(snapshot.last_media_timestamp_us, Some(123_456));
        assert_eq!(
            snapshot.last_media_payload_hash.as_deref(),
            Some("fnv1a64:abc123")
        );
    }
}
