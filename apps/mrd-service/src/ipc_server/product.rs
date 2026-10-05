//! Narrow installed-UI boundary. A pipe name or JSON PID never authorizes a caller.

use super::IpcServer;
use crate::agent_runtime::{verify_connected_windows_interactive_peer, VerifiedWindowsAgentPeer};
use mrd_ipc::{
    transport::{
        windows_product::{verify_installed_process, InstalledImage, VerifiedInstalledProcess},
        IpcStream,
    },
    IpcRequest, IpcResponse, RemotePermissionScope,
};
use windows::Win32::{
    Foundation::HWND, System::RemoteDesktop::WTSGetActiveConsoleSessionId,
    UI::WindowsAndMessaging::GetWindowThreadProcessId,
};

pub(super) struct VerifiedProductCaller {
    peer: VerifiedWindowsAgentPeer,
    image: VerifiedInstalledProcess,
}

impl VerifiedProductCaller {
    pub(super) fn inspect(stream: &IpcStream) -> anyhow::Result<Self> {
        let IpcStream::Server(pipe) = stream else {
            anyhow::bail!("Product caller requires a server pipe");
        };
        let peer = verify_connected_windows_interactive_peer(pipe)?;
        let image = verify_installed_process(peer.identity().process_id, InstalledImage::Ui)?;
        Ok(Self { peer, image })
    }

    /// The process objects and protected files remain held by this guard while
    /// the async handler executes. Only the compact verified identity is cloned.
    pub(super) fn bind(&self, server: &IpcServer) -> IpcServer {
        let mut bound = server.clone();
        bound.product_caller = Some(self.peer.cloned_identity());
        bound
    }

