#![allow(dead_code)]

// mrd-service application state
//
// This module defines the shared state owned by mrd-service.
// After the hard-cut migration, this becomes the single source
// of truth for all session orchestration, transport runtime,
// and media control.

use crate::control_input::ControlInputRegistry;
use mrd_ipc::CapabilitySnapshot;
#[cfg(test)]
use mrd_ipc::{MediaProfile, MediaStageMetrics};
#[cfg(test)]
use mrd_proto::DeviceId;
#[cfg(test)]
use mrd_proto::SessionId;
#[cfg(any(test, target_os = "macos"))]
use mrd_render::RenderFrame;
use std::sync::Arc;
use tokio::sync::Mutex;

mod audit_log_registry;
mod capability_snapshot_registry;
mod capture_source_registry;
mod device_identity_registry;
mod device_registry;
mod display_mode_registry;
mod lan_identity;
mod media_pipeline_registry;
mod media_profile_registry;
#[cfg(any(windows, target_os = "macos"))]
mod media_render_queue_registry;
#[cfg(any(windows, target_os = "macos"))]
mod media_surface_renderer_registry;
mod media_task_registry;
mod peer_media_capability_registry;
#[cfg(any(windows, target_os = "macos"))]
mod platform_surface_renderer;
mod probe_registry;
mod session_registry;
mod shell_state;
pub use audit_log_registry::AuditLogRegistry;
pub use capability_snapshot_registry::CapabilitySnapshotRegistry;
pub use capture_source_registry::CaptureSourceRegistry;
pub use device_identity_registry::DeviceIdentityRegistry;
pub use device_registry::DeviceRegistry;
pub use display_mode_registry::DisplayModeRegistry;
pub use lan_identity::default_lan_device_identity;
#[cfg(test)]
pub(crate) use lan_identity::lan_device_identity_from;
pub use media_pipeline_registry::MediaPipelineRegistry;
pub use media_profile_registry::MediaProfileRegistry;
#[cfg(any(windows, target_os = "macos"))]
pub use media_render_queue_registry::{
    MediaRenderFrame, MediaRenderQueueEnqueue, MediaRenderQueueRegistry,
};
#[cfg(any(windows, target_os = "macos"))]
pub use media_surface_renderer_registry::MediaSurfaceRendererRegistry;
pub use media_task_registry::MediaTaskRegistry;
pub use peer_media_capability_registry::SessionPeerMediaCapabilityRegistry;
#[cfg(any(windows, target_os = "macos"))]
pub(crate) use platform_surface_renderer::{
    create_platform_surface_renderer, surface_backend_matches_platform,
};
pub use probe_registry::{DecodedVideoFrameStats, MediaProbeFrameStats, ProbeRegistry};
pub use session_registry::SessionRegistry;
pub use shell_state::{ShellState, TrayPortRef};

