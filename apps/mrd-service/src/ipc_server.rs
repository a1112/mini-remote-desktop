#![allow(dead_code)]

// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use crate::{
    app_state::AppState,
    handlers::{session, transport as transport_handlers},
    shell::{AutostartPortRef, UiLauncherPortRef},
};
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{transport, IpcRequest, IpcResponse};
use mrd_proto::{DeviceId, SessionId};
use std::{io::ErrorKind, sync::Arc, time::Duration};

const LAN_DISCOVERY_REFRESH_WAIT_MS: u64 = 450;
#[cfg(windows)]
const WINDOWS_IPC_ACCEPT_BACKLOG: usize = 32;

/// IPC server - handles requests from Rdesk shell
#[derive(Clone)]
pub struct IpcServer {
    app_state: Arc<AppState>,
    endpoint: transport::IpcEndpoint,
    ui_launcher: UiLauncherPortRef,
    autostart: AutostartPortRef,
}

impl IpcServer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self::new_with_endpoint(
            app_state,
            transport::IpcEndpoint::service_from_env_or_default(),
        )
    }

    pub fn new_with_endpoint(app_state: Arc<AppState>, endpoint: transport::IpcEndpoint) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher: crate::shell::default_ui_launcher(),
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    pub fn new_with_launcher(
        app_state: Arc<AppState>,
        endpoint: transport::IpcEndpoint,
        ui_launcher: UiLauncherPortRef,
    ) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher,
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    /// Handle a single connection
    pub async fn handle_connection(&self, mut stream: transport::IpcStream) -> anyhow::Result<()> {
        loop {
            match stream.recv_request().await {
                Ok(request) => {
                    let response = self.handle_request(request).await;
                    if let Err(e) = stream.send_response(&response).await {
                        eprintln!("Failed to send IPC response: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    if !is_connection_closed_error(&e) {
                        eprintln!("IPC request error: {}", e);
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    /// Handle an IPC request and return a response
    pub async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        match request {
            IpcRequest::RegisterDevice {
                device_id,
                device_name,
            } => {
                tracing::info!("Registering device: {} ({})", device_id.0, device_name);
                let mut devices = self.app_state.devices.lock().await;
                devices.register(device_id.clone(), device_name);
                drop(devices);
                self.record_audit_event(
                    "device.register",
                    "success",
                    None,
                    Some(device_id.clone()),
                    None,
                    None,
                    None,
                    Vec::new(),
                )
                .await;
                IpcResponse::DeviceRegistered { device_id }
            }

            IpcRequest::ListDevices => {
                let devices = self.app_state.devices.lock().await;
                // Return the registered device, if any
                let device_list = if let Some((id, name)) = devices.get_local_device() {
                    vec![mrd_ipc::DeviceInfo {
                        device_id: id.clone(),
                        device_name: name.clone(),
                        is_online: true, // Local device is always online
                    }]
                } else {
                    vec![]
                };
                IpcResponse::DeviceList {
                    devices: device_list,
                }
            }

            IpcRequest::LanDiscoverySnapshot => IpcResponse::LanDiscoverySnapshot {
                snapshot: self.app_state.lan_discovery.snapshot().await,
            },

            IpcRequest::RefreshLanDiscovery => IpcResponse::LanDiscoverySnapshot {
                snapshot: self
                    .app_state
                    .lan_discovery
                    .request_probe_and_wait(Duration::from_millis(LAN_DISCOVERY_REFRESH_WAIT_MS))
                    .await,
            },

            IpcRequest::ListSessions => {
                let sessions = self.app_state.sessions.lock().await;
                let session_list = sessions
                    .list_all()
                    .into_iter()
                    .map(|snap| mrd_ipc::SessionInfo {
                        session_id: snap.session_id.clone(),
                        role: if snap.target_device_id.is_some() {
                            "controller".to_string()
                        } else if snap.source_device_id.is_some() {
                            "agent".to_string()
                        } else {
                            "unknown".to_string()
                        },
                        state: snap.lifecycle_state.clone(),
                        transport_kind: snap.transport.clone(),
                        last_error: snap.last_error.clone(),
                        sender_active: snap.sender_active,
                        receiver_active: snap.receiver_active,
                    })
                    .collect();
                IpcResponse::SessionList {
                    sessions: session_list,
                }
            }

            IpcRequest::StartSession {
                session_id,
                target_device_id,
                transport_kind,
            } => {
                let response = session::start_session(
                    &self.app_state,
                    session_id.clone(),
                    target_device_id.clone(),
                    transport_kind.clone(),
                )
                .await;
                let (outcome, reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.start",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    Some(target_device_id),
                    Some(transport_kind),
                    reason,
                    Vec::new(),
                )
                .await;
                response
            }

            IpcRequest::StartLanRemoteSession {
                session_id,
                target_device_id,
                transport_kind,
                requested_profile,
            } => {
                let mut details = Vec::new();
                if let Some(profile) = requested_profile.as_ref() {
                    details.push((
                        "requested_profile".to_string(),
                        format!(
                            "{}x{}@{}/{}Mbps/{}",
                            profile.width,
                            profile.height,
                            profile.fps,
                            profile.bitrate_mbps,
                            profile.codec
                        ),
                    ));
                }
                let response = session::start_lan_remote_session(
                    &self.app_state,
                    session_id.clone(),
                    target_device_id.clone(),
                    transport_kind.clone(),
                    requested_profile,
                )
                .await;
                let (outcome, reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.start_lan",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    Some(target_device_id),
                    Some(transport_kind),
                    reason,
                    details,
                )
                .await;
                response
            }

            IpcRequest::UpdateMediaProfile {
                session_id,
                requested_profile,
            } => {
                session::update_media_profile(&self.app_state, session_id, requested_profile).await
            }

            IpcRequest::ConfigureMediaAdaptation { session_id, config } => {
                session::configure_media_adaptation(&self.app_state, session_id, config).await
            }

            IpcRequest::ListRemoteCaptureSources {
                session_id,
                include_previews,
                limit,
            } => {
                session::list_remote_capture_sources(
                    &self.app_state,
                    session_id,
                    include_previews,
                    limit,
                )
                .await
            }

            IpcRequest::SelectRemoteCaptureSource {
                session_id,
                source_id,
            } => {
                session::select_remote_capture_source(&self.app_state, session_id, source_id).await
            }

            IpcRequest::ListRemoteDisplayModes { session_id } => {
                session::list_remote_display_modes(&self.app_state, session_id).await
            }

            IpcRequest::SetRemoteDisplayMode {
                session_id,
                mode,
                restore_after_session,
            } => {
                session::set_remote_display_mode(
                    &self.app_state,
                    session_id,
                    mode,
                    restore_after_session,
                )
                .await
            }

            IpcRequest::RestoreRemoteDisplayMode { session_id } => {
                session::restore_remote_display_mode(&self.app_state, session_id).await
            }

            IpcRequest::AttachRenderSurface {
                session_id,
                surface_id,
                backend,
                window_handle,
            } => {
                transport_handlers::attach_render_surface(
                    &self.app_state,
                    session_id,
                    surface_id,
                    backend,
                    window_handle,
                )
                .await
            }

            IpcRequest::DetachRenderSurface {
                session_id,
                surface_id,
            } => {
                transport_handlers::detach_render_surface(&self.app_state, session_id, surface_id)
                    .await
            }

            IpcRequest::AcceptSession {
                session_id,
                source_device_id,
            } => {
                let response = session::accept_session(
                    &self.app_state,
                    session_id.clone(),
                    source_device_id.clone(),
                )
                .await;
                let (outcome, reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.accept",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    Some(source_device_id),
                    None,
                    reason,
                    Vec::new(),
                )
                .await;
                response
            }

            IpcRequest::StartSender { session_id } => {
                transport_handlers::start_sender(&self.app_state, session_id).await
            }

            IpcRequest::StartReceiver { session_id } => {
                transport_handlers::start_receiver(&self.app_state, session_id).await
            }

            IpcRequest::StopSession { session_id } => {
                let (peer_device_id, transport_kind) =
                    self.session_audit_context(&session_id).await;
                let response = session::stop_session(&self.app_state, session_id.clone()).await;
                let (outcome, reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.stop",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    peer_device_id,
                    transport_kind,
                    reason,
                    Vec::new(),
                )
                .await;
                response
            }

            IpcRequest::FailSession { session_id, reason } => {
                let (peer_device_id, transport_kind) =
                    self.session_audit_context(&session_id).await;
                let response =
                    session::fail_session(&self.app_state, session_id.clone(), reason.clone())
                        .await;
                let (outcome, response_reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.fail",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    peer_device_id,
                    transport_kind,
                    response_reason.or(Some(reason)),
                    Vec::new(),
                )
                .await;
                response
            }

            IpcRequest::RecoverSession { session_id } => {
                let (peer_device_id, transport_kind) =
                    self.session_audit_context(&session_id).await;
                let response = session::recover_session(&self.app_state, session_id.clone()).await;
                let (outcome, reason) = audit_outcome(&response);
                self.record_audit_event(
                    "session.recover",
                    outcome,
                    Some(session_id),
                    self.local_device_id().await,
                    peer_device_id,
                    transport_kind,
                    reason,
                    Vec::new(),
                )
                .await;
                response
            }

            IpcRequest::SessionRuntimeSnapshot { session_id } => {
                session::session_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::RuntimeSnapshot => {
                let sessions = self.app_state.sessions.lock().await;
                let devices = self.app_state.devices.lock().await;

                let session_snapshots: Vec<mrd_ipc::SessionRuntimeSnapshot> = sessions
                    .list_all()
                    .into_iter()
                    .filter_map(|snap| self.snapshot_to_ipc(&snap))
                    .collect();

                let device_id = devices.get_local_device().map(|(id, _)| id.clone());

                IpcResponse::RuntimeSnapshot {
                    snapshot: mrd_ipc::RuntimeSnapshot {
                        sessions: session_snapshots,
                        device_id,
                        is_registered: devices.is_registered(),
                    },
                }
            }

            IpcRequest::AuditLog { query } => {
                let audit_log = self.app_state.audit_log.lock().await;
                IpcResponse::AuditLog {
                    events: audit_log.query(&query),
                }
            }

            IpcRequest::CapabilitySnapshot => IpcResponse::CapabilitySnapshot {
                snapshot: crate::capabilities::local_capability_snapshot(),
            },

            IpcRequest::EvaluateScenarioProfile {
                scenario_id,
                peer_device_id,
                requested_profile,
            } => {
                if let Some(peer_device_id) = peer_device_id {
                    let snapshot = self.app_state.lan_discovery.snapshot().await;
                    if !snapshot
                        .peers
                        .iter()
                        .any(|peer| peer.device_id == peer_device_id)
                    {
                        return IpcResponse::ScenarioProfileEvaluated {
                            evaluation: peer_not_found_evaluation(scenario_id, peer_device_id),
                        };
                    }
                }
                IpcResponse::ScenarioProfileEvaluated {
                    evaluation: crate::capabilities::evaluate_scenario_profile(
                        &scenario_id,
                        requested_profile,
                    ),
                }
            }

            IpcRequest::GetPeerCapabilitySnapshot { peer_device_id } => {
                let snapshot = self.app_state.lan_discovery.snapshot().await;
                let capability_snapshot = snapshot
                    .peers
                    .iter()
                    .find(|peer| peer.device_id == peer_device_id)
                    .map(crate::capabilities::peer_capability_snapshot);
                IpcResponse::PeerCapabilitySnapshot {
                    peer_device_id,
                    snapshot: capability_snapshot,
                }
            }

            IpcRequest::SetTransportPolicy { session_id, policy } => {
                IpcResponse::TransportPolicyUpdated {
                    snapshot: transport_policy_snapshot(Some(session_id), &policy),
                }
            }

            IpcRequest::GetControlChannelSnapshot { session_id } => {
                IpcResponse::ControlChannelSnapshot {
                    snapshot: control_channel_snapshot(session_id),
                }
            }

            IpcRequest::PairDevice {
                device_id,
                certificate_fingerprint,
            } => {
                self.app_state.device_identities.lock().await.upsert(
                    device_id.clone(),
                    certificate_fingerprint,
                    "pending",
                );
                self.record_audit_event(
                    "device.pair",
                    "success",
                    None,
                    self.local_device_id().await,
                    Some(device_id),
                    None,
                    None,
                    Vec::new(),
                )
                .await;
                IpcResponse::PairingUpdated {
                    snapshot: self.identity_snapshot().await,
                }
            }

            IpcRequest::ApprovePairing { device_id } => {
                self.app_state.device_identities.lock().await.upsert(
                    device_id.clone(),
                    None,
                    "paired",
                );
                self.record_audit_event(
                    "device.approve_pairing",
                    "success",
                    None,
                    self.local_device_id().await,
                    Some(device_id),
                    None,
                    None,
                    Vec::new(),
                )
                .await;
                IpcResponse::PairingUpdated {
                    snapshot: self.identity_snapshot().await,
                }
            }

            IpcRequest::RevokeDevice { device_id } => {
                self.app_state
                    .device_identities
                    .lock()
                    .await
                    .revoke(&device_id);
                self.record_audit_event(
                    "device.revoke",
                    "success",
                    None,
                    self.local_device_id().await,
                    Some(device_id),
                    None,
                    None,
                    Vec::new(),
                )
                .await;
                IpcResponse::PairingUpdated {
                    snapshot: self.identity_snapshot().await,
                }
            }

            IpcRequest::GetDeviceIdentitySnapshot => IpcResponse::DeviceIdentitySnapshot {
                snapshot: self.identity_snapshot().await,
            },

            IpcRequest::GetTelemetryBundle { run_id, session_id } => IpcResponse::TelemetryBundle {
                bundle: mrd_ipc::TelemetryBundle {
                    run_id,
                    session_id,
                    metrics: Vec::new(),
                    event_count: 0,
                    log_count: 0,
                    artifacts: Vec::new(),
                },
            },

            IpcRequest::MediaPipelineSnapshot { session_id } => {
                transport_handlers::media_pipeline_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::ServiceHealth => IpcResponse::ServiceHealth {
                status: mrd_ipc::ServiceStatus {
                    running: true,
                    healthy: true,
                    pid: Some(std::process::id()),
                },
            },

            IpcRequest::ProbeSnapshot { session_id } => {
                transport_handlers::probe_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::StreamProbeEvents => IpcResponse::Error {
                code: "E501".to_string(),
                message: "Probe streaming not implemented yet".to_string(),
            },

            // === Shell / Lifecycle Commands (Phase 2) ===
            IpcRequest::OpenUi { reason } => {
                // Phase 3: Use UI launcher to launch or focus UI
                tracing::info!("OpenUi requested: reason={:?}", reason);
                let launcher = self.ui_launcher.lock().unwrap();
                let request = crate::shell::UiLaunchRequest {
                    reason: format!("{:?}", reason),
                };
                match launcher.launch_or_focus(request) {
                    Ok(crate::shell::UiLaunchResult::FocusedExisting { pid }) => {
                        tracing::info!("Focused existing UI: pid={}", pid);
                        IpcResponse::UiOpenResult {
                            status: mrd_ipc::UiOpenStatus::FocusedExisting,
                            pid: Some(pid),
                        }
                    }
                    Ok(crate::shell::UiLaunchResult::SpawnedNew { pid }) => {
                        tracing::info!("Spawned new UI: pid={}", pid);
                        IpcResponse::UiOpenResult {
                            status: mrd_ipc::UiOpenStatus::SpawnedNew,
                            pid: Some(pid),
                        }
                    }
                    Ok(crate::shell::UiLaunchResult::Unavailable) => {
                        tracing::warn!("UI launch unavailable - no configured path");
                        IpcResponse::UiOpenResult {
                            status: mrd_ipc::UiOpenStatus::Unavailable,
                            pid: None,
                        }
                    }
                    Ok(crate::shell::UiLaunchResult::Failed { error }) => {
                        tracing::error!("UI launch failed: {}", error);
                        IpcResponse::Error {
                            code: "E500".to_string(),
                            message: error,
                        }
                    }
                    Err(e) => {
                        tracing::error!("UI launch error: {}", e);
                        IpcResponse::Error {
                            code: "E500".to_string(),
                            message: e.to_string(),
                        }
                    }
                }
            }

            IpcRequest::FocusUi => {
                // Phase 3: Use UI launcher to focus existing UI
                tracing::info!("FocusUi requested");
                let launcher = self.ui_launcher.lock().unwrap();
                let request = crate::shell::UiLaunchRequest {
                    reason: "focus".to_string(),
                };
                match launcher.launch_or_focus(request) {
                    Ok(crate::shell::UiLaunchResult::FocusedExisting { .. }) => IpcResponse::Ack,
                    Ok(crate::shell::UiLaunchResult::SpawnedNew { .. }) => IpcResponse::Ack,
                    Ok(crate::shell::UiLaunchResult::Unavailable) => IpcResponse::Error {
                        code: "E404".to_string(),
                        message: "UI not available".to_string(),
                    },
                    Ok(crate::shell::UiLaunchResult::Failed { error }) => IpcResponse::Error {
                        code: "E500".to_string(),
                        message: error,
                    },
                    Err(e) => IpcResponse::Error {
                        code: "E500".to_string(),
                        message: e.to_string(),
                    },
                }
            }

            IpcRequest::UiAttached {
                pid,
                executable_path,
            } => {
                tracing::info!("UI attached: pid={} path={:?}", pid, executable_path);
                let mut shell = self.app_state.shell.lock().await;
                shell.ui_pid = Some(pid);
                shell.ui_executable_path = executable_path.clone();
                shell.last_error = None;

                // Update launcher state and persist path
                if let Some(path) = executable_path {
                    let launcher = self.ui_launcher.lock().unwrap();
                    let _ = launcher.set_ui_path(std::path::PathBuf::from(path));
                }

                IpcResponse::Ack
            }

            IpcRequest::UiDetached { pid, reason } => {
                tracing::info!("UI detached: pid={} reason={:?}", pid, reason);
                let mut shell = self.app_state.shell.lock().await;
                // Only clear if the PID matches (or if it's the same UI)
                if shell.ui_pid == Some(pid) {
                    shell.ui_pid = None;
                }
                IpcResponse::Ack
            }

            IpcRequest::GetShellStatus => {
                let shell = self.app_state.shell.lock().await;
                let sessions = self.app_state.sessions.lock().await;
                let active_session_count = sessions
                    .list_all()
                    .into_iter()
                    .filter(|session| session.lifecycle_state != "closed")
                    .count();
                IpcResponse::ShellStatus {
                    status: mrd_ipc::ShellStatusSnapshot {
                        service_pid: std::process::id(),
                        ui_pid: shell.ui_pid,
                        tray_available: shell.tray_available,
                        autostart_enabled: shell.autostart_enabled,
                        active_session_count,
                        last_error: shell.last_error.clone(),
                    },
                }
            }

            IpcRequest::SetAutostart { enabled } => {
                // Phase 5: Use autostart port
                tracing::info!("SetAutostart: enabled={}", enabled);
                let result = {
                    let autostart = self.autostart.lock().unwrap();
                    let supported = autostart.is_supported();
                    let set_result = autostart.set_enabled(enabled);
                    (supported, set_result)
                };

                match result {
                    (supported, Ok(())) => {
                        // Update shell state (now that autostart lock is released)
                        let mut shell = self.app_state.shell.lock().await;
                        shell.autostart_enabled = if supported { Some(enabled) } else { None };
                        IpcResponse::Ack
                    }
                    (_supported, Err(e)) => {
                        tracing::error!("SetAutostart failed: {}", e);
                        IpcResponse::Error {
                            code: "E500".to_string(),
                            message: e.to_string(),
                        }
                    }
                }
            }

            IpcRequest::GetAutostartStatus => {
                let autostart = self.autostart.lock().unwrap();
                let enabled = autostart.is_enabled().unwrap_or(false);
                let supported = autostart.is_supported();
                IpcResponse::AutostartStatus { enabled, supported }
            }

            IpcRequest::ShutdownService { mode } => {
                tracing::info!("ShutdownService requested: mode={:?}", mode);
                // Phase 2: Log only - actual shutdown will be implemented later
                // For now, return Ack to acknowledge the request
                match mode {
                    mrd_ipc::ShutdownMode::Force => {
                        // In Phase 3+, this would trigger immediate shutdown
                        IpcResponse::Error {
                            code: "E501".to_string(),
                            message: "Force shutdown not yet implemented".to_string(),
                        }
                    }
                    mrd_ipc::ShutdownMode::Graceful | mrd_ipc::ShutdownMode::AfterSessions => {
                        IpcResponse::Error {
                            code: "E501".to_string(),
                            message: "Service shutdown not yet implemented".to_string(),
                        }
                    }
                }
            }
        }
    }

    /// Convert a session snapshot to IPC format
    fn snapshot_to_ipc(&self, snap: &SessionSnapshot) -> Option<mrd_ipc::SessionRuntimeSnapshot> {
        // Determine role based on which device ID is set
        let role = if snap.target_device_id.is_some() {
            "controller"
        } else if snap.source_device_id.is_some() {
            "agent"
        } else {
            "unknown"
        }
        .to_string();

        // Use explicit lifecycle state from domain model
        let state = snap.lifecycle_state.clone();

        Some(mrd_ipc::SessionRuntimeSnapshot {
            session_id: snap.session_id.clone(),
            role,
            state,
            transport_kind: snap.transport.clone(),
            local_bootstrap: if snap.local_listen_addr.is_some() || snap.local_server_name.is_some()
            {
                Some(mrd_ipc::SessionBootstrap {
                    listen_addr: snap.local_listen_addr.clone(),
                    server_name: snap.local_server_name.clone(),
                    cert_der: snap.local_cert_der_b64.clone(),
                })
            } else {
                None
            },
            remote_bootstrap: if snap.remote_listen_addr.is_some()
                || snap.remote_server_name.is_some()
            {
                Some(mrd_ipc::SessionBootstrap {
                    listen_addr: snap.remote_listen_addr.clone(),
                    server_name: snap.remote_server_name.clone(),
                    cert_der: snap.remote_cert_der_b64.clone(),
                })
            } else {
                None
            },
            last_error: snap.last_error.clone(),
            sender_active: snap.sender_active,
            receiver_active: snap.receiver_active,
        })
    }

    /// Get access to the app state (for testing/integration)
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.app_state
    }

    async fn local_device_id(&self) -> Option<DeviceId> {
        self.app_state
            .devices
            .lock()
            .await
            .get_local_device()
            .map(|(device_id, _)| device_id.clone())
    }

    async fn identity_snapshot(&self) -> mrd_ipc::DeviceIdentitySnapshot {
        let devices = self.app_state.devices.lock().await;
        let (local_device_id, display_name) = devices
            .get_local_device()
            .map(|(device_id, name)| (Some(device_id.clone()), Some(name.clone())))
            .unwrap_or((None, None));
        drop(devices);
        let paired_devices = self.app_state.device_identities.lock().await.list();
        mrd_ipc::DeviceIdentitySnapshot {
            local_device_id,
            display_name,
            certificate_fingerprint: None,
            consent_required: true,
            paired_devices,
        }
    }

    async fn session_audit_context(
        &self,
        session_id: &SessionId,
    ) -> (Option<DeviceId>, Option<String>) {
        let sessions = self.app_state.sessions.lock().await;
        let Some(snapshot) = sessions.get(session_id) else {
            return (None, None);
        };
        let peer_device_id = snapshot
            .target_device_id
            .clone()
            .or_else(|| snapshot.source_device_id.clone());
        (peer_device_id, Some(snapshot.transport.clone()))
    }

    async fn record_audit_event(
        &self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) {
        self.app_state.audit_log.lock().await.record(
            action,
            outcome,
            session_id,
            actor_device_id,
            peer_device_id,
            transport_kind,
            reason,
            details,
        );
    }

    /// Run the IPC server (accepts connections in a loop)
    pub async fn run(&self) -> anyhow::Result<()> {
        #[cfg(windows)]
        let server =
            Arc::new(transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?);

        #[cfg(not(windows))]
        let server = transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?;

        tracing::info!("IPC server listening");

        #[cfg(windows)]
        {
            let mut workers = tokio::task::JoinSet::new();
            for _ in 0..WINDOWS_IPC_ACCEPT_BACKLOG {
                let pipe_server = server.clone();
                let connection_server = self.clone();
                workers.spawn(async move {
                    loop {
                        match pipe_server.accept().await {
                            Ok(stream) => {
                                if let Err(e) = connection_server.handle_connection(stream).await {
                                    eprintln!("IPC connection error: {}", e);
                                }
                            }
                            Err(e) => {
                                eprintln!("IPC accept error: {}", e);
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            }
                        }
                    }
                });
            }

            while let Some(result) = workers.join_next().await {
                if let Err(e) = result {
                    eprintln!("IPC accept worker stopped: {}", e);
                }
            }

            Ok(())
        }

        #[cfg(not(windows))]
        {
            let app_state = self.app_state.clone();
            let ui_launcher = self.ui_launcher.clone();
            loop {
                match server.accept().await {
                    Ok(stream) => {
                        let server_clone = IpcServer {
                            app_state: app_state.clone(),
                            endpoint: self.endpoint.clone(),
                            ui_launcher: ui_launcher.clone(),
                            autostart: crate::shell::default_autostart("mrd-service"),
                        };
                        tokio::spawn(async move {
                            if let Err(e) = server_clone.handle_connection(stream).await {
                                eprintln!("IPC connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("IPC accept error: {}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

fn is_connection_closed_error(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<std::io::Error>() {
        Some(io_error) => matches!(
            io_error.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
        ),
        None => false,
    }
}

fn audit_outcome(response: &IpcResponse) -> (&'static str, Option<String>) {
    match response {
        IpcResponse::Error { message, .. } => ("error", Some(message.clone())),
        _ => ("success", None),
    }
}

fn peer_not_found_evaluation(
    scenario_id: String,
    peer_device_id: DeviceId,
) -> mrd_ipc::ScenarioEvaluation {
    mrd_ipc::ScenarioEvaluation {
        scenario_id,
        status: mrd_ipc::ScenarioEvaluationStatus::Skipped,
        selected_profile: None,
        transport_kind: None,
        reasons: vec![mrd_ipc::ScenarioEvaluationReason {
            code: "peer_not_found".to_string(),
            severity: "warning".to_string(),
            message: format!("LAN peer {} is not currently discovered.", peer_device_id.0),
            capability_id: None,
        }],
        required_capabilities: Vec::new(),
        missing_capabilities: Vec::new(),
        fallback_profile: None,
    }
}

fn transport_policy_snapshot(
    session_id: Option<SessionId>,
    policy: &mrd_ipc::TransportPolicyConfig,
) -> mrd_ipc::TransportPolicySnapshot {
    let mut candidates = Vec::new();
    if policy.allow_lan_quic {
        candidates.push("quic".to_string());
    }
    if policy.allow_webrtc {
        candidates.push("webrtc".to_string());
    }

    let preferred = policy.preferred_transport.as_deref();
    let selected = match preferred {
        Some("quic") if policy.allow_lan_quic => "quic",
        Some("webrtc") if policy.allow_webrtc => "webrtc",
        _ if policy.mode == "wan" && policy.allow_webrtc => "webrtc",
        _ if policy.allow_lan_quic => "quic",
        _ if policy.allow_webrtc => "webrtc",
        _ => "none",
    };

    let relay_required = selected == "webrtc" && policy.mode == "wan" && policy.allow_relay;
    let fallback_reason = preferred
        .filter(|preferred| *preferred != selected)
        .map(|preferred| {
            format!("{preferred} was requested but is not allowed by the active transport policy.")
        });

    mrd_ipc::TransportPolicySnapshot {
        session_id,
        mode: policy.mode.clone(),
        selected_transport: selected.to_string(),
        candidate_transports: candidates,
        relay_required,
        reason: Some(match selected {
            "quic" => "LAN/high-refresh route selected QUIC datagram media.".to_string(),
            "webrtc" if relay_required => {
                "WAN route selected WebRTC with relay allowed.".to_string()
            }
            "webrtc" => "WebRTC route selected by transport policy.".to_string(),
            _ => "No transport is allowed by the active transport policy.".to_string(),
        }),
        fallback_reason,
    }
}

fn control_channel_snapshot(session_id: SessionId) -> mrd_ipc::ControlChannelSnapshot {
    mrd_ipc::ControlChannelSnapshot {
        session_id,
        reliable: mrd_ipc::ControlChannelLaneSnapshot {
            name: "ctrl_rel".to_string(),
            reliability: mrd_ipc::ControlChannelReliability::ReliableOrdered,
            ordered: true,
            max_retransmits: None,
            queued_messages: 0,
            dropped_messages: 0,
            coalesced_messages: 0,
        },
        realtime: mrd_ipc::ControlChannelLaneSnapshot {
            name: "ctrl_rt".to_string(),
            reliability: mrd_ipc::ControlChannelReliability::UnreliableRealtime,
            ordered: false,
            max_retransmits: Some(0),
            queued_messages: 0,
            dropped_messages: 0,
            coalesced_messages: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::{DeviceId, SessionId};
    use std::sync::Arc;

    #[test]
    fn closed_ipc_connection_is_not_treated_as_request_error() {
        let error = anyhow::Error::new(std::io::Error::new(ErrorKind::UnexpectedEof, "early eof"));

        assert!(is_connection_closed_error(&error));
    }

    #[tokio::test]
    async fn session_snapshot_returns_correct_ipc_format() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: Some("127.0.0.1:4433".to_string()),
            local_server_name: Some("localhost".to_string()),
            local_cert_der_b64: Some("AQID".to_string()),
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: "listening".to_string(),
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        server
            .app_state()
            .sessions()
            .lock()
            .await
            .insert(session_id.clone(), snapshot);

        let request = IpcRequest::SessionRuntimeSnapshot {
            session_id: session_id.clone(),
        };
        let response = server.handle_request(request).await;

        match response {
            IpcResponse::SessionSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.state, "listening"); // Only local bootstrap
                assert_eq!(snapshot.transport_kind, "quic");
            }
            _ => panic!("Expected SessionSnapshot response"),
        }
    }

    #[tokio::test]
    async fn list_sessions_returns_active_sessions() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: None,
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

        server
            .app_state()
            .sessions()
            .lock()
            .await
            .insert(session_id.clone(), snapshot);

        let response = server.handle_request(IpcRequest::ListSessions).await;

        match response {
            IpcResponse::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].session_id, session_id);
                assert_eq!(sessions[0].role, "controller");
            }
            _ => panic!("Expected SessionList response"),
        }
    }

    #[tokio::test]
    async fn runtime_snapshot_aggregates_state() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let device_id = DeviceId("test-device".to_string());
        let _ = server
            .handle_request(IpcRequest::RegisterDevice {
                device_id: device_id.clone(),
                device_name: "Test Device".to_string(),
            })
            .await;

        let response = server.handle_request(IpcRequest::RuntimeSnapshot).await;

        match response {
            IpcResponse::RuntimeSnapshot { snapshot } => {
                assert_eq!(snapshot.is_registered, true);
                assert_eq!(snapshot.device_id, Some(device_id));
            }
            _ => panic!("Expected RuntimeSnapshot response"),
        }
    }

    #[tokio::test]
    async fn capability_snapshot_reports_structured_capabilities() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        match response {
            IpcResponse::CapabilitySnapshot { snapshot } => {
                assert_eq!(snapshot.schema_version, 1);
                assert!(snapshot
                    .capabilities
                    .iter()
                    .any(|item| item.id == "transport.quic_datagram"));
                assert!(snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == "lan.2k144"));
                assert!(snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == "lan.1600p165"));
            }
            _ => panic!("Expected CapabilitySnapshot response"),
        }
    }
}
