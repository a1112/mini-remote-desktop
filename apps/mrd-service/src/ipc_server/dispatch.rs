use super::{audit_outcome, IpcServer};
use crate::handlers::control;
use crate::handlers::{
    capability, device, files, identity, lan, preflight, session, shell as shell_handlers,
    telemetry, transport as transport_handlers,
};
use mrd_ipc::{IpcRequest, IpcResponse};

pub(super) async fn dispatch_request(server: &IpcServer, request: IpcRequest) -> IpcResponse {
    server.dispatch_request_inner(request).await
}

impl IpcServer {
    async fn dispatch_request_inner(&self, request: IpcRequest) -> IpcResponse {
        match request {
            IpcRequest::RegisterDevice {
                device_id,
                device_name,
            } => {
                let response =
                    device::register_device(&self.app_state, device_id.clone(), device_name).await;
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
                response
            }

            IpcRequest::ListDevices => device::list_devices(&self.app_state).await,

            IpcRequest::LanDiscoverySnapshot => lan::lan_discovery_snapshot(&self.app_state).await,

            IpcRequest::RefreshLanDiscovery => lan::refresh_lan_discovery(&self.app_state).await,

            IpcRequest::ListDirectory { path } => files::list_directory(path),

            IpcRequest::WakeOnLan {
                device_id,
                mac_address,
                broadcast_addr,
            } => device::wake_on_lan(device_id, mac_address, broadcast_addr),

            IpcRequest::RequestRemoteDevicePowerAction { device_id, action } => {
                device::request_remote_device_power_action(&self.app_state, device_id, action).await
            }

            IpcRequest::ListSessions => session::list_sessions(&self.app_state).await,

            IpcRequest::StartSession {
                session_id,
                target_device_id,
                transport_kind,
            } => {
                let response = match preflight::preflight_session_start(
                    &self.app_state,
                    &target_device_id,
                    &transport_kind,
                    None,
                    false,
                )
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
                let response = match preflight::preflight_session_start(
                    &self.app_state,
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

            IpcRequest::RuntimeSnapshot => session::runtime_snapshot(&self.app_state).await,

            IpcRequest::AuditLog { query } => telemetry::audit_log(&self.app_state, query).await,

            IpcRequest::CapabilitySnapshot => {
                capability::capability_snapshot(&self.app_state).await
            }

            IpcRequest::EvaluateScenarioProfile {
                scenario_id,
                peer_device_id,
                requested_profile,
            } => {
                capability::evaluate_scenario_profile(
                    &self.app_state,
                    scenario_id,
                    peer_device_id,
                    requested_profile,
                )
                .await
            }

            IpcRequest::GetPeerCapabilitySnapshot { peer_device_id } => {
                capability::peer_capability_snapshot(&self.app_state, peer_device_id).await
            }

            IpcRequest::SetTransportPolicy { session_id, policy } => {
                control::set_transport_policy(session_id, policy)
            }

            IpcRequest::GetControlChannelSnapshot { session_id } => {
                control::control_channel_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::SendControlInput { session_id, event } => {
                session::send_control_input(&self.app_state, session_id, event).await
            }

            IpcRequest::PairDevice {
                device_id,
                certificate_fingerprint,
            } => {
                let response = identity::pair_device(
                    &self.app_state,
                    device_id.clone(),
                    certificate_fingerprint,
                )
                .await;
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
                response
            }

            IpcRequest::ApprovePairing { device_id } => {
                let response = identity::approve_pairing(&self.app_state, device_id.clone()).await;
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
                response
            }

            IpcRequest::RevokeDevice { device_id } => {
                let response = identity::revoke_device(&self.app_state, device_id.clone()).await;
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
                response
            }

            IpcRequest::GetDeviceIdentitySnapshot => {
                identity::get_device_identity_snapshot(&self.app_state).await
            }

            IpcRequest::GetTelemetryBundle { run_id, session_id } => {
                telemetry::telemetry_bundle(run_id, session_id)
            }

            IpcRequest::MediaPipelineSnapshot { session_id } => {
                transport_handlers::media_pipeline_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::ServiceHealth => telemetry::service_health(),

            IpcRequest::ProbeSnapshot { session_id } => {
                transport_handlers::probe_snapshot(&self.app_state, session_id).await
            }

            IpcRequest::StreamProbeEvents => telemetry::stream_probe_events(),

            IpcRequest::OpenUi { reason } => shell_handlers::open_ui(&self.ui_launcher, reason),

            IpcRequest::FocusUi => shell_handlers::focus_ui(&self.ui_launcher),

            IpcRequest::UiAttached {
                pid,
                executable_path,
            } => {
                shell_handlers::ui_attached(
                    &self.app_state,
                    &self.ui_launcher,
                    pid,
                    executable_path,
                )
                .await
            }

            IpcRequest::UiDetached { pid, reason } => {
                shell_handlers::ui_detached(&self.app_state, pid, reason).await
            }

            IpcRequest::GetShellStatus => shell_handlers::shell_status(&self.app_state).await,

            IpcRequest::SetAutostart { enabled } => {
                shell_handlers::set_autostart(&self.app_state, &self.autostart, enabled).await
            }

            IpcRequest::GetAutostartStatus => shell_handlers::autostart_status(&self.autostart),

            IpcRequest::ShutdownService { mode } => shell_handlers::shutdown_service(mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch_request;
    use crate::app_state::AppState;
    use crate::ipc_server::IpcServer;
    use mrd_ipc::{IpcRequest, IpcResponse};
    use std::sync::Arc;

    #[tokio::test]
    async fn dispatch_request_routes_capability_snapshot_without_accept_loop() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);

        let response = dispatch_request(&server, IpcRequest::CapabilitySnapshot).await;

        assert!(matches!(response, IpcResponse::CapabilitySnapshot { .. }));
    }
}
