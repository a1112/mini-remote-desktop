// mrd-service application state
//
// This module defines the shared state owned by mrd-service.
// After the hard-cut migration, this becomes the single source
// of truth for all session orchestration, transport runtime,
// and media control.

use base64::{engine::general_purpose, Engine as _};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{
    AttachedRenderSurface, CaptureSourceSelection, MediaPipelineSnapshot, MediaProfileNegotiation,
};
use mrd_proto::{DeviceId, SessionId};
use std::sync::Arc;
use tokio::{sync::Mutex, task::AbortHandle};

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

/// Runtime media profile negotiation state keyed by session.
#[derive(Debug, Default)]
pub struct MediaProfileRegistry {
    profiles: std::collections::HashMap<SessionId, MediaProfileNegotiation>,
}

impl MediaProfileRegistry {
    pub fn set(&mut self, session_id: SessionId, negotiation: MediaProfileNegotiation) {
        self.profiles.insert(session_id, negotiation);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.remove(session_id)
    }
}

/// Runtime capture source selection state keyed by session.
#[derive(Debug, Default)]
pub struct CaptureSourceRegistry {
    selections: std::collections::HashMap<SessionId, CaptureSourceSelection>,
}

impl CaptureSourceRegistry {
    pub fn set(&mut self, session_id: SessionId, selection: CaptureSourceSelection) {
        self.selections.insert(session_id, selection);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.remove(session_id)
    }
}

/// Peer media capabilities observed for each active session.
#[derive(Debug, Default)]
pub struct SessionPeerMediaCapabilityRegistry {
    capabilities: std::collections::HashMap<SessionId, Vec<String>>,
}

impl SessionPeerMediaCapabilityRegistry {
    pub fn set(&mut self, session_id: SessionId, capabilities: Vec<String>) {
        self.capabilities.insert(session_id, capabilities);
    }

    pub fn supports(&self, session_id: &SessionId, capability: &str) -> bool {
        self.capabilities
            .get(session_id)
            .map(|capabilities| capabilities.iter().any(|value| value == capability))
            .unwrap_or(false)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.remove(session_id)
    }
}

/// Runtime receiver media pipeline state keyed by session.
#[derive(Debug, Default)]
pub struct MediaPipelineRegistry {
    pipelines: std::collections::HashMap<SessionId, MediaPipelineState>,
}

#[derive(Debug, Clone, Default)]
struct MediaPipelineState {
    attached_surfaces: std::collections::HashMap<String, AttachedRenderSurface>,
    active_decoder: Option<String>,
    active_renderer: Option<String>,
    queue_depth: u32,
    dropped_frames: u64,
}

impl MediaPipelineRegistry {
    pub fn attach_surface(&mut self, session_id: SessionId, surface: AttachedRenderSurface) {
        let state = self.pipelines.entry(session_id).or_default();
        if state.active_renderer.is_none() {
            state.active_renderer = Some(surface.backend.clone());
        }
        state
            .attached_surfaces
            .insert(surface.surface_id.clone(), surface);
    }

    pub fn detach_surface(&mut self, session_id: &SessionId, surface_id: &str) -> bool {
        let Some(state) = self.pipelines.get_mut(session_id) else {
            return false;
        };
        let removed = state.attached_surfaces.remove(surface_id).is_some();
        if state.attached_surfaces.is_empty() {
            state.active_renderer = None;
        }
        removed
    }

    pub fn set_active_decoder(&mut self, session_id: SessionId, decoder: impl Into<String>) {
        self.pipelines.entry(session_id).or_default().active_decoder = Some(decoder.into());
    }

    pub fn record_queue_depth(&mut self, session_id: SessionId, queue_depth: u32) {
        self.pipelines.entry(session_id).or_default().queue_depth = queue_depth;
    }

    pub fn increment_dropped_frames(&mut self, session_id: SessionId, count: u64) {
        let state = self.pipelines.entry(session_id).or_default();
        state.dropped_frames = state.dropped_frames.saturating_add(count);
    }

    pub fn snapshot(&self, session_id: &SessionId) -> MediaPipelineSnapshot {
        let state = self.pipelines.get(session_id);
        MediaPipelineSnapshot {
            session_id: session_id.clone(),
            attached_surfaces: state
                .map(|state| state.attached_surfaces.values().cloned().collect())
                .unwrap_or_default(),
            active_decoder: state.and_then(|state| state.active_decoder.clone()),
            active_renderer: state.and_then(|state| state.active_renderer.clone()),
            queue_depth: state.map_or(0, |state| state.queue_depth),
            dropped_frames: state.map_or(0, |state| state.dropped_frames),
            stage_metrics: Vec::new(),
        }
    }