    pub(super) fn normalize(&self, request: &mut IpcRequest) -> Result<(), IpcResponse> {
        match request {
            IpcRequest::UiAttached {
                pid,
                executable_path,
            } => {
                if *pid != self.peer.identity().process_id {
                    return Err(caller_denied());
                }
                *executable_path = Some(self.image.image_path().to_string_lossy().into_owned());
            }
            IpcRequest::UiDetached { pid, .. } if *pid != self.peer.identity().process_id => {
                return Err(caller_denied())
            }
            IpcRequest::AttachRenderSurface {
                window_handle,
                render_proxy_endpoint,
                ..
            } => {
                if render_proxy_endpoint.is_some() {
                    return Err(error(
                        "E_PRODUCT_RENDER_PROXY_DENIED",
                        "This Windows UI must use its own native render window",
                    ));
                }
                let Some(window) = window_handle.filter(|window| *window != 0) else {
                    return Err(error(
                        "E_PRODUCT_RENDER_WINDOW_DENIED",
                        "A render window owned by this UI is required",
                    ));
                };
                let mut owner_pid = 0;
                let thread = unsafe {
                    GetWindowThreadProcessId(HWND(window as usize as *mut _), Some(&mut owner_pid))
                };
                if thread == 0 || owner_pid != self.peer.identity().process_id {
                    return Err(error(
                        "E_PRODUCT_RENDER_WINDOW_DENIED",
                        "The render window does not belong to this UI",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub(super) fn caller_denied() -> IpcResponse {
    error(
        "E_PRODUCT_CALLER_DENIED",
        "Only the installed UI in a verified interactive logon may use this endpoint",
    )
}

fn error(code: &str, message: &str) -> IpcResponse {
    IpcResponse::Error {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

/// Keep this separate from is_secure_remote: that broader contract includes
/// administrative trust and unattended-policy changes which are not UI rights.
fn product_request_is_allowed(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::GetPublicServerStatus
            | IpcRequest::GetPublicDeviceBindingProtocol
            | IpcRequest::BindPublicDevice { .. }
            | IpcRequest::UnbindPublicDevice { .. }
            | IpcRequest::ServiceHealth
            | IpcRequest::GetShellStatus
            | IpcRequest::UiAttached { .. }
            | IpcRequest::UiDetached { .. }
            | IpcRequest::ListDevices
            | IpcRequest::GetDevicePreferences
            | IpcRequest::UpdateDevicePreference { .. }
            | IpcRequest::LanDiscoverySnapshot
            | IpcRequest::RefreshLanDiscovery
            | IpcRequest::ListSessions
            | IpcRequest::GetRemoteSession { .. }
            | IpcRequest::RequestRemoteSession { .. }
            | IpcRequest::RespondToConsent { .. }
            | IpcRequest::SubscribeSessionEvents { .. }
            | IpcRequest::GetRouteEvidence { .. }
            | IpcRequest::RuntimeSnapshot
            | IpcRequest::SessionRuntimeSnapshot { .. }
            | IpcRequest::CapabilitySnapshot
            | IpcRequest::EvaluateScenarioProfile { .. }
            | IpcRequest::GetPeerCapabilitySnapshot { .. }
            | IpcRequest::GetDeviceIdentitySnapshot
            | IpcRequest::GetControlChannelSnapshot { .. }
            | IpcRequest::MediaPipelineSnapshot { .. }
            | IpcRequest::ProbeSnapshot { .. }
            | IpcRequest::UpdateMediaProfile { .. }
            | IpcRequest::ConfigureMediaAdaptation { .. }
            | IpcRequest::ListRemoteCaptureSources { .. }
            | IpcRequest::SelectRemoteCaptureSource { .. }
            | IpcRequest::ListRemoteDisplayModes { .. }
            | IpcRequest::SetRemoteDisplayMode { .. }
            | IpcRequest::RestoreRemoteDisplayMode { .. }
            | IpcRequest::AttachRenderSurface { .. }
            | IpcRequest::DetachRenderSurface { .. }
            | IpcRequest::StartSender { .. }
            | IpcRequest::StartReceiver { .. }
            | IpcRequest::StopSession { .. }
            | IpcRequest::SendControlInput { .. }
    )
}

fn machine_public_request(request: &IpcRequest) -> bool {
    matches!(
        request,
        IpcRequest::GetPublicServerStatus
            | IpcRequest::GetPublicDeviceBindingProtocol
            | IpcRequest::ServiceHealth
            | IpcRequest::GetShellStatus
            | IpcRequest::UiAttached { .. }
            | IpcRequest::UiDetached { .. }
    )
}

fn authorized_media_session(
    request: &IpcRequest,
) -> Option<(&mrd_proto::SessionId, RemotePermissionScope)> {
    use RemotePermissionScope::{DisplaySwitch, InputKeyboard, InputPointer, ScreenView};
    match request {
        IpcRequest::SendControlInput { session_id, event } => Some((
            session_id,
            match event {
                mrd_ipc::ControlInputEvent::Key { .. } => InputKeyboard,
                _ => InputPointer,
            },
        )),
        IpcRequest::SetRemoteDisplayMode { session_id, .. }
        | IpcRequest::RestoreRemoteDisplayMode { session_id }
        | IpcRequest::SelectRemoteCaptureSource { session_id, .. } => {
            Some((session_id, DisplaySwitch))
        }
        IpcRequest::UpdateMediaProfile { session_id, .. }
        | IpcRequest::ConfigureMediaAdaptation { session_id, .. }
        | IpcRequest::ListRemoteCaptureSources { session_id, .. }
        | IpcRequest::ListRemoteDisplayModes { session_id }
        | IpcRequest::AttachRenderSurface { session_id, .. }
        | IpcRequest::DetachRenderSurface { session_id, .. }
        | IpcRequest::StartSender { session_id }
        | IpcRequest::StartReceiver { session_id } => Some((session_id, ScreenView)),
        _ => None,
    }
}

impl IpcServer {
    pub(super) async fn product_request_denial(&self, request: &IpcRequest) -> Option<IpcResponse> {
        let Some(caller) = &self.product_caller else {
            return Some(caller_denied());
        };
        if !product_request_is_allowed(request) {
            return Some(error(
                "E_PRODUCT_COMMAND_DENIED",
                "This operation requires an administrator channel",
            ));
        }
        if !machine_public_request(request) {
            let active = unsafe { WTSGetActiveConsoleSessionId() };
            if active == 0 || active == u32::MAX || caller.windows_session_id != active {
                return Some(error(
                    "E_PRODUCT_DESKTOP_DENIED",
                    "This operation belongs to the currently active local desktop",
                ));
            }
            if let Some(agent) = self
                .app_state
                .agent_registry
                .active_for_session_at(active, now_ms())
            {
                if agent.identity.logon_sid_hash != caller.logon_sid_hash {
                    return Some(error(
                        "E_PRODUCT_DESKTOP_DENIED",
                        "The desktop logon has changed; reopen the UI in the active desktop",
                    ));
                }
            }
        }
        if let Some((session_id, scope)) = authorized_media_session(request) {
            if !self
                .app_state
                .session_authorizations
                .allows_scope(session_id, scope, now_ms())
                .await
            {
                return Some(error(
                    "E_PRODUCT_SESSION_GRANT_DENIED",
                    "This media or input operation requires a current remote-session permission",
                ));
            }
        }
        if let IpcRequest::RespondToConsent { response } = request {
            if let Err(reason) = self
                .app_state
                .console_capture
                .prepare_consent(&self.app_state, caller, response)
                .await
            {
                tracing::warn!("Local console consent could not be completed: {reason:#}");
                return Some(error(
                    "E_PRODUCT_DESKTOP_CONSENT_DENIED",
                    "Confirm this request in the active local desktop. The installed capture Agent must be running and unlocked.",
                ));
            }
        }
        None
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_commands_cannot_mutate_machine_trust_or_execute_privileged_operations() {
        for request in [
            IpcRequest::RegisterDevice {
                device_id: mrd_proto::DeviceId("spoofed".into()),
                device_name: "spoofed".into(),
            },
            IpcRequest::EnrollPublicDevice {
                enrollment_token: "secret".to_owned().into(),
                device_name: "spoofed".into(),
            },
            IpcRequest::RecoverPublicDevice {
                device_token: "secret".to_owned().into(),
            },
            IpcRequest::ListDirectory {
                path: Some("C:\\Windows".into()),
            },
            IpcRequest::StartSession {
                session_id: mrd_proto::SessionId("bypass".into()),
                target_device_id: mrd_proto::DeviceId("peer".into()),
                transport_kind: "quic".into(),
            },
            IpcRequest::ApprovePairing {
                device_id: mrd_proto::DeviceId("peer".into()),
            },
            IpcRequest::SetAutostart { enabled: true },
            IpcRequest::ShutdownService {
                mode: mrd_ipc::ShutdownMode::Force,
            },
        ] {
            assert!(
                !product_request_is_allowed(&request),
                "Denied command was exposed"
            );
        }
        assert!(product_request_is_allowed(&IpcRequest::RuntimeSnapshot));
        assert!(product_request_is_allowed(
            &IpcRequest::GetPublicServerStatus
        ));
        assert!(product_request_is_allowed(
            &IpcRequest::GetPublicDeviceBindingProtocol
        ));
        for request in [
            IpcRequest::BindPublicDevice {
                protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
                user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                    .unwrap(),
            },
            IpcRequest::UnbindPublicDevice {
                protocol_minor: mrd_ipc::PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
                user_token: mrd_ipc::PublicUserCredential::try_from("user.access.token".to_owned())
                    .unwrap(),
            },
        ] {
            assert!(product_request_is_allowed(&request));
            assert!(
                !machine_public_request(&request),
                "binding must still verify the active desktop caller"
            );
        }
    }
}
