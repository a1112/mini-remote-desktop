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
use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
use mrd_ipc::{
    transport, CapabilitySnapshot, CapabilityStatus, IpcRequest, IpcResponse, MediaProfile,
    ScenarioEvaluationStatus,
};
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
                        state: snap.lifecycle_state.as_str().to_string(),
                        transport_kind: snap.transport.clone(),
                        last_error: snap.last_error.clone(),
                        sender_active: snap.sender_active,
                        receiver_active: snap.receiver_active,
                        peer_device_id: snap
                            .target_device_id
                            .clone()
                            .or_else(|| snap.source_device_id.clone()),
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
                let response = match self
                    .preflight_session_start(&target_device_id, &transport_kind, None, false)
                    .await
                {
                    Ok(()) => {
                        session::start_session(
                            &self.app_state,
                            session_id.clone(),
                            target_device_id.clone(),
                            transport_kind.clone(),
                        )
                        .await
                    }
                    Err(message) => IpcResponse::Error {
                        code: "E_PREFLIGHT".to_string(),
                        message,
                    },
                };
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
                let response = match self
                    .preflight_session_start(
                        &target_device_id,
                        &transport_kind,
                        requested_profile.as_ref(),
                        true,
                    )
                    .await
                {
                    Ok(()) => {
                        session::start_lan_remote_session(
                            &self.app_state,
                            session_id.clone(),
                            target_device_id.clone(),
                            transport_kind.clone(),
                            requested_profile,
                        )
                        .await
                    }
                    Err(message) => IpcResponse::Error {
                        code: "E_PREFLIGHT".to_string(),
                        message,
                    },
                };
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

            IpcRequest::ListLocalCaptureSources {
                include_previews,
                limit,
            } => match crate::capture_source::list_capture_sources(include_previews, limit) {
                Ok(sources) => IpcResponse::LocalCaptureSourceList { sources },
                Err(error) => IpcResponse::Error {
                    code: "CAPTURE_SOURCE_LIST_FAILED".to_string(),
                    message: error.to_string(),
                },
            },

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
                render_proxy_endpoint,
            } => {
                transport_handlers::attach_render_surface(
                    &self.app_state,
                    session_id,
                    surface_id,
                    backend,
                    window_handle,
                    render_proxy_endpoint,
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

            IpcRequest::CapabilitySnapshot => {
                let snapshot = self.app_state.cached_capability_snapshot().await;
                self.app_state.refresh_capability_snapshot_in_background();
                IpcResponse::CapabilitySnapshot { snapshot }
            }

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
                let snapshot = self.app_state.cached_capability_snapshot().await;
                self.app_state.refresh_capability_snapshot_in_background();
                IpcResponse::ScenarioProfileEvaluated {
                    evaluation: crate::capabilities::evaluate_scenario_profile_against_snapshot(
                        &snapshot,
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
                let snapshot = self
                    .app_state
                    .control_input()
                    .lock()
                    .await
                    .snapshot(session_id);
                IpcResponse::ControlChannelSnapshot { snapshot }
            }

            IpcRequest::SendControlInput { session_id, event } => {
                session::send_control_input(&self.app_state, session_id, event).await
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
                    .filter(|session| session.lifecycle_state != SessionLifecycleState::Closed)
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
        let state = snap.lifecycle_state.as_str().to_string();

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

    #[allow(clippy::too_many_arguments)]
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

    async fn preflight_session_start(
        &self,
        target_device_id: &DeviceId,
        transport_kind: &str,
        requested_profile: Option<&MediaProfile>,
        require_lan_peer: bool,
    ) -> Result<(), String> {
        let snapshot = self.app_state.cached_capability_snapshot().await;
        self.app_state.refresh_capability_snapshot_in_background();

        ensure_transport_preflight(&snapshot, transport_kind)?;

        if let Some(profile) = requested_profile {
            let scenario_id = scenario_id_for_profile(profile);
            let evaluation = crate::capabilities::evaluate_scenario_profile_against_snapshot(
                &snapshot,
                scenario_id,
                Some(profile.clone()),
            );
            if matches!(evaluation.status, ScenarioEvaluationStatus::Blocked) {
                return Err(format_preflight_evaluation_failure(&evaluation));
            }
        }

        if require_lan_peer {
            let discovery = self.app_state.lan_discovery.snapshot().await;
            if !discovery
                .peers
                .iter()
                .any(|peer| &peer.device_id == target_device_id)
            {
                return Err(format!(
                    "LAN peer {} was not found during session preflight.",
                    target_device_id.0
                ));
            }
        }

        Ok(())
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

fn ensure_transport_preflight(
    snapshot: &CapabilitySnapshot,
    transport_kind: &str,
) -> Result<(), String> {
    let capability_id = transport_capability_id(transport_kind);
    let Some(capability) = snapshot
        .capabilities
        .iter()
        .find(|item| item.id == capability_id)
    else {
        return Err(format!(
            "{capability_id} is not advertised by local service capability preflight."
        ));
    };

    if capability_status_runs(&capability.status) {
        return Ok(());
    }

    Err(format!(
        "{} preflight failed: {}",
        capability.id,
        capability.reason.clone().unwrap_or_else(|| {
            format!("status {:?} cannot start this session.", capability.status)
        })
    ))
}

fn transport_capability_id(transport_kind: &str) -> &'static str {
    let kind = transport_kind.to_ascii_lowercase();
    if kind.contains("webrtc") {
        "transport.webrtc"
    } else if kind.contains("quic_datagram") {
        "transport.quic_datagram"
    } else if kind.contains("quic") {
        "transport.quic"
    } else {
        "transport.loopback"
    }
}

fn scenario_id_for_profile(profile: &MediaProfile) -> &'static str {
    if cfg!(target_os = "macos") && profile.codec.eq_ignore_ascii_case("hevc") {
        return "lan.macos.hevc.2k144";
    }
    if cfg!(target_os = "macos") && profile.codec.eq_ignore_ascii_case("h264") {
        return "lan.macos.2k144";
    }
    if profile.width >= 3840 || profile.height >= 2160 {
        "quality.4k60"
    } else if profile.height >= 1600 && profile.fps >= 165 {
        "lan.1600p165"
    } else if profile.width >= 2560 && profile.height >= 1440 && profile.fps >= 144 {
        "lan.2k144"
    } else {
        "interactive.1080p60"
    }
}

fn format_preflight_evaluation_failure(evaluation: &mrd_ipc::ScenarioEvaluation) -> String {
    let mut parts = vec![format!(
        "Scenario {} was blocked by session preflight.",
        evaluation.scenario_id
    )];
    if !evaluation.missing_capabilities.is_empty() {
        parts.push(format!(
            "missing capabilities: {}",
            evaluation.missing_capabilities.join(", ")
        ));
    }
    for reason in &evaluation.reasons {
        if reason.severity == "error" {
            parts.push(reason.message.clone());
        }
    }
    parts.join(" ")
}

fn capability_status_runs(status: &CapabilityStatus) -> bool {
    matches!(
        status,
        CapabilityStatus::Available
            | CapabilityStatus::Usable
            | CapabilityStatus::Supported
            | CapabilityStatus::Degraded
    )
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
            lifecycle_state: SessionLifecycleState::Listening,
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
            lifecycle_state: SessionLifecycleState::Created,
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
                assert_eq!(
                    sessions[0].peer_device_id,
                    Some(DeviceId("agent".to_string()))
                );
            }
            _ => panic!("Expected SessionList response"),
        }
    }

    #[tokio::test]
    async fn list_local_capture_sources_returns_local_response_or_error() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = server
            .handle_request(IpcRequest::ListLocalCaptureSources {
                include_previews: false,
                limit: Some(4),
            })
            .await;

        match response {
            IpcResponse::LocalCaptureSourceList { sources } => {
                assert!(sources.len() <= 4);
            }
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "CAPTURE_SOURCE_LIST_FAILED");
                assert!(!message.trim().is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
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
                assert!(snapshot.is_registered);
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

    #[tokio::test]
    async fn capability_snapshot_returns_cached_app_state_fact_source() {
        let app_state = Arc::new(AppState::new());
        let cached = crate::capabilities::local_capability_snapshot_static();
        app_state
            .replace_capability_snapshot_for_test(cached.clone())
            .await;
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        match response {
            IpcResponse::CapabilitySnapshot { snapshot } => {
                assert_eq!(snapshot.updated_at_ms, cached.updated_at_ms);
                assert_eq!(snapshot.capabilities.len(), cached.capabilities.len());
            }
            _ => panic!("Expected CapabilitySnapshot response"),
        }
    }

    #[tokio::test]
    async fn capability_snapshot_marks_keyboard_mouse_unavailable_when_injector_is_unavailable() {
        let app_state = Arc::new(AppState::new());
        let cached = crate::capabilities::local_capability_snapshot_static();
        app_state.replace_capability_snapshot_for_test(cached).await;
        app_state
            .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new(
                "blocked by test",
            ))
            .await;
        let server = IpcServer::new(app_state);

        let response = server.handle_request(IpcRequest::CapabilitySnapshot).await;

        let snapshot = match response {
            IpcResponse::CapabilitySnapshot { snapshot } => snapshot,
            other => panic!("expected capability snapshot, got {other:?}"),
        };
        let control = snapshot
            .capabilities
            .iter()
            .find(|item| item.id == "control.keyboard_mouse")
            .expect("keyboard/mouse control capability");
        assert_eq!(control.status, CapabilityStatus::Unsupported);
        assert!(control
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("Input injector is unavailable"));
    }

    #[tokio::test]
    async fn start_session_preflight_blocks_unsupported_transport_before_start() {
        let app_state = Arc::new(AppState::new());
        let mut cached = crate::capabilities::local_capability_snapshot_static();
        set_capability_status(
            &mut cached,
            "transport.quic",
            mrd_ipc::CapabilityStatus::Unsupported,
            "QUIC disabled for preflight test",
        );
        app_state.replace_capability_snapshot_for_test(cached).await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("preflight-blocked-session".to_string());

        let response = server
            .handle_request(IpcRequest::StartSession {
                session_id: session_id.clone(),
                target_device_id: DeviceId("target".to_string()),
                transport_kind: "quic".to_string(),
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_PREFLIGHT");
                assert!(message.contains("transport.quic"));
            }
            _ => panic!("Expected preflight error response"),
        }
        assert!(app_state.sessions.lock().await.get(&session_id).is_none());
    }

    #[tokio::test]
    async fn start_lan_remote_session_preflight_blocks_before_peer_request() {
        let app_state = Arc::new(AppState::new());
        let mut cached = crate::capabilities::local_capability_snapshot_static();
        set_capability_status(
            &mut cached,
            "transport.quic_datagram",
            mrd_ipc::CapabilityStatus::Unsupported,
            "QUIC datagram disabled for preflight test",
        );
        app_state.replace_capability_snapshot_for_test(cached).await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("lan-preflight-blocked-session".to_string());

        let response = server
            .handle_request(IpcRequest::StartLanRemoteSession {
                session_id: session_id.clone(),
                target_device_id: DeviceId("target".to_string()),
                transport_kind: "quic".to_string(),
                requested_profile: Some(mrd_ipc::MediaProfile {
                    width: 2560,
                    height: 1440,
                    fps: 144,
                    bitrate_mbps: 64,
                    codec: "h264".to_string(),
                    ..mrd_ipc::MediaProfile::default()
                }),
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_PREFLIGHT");
                assert!(message.contains("transport.quic_datagram"));
            }
            _ => panic!("Expected preflight error response"),
        }
        assert!(app_state.sessions.lock().await.get(&session_id).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_h264_2k144_profile_uses_native_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "h264".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(scenario_id_for_profile(&profile), "lan.macos.2k144");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_hevc_2k144_profile_uses_native_hevc_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            bitrate_mbps: 80,
            codec: "hevc".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(scenario_id_for_profile(&profile), "lan.macos.hevc.2k144");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_h264_smoke_profile_uses_native_preflight_profile() {
        let profile = mrd_ipc::MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 10,
            codec: "h264".to_string(),
            ..mrd_ipc::MediaProfile::default()
        };

        assert_eq!(scenario_id_for_profile(&profile), "lan.macos.2k144");
    }

    #[tokio::test]
    async fn control_input_request_updates_reliable_lane_counters() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("control-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Listening,
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            })
            .await;

        assert_eq!(
            response,
            IpcResponse::ControlInputAccepted {
                session_id: session_id.clone(),
                lane: mrd_ipc::ControlInputLane::Reliable,
                event_count: 1,
            }
        );

        let snapshot = match server
            .handle_request(IpcRequest::GetControlChannelSnapshot {
                session_id: session_id.clone(),
            })
            .await
        {
            IpcResponse::ControlChannelSnapshot { snapshot } => snapshot,
            other => panic!("expected control snapshot, got {other:?}"),
        };

        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 1);
        assert_eq!(snapshot.reliable.failed_messages, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_rejects_missing_session() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("missing-control-input-session".to_string());

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("session not found"));
            }
            other => panic!("expected missing session control input error, got {other:?}"),
        }
        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.reliable.injected_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_rejects_closed_session() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("closed-control-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Closed,
                last_error: None,
                sender_active: false,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.to_ascii_lowercase().contains("closed"));
            }
            other => panic!("expected closed session control input error, got {other:?}"),
        }
        let snapshot = app_state.control_input().lock().await.snapshot(session_id);
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.reliable.injected_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_for_controller_session_requires_lan_peer() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        app_state
            .replace_control_input_for_test(mrd_input::RecordingInputInjector::available())
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("controller-input-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: None,
                target_device_id: Some(DeviceId("target-device".to_string())),
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Connected,
                last_error: None,
                sender_active: false,
                receiver_active: true,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::MouseMove { x: 10, y: 20 },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("LAN peer not found"));
            }
            other => panic!("expected control input routing error, got {other:?}"),
        }

        let snapshot = app_state
            .control_input()
            .lock()
            .await
            .snapshot(session_id.clone());
        assert_eq!(snapshot.reliable.accepted_messages, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 0);
    }

    #[tokio::test]
    async fn control_input_request_records_injection_failure() {
        let app_state = Arc::new(AppState::new());
        app_state
            .replace_control_input_for_test(mrd_input::UnsupportedInputInjector::new(
                "blocked by test",
            ))
            .await;
        let server = IpcServer::new(app_state.clone());
        let session_id = SessionId("control-input-failure-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Listening,
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );

        let response = server
            .handle_request(IpcRequest::SendControlInput {
                session_id: session_id.clone(),
                event: mrd_ipc::ControlInputEvent::Key {
                    key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            })
            .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_CONTROL_INPUT");
                assert!(message.contains("blocked by test"));
            }
            other => panic!("expected control input error, got {other:?}"),
        }

        let snapshot = match server
            .handle_request(IpcRequest::GetControlChannelSnapshot {
                session_id: session_id.clone(),
            })
            .await
        {
            IpcResponse::ControlChannelSnapshot { snapshot } => snapshot,
            other => panic!("expected control snapshot, got {other:?}"),
        };

        assert_eq!(snapshot.reliable.accepted_messages, 1);
        assert_eq!(snapshot.reliable.injected_messages, 0);
        assert_eq!(snapshot.reliable.failed_messages, 1);
        assert_eq!(
            snapshot.reliable.last_error.as_deref(),
            Some("input injector unavailable: blocked by test")
        );
    }

    fn set_capability_status(
        snapshot: &mut mrd_ipc::CapabilitySnapshot,
        id: &str,
        status: mrd_ipc::CapabilityStatus,
        reason: &str,
    ) {
        let capability = snapshot
            .capabilities
            .iter_mut()
            .find(|item| item.id == id)
            .expect("capability in snapshot");
        capability.status = status;
        capability.reason = Some(reason.to_string());
    }
}