    pub fn remove(&mut self, session_id: &SessionId) {
        self.pipelines.remove(session_id);
    }
}

/// Runtime media tasks keyed by session.
#[derive(Default)]
pub struct MediaTaskRegistry {
    tasks: std::collections::HashMap<SessionId, Vec<AbortHandle>>,
}

impl MediaTaskRegistry {
    pub fn register(&mut self, session_id: SessionId, abort_handle: AbortHandle) {
        self.tasks.entry(session_id).or_default().push(abort_handle);
    }

    pub fn abort_session(&mut self, session_id: &SessionId) -> usize {
        let handles = self.tasks.remove(session_id).unwrap_or_default();
        let count = handles.len();
        for handle in handles {
            handle.abort();
        }
        count
    }

    pub fn active_count(&self, session_id: &SessionId) -> usize {
        self.tasks.get(session_id).map_or(0, Vec::len)
    }
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
    latest_frame: Option<DecodedPreviewFrame>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct DecodedPreviewFrame {
    width: u32,
    height: u32,
    pixel_format: String,
    data_url: Option<String>,
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

#[derive(Debug, Clone)]
pub struct DecodedVideoFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub encoded_bytes: u32,
    pub pixel_format: String,
    pub payload_hash: String,
    pub preview_width: Option<u32>,
    pub preview_height: Option<u32>,
    pub rgb24: Option<Vec<u8>>,
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

    pub fn record_decoded_video_frame(
        &mut self,
        session_id: &SessionId,
        frame: DecodedVideoFrameStats,
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
        stats.media_probe_format = Some("h264_desktop_frame".to_string());
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.encoded_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        if let Some(rgb24) = frame.rgb24 {
            let preview_width = frame.preview_width.unwrap_or(frame.width);
            let preview_height = frame.preview_height.unwrap_or(frame.height);
            let data_url = encode_rgb24_png_data_url(preview_width, preview_height, &rgb24);
            stats.latest_frame = Some(DecodedPreviewFrame {
                width: preview_width,
                height: preview_height,
                pixel_format: frame.pixel_format,
                data_url,
            });
        }
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

    pub fn record_transient_frame_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
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
                latest_frame_data_url: None,
                latest_frame_width: None,
                latest_frame_height: None,
                latest_frame_pixel_format: None,
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
            latest_frame_data_url: stats
                .latest_frame
                .as_ref()
                .and_then(|frame| frame.data_url.clone()),
            latest_frame_width: stats.latest_frame.as_ref().map(|frame| frame.width),
            latest_frame_height: stats.latest_frame.as_ref().map(|frame| frame.height),
            latest_frame_pixel_format: stats
                .latest_frame
                .as_ref()
                .map(|frame| frame.pixel_format.clone()),
            last_error: stats.last_error.clone(),
        }
    }
}