const AUDIT_EVENT_LIMIT: usize = 1_000;

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
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
    /// Service-owned security and operations audit events.
    pub audit_log: Arc<Mutex<AuditLogRegistry>>,
    /// Service-owned device pairing and identity state.
    pub device_identities: Arc<Mutex<DeviceIdentityRegistry>>,
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
    /// Temporary display mode state keyed by session.
    pub display_modes: Arc<Mutex<DisplayModeRegistry>>,
    /// Peer media capabilities keyed by session.
    pub peer_media_capabilities: Arc<Mutex<SessionPeerMediaCapabilityRegistry>>,
    /// Cached local capability facts refreshed outside request handling.
    pub capability_snapshot: Arc<Mutex<CapabilitySnapshotRegistry>>,
    /// Service-owned keyboard and mouse injection state.
    pub control_input: Arc<Mutex<ControlInputRegistry>>,
    /// Receiver pipeline state keyed by session.
    pub media_pipelines: Arc<Mutex<MediaPipelineRegistry>>,
    /// Native renderer instances keyed by receiver session/surface.
    #[cfg(any(windows, target_os = "macos"))]
    pub media_surface_renderers: Arc<Mutex<MediaSurfaceRendererRegistry>>,
    /// Drop-oldest receiver render queues keyed by session.
    #[cfg(any(windows, target_os = "macos"))]
    pub media_render_queues: Arc<Mutex<MediaRenderQueueRegistry>>,
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
        Self::with_tray_and_lan_discovery_config(
            tray,
            crate::lan_discovery::LanDiscoveryConfig::default(),
        )
    }

    pub fn with_tray_and_lan_discovery_config(
        tray: TrayPortRef,
        lan_discovery_config: crate::lan_discovery::LanDiscoveryConfig,
    ) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionRegistry::default())),
            devices: Arc::new(Mutex::new(DeviceRegistry::default())),
            audit_log: Arc::new(Mutex::new(AuditLogRegistry::default())),
            device_identities: Arc::new(Mutex::new(DeviceIdentityRegistry::default())),
            shell: Arc::new(Mutex::new(ShellState::default())),
            tray,
            lan_discovery: Arc::new(crate::lan_discovery::LanDiscoveryState::new(
                lan_discovery_config,
            )),
            probes: Arc::new(Mutex::new(ProbeRegistry::default())),
            media_profiles: Arc::new(Mutex::new(MediaProfileRegistry::default())),
            capture_sources: Arc::new(Mutex::new(CaptureSourceRegistry::default())),
            display_modes: Arc::new(Mutex::new(DisplayModeRegistry::default())),
            peer_media_capabilities: Arc::new(Mutex::new(
                SessionPeerMediaCapabilityRegistry::default(),
            )),
            capability_snapshot: Arc::new(Mutex::new(CapabilitySnapshotRegistry::default())),
            control_input: Arc::new(Mutex::new(ControlInputRegistry::default())),
            media_pipelines: Arc::new(Mutex::new(MediaPipelineRegistry::default())),
            #[cfg(any(windows, target_os = "macos"))]
            media_surface_renderers: Arc::new(Mutex::new(MediaSurfaceRendererRegistry::default())),
            #[cfg(any(windows, target_os = "macos"))]
            media_render_queues: Arc::new(Mutex::new(MediaRenderQueueRegistry::default())),
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

    /// Get a clone of the service audit log registry.
    pub fn audit_log(&self) -> Arc<Mutex<AuditLogRegistry>> {
        self.audit_log.clone()
    }

    /// Get a clone of the device identity registry.
    pub fn device_identities(&self) -> Arc<Mutex<DeviceIdentityRegistry>> {
        self.device_identities.clone()
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

    /// Get a clone of the display mode registry.
    pub fn display_modes(&self) -> Arc<Mutex<DisplayModeRegistry>> {
        self.display_modes.clone()
    }

    /// Get a clone of the peer media capability registry.
    pub fn peer_media_capabilities(&self) -> Arc<Mutex<SessionPeerMediaCapabilityRegistry>> {
        self.peer_media_capabilities.clone()
    }

    /// Get a clone of the local capability snapshot registry.
    pub fn capability_snapshot(&self) -> Arc<Mutex<CapabilitySnapshotRegistry>> {
        self.capability_snapshot.clone()
    }

    /// Get a clone of the service-owned control input registry.
    pub fn control_input(&self) -> Arc<Mutex<ControlInputRegistry>> {
        self.control_input.clone()
    }

    /// Return the currently cached local capability snapshot without running runtime probes.
    pub async fn cached_capability_snapshot(&self) -> CapabilitySnapshot {
        let mut snapshot = self.capability_snapshot.lock().await.snapshot();
        let input_injector_available = self.control_input.lock().await.is_available();
        crate::capabilities::apply_control_input_capability_status(
            &mut snapshot,
            input_injector_available,
        );
        snapshot
    }

    /// Refresh the local capability snapshot on a blocking worker without delaying IPC handlers.
    pub fn refresh_capability_snapshot_in_background(self: &Arc<Self>) {
        let app_state = Arc::clone(self);
        tokio::spawn(async move {
            let should_refresh = {
                let mut registry = app_state.capability_snapshot.lock().await;
                registry.begin_refresh()
            };
            if !should_refresh {
                return;
            }

            let snapshot =
                tokio::task::spawn_blocking(crate::capabilities::local_capability_snapshot)
                    .await
                    .map_err(|error| {
                        tracing::warn!("capability snapshot refresh task failed: {}", error);
                        error
                    })
                    .ok();
            app_state
                .capability_snapshot
                .lock()
                .await
                .finish_refresh(snapshot);
        });
    }

    #[cfg(test)]
    pub async fn replace_capability_snapshot_for_test(&self, snapshot: CapabilitySnapshot) {
        self.capability_snapshot.lock().await.replace(snapshot);
    }

    #[cfg(test)]
    pub async fn replace_control_input_for_test<I>(&self, injector: I)
    where
        I: mrd_input::InputInjector + 'static,
    {
        *self.control_input.lock().await = ControlInputRegistry::with_injector(injector);
    }

    /// Get a clone of the receiver media pipeline registry.
    pub fn media_pipelines(&self) -> Arc<Mutex<MediaPipelineRegistry>> {
        self.media_pipelines.clone()
    }

    /// Get a clone of the native receiver renderer registry.
    #[cfg(any(windows, target_os = "macos"))]
    pub fn media_surface_renderers(&self) -> Arc<Mutex<MediaSurfaceRendererRegistry>> {
        self.media_surface_renderers.clone()
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn media_render_queues(&self) -> Arc<Mutex<MediaRenderQueueRegistry>> {
        self.media_render_queues.clone()
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
    use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};

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
            lifecycle_state: SessionLifecycleState::Created,
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
    fn device_registry_keeps_explicit_registration() {
        let mut registry = DeviceRegistry::default();
        registry.register(
            DeviceId("explicit-device".to_string()),
            "Explicit Device".to_string(),
        );

        let registered = registry
            .register_if_unregistered(
                DeviceId("fallback-device".to_string()),
                "Fallback Device".to_string(),
            )
            .expect("registered device");

        assert_eq!(registered.0, DeviceId("explicit-device".to_string()));
        assert_eq!(registered.1, "Explicit Device");
    }

    #[test]
    fn default_lan_identity_uses_configured_id_and_name() {
        let (device_id, device_name) = lan_device_identity_from(
            Some(" lan-MOCK7EBPZ3RC ".to_string()),
            Some(" Target PC ".to_string()),
            Some("ignored-host".to_string()),
        );

        assert_eq!(device_id, DeviceId("lan-MOCK7EBPZ3RC".to_string()));
        assert_eq!(device_name, "Target PC");
    }

    #[test]
    fn default_lan_identity_falls_back_to_hostname() {
        let (device_id, device_name) =
            lan_device_identity_from(None, None, Some("DESKTOP-ABC/123".to_string()));

        assert_eq!(device_id, DeviceId("lan-DESKTOPABC123".to_string()));
        assert_eq!(device_name, "DESKTOP-ABC/123");
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
    fn probe_registry_exposes_latest_decoded_video_metadata_without_image() {
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
                format: "h264_desktop_frame".to_string(),
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
        assert!(snapshot.latest_frame_data_url.is_none());
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
                format: "hevc_desktop_frame".to_string(),
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
            snapshot.media_probe_format.as_deref(),
            Some("hevc_desktop_frame")
        );
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
    fn probe_registry_breaks_down_drop_causes() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("drop-breakdown-session".to_string());

        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 10,
                timestamp_us: 100_000,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 2048,
                format: "hevc_desktop_frame".to_string(),
                pixel_format: "d3d11_shared_nv12".to_string(),
                payload_hash: "fnv1a64:first".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            1_000,
        );
        registry.record_decoded_video_frame(
            &session_id,
            DecodedVideoFrameStats {
                bytes_received: 2048,
                sequence: 13,
                timestamp_us: 120_000,
                width: 1920,
                height: 1080,
                target_fps: 144,
                target_bitrate_mbps: 64,
                encoded_bytes: 2048,
                format: "hevc_desktop_frame".to_string(),
                pixel_format: "d3d11_shared_nv12".to_string(),
                payload_hash: "fnv1a64:gap".to_string(),
                preview_width: None,
                preview_height: None,
                rgb24: None,
            },
            1_020,
        );
        registry.record_probe_drop(&session_id, 512, 1_030, "decode failed");
        registry.record_transient_frame_drop(&session_id, 256, 1_040);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.frames_dropped, 4);
        assert_eq!(snapshot.sequence_gap_drops, 2);
        assert_eq!(snapshot.decode_error_drops, 1);
        assert_eq!(snapshot.transient_drops, 1);
    }

    #[test]
    fn media_pipeline_registry_exposes_stage_metrics() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("metrics-session".to_string());

        registry.record_stage_duration_ms(session_id.clone(), "sender.capture", 1.0);
        registry.record_stage_duration_ms(session_id.clone(), "sender.capture", 3.0);
        registry.set_stage_metrics(
            session_id.clone(),
            [MediaStageMetrics {
                stage: "sender.encode".to_string(),
                p50_ms: Some(2.5),
                p95_ms: Some(4.5),
            }],
        );

        let snapshot = registry.snapshot(&session_id);

        assert!(snapshot.stage_metrics.iter().any(|metric| {
            metric.stage == "sender.capture"
                && metric.p50_ms == Some(3.0)
                && metric.p95_ms == Some(3.0)
        }));
        assert!(snapshot.stage_metrics.iter().any(|metric| {
            metric.stage == "sender.encode"
                && metric.p50_ms == Some(2.5)
                && metric.p95_ms == Some(4.5)
        }));
    }

    #[test]
    fn media_pipeline_registry_separates_render_drop_counters() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-drops-session".to_string());

        registry.increment_render_queue_replacements(session_id.clone(), 3);
        registry.increment_render_stale_frame_drops(session_id.clone(), 5);
        registry.increment_render_lock_drops(session_id.clone(), 2);
        registry.increment_render_present_skips(session_id.clone(), 4);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.dropped_frames, 14);
        assert_eq!(snapshot.render_queue_replacements, 3);
        assert_eq!(snapshot.render_stale_frame_drops, 5);
        assert_eq!(snapshot.render_lock_drops, 2);
        assert_eq!(snapshot.render_present_skips, 4);
    }

    #[test]
    fn media_pipeline_registry_can_record_queue_replacement_without_double_counting_drop() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-replacement-only-session".to_string());

        registry.record_render_queue_replacements(session_id.clone(), 3);
        registry.increment_render_stale_frame_drops(session_id.clone(), 3);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.dropped_frames, 3);
        assert_eq!(snapshot.render_queue_replacements, 3);
        assert_eq!(snapshot.render_stale_frame_drops, 3);
    }

    #[test]
    fn media_pipeline_registry_exposes_render_queue_policy() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-policy-session".to_string());

        registry.set_render_queue_policy(session_id.clone(), Some("latest"));

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.render_queue_policy.as_deref(), Some("latest"));
    }

    #[cfg(windows)]
    #[test]
    fn media_pipeline_registry_exposes_renderer_swapchain_pacing_metadata() {
        use mrd_render::RendererSnapshot;

        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("render-swapchain-session".to_string());

        registry.record_renderer_snapshot(
            session_id.clone(),
            &RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: 4,
                presented_frame_count: 4,
                present_skipped_count: 0,
                render_queue_replacements: None,
                last_present_status: Some("presented".to_string()),
                low_latency_frame_latency_target: Some(1),
                swap_chain_max_frame_latency: Some(1),
                swap_chain_allow_tearing: Some(true),
                swap_chain_waitable_object: Some(true),
                swap_chain_present_mode: Some("waitable".to_string()),
                display_refresh_hz: Some(144),
                render_thread_priority: Some("highest".to_string()),
                waitable_wait_count: Some(2),
                waitable_wait_total_ms: Some(1.25),
                waitable_timeout_count: Some(1),
                last_waitable_wait_ms: Some(0.75),
                last_render_prepare_wait_ms: Some(0.05),
                last_render_shared_resource_ms: Some(0.02),
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: Some(0.7),
                last_width: 2560,
                last_height: 1440,
                last_pixel_format: None,
            },
        );
        registry.increment_render_waitable_timeouts(session_id.clone(), 1);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.swap_chain_max_frame_latency, Some(1));
        assert_eq!(snapshot.swap_chain_allow_tearing, Some(true));
        assert_eq!(snapshot.swap_chain_waitable_object, Some(true));
        assert_eq!(
            snapshot.swap_chain_present_mode.as_deref(),
            Some("waitable")
        );
        assert_eq!(snapshot.display_refresh_hz, Some(144));
        assert_eq!(snapshot.render_thread_priority.as_deref(), Some("highest"));
        assert_eq!(snapshot.render_waitable_timeouts, 1);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_surface_renderer_registry_returns_shared_session_renderers() {
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingRenderer {
            uploads: Arc<AtomicUsize>,
        }

        impl RendererInstance for CountingRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                self.uploads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: self.uploads.load(Ordering::SeqCst) as u64,
                    presented_frame_count: self.uploads.load(Ordering::SeqCst) as u64,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: Some("presented".to_string()),
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 1,
                    last_height: 1,
                    last_pixel_format: None,
                }
            }
        }

        let mut registry = MediaSurfaceRendererRegistry::default();
        let session_a = SessionId("surface-session-a".to_string());
        let session_b = SessionId("surface-session-b".to_string());
        let uploads_a = Arc::new(AtomicUsize::new(0));
        let uploads_b = Arc::new(AtomicUsize::new(0));

        registry.insert_renderer_for_test(
            &session_a,
            "surface-a",
            Box::new(CountingRenderer {
                uploads: uploads_a.clone(),
            }),
        );
        registry.insert_renderer_for_test(
            &session_b,
            "surface-b",
            Box::new(CountingRenderer {
                uploads: uploads_b.clone(),
            }),
        );

        let session_a_renderers = registry.renderers_for_session(&session_a);
        assert_eq!(session_a_renderers.len(), 1);
        drop(registry);

        let frame = RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]);
        session_a_renderers[0]
            .lock()
            .expect("renderer lock")
            .upload_frame(frame)
            .expect("upload frame");

        assert_eq!(uploads_a.load(Ordering::SeqCst), 1);
        assert_eq!(uploads_b.load(Ordering::SeqCst), 0);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_surface_renderer_registry_reuses_existing_surface_on_duplicate_attach() {
        use mrd_ipc::AttachedRenderSurface;
        use mrd_render::{RenderError, RenderTarget, RendererInstance, RendererSnapshot};

        struct NoopRenderer;

        impl RendererInstance for NoopRenderer {
            fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
                Ok(())
            }

            fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
                Ok(())
            }

            fn snapshot(&self) -> RendererSnapshot {
                RendererSnapshot {
                    attached_to_target: true,
                    uploaded_frame_count: 0,
                    presented_frame_count: 0,
                    present_skipped_count: 0,
                    render_queue_replacements: None,
                    last_present_status: None,
                    low_latency_frame_latency_target: None,
                    swap_chain_max_frame_latency: None,
                    swap_chain_allow_tearing: None,
                    swap_chain_waitable_object: None,
                    swap_chain_present_mode: None,
                    display_refresh_hz: None,
                    render_thread_priority: None,
                    waitable_wait_count: None,
                    waitable_wait_total_ms: None,
                    waitable_timeout_count: None,
                    last_waitable_wait_ms: None,
                    last_render_prepare_wait_ms: None,
                    last_render_shared_resource_ms: None,
                    last_render_wait_for_drawable_ms: None,
                    last_render_encode_commit_ms: None,
                    last_render_draw_present_ms: None,
                    last_width: 1,
                    last_height: 1,
                    last_pixel_format: None,
                }
            }
        }

        let mut registry = MediaSurfaceRendererRegistry::default();
        let session_id = SessionId("surface-session".to_string());
        registry.insert_renderer_for_test(&session_id, "surface-1", Box::new(NoopRenderer));

        let result = registry.attach_surface(
            &session_id,
            &AttachedRenderSurface {
                surface_id: "surface-1".to_string(),
                backend: platform_surface_backend_for_test().to_string(),
                window_handle: Some(0),
                render_proxy_endpoint: None,
            },
        );

        assert!(result.is_ok());
        assert_eq!(registry.session_surface_count(&session_id), 1);
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn platform_surface_backend_for_test() -> &'static str {
        #[cfg(windows)]
        {
            "d3d11"
        }
        #[cfg(target_os = "macos")]
        {
            "macos"
        }
    }

    #[test]
    fn media_pipeline_registry_exposes_active_media_profile_sampling() {
        let mut registry = MediaPipelineRegistry::default();
        let session_id = SessionId("active-profile-session".to_string());
        let profile = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
            color_mode: Some("monochrome".to_string()),
            color_pipeline: Some("sdr8".to_string()),
            ..MediaProfile::default()
        };

        registry.set_active_media_profile(session_id.clone(), &profile);

        let snapshot = registry.snapshot(&session_id);

        assert_eq!(snapshot.active_codec.as_deref(), Some("hevc"));
        assert_eq!(snapshot.active_codec_profile.as_deref(), Some("main"));
        assert_eq!(snapshot.active_bit_depth, Some(8));
        assert_eq!(snapshot.active_chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(snapshot.active_pixel_format.as_deref(), Some("nv12"));
        assert_eq!(snapshot.active_hdr_enabled, Some(false));
        assert_eq!(snapshot.active_color_mode.as_deref(), Some("monochrome"));
        assert_eq!(snapshot.active_color_pipeline.as_deref(), Some("sdr8"));
        assert_eq!(snapshot.active_width, Some(2560));
        assert_eq!(snapshot.active_height, Some(1440));
        assert_eq!(snapshot.active_fps, Some(144));
        assert_eq!(snapshot.active_bitrate_mbps, Some(80));

        registry.record_active_media_sample(
            session_id.clone(),
            &profile,
            2560,
            1440,
            "d3d11_shared_nv12",
        );
        let snapshot = registry.snapshot(&session_id);
        assert_eq!(
            snapshot.active_pixel_format.as_deref(),
            Some("d3d11_shared_nv12")
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_keeps_latest_frame_while_worker_is_running() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));

        match registry.enqueue_latest(session_id.clone(), first.clone()) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        assert_eq!(
            registry.enqueue_latest(session_id.clone(), second),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 1
            }
        );
        assert_eq!(
            registry.enqueue_latest(session_id.clone(), third.clone()),
            MediaRenderQueueEnqueue::Queued {
                replaced: true,
                depth: 1
            }
        );

        assert_eq!(registry.take_next_or_finish(&session_id), Some(third));
        assert_eq!(registry.take_next_or_finish(&session_id), None);
        match registry.enqueue_latest(session_id.clone(), first.clone()) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker restart, got {other:?}"),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_can_hold_a_small_paced_backlog() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-paced-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
        let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));
        let fifth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![13, 14, 15]));

        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), second.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 1
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 1);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), third.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 2
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 2);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 3
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 3);
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), fifth.clone(), 3),
            MediaRenderQueueEnqueue::Queued {
                replaced: true,
                depth: 3
            }
        );
        assert_eq!(registry.pending_depth(&session_id), 3);

        assert_eq!(registry.take_next_or_finish(&session_id), Some(third));
        assert_eq!(registry.pending_depth(&session_id), 2);
        assert_eq!(registry.take_next_or_finish(&session_id), Some(fourth));
        assert_eq!(registry.pending_depth(&session_id), 1);
        assert_eq!(registry.take_next_or_finish(&session_id), Some(fifth));
        assert_eq!(registry.pending_depth(&session_id), 0);
        assert_eq!(registry.take_next_or_finish(&session_id), None);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_can_take_latest_and_drop_stale_backlog() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-latest-session".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));
        let fourth = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![10, 11, 12]));

        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        registry.enqueue_bounded(session_id.clone(), second, 3);
        registry.enqueue_bounded(session_id.clone(), third, 3);
        registry.enqueue_bounded(session_id.clone(), fourth.clone(), 3);

        let (latest, dropped) = registry.take_latest_or_finish(&session_id);

        assert_eq!(latest, Some(fourth));
        assert_eq!(dropped, 2);
        assert_eq!(registry.pending_depth(&session_id), 0);
        assert_eq!(registry.take_latest_or_finish(&session_id), (None, 0));
        match registry.enqueue_bounded(session_id.clone(), first.clone(), 3) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker restart, got {other:?}"),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_paces_early_frames_to_target_fps() {
        use std::time::Duration;
        use tokio::time::Instant;

        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-pacing-session".to_string());
        let now = Instant::now();

        assert_eq!(registry.pacing_delay(&session_id, 165, now), Duration::ZERO);

        registry.record_presented(&session_id, now);
        let early = now + Duration::from_millis(2);
        let early_delay = registry.pacing_delay(&session_id, 165, early);
        assert!(
            early_delay >= Duration::from_millis(3),
            "expected pacing delay for early frame, got {early_delay:?}"
        );
        assert!(
            early_delay <= Duration::from_millis(5),
            "expected bounded pacing delay for early frame, got {early_delay:?}"
        );

        let late = now + Duration::from_millis(10);
        assert_eq!(
            registry.pacing_delay(&session_id, 165, late),
            Duration::ZERO
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn media_render_queue_records_enqueue_and_present_gaps() {
        use std::time::Duration;
        use tokio::time::Instant;

        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-gap-session".to_string());
        let now = Instant::now();

        assert_eq!(registry.record_enqueued(&session_id, now), None);
        assert_eq!(
            registry.record_enqueued(&session_id, now + Duration::from_millis(7)),
            Some(Duration::from_millis(7))
        );

        assert_eq!(
            registry.record_presented(&session_id, now + Duration::from_millis(1)),
            None
        );
        assert_eq!(
            registry.record_presented(&session_id, now + Duration::from_millis(9)),
            Some(Duration::from_millis(8))
        );
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
            ..mrd_ipc::MediaProfile::default()
        };
        let negotiation = mrd_ipc::MediaProfileNegotiation {
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
            preview_data_url: Some("legacy-preview-token".to_string()),
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

    #[test]
    fn display_mode_registry_tracks_temporary_mode_for_restore() {
        let mut registry = DisplayModeRegistry::default();
        let session_id = SessionId("display-mode-session".to_string());
        let original = mrd_ipc::DisplayMode {
            id: "windows:display:0:2560x1600@60".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 2560,
            height: 1600,
            refresh_hz: 60,
            bit_depth: Some(32),
            is_current: true,
        };
        let requested = mrd_ipc::DisplayMode {
            id: "windows:display:0:1920x1080@60".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            bit_depth: Some(32),
            is_current: false,
        };

        let change = registry.record_change(
            session_id.clone(),
            requested.clone(),
            Some(original.clone()),
            requested.clone(),
            true,
        );

        assert_eq!(change.status, "changed");
        assert!(change.restore_required);
        assert_eq!(registry.restore_mode(&session_id), Some(original.clone()));

        let restored = registry.record_restore(session_id.clone(), requested, original.clone());
        assert_eq!(restored.status, "restored");
        assert!(!restored.restore_required);
        assert_eq!(restored.active, Some(original));
        assert!(registry.restore_mode(&session_id).is_none());
    }
}
