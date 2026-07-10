use super::{
    audit::{audit_outcome, security_store_unavailable_response},
    IpcServer,
};
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
        let mut security_unhealthy = !self.app_state.security_is_healthy();
        if security_unhealthy && !allowed_when_security_unhealthy(&request) {
            return security_store_unavailable_response();
        }
        if !security_unhealthy
            && requires_durable_audit_preflight(&request)
            && self.verify_audit_integrity().await.is_err()
        {
            security_unhealthy = true;
            if !is_emergency_safety_command(&request) {
                return security_store_unavailable_response();
            }
        }
        match request {
            IpcRequest::RegisterDevice {
                device_id,
                device_name,
            } => {
                let response =
                    device::register_device(&self.app_state, device_id.clone(), device_name).await;
                if !security_unhealthy
                    && self
                        .record_audit_event(
                            "device.register",
                            "success",
                            None,
                            Some(device_id.clone()),
                            None,
                            None,
                            None,
                            Vec::new(),
                        )
                        .await
                        .is_err()
                {
                    return security_store_unavailable_response();
                }
                response
            }

            IpcRequest::ListDevices => device::list_devices(&self.app_state).await,

            IpcRequest::GetDevicePreferences => {
                device::list_device_preferences(&self.app_state).await
            }

            IpcRequest::UpdateDevicePreference { device_id, update } => {
                device::update_device_preference(&self.app_state, device_id, update).await
            }

            IpcRequest::LanDiscoverySnapshot => lan::lan_discovery_snapshot(&self.app_state).await,

            IpcRequest::RefreshLanDiscovery => lan::refresh_lan_discovery(&self.app_state).await,

            IpcRequest::ListDirectory { path } => files::list_directory(path),

            IpcRequest::StartFileTransfer { request } => {
                files::start_file_transfer(&self.app_state, request).await
            }

            IpcRequest::ListFileTransfers => files::list_file_transfers(&self.app_state).await,

            IpcRequest::ListFileTransferProviders => files::list_file_transfer_providers(),

            IpcRequest::CancelFileTransfer { transfer_id } => {
                files::cancel_file_transfer(&self.app_state, transfer_id).await
            }

            IpcRequest::WakeOnLan {
                device_id,
                mac_address,
                broadcast_addr,
            } => device::wake_on_lan(device_id, mac_address, broadcast_addr),

            IpcRequest::RequestRemoteDevicePowerAction { device_id, action } => {
                device::request_remote_device_power_action(&self.app_state, device_id, action).await
            }

            IpcRequest::ListSessions => session::list_sessions(&self.app_state).await,

            IpcRequest::ListTrustedDevices { include_revoked } => {
                identity::list_trusted_devices(&self.app_state, include_revoked).await
            }

            IpcRequest::ApproveTrustedDevice { .. } => IpcResponse::Error {
                code: "E_AUTHENTICATED_PEER_REQUIRED".to_string(),
                message: "trusted-device approval requires an authenticated pending peer key"
                    .to_string(),
            },

            IpcRequest::SuspendTrustedDevice {
                peer_key_id,
                expected_trust_revision,
            } => {
                identity::suspend_trusted_device(
                    &self.app_state,
                    peer_key_id,
                    expected_trust_revision.get(),
                )
                .await
            }

            IpcRequest::RevokeTrustedDevice {
                peer_key_id,
                expected_trust_revision,
            } => {
                identity::revoke_trusted_device(
                    &self.app_state,
                    peer_key_id,
                    expected_trust_revision.get(),
                )
                .await
            }

            IpcRequest::GetRemoteSession { .. }
            | IpcRequest::RequestRemoteSession { .. }
            | IpcRequest::RespondToConsent { .. }
            | IpcRequest::EnableUnattendedAccess { .. }
            | IpcRequest::DisableUnattendedAccess { .. }
            | IpcRequest::RotateUnattendedAccess { .. }
            | IpcRequest::RotateTrustedDevice { .. }
            | IpcRequest::ChangeSessionPermissions { .. }
            | IpcRequest::SubscribeSessionEvents { .. }
            | IpcRequest::GetRouteEvidence { .. }
            | IpcRequest::GetAuditEventsV2 { .. } => IpcResponse::Error {
                code: "E_SECURE_REMOTE_UNAVAILABLE".to_string(),
                message: "secure remote session operations are unavailable in this service build"
                    .to_string(),
            },

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
                if !security_unhealthy
                    && self
                        .record_audit_event(
                            "session.start",
                            outcome,
                            Some(session_id),
                            self.local_device_id().await,
                            Some(target_device_id),
                            Some(transport_kind),
                            reason,
                            Vec::new(),
                        )
                        .await
                        .is_err()
                {
                    return security_store_unavailable_response();
                }
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
                if self
                    .record_audit_event(
                        "session.start_lan",
                        outcome,
                        Some(session_id),
                        self.local_device_id().await,
                        Some(target_device_id),
                        Some(transport_kind),
                        reason,
                        details,
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
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
                if self
                    .record_audit_event(
                        "session.accept",
                        outcome,
                        Some(session_id),
                        self.local_device_id().await,
                        Some(source_device_id),
                        None,
                        reason,
                        Vec::new(),
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
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
                if !security_unhealthy
                    && self
                        .record_audit_event(
                            "session.stop",
                            outcome,
                            Some(session_id),
                            self.local_device_id().await,
                            peer_device_id,
                            transport_kind,
                            reason,
                            Vec::new(),
                        )
                        .await
                        .is_err()
                {
                    return security_store_unavailable_response();
                }
                response
            }

            IpcRequest::FailSession { session_id, reason } => {
                let (peer_device_id, transport_kind) =
                    self.session_audit_context(&session_id).await;
                let response =
                    session::fail_session(&self.app_state, session_id.clone(), reason.clone())
                        .await;
                let (outcome, response_reason) = audit_outcome(&response);
                if !security_unhealthy
                    && self
                        .record_audit_event(
                            "session.fail",
                            outcome,
                            Some(session_id),
                            self.local_device_id().await,
                            peer_device_id,
                            transport_kind,
                            response_reason.or(Some("session_failed".to_string())),
                            Vec::new(),
                        )
                        .await
                        .is_err()
                {
                    return security_store_unavailable_response();
                }
                response
            }

            IpcRequest::RecoverSession { session_id } => {
                let (peer_device_id, transport_kind) =
                    self.session_audit_context(&session_id).await;
                let response = session::recover_session(&self.app_state, session_id.clone()).await;
                let (outcome, reason) = audit_outcome(&response);
                if self
                    .record_audit_event(
                        "session.recover",
                        outcome,
                        Some(session_id),
                        self.local_device_id().await,
                        peer_device_id,
                        transport_kind,
                        reason,
                        Vec::new(),
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
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

            IpcRequest::CrossE2EInjectFault {
                session_id,
                fault_type,
                duration_ms,
            } => {
                session::cross_e2e_inject_fault(
                    &self.app_state,
                    session_id,
                    fault_type,
                    duration_ms,
                )
                .await
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
                let (outcome, reason) = audit_outcome(&response);
                if self
                    .record_audit_event(
                        "device.pair",
                        outcome,
                        None,
                        self.local_device_id().await,
                        Some(device_id),
                        None,
                        reason,
                        Vec::new(),
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
                response
            }

            IpcRequest::ApprovePairing { device_id } => {
                let response = identity::approve_pairing(&self.app_state, device_id.clone()).await;
                let (outcome, reason) = audit_outcome(&response);
                if self
                    .record_audit_event(
                        "device.approve_pairing",
                        outcome,
                        None,
                        self.local_device_id().await,
                        Some(device_id),
                        None,
                        reason,
                        Vec::new(),
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
                response
            }

            IpcRequest::RevokeDevice { device_id } => {
                let response = identity::revoke_device(&self.app_state, device_id.clone()).await;
                let (outcome, reason) = audit_outcome(&response);
                if self
                    .record_audit_event(
                        "device.revoke",
                        outcome,
                        None,
                        self.local_device_id().await,
                        Some(device_id),
                        None,
                        reason,
                        Vec::new(),
                    )
                    .await
                    .is_err()
                {
                    return security_store_unavailable_response();
                }
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

            IpcRequest::ServiceHealth => telemetry::service_health(&self.app_state),

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

fn allowed_when_security_unhealthy(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::ServiceHealth
            | IpcRequest::ListSessions
            | IpcRequest::SessionRuntimeSnapshot { .. }
            | IpcRequest::RuntimeSnapshot
            | IpcRequest::StopSession { .. }
            | IpcRequest::FailSession { .. }
            | IpcRequest::SuspendTrustedDevice { .. }
            | IpcRequest::RevokeTrustedDevice { .. }
            | IpcRequest::GetShellStatus
            | IpcRequest::ShutdownService { .. }
    )
}

fn requires_durable_audit_preflight(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::RegisterDevice { .. }
            | IpcRequest::StartSession { .. }
            | IpcRequest::StartLanRemoteSession { .. }
            | IpcRequest::AcceptSession { .. }
            | IpcRequest::StopSession { .. }
            | IpcRequest::FailSession { .. }
            | IpcRequest::RecoverSession { .. }
            | IpcRequest::PairDevice { .. }
            | IpcRequest::ApprovePairing { .. }
            | IpcRequest::RevokeDevice { .. }
    )
}

fn is_emergency_safety_command(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::StopSession { .. } | IpcRequest::FailSession { .. }
    )
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

    #[tokio::test]
    async fn secure_remote_contract_fails_closed_until_handlers_are_available() {
        let app_state = Arc::new(AppState::new());
        let server = IpcServer::new(app_state);
        let requests = [
            serde_json::json!({"type":"GetRemoteSession","session_id":"session-1"}),
            serde_json::json!({"type":"RequestRemoteSession","request":{"session_id":"session-1","target_device_id":"device-1","access_mode":"attended","requested_scopes":["screen.view"],"requested_profile":null}}),
            serde_json::json!({"type":"RespondToConsent","response":{"session_id":"session-1","decision":"approve","approved_scopes":["screen.view"],"expected_policy_revision":"7"}}),
            serde_json::json!({"type":"EnableUnattendedAccess","policy":{"trusted_devices_only":true,"allowed_peer_key_ids":["sha256:peer"],"permission_ceiling":["screen.view"],"expires_at_ms":null}}),
            serde_json::json!({"type":"DisableUnattendedAccess","expected_policy_revision":"7"}),
            serde_json::json!({"type":"RotateUnattendedAccess","expected_policy_revision":"7"}),
            serde_json::json!({"type":"ListTrustedDevices","include_revoked":false}),
            serde_json::json!({"type":"ApproveTrustedDevice","approval":{"peer_key_id":"sha256:peer","key_epoch":"2","permission_ceiling":["screen.view"]}}),
            serde_json::json!({"type":"SuspendTrustedDevice","peer_key_id":"sha256:peer","expected_trust_revision":"9"}),
            serde_json::json!({"type":"RevokeTrustedDevice","peer_key_id":"sha256:peer","expected_trust_revision":"9"}),
            serde_json::json!({"type":"RotateTrustedDevice","rotation":{"peer_key_id":"sha256:peer","new_peer_key_id":"sha256:new-peer","new_key_epoch":"3","expected_trust_revision":"9"}}),
            serde_json::json!({"type":"ChangeSessionPermissions","change":{"session_id":"session-1","requested_scopes":["screen.view"],"expected_policy_revision":"7"}}),
            serde_json::json!({"type":"SubscribeSessionEvents","query":{"session_id":"session-1","after_sequence":"41","limit":32,"wait_timeout_ms":15000}}),
            serde_json::json!({"type":"GetRouteEvidence","session_id":"session-1"}),
            serde_json::json!({"type":"GetAuditEventsV2","query":{"after_sequence":"8","limit":50,"session_id":"session-1","action":"session.authorized","outcome":"allowed","peer_device_id":"device-1"}}),
        ]
        .into_iter()
        .map(|value| serde_json::from_value::<IpcRequest>(value).expect("valid secure request"));

        for request in requests {
            assert!(request.is_secure_remote());
            let response = dispatch_request(&server, request).await;
            assert!(matches!(response, IpcResponse::Error { .. }));
        }
    }
}