fn encode_rgb24_png_data_url(width: u32, height: u32, rgb24: &[u8]) -> Option<String> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(3)?;
    if width == 0 || height == 0 || rgb24.len() != expected_len {
        return None;
    }

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(rgb24, width, height, ColorType::Rgb8.into())
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(png)
    ))
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
    /// Negotiated media profile keyed by session.
    pub media_profiles: Arc<Mutex<MediaProfileRegistry>>,
    /// Selected capture source keyed by session.
    pub capture_sources: Arc<Mutex<CaptureSourceRegistry>>,
    /// Peer media capabilities keyed by session.
    pub peer_media_capabilities: Arc<Mutex<SessionPeerMediaCapabilityRegistry>>,
    /// Receiver pipeline state keyed by session.
    pub media_pipelines: Arc<Mutex<MediaPipelineRegistry>>,
    /// Abort handles for active media tasks keyed by session.
    pub media_tasks: Arc<Mutex<MediaTaskRegistry>>,
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
            media_profiles: Arc::new(Mutex::new(MediaProfileRegistry::default())),
            capture_sources: Arc::new(Mutex::new(CaptureSourceRegistry::default())),
            peer_media_capabilities: Arc::new(Mutex::new(
                SessionPeerMediaCapabilityRegistry::default(),
            )),
            media_pipelines: Arc::new(Mutex::new(MediaPipelineRegistry::default())),
            media_tasks: Arc::new(Mutex::new(MediaTaskRegistry::default())),
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

    /// Get a clone of the media profile registry.
    pub fn media_profiles(&self) -> Arc<Mutex<MediaProfileRegistry>> {
        self.media_profiles.clone()
    }

    /// Get a clone of the capture source registry.
    pub fn capture_sources(&self) -> Arc<Mutex<CaptureSourceRegistry>> {
        self.capture_sources.clone()
    }

    /// Get a clone of the peer media capability registry.
    pub fn peer_media_capabilities(&self) -> Arc<Mutex<SessionPeerMediaCapabilityRegistry>> {
        self.peer_media_capabilities.clone()
    }

    /// Get a clone of the receiver media pipeline registry.
    pub fn media_pipelines(&self) -> Arc<Mutex<MediaPipelineRegistry>> {
        self.media_pipelines.clone()
    }

    /// Get a clone of the media task registry.
    pub fn media_tasks(&self) -> Arc<Mutex<MediaTaskRegistry>> {
        self.media_tasks.clone()
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

    #[test]
    fn probe_registry_exposes_latest_decoded_video_preview() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("decoded-video-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 4096,
                sequence: 11,
                timestamp_us: 987_654,
                width: 2,
                height: 2,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 1024,
                pixel_format: "rgb24".to_string(),
                payload_hash: "fnv1a64:preview".to_string(),
                preview_width: Some(2),
                preview_height: Some(2),
                rgb24: Some(vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
            },
            3_000,
        );

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some("h264_desktop_frame")
        );
        assert_eq!(snapshot.latest_frame_width, Some(2));
        assert_eq!(snapshot.latest_frame_height, Some(2));
        assert_eq!(snapshot.latest_frame_pixel_format.as_deref(), Some("rgb24"));
        assert!(snapshot
            .latest_frame_data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,")));
    }

    #[test]
    fn probe_registry_counts_decoded_video_without_preview_copy() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("decoded-video-metadata-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 12,
                timestamp_us: 1_111_111,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 20,
                encoded_bytes: 2048,
                pixel_format: "cpu_nv12".to_string(),
                payload_hash: "fnv1a64:encoded".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            4_000,
        );

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 1);
        assert_eq!(snapshot.media_probe_width, Some(1920));
        assert_eq!(snapshot.media_probe_height, Some(1080));
        assert_eq!(
            snapshot.last_media_payload_hash.as_deref(),
            Some("fnv1a64:encoded")
        );
        assert!(snapshot.latest_frame_data_url.is_none());
    }

    #[test]
    fn probe_registry_counts_transient_drop_without_latching_error() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("transient-drop-session".to_string());

        registry.record_transient_frame_drop(&session_id, 512, 1_000);

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 1);
        assert_eq!(snapshot.frames_decoded, 0);
        assert_eq!(snapshot.frames_dropped, 1);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn media_profile_registry_tracks_negotiated_profile() {
        let mut registry = MediaProfileRegistry::default();
        let session_id = SessionId("profile-session".to_string());
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 64,
            codec: "h264".to_string(),
        };
        let negotiation = MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: None,
            selected_height: None,
            downgrade_reason: None,
        };

        registry.set(session_id.clone(), negotiation.clone());

        assert_eq!(registry.get(&session_id), Some(negotiation));
        assert!(registry.remove(&session_id).is_some());
        assert!(registry.get(&session_id).is_none());
    }

    #[test]
    fn capture_source_registry_tracks_selected_source() {
        let mut registry = CaptureSourceRegistry::default();
        let session_id = SessionId("capture-source-session".to_string());
        let source = mrd_ipc::CaptureSource {
            id: "windows:window:0x1234".to_string(),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: "Target App".to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: Some("Target App".to_string()),
            bundle_identifier: None,
            preview_data_url: Some("data:image/png;base64,AAAA".to_string()),
            preview_width: Some(320),
            preview_height: Some(180),
        };
        let selection = mrd_ipc::CaptureSourceSelection {
            session_id: session_id.clone(),
            source: source.clone(),
            status: "selected".to_string(),
            reason: None,
        };

        registry.set(session_id.clone(), selection);

        assert_eq!(
            registry.get(&session_id).expect("selection").source.id,
            source.id
        );
        assert!(registry.remove(&session_id).is_some());
        assert!(registry.get(&session_id).is_none());
    }
}
