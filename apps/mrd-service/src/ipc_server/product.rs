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
            | IpcRequest::ListLanPairingCandidates
            | IpcRequest::ApproveLanPairing { .. }
            | IpcRequest::ListTrustedDevices { .. }
            | IpcRequest::GetAuditEventsV2 { .. }
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
    pub(super) fn validate_first_pairing_caller(&self) -> Result<(), mrd_store_sqlite::StoreError> {
        let caller = self
            .product_caller
            .as_ref()
            .filter(|_| self.product_only)
            .ok_or_else(|| {
                mrd_store_sqlite::StoreError::TrustTransition(
                    "verified installed pairing UI is required".to_owned(),
                )
            })?;
        let active = unsafe { WTSGetActiveConsoleSessionId() };
        let agent = self
            .app_state
            .agent_registry
            .active_for_session_at(active, now_ms());
        if active == 0
            || active == u32::MAX
            || caller.windows_session_id != active
            || !agent.is_some_and(|agent| agent.identity.logon_sid_hash == caller.logon_sid_hash)
        {
            return Err(mrd_store_sqlite::StoreError::TrustTransition(
                "active pairing desktop changed".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) async fn product_audit_cursor(&self) -> Result<Option<u64>, IpcResponse> {
        if !self.product_only || self.product_caller.is_none() {
            return Ok(None);
        }
        let log = self.app_state.audit_log.clone();
        match tokio::task::spawn_blocking(move || log.last_sequence()).await {
            Ok(Ok(sequence)) => Ok(Some(sequence)),
            _ => {
                self.app_state.mark_security_unhealthy();
                Err(super::audit::security_store_unavailable_response())
            }
        }
    }

    /// Only a successful birth or successful native-consent resolution may
    /// register ownership. Reading a snapshot is never proof of ownership.
    pub(super) async fn remember_product_response(
        &self,
        expected_id: &mrd_proto::SessionId,
        response: &IpcResponse,
        expected_role: mrd_ipc::RemoteSessionRole,
        after_sequence: Option<u64>,
        proof: Option<ProductAuditOwnerProof>,
    ) {
        let Some(caller) = self.product_caller.as_ref().filter(|_| self.product_only) else {
            return;
        };
        let Some(after_sequence) = after_sequence else {
            return;
        };
        let session = match (expected_role, response) {
            (
                mrd_ipc::RemoteSessionRole::Controller,
                IpcResponse::RemoteSessionRequested { session },
            )
            | (mrd_ipc::RemoteSessionRole::Agent, IpcResponse::ConsentRecorded { session })
                if &session.session_id == expected_id && session.role == expected_role =>
            {
                session
            }
            _ => return,
        };
        let _gate = self.app_state.authorization_security_gate.lock().await;
        let Some((instance_id, current)) = self
            .app_state
            .session_authorizations
            .product_audit_binding(expected_id)
            .await
        else {
            return;
        };
        if !same_aggregate_identity(session, &current)
            || proof
                .as_ref()
                .and_then(|proof| proof.instance_for_role(expected_role))
                != Some(instance_id)
        {
            return;
        }
        let mut owners = self
            .product_audit_owners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A repeated response cannot revive an expired owner for this instance.
        if owners.get(expected_id).is_some_and(|owner| {
            owner.instance_id == instance_id && owner.remembered_at.elapsed() >= AUDIT_OWNER_TTL
        }) {
            return;
        }
        owners.retain(|_, owner| owner.remembered_at.elapsed() < AUDIT_OWNER_TTL);
        if let Some(owner) = owners.get_mut(&session.session_id) {
            if owner.instance_id == instance_id {
                if owner.matches(caller, instance_id, &current) {
                    owner.screen_view_seen |= current
                        .granted_scopes
                        .contains(&RemotePermissionScope::ScreenView);
                }
                return;
            }
        }
        if owners.contains_key(expected_id) || owners.len() < 128 {
            owners.insert(
                session.session_id.clone(),
                ProductAuditOwner {
                    windows_session_id: caller.windows_session_id,
                    logon_sid_hash: caller.logon_sid_hash,
                    instance_id,
                    created_at_ms: current.created_at_ms,
                    role: current.role,
                    peer_key_id: current.peer_key_id,
                    peer_device_id: current.peer_device_id,
                    after_sequence,
                    screen_view_seen: current
                        .granted_scopes
                        .contains(&RemotePermissionScope::ScreenView),
                    remembered_at: std::time::Instant::now(),
                },
            );
        }
    }

    pub(super) async fn observe_owned_product_session(
        &self,
        session: &mrd_ipc::RemoteSessionSnapshot,
    ) {
        let Some(caller) = self.product_caller.as_ref().filter(|_| self.product_only) else {
            return;
        };
        let _gate = self.app_state.authorization_security_gate.lock().await;
        let binding = self
            .app_state
            .session_authorizations
            .product_audit_binding(&session.session_id)
            .await;
        let mut owners = self
            .product_audit_owners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(owner) = owners.get_mut(&session.session_id) else {
            return;
        };
        let Some((instance_id, current)) = binding else {
            owners.remove(&session.session_id);
            return;
        };
        if owner.instance_id != instance_id || owner.remembered_at.elapsed() >= AUDIT_OWNER_TTL {
            owners.remove(&session.session_id);
            return;
        }
        if same_aggregate_identity(session, &current)
            && owner.matches(caller, instance_id, &current)
        {
            owner.screen_view_seen |= current
                .granted_scopes
                .contains(&RemotePermissionScope::ScreenView);
        }
    }

    async fn product_audit_query(
        &self,
        query: &mrd_ipc::AuditEventsQueryV2,
    ) -> Result<(mrd_ipc::AuditEventsQueryV2, u64), IpcResponse> {
        let caller = self
            .product_caller
            .as_ref()
            .filter(|_| self.product_only)
            .ok_or_else(audit_denied)?;
        let id = query.session_id.as_ref().ok_or_else(audit_denied)?;
        if query.limit == 0 || query.limit > 64 {
            return Err(audit_denied());
        }
        let (instance_id, current) = self
            .app_state
            .session_authorizations
            .product_audit_binding(id)
            .await
            .ok_or_else(audit_denied)?;
        let owners = self
            .product_audit_owners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let owner = owners
            .get(id)
            .filter(|owner| {
                owner.matches(caller, instance_id, &current)
                    && owner.screen_view_seen
                    && owner.remembered_at.elapsed() < AUDIT_OWNER_TTL
            })
            .ok_or_else(audit_denied)?;
        let mut normalized = query.clone();
        normalized.after_sequence = Some(mrd_ipc::DecimalU64::new(
            query
                .after_sequence
                .map(mrd_ipc::DecimalU64::get)
                .unwrap_or(0)
                .max(owner.after_sequence),
        ));
        Ok((normalized, instance_id))
    }

    async fn product_audit_is_allowed(&self, query: &mrd_ipc::AuditEventsQueryV2) -> bool {
        self.product_audit_query(query).await.is_ok()
    }

    /// Freeze normal aggregate births while reading, and revalidate afterward
    /// as defense against callers of the domain registry outside that gate.
    pub(super) async fn read_product_audit(
        &self,
        query: mrd_ipc::AuditEventsQueryV2,
    ) -> IpcResponse {
        self.read_product_audit_with(query, |normalized| {
            crate::handlers::telemetry::audit_events_v2(&self.app_state, normalized)
        })
        .await
    }

    // Production supplies only the durable telemetry reader. Admission and
    // the post-read fence stay together around the asynchronous operation.
    async fn read_product_audit_with<F, Fut>(
        &self,
        query: mrd_ipc::AuditEventsQueryV2,
        read: F,
    ) -> IpcResponse
    where
        F: FnOnce(mrd_ipc::AuditEventsQueryV2) -> Fut,
        Fut: std::future::Future<Output = IpcResponse>,
    {
        let _gate = self.app_state.authorization_security_gate.lock().await;
        let (normalized, instance_id) = match self.product_audit_query(&query).await {
            Ok(binding) => binding,
            Err(denial) => return denial,
        };
        let result = read(normalized).await;
        if !matches!(self.product_audit_query(&query).await, Ok((_, current)) if current == instance_id)
        {
            return audit_denied();
        }
        result
    }

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
        if let IpcRequest::GetAuditEventsV2 { query } = request {
            if !self.product_audit_is_allowed(query).await {
                return Some(error("E_PRODUCT_AUDIT_SESSION_DENIED", "Only bounded evidence for this user's authorized screen-view session may be read"));
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

pub(super) struct ProductAuditOwner {
    windows_session_id: u32,
    logon_sid_hash: [u8; 32],
    screen_view_seen: bool,
    remembered_at: std::time::Instant,
    instance_id: u64,
    created_at_ms: u64,
    role: mrd_ipc::RemoteSessionRole,
    peer_key_id: String,
    peer_device_id: mrd_proto::DeviceId,
    after_sequence: u64,
}

pub(super) enum ProductAuditOwnerProof {
    Outgoing(crate::session_authorization::OutgoingBirthReceipt),
    Consent(u64),
}

impl ProductAuditOwnerProof {
    fn instance_for_role(&self, role: mrd_ipc::RemoteSessionRole) -> Option<u64> {
        match (self, role) {
            (Self::Outgoing(receipt), mrd_ipc::RemoteSessionRole::Controller) => {
                receipt.instance_id()
            }
            (Self::Consent(instance), mrd_ipc::RemoteSessionRole::Agent) => Some(*instance),
            _ => None,
        }
    }
}

const AUDIT_OWNER_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

impl ProductAuditOwner {
    fn matches(
        &self,
        caller: &crate::agent_runtime::ObservedAgentIdentity,
        instance_id: u64,
        snapshot: &mrd_ipc::RemoteSessionSnapshot,
    ) -> bool {
        self.windows_session_id == caller.windows_session_id
            && self.logon_sid_hash == caller.logon_sid_hash
            && self.instance_id == instance_id
            && self.created_at_ms == snapshot.created_at_ms
            && self.role == snapshot.role
            && self.peer_key_id == snapshot.peer_key_id
            && self.peer_device_id == snapshot.peer_device_id
    }
}

fn same_aggregate_identity(
    a: &mrd_ipc::RemoteSessionSnapshot,
    b: &mrd_ipc::RemoteSessionSnapshot,
) -> bool {
    a.session_id == b.session_id
        && a.created_at_ms == b.created_at_ms
        && a.role == b.role
        && a.peer_key_id == b.peer_key_id
        && a.peer_device_id == b.peer_device_id
}

fn audit_denied() -> IpcResponse {
    error(
        "E_PRODUCT_AUDIT_SESSION_DENIED",
        "Only bounded evidence for this user's authorized screen-view session may be read",
    )
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

    fn audit_test_server(logon: u8) -> IpcServer {
        let mut server = IpcServer::new(std::sync::Arc::new(crate::app_state::AppState::default()));
        server.product_only = true;
        // Isolated policy fixture, never a live pipe caller or capture Agent.
        server.product_caller = Some(crate::agent_runtime::ObservedAgentIdentity {
            caller_kind: crate::agent_runtime::AgentCallerKind::InteractiveUser,
            process_id: 42,
            process_creation_time: 123,
            logon_sid_hash: [logon; 32],
            windows_session_id: match unsafe { WTSGetActiveConsoleSessionId() } {
                0 | u32::MAX => 1,
                active => active,
            },
        });
        server
    }

    fn audit_query(id: &mrd_proto::SessionId) -> mrd_ipc::AuditEventsQueryV2 {
        mrd_ipc::AuditEventsQueryV2 {
            after_sequence: None,
            limit: 64,
            session_id: Some(id.clone()),
            action: None,
            outcome: None,
            peer_device_id: None,
        }
    }

    struct GrantedAuditFixture {
        snapshot: mrd_ipc::RemoteSessionSnapshot,
        receipt: crate::session_authorization::OutgoingBirthReceipt,
    }

    impl std::ops::Deref for GrantedAuditFixture {
        type Target = mrd_ipc::RemoteSessionSnapshot;
        fn deref(&self) -> &Self::Target {
            &self.snapshot
        }
    }

    async fn remember_fixture(server: &IpcServer, snapshot: &GrantedAuditFixture, cursor: u64) {
        server
            .remember_product_response(
                &snapshot.session_id,
                &IpcResponse::RemoteSessionRequested {
                    session: snapshot.snapshot.clone(),
                },
                mrd_ipc::RemoteSessionRole::Controller,
                Some(cursor),
                Some(ProductAuditOwnerProof::Outgoing(snapshot.receipt.clone())),
            )
            .await;
    }

    async fn granted_audit_session(server: &IpcServer, id: &str, at: u64) -> GrantedAuditFixture {
        let peer =
            mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
        granted_audit_session_for_peer(server, id, at, &peer).await
    }

    async fn granted_audit_session_for_peer(
        server: &IpcServer,
        id: &str,
        at: u64,
        peer: &mrd_identity::DeviceIdentity,
    ) -> GrantedAuditFixture {
        let session_id = mrd_proto::SessionId(id.to_owned());
        let receipt = server
            .app_state
            .session_authorizations
            .observe_outgoing_birth(&session_id)
            .await
            .expect("fixture subscribes to an actually empty slot before its birth");
        let scopes = vec![RemotePermissionScope::ScreenView];
        server
            .app_state
            .session_authorizations
            .begin_outgoing(
                crate::session_authorization::VerifiedIncomingAuthorizationRequest {
                    session_id: session_id.clone(),
                    peer_device_id: mrd_proto::DeviceId("fixture-peer".into()),
                    peer_key_id: peer.key_id().to_owned(),
                    peer_key_epoch: 1,
                    access_mode: mrd_ipc::RemoteAccessMode::Attended,
                    requested_scopes: scopes.clone(),
                    peer_permission_ceiling: scopes.clone(),
                    machine_permission_ceiling: scopes.clone(),
                    runtime_capabilities: scopes.clone(),
                    transport_kind: "quic".into(),
                    request_nonce: [4; 16],
                    created_at_ms: at,
                    expires_at_ms: at + 300_000,
                },
            )
            .await
            .unwrap();
        server
            .app_state
            .session_authorizations
            .bind_authenticated_peer_key(&session_id, peer.public_key(), at)
            .await
            .unwrap();
        let snapshot = server
            .app_state
            .session_authorizations
            .install_verified_grant(
                crate::session_authorization::VerifiedSessionGrant {
                    grant_id: "fixture-only-grant".into(),
                    session_id,
                    granted_scopes: scopes,
                    issued_at_ms: at,
                    expires_at_ms: at + 300_000,
                    policy_revision: 1,
                    route_constraint: "quic".into(),
                    transport_fingerprint_sha256: [5; 32],
                },
                at,
            )
            .await
            .unwrap();
        GrantedAuditFixture { snapshot, receipt }
    }

    #[tokio::test]
    async fn audit_owner_failed_duplicate_request_does_not_claim_an_existing_grant() {
        let server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-duplicate", now_ms()).await;
        let response = server
            .handle_request(IpcRequest::RequestRemoteSession {
                request: mrd_ipc::RemoteSessionRequest {
                    session_id: snapshot.session_id.clone(),
                    target_device_id: snapshot.peer_device_id.clone(),
                    // Rejected before any network activity: this is a policy fixture only.
                    access_mode: mrd_ipc::RemoteAccessMode::Unattended,
                    route_preference: mrd_ipc::RemoteRoutePreference::Lan,
                    requested_scopes: vec![RemotePermissionScope::ScreenView],
                    requested_profile: None,
                },
            })
            .await;
        assert!(!matches!(
            response,
            IpcResponse::RemoteSessionRequested { .. }
        ));
        assert!(
            server.product_audit_owners.lock().unwrap().is_empty(),
            "a failed request cannot claim an existing aggregate"
        );
    }

    #[tokio::test]
    async fn audit_owner_observation_never_creates_a_missing_owner() {
        let server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-no-owner", now_ms()).await;
        server.observe_owned_product_session(&snapshot).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_observation_does_not_renew_an_expired_owner() {
        let server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-expired", now_ms()).await;
        remember_fixture(&server, &snapshot, 0).await;
        server
            .product_audit_owners
            .lock()
            .unwrap()
            .get_mut(&snapshot.session_id)
            .unwrap()
            .remembered_at = std::time::Instant::now() - std::time::Duration::from_secs(3601);
        server.observe_owned_product_session(&snapshot).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await,
            "reading cannot recreate an expired owner"
        );
    }

    #[tokio::test]
    async fn audit_owner_wrong_logon_cannot_read_an_owned_session() {
        let mut server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-wrong-logon", now_ms()).await;
        remember_fixture(&server, &snapshot, 0).await;
        server.product_caller.as_mut().unwrap().logon_sid_hash = [2; 32];
        server.observe_owned_product_session(&snapshot).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_session_id_reuse_does_not_authorize_the_old_owner() {
        let server = audit_test_server(1);
        let at = now_ms();
        let snapshot = granted_audit_session(&server, "audit-reused-id", at).await;
        remember_fixture(&server, &snapshot, 0).await;
        server
            .app_state
            .session_authorizations
            .record_failure(
                &snapshot.session_id,
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "fixture closed".into(),
                    suggested_action: None,
                },
                at,
            )
            .await;
        granted_audit_session(&server, "audit-reused-id-prune", at + 600_001).await;
        let reused = granted_audit_session(&server, "audit-reused-id", at + 600_001).await;
        server.observe_owned_product_session(&reused).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await,
            "ownership belongs to one aggregate, not every reuse of its ID"
        );
    }

    #[tokio::test]
    async fn audit_owner_query_excludes_history_before_this_aggregate() {
        let server = audit_test_server(1);
        let id = mrd_proto::SessionId("audit-history".into());
        server
            .app_state
            .audit_log
            .record(
                "old.aggregate",
                "allowed",
                Some(id.clone()),
                None,
                None,
                None,
                None,
                vec![],
            )
            .unwrap();
        let snapshot = granted_audit_session(&server, &id.0, now_ms()).await;
        remember_fixture(&server, &snapshot, 1).await;
        server
            .app_state
            .audit_log
            .record(
                "new.aggregate",
                "allowed",
                Some(id.clone()),
                None,
                None,
                None,
                None,
                vec![],
            )
            .unwrap();
        let response = server.read_product_audit(audit_query(&id)).await;
        let IpcResponse::AuditEventsV2 { page } = response else {
            panic!("owned audit query failed: {response:?}");
        };
        assert_eq!(
            page.events.len(),
            1,
            "a reused ID does not expose its previous aggregate history"
        );
        assert_eq!(page.events[0].action, "new.aggregate");
    }

    #[tokio::test]
    async fn audit_owner_successful_start_preserves_read_after_cleanup() {
        let server = audit_test_server(1);
        let at = now_ms();
        let snapshot = granted_audit_session(&server, "audit-closed", at).await;
        remember_fixture(&server, &snapshot, 0).await;
        assert!(
            server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await
        );
        let closed = server
            .app_state
            .session_authorizations
            .record_failure(
                &snapshot.session_id,
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "fixture cleanup".into(),
                    suggested_action: None,
                },
                at + 1,
            )
            .await
            .unwrap();
        assert!(closed.granted_scopes.is_empty());
        server.observe_owned_product_session(&closed).await;
        assert!(
            server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await,
            "cleanup retains only this owner's previously authorized evidence"
        );
    }

    #[tokio::test]
    async fn audit_owner_new_birth_belongs_only_to_the_new_logon() {
        let server = audit_test_server(1);
        let at = now_ms();
        let original = granted_audit_session(&server, "audit-new-owner", at).await;
        remember_fixture(&server, &original, 0).await;
        server
            .app_state
            .session_authorizations
            .record_failure(
                &original.session_id,
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "fixture cleanup".into(),
                    suggested_action: None,
                },
                at,
            )
            .await;
        granted_audit_session(&server, "audit-new-owner-prune", at + 600_001).await;
        let mut new_user = server.clone();
        new_user.product_caller.as_mut().unwrap().logon_sid_hash = [2; 32];
        let current = granted_audit_session(&new_user, "audit-new-owner", at + 600_001).await;
        remember_fixture(&new_user, &current, 0).await;
        assert!(
            new_user
                .product_audit_is_allowed(&audit_query(&current.session_id))
                .await
        );
        server.observe_owned_product_session(&current).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&current.session_id))
                .await
        );
        assert!(
            new_user
                .product_audit_is_allowed(&audit_query(&current.session_id))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_response_must_prove_an_exact_successful_new_birth() {
        let server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-success-proof", now_ms()).await;
        let success = IpcResponse::RemoteSessionRequested {
            session: snapshot.snapshot.clone(),
        };
        let same_instance = server
            .app_state
            .session_authorizations
            .product_audit_binding(&snapshot.session_id)
            .await
            .unwrap()
            .0;
        for (id, result, prior) in [
            (
                snapshot.session_id.clone(),
                IpcResponse::Error {
                    code: "denied".into(),
                    message: "fixture rejection".into(),
                },
                Some(ProductAuditOwnerProof::Outgoing(snapshot.receipt.clone())),
            ),
            (
                mrd_proto::SessionId("different-id".into()),
                success.clone(),
                Some(ProductAuditOwnerProof::Outgoing(snapshot.receipt.clone())),
            ),
            (
                snapshot.session_id.clone(),
                success,
                Some(ProductAuditOwnerProof::Consent(same_instance)),
            ),
        ] {
            server
                .remember_product_response(
                    &id,
                    &result,
                    mrd_ipc::RemoteSessionRole::Controller,
                    Some(0),
                    prior,
                )
                .await;
            assert!(server.product_audit_owners.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn audit_owner_query_enforces_exact_session_bounds_and_cursor_floor() {
        let server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-bounds", now_ms()).await;
        remember_fixture(&server, &snapshot, 17).await;
        for requested in [None, Some(0), Some(16), Some(17), Some(23)] {
            let mut query = audit_query(&snapshot.session_id);
            query.after_sequence = requested.map(mrd_ipc::DecimalU64::new);
            let (normalized, _) = server.product_audit_query(&query).await.unwrap();
            assert_eq!(
                normalized.after_sequence.unwrap().get(),
                requested.unwrap_or(0).max(17)
            );
        }
        for limit in [0, 65, 1000] {
            let mut query = audit_query(&snapshot.session_id);
            query.limit = limit;
            assert!(!server.product_audit_is_allowed(&query).await);
        }
        let mut global = audit_query(&snapshot.session_id);
        global.session_id = None;
        assert!(!server.product_audit_is_allowed(&global).await);
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&mrd_proto::SessionId("not-owned".into())))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_wrong_windows_session_cannot_read() {
        let mut server = audit_test_server(1);
        let snapshot = granted_audit_session(&server, "audit-wrong-session", now_ms()).await;
        remember_fixture(&server, &snapshot, 0).await;
        server.product_caller.as_mut().unwrap().windows_session_id += 1;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_unobserved_screen_permission_cannot_read() {
        let server = audit_test_server(1);
        let at = now_ms();
        let mut snapshot = granted_audit_session(&server, "audit-unobserved-screen", at).await;
        let closed = server
            .app_state
            .session_authorizations
            .record_failure(
                &snapshot.session_id,
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "fixture cleanup".into(),
                    suggested_action: None,
                },
                at,
            )
            .await
            .unwrap();
        snapshot.snapshot = closed;
        remember_fixture(&server, &snapshot, 0).await;
        assert!(
            !server
                .product_audit_is_allowed(&audit_query(&snapshot.session_id))
                .await
        );
    }

    #[tokio::test]
    async fn audit_owner_instance_change_during_a_read_discards_the_entire_page() {
        let server = audit_test_server(1);
        let at = now_ms();
        let original = granted_audit_session(&server, "audit-read-race", at).await;
        remember_fixture(&server, &original, 0).await;
        server
            .app_state
            .audit_log
            .record(
                "session.test",
                "allowed",
                Some(original.session_id.clone()),
                None,
                None,
                None,
                None,
                vec![],
            )
            .unwrap();
        let result = server
            .read_product_audit_with(audit_query(&original.session_id), |normalized| {
                let server = &server;
                let original = &original;
                async move {
                    assert!(
                        server
                            .app_state
                            .authorization_security_gate
                            .try_lock()
                            .is_err(),
                        "normal aggregate births are fenced for the complete read"
                    );
                    let page = server.app_state.audit_log.query_v2(&normalized).unwrap();
                    assert_eq!(page.events.len(), 1);
                    // Isolated domain fixture models a caller outside the normal gate.
                    server
                        .app_state
                        .session_authorizations
                        .record_failure(
                            &original.session_id,
                            mrd_ipc::RemoteAuthorizationState::Revoked,
                            mrd_ipc::RemoteFailure {
                                code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                                message: "fixture cleanup".into(),
                                suggested_action: None,
                            },
                            at,
                        )
                        .await;
                    granted_audit_session(server, "audit-read-race-prune", at + 600_001).await;
                    granted_audit_session(server, "audit-read-race", at + 600_001).await;
                    IpcResponse::AuditEventsV2 { page }
                }
            })
            .await;
        assert!(
            matches!(result, IpcResponse::Error { code, .. } if code == "E_PRODUCT_AUDIT_SESSION_DENIED"),
            "a page cannot escape after its authoritative aggregate was replaced"
        );
    }

    #[tokio::test]
    async fn audit_owner_consent_success_requires_the_exact_preconsent_instance() {
        let server = audit_test_server(1);
        let at = now_ms();
        let id = mrd_proto::SessionId("audit-consent".into());
        let peer =
            mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
        let scopes = vec![RemotePermissionScope::ScreenView];
        server
            .app_state
            .session_authorizations
            .begin_verified_incoming(
                crate::session_authorization::VerifiedIncomingAuthorizationRequest {
                    session_id: id.clone(),
                    peer_device_id: mrd_proto::DeviceId("fixture-controller".into()),
                    peer_key_id: peer.key_id().to_owned(),
                    peer_key_epoch: 1,
                    access_mode: mrd_ipc::RemoteAccessMode::Attended,
                    requested_scopes: scopes.clone(),
                    peer_permission_ceiling: scopes.clone(),
                    machine_permission_ceiling: scopes.clone(),
                    runtime_capabilities: scopes.clone(),
                    transport_kind: "quic".into(),
                    request_nonce: [8; 16],
                    created_at_ms: at,
                    expires_at_ms: at + 5000,
                },
            )
            .await
            .unwrap();
        let instance = server
            .app_state
            .session_authorizations
            .product_audit_binding(&id)
            .await
            .unwrap()
            .0;
        // Domain fixture only; real dispatch must first obtain Agent native consent.
        let approved = server
            .app_state
            .session_authorizations
            .respond_to_consent_with_audit(
                mrd_ipc::ConsentResponse {
                    session_id: id.clone(),
                    decision: mrd_ipc::ConsentDecision::Approve,
                    approved_scopes: scopes,
                    expected_policy_revision: mrd_ipc::DecimalU64::new(1),
                },
                at,
                |_, _| {
                    server
                        .app_state
                        .audit_log
                        .record(
                            "session.consent",
                            "allowed",
                            Some(id.clone()),
                            None,
                            None,
                            None,
                            None,
                            vec![],
                        )
                        .is_ok()
                },
            )
            .await
            .unwrap();
        let response = IpcResponse::ConsentRecorded { session: approved };
        server
            .remember_product_response(
                &id,
                &response,
                mrd_ipc::RemoteSessionRole::Agent,
                Some(0),
                Some(ProductAuditOwnerProof::Consent(instance + 1)),
            )
            .await;
        assert!(server.product_audit_owners.lock().unwrap().is_empty());
        server
            .remember_product_response(
                &id,
                &response,
                mrd_ipc::RemoteSessionRole::Agent,
                Some(0),
                Some(ProductAuditOwnerProof::Consent(instance)),
            )
            .await;
        assert!(server.product_audit_is_allowed(&audit_query(&id)).await);
    }

    #[tokio::test]
    async fn audit_owner_old_success_response_cannot_claim_an_identical_later_birth() {
        let server = audit_test_server(1);
        let at = now_ms();
        let peer =
            mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
        let first =
            granted_audit_session_for_peer(&server, "audit-birth-receipt-race", at, &peer).await;
        let first_instance = server
            .app_state
            .session_authorizations
            .product_audit_binding(&first.session_id)
            .await
            .unwrap()
            .0;
        let response = IpcResponse::RemoteSessionRequested {
            session: first.snapshot.clone(),
        };
        server
            .app_state
            .session_authorizations
            .record_failure(
                &first.session_id,
                mrd_ipc::RemoteAuthorizationState::Revoked,
                mrd_ipc::RemoteFailure {
                    code: mrd_ipc::RemoteReasonCode::GrantRevoked,
                    message: "fixture cleanup".into(),
                    suggested_action: None,
                },
                at,
            )
            .await;
        // A different real birth advances pruning; then model a rollback/frozen
        // wall clock. Both old and new snapshots have exactly the same tuple.
        granted_audit_session_for_peer(&server, "audit-prune-trigger", at + 600_001, &peer).await;
        let second =
            granted_audit_session_for_peer(&server, "audit-birth-receipt-race", at, &peer).await;
        assert_eq!(first.snapshot, second.snapshot);
        assert_ne!(
            first_instance,
            server
                .app_state
                .session_authorizations
                .product_audit_binding(&second.session_id)
                .await
                .unwrap()
                .0
        );
        server
            .remember_product_response(
                &first.session_id,
                &response,
                mrd_ipc::RemoteSessionRole::Controller,
                Some(0),
                Some(ProductAuditOwnerProof::Outgoing(first.receipt.clone())),
            )
            .await;
        assert!(
            server.product_audit_owners.lock().unwrap().is_empty(),
            "a public success snapshot cannot prove which private birth this request created"
        );
    }

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
