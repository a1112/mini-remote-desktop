use super::{
    AuditLogRegistry, CapabilitySnapshotRegistry, CaptureSourceRegistry, DeviceIdentityRegistry,
    DeviceRegistry, DisplayModeRegistry, FileTransferRegistry, MediaPipelineRegistry,
    MediaProfileRegistry, MediaTaskRegistry, ProbeRegistry, SessionPeerMediaCapabilityRegistry,
    SessionRegistry, ShellState, TrayPortRef,
};
#[cfg(any(windows, target_os = "macos"))]
use super::{MediaRenderQueueRegistry, MediaSurfaceRendererRegistry};
use crate::control_input::ControlInputRegistry;
use mrd_ipc::CapabilitySnapshot;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    /// Service-owned file transfer task snapshots.
    pub file_transfers: Arc<Mutex<FileTransferRegistry>>,
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
            file_transfers: Arc::new(Mutex::new(FileTransferRegistry::default())),
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

    /// Get a clone of the file transfer registry.
    pub fn file_transfers(&self) -> Arc<Mutex<FileTransferRegistry>> {
        self.file_transfers.clone()
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

    #[test]
    fn app_state_core_initializes_empty_runtime_registries() {
        let state = AppState::new();

        assert!(!state.devices.try_lock().expect("devices").is_registered());
        assert_eq!(
            state
                .media_tasks
                .try_lock()
                .expect("media tasks")
                .active_count(&mrd_proto::SessionId("missing-session".to_string())),
            0
        );
    }
}
