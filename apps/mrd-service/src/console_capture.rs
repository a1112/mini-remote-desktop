//! Consent and encoded capture bound to one exact local Windows desktop.
//! The resident never captures Session 0 or chooses a replacement Agent implicitly.

use crate::{
    agent_runtime::{
        AgentBinding, ExecuteGrantIssuer, ExecuteGrantTemplate, ObservedAgentIdentity,
    },
    AppState,
};
use anyhow::{anyhow, ensure, Context, Result};
use mrd_agent_ipc::{
    AgentCapability, AgentCaptureProfile, AgentCommand, CaptureSourceBounds, CommandOutcome,
    DesktopKind, MediaAccessUnit, MediaCodec, PeerBinding,
};
use mrd_ipc::{
    ConsentDecision, ConsentResponse, MediaProfile, RemoteAccessMode, RemoteAuthorizationState,
    RemotePermissionScope, RemoteSessionRole,
};
use mrd_proto::SessionId;
use mrd_session::{PermissionScope, PermissionScopes};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

const MAX_CONSOLE_TARGETS: usize = 64;

#[derive(Clone)]
struct Target {
    binding: AgentBinding,
    logon_hash: [u8; 32],
    authorization_expires_at_ms: u64,
    resource: Option<Resource>,
    input: HashMap<PermissionScope, InputResource>,
    closing: bool,
}

#[derive(Clone)]
struct InputResource {
    binding: AgentBinding,
    id: [u8; 16],
    start_grant_id: [u8; 32],
    peer: PeerBinding,
    policy_revision: u64,
    scopes: PermissionScopes,
    expires_at_ms: u64,
    last_sequence: u64,
    cleanup_pending: bool,
    stop_command: Arc<Mutex<Option<mrd_agent_ipc::ExecuteCommand>>>,
}

#[derive(Clone)]
struct Resource {
    id: [u8; 16],
    peer: PeerBinding,
    policy_revision: u64,
    expires_at_ms: u64,
    profile: MediaProfile,
    source_bounds: Option<CaptureSourceBounds>,
    stop_command: Arc<Mutex<Option<mrd_agent_ipc::ExecuteCommand>>>,
}

#[derive(Default)]
pub struct ConsoleCaptureState {
    targets: Mutex<HashMap<SessionId, Target>>,
    issuer: RwLock<Option<Arc<ExecuteGrantIssuer>>>,
    operation: Mutex<()>,
    cleanup_cursor: std::sync::atomic::AtomicUsize,
}

impl ConsoleCaptureState {
    pub(crate) fn is_enabled(&self) -> bool {
        // An unavailable authority lock still selects the strict Agent route;
        // issuer() then fails closed instead of enabling a resident fallback.
        self.issuer
            .read()
            .map(|issuer| issuer.is_some())
            .unwrap_or(true)
    }
    pub(crate) fn bind_issuer(&self, issuer: Arc<ExecuteGrantIssuer>) {
        *self.issuer.write().expect("capture issuer lock") = Some(issuer);
    }

    fn issuer(&self) -> Result<Arc<ExecuteGrantIssuer>> {
        self.issuer
            .read()
            .map_err(|_| anyhow!("Capture authority unavailable"))?
            .clone()
            .context("Capture authority unavailable")
    }

    /// Called only from the already kernel-authenticated product consent path.
    pub(crate) async fn prepare_consent(
        &self,
        state: &AppState,
        caller: &ObservedAgentIdentity,
        response: &ConsentResponse,
    ) -> Result<()> {
        if response.decision != ConsentDecision::Approve
            || !response.approved_scopes.iter().any(|scope| {
                matches!(
                    scope,
                    RemotePermissionScope::ScreenView
                        | RemotePermissionScope::InputPointer
                        | RemotePermissionScope::InputKeyboard
                )
            })
        {
            return Ok(());
        }
        let _operation = self.operation.lock().await;
        let snapshot = state
            .session_authorizations
            .snapshot(&response.session_id)
            .await
            .context("Consent request no longer exists")?;
        ensure!(
            snapshot.role == RemoteSessionRole::Agent
                && snapshot.access_mode == RemoteAccessMode::Attended,
            "Consent is not for this target desktop"
        );
        ensure!(
            snapshot.authorization_state == RemoteAuthorizationState::AwaitingLocalConsent,
            "Consent is no longer pending"
        );
        ensure!(
            snapshot.policy_revision == response.expected_policy_revision,
            "Consent policy changed"
        );
        let active = active_console()?;
        ensure!(
            caller.windows_session_id == active,
            "Consent caller is not in the active console"
        );
        let observed = state
            .agent_registry
            .active_for_session_at(active, now_ms())
            .context("No capture Agent owns this console")?;
        ensure!(
            observed.identity.logon_sid_hash == caller.logon_sid_hash,
            "Capture Agent belongs to another logon"
        );
        ensure!(
            observed.identity.protocol_minor
                >= mrd_agent_ipc::AGENT_IPC_INPUT_DEADLINE_PROTOCOL_MINOR,
            "Capture Agent must be updated"
        );
        ensure!(
            response
                .approved_scopes
                .iter()
                .all(|scope| snapshot.requested_scopes.contains(scope)),
            "Consent cannot expand requested permissions"
        );
        let binding =
            state
                .agent_registry
                .bind_active_session(active, AgentCapability::Capture, now_ms())?;
        {
            let mut targets = self.targets.lock().await;
            targets.retain(|_, target| {
                target.resource.is_some()
                    || !target.input.is_empty()
                    || target.authorization_expires_at_ms > now_ms()
            });
            ensure!(
                targets.len() < MAX_CONSOLE_TARGETS || targets.contains_key(&response.session_id),
                "Capture session limit reached"
            );
            if let Some(existing) = targets.get(&response.session_id) {
                ensure!(
                    !existing.closing
                        && !existing.input.values().any(|input| input.cleanup_pending),
                    "Desktop cleanup is still pending"
                );
                ensure!(
                    existing.binding == binding && existing.logon_hash == caller.logon_sid_hash,
                    "Consent cannot retarget an existing desktop"
                );
            }
        }
        let now = now_ms();
        let expires = snapshot
            .authorization_expires_at_ms
            .context("Consent deadline missing")?
            .min(now.saturating_add(15_000));
        ensure!(expires > now, "Consent request expired");
        let native = state
            .console_agent_server()
            .context("Capture Agent server unavailable")?
            .request_consent(mrd_agent_ipc::ConsentRequest {
                request_token: 1,
                request_id: random_id()?,
                session_id: response.session_id.clone(),
                peer: PeerBinding {
                    device_id: snapshot.peer_device_id.clone(),
                    key_id: decode_peer_key(&snapshot.peer_key_id)?,
                },
                requested_scopes: response
                    .approved_scopes
                    .iter()
                    .copied()
                    .map(PermissionScope::from)
                    .collect(),
                policy_revision: snapshot.policy_revision.get(),
                windows_session_id: active,
                issued_at_ms: now,
                expires_at_ms: expires,
                authorization_expires_at_ms: now
                    .saturating_add(mrd_agent_ipc::AGENT_CONSENT_MAX_LIFETIME_MS),
            })
            .await?;
        ensure!(
            native.consent().decision() == mrd_agent_ipc::ConsentDecision::Approved,
            "Local desktop consent was not approved"
        );
        let approved: PermissionScopes = response
            .approved_scopes
            .iter()
            .copied()
            .map(PermissionScope::from)
            .collect();
        ensure!(
            native.consent().approved_scopes() == &approved,
            "Local desktop approval did not include all selected permissions"
        );
        let consent_binding = native.binding();
        ensure!(
            consent_binding.registration_id() == binding.registration_id()
                && consent_binding.registration_epoch() == binding.registration_epoch()
                && consent_binding.desktop_epoch() == binding.desktop_epoch()
                && consent_binding.windows_session_id() == binding.windows_session_id(),
            "Consent desktop changed while awaiting approval"
        );
        let target = Target {
            binding,
            logon_hash: caller.logon_sid_hash,
            authorization_expires_at_ms: native.consent().authorization_expires_at_ms(),
            resource: None,
            input: HashMap::new(),
            closing: false,
        };
        self.validate_target(state, &target)?;
        let mut targets = self.targets.lock().await;
        targets.retain(|_, target| {
            target.resource.is_some()
                || !target.input.is_empty()
                || target.authorization_expires_at_ms > now_ms()
        });
        ensure!(
            targets.len() < MAX_CONSOLE_TARGETS || targets.contains_key(&response.session_id),
            "Capture session limit reached"
        );
        if let Some(existing) = targets.get(&response.session_id) {
            ensure!(
                !existing.closing && !existing.input.values().any(|input| input.cleanup_pending),
                "Desktop cleanup is still pending"
            );
            ensure!(
                same_target(existing, &target),
                "Consent cannot retarget an existing desktop"
            );
        } else {
            targets.insert(response.session_id.clone(), target);
        }
        Ok(())
    }

    /// Resolve the approved exact target, start its capture resource, and keep
    /// the grant metadata for a bounded, command-specific stop at cleanup.
    pub(crate) async fn start(
        &self,
        state: &AppState,
        session_id: &SessionId,
        profile: &MediaProfile,
    ) -> Result<()> {
        let _operation = self.operation.lock().await;
        let snapshot = state
            .session_authorizations
            .snapshot(session_id)
            .await
            .context("Capture session missing")?;
        ensure!(
            snapshot.role == RemoteSessionRole::Agent
                && snapshot.authorization_state == RemoteAuthorizationState::Granted,
            "Capture requires target authorization"
        );
        ensure!(
            state
                .session_authorizations
                .allows_scope(session_id, RemotePermissionScope::ScreenView, now_ms())
                .await,
            "Capture requires a current screen.view grant"
        );
        let grant = state
            .session_authorizations
            .active_grant(session_id)
            .await
            .context("Capture grant missing")?;
        ensure!(grant.expires_at_ms > now_ms(), "Capture grant expired");
        ensure!(
            profile.codec.eq_ignore_ascii_case("h264"),
            "The installed capture Agent supports H264"
        );
        let mut target = self
            .targets
            .lock()
            .await
            .get(session_id)
            .cloned()
            .context("Attended consent did not bind a target desktop")?;
        self.validate_target(state, &target)?;
        ensure!(
            !target.closing && !target.input.values().any(|input| input.cleanup_pending),
            "Desktop cleanup is still pending"
        );
        if let Some(resource) = &target.resource {
            ensure!(
                resource.profile == *profile,
                "Changing the active capture profile requires a new approved session"
            );
            return Ok(());
        }
        let peer_key: [u8; 32] = decode_peer_key(&snapshot.peer_key_id)?;
        let peer = PeerBinding {
            device_id: snapshot.peer_device_id,
            key_id: peer_key,
        };
        let resource = Resource {
            id: random_id()?,
            peer,
            policy_revision: grant.policy_revision,
            expires_at_ms: grant.expires_at_ms.min(target.authorization_expires_at_ms),
            profile: profile.clone(),
            source_bounds: None,
            stop_command: Arc::new(Mutex::new(None)),
        };
        let now = now_ms();
        let template = grant_template(
            &target.binding,
            session_id,
            &resource,
            now,
            resource.expires_at_ms.min(now.saturating_add(60_000)),
        )?;
        let execute = self.issuer()?.issue(
            random_id()?,
            random_id()?,
            AgentCommand::StartCapture {
                resource_id: resource.id,
                display_id: 0,
                profile: Some(AgentCaptureProfile {
                    width: profile.width,
                    height: profile.height,
                    fps: profile.fps,
                    bitrate_bps: profile.bitrate_mbps.saturating_mul(1_000_000),
                }),
            },
            template.clone(),
        )?;
        *resource.stop_command.lock().await = Some(self.issuer()?.issue(
            random_id()?,
            random_id()?,
            AgentCommand::StopCapture {
                resource_id: resource.id,
            },
            template.with_scopes(PermissionScopes::new()),
        )?);
        // Reserve ownership before any frame can arrive. The loop remains on
        // the Agent path even when no first frame has been produced yet.
        target.resource = Some(resource.clone());
        self.targets
            .lock()
            .await
            .insert(session_id.clone(), target.clone());
        state.agent_media_ingress.lock().await.reserve_resource(
            &session_id.0,
            crate::agent_runtime::AdmittedMediaResource {
                resource_id: resource.id,
                registration_id: *target.binding.registration_id(),
                registration_epoch: target.binding.registration_epoch(),
                windows_session_id: target.binding.windows_session_id(),
                desktop_epoch: target.binding.desktop_epoch(),
            },
        );
        let server = state
            .console_agent_server()
            .context("Capture Agent server unavailable")?;
        let result = server.request_execute(&target.binding, execute).await;
        if !matches!(result, Ok(ref outcome) if outcome.outcome == CommandOutcome::Completed) {
            self.stop_inner(state, session_id).await;
            return match result {
                Ok(outcome) => Err(anyhow!(
                    "Capture Agent rejected the approved capture resource: {:?}",
                    outcome.outcome
                )),
                Err(reason) => Err(anyhow!(reason).context("Capture Agent request failed")),
            };
        }
        if let Err(error) = self.validate_target(state, &target) {
            self.stop_inner(state, session_id).await;
            return Err(error);
        }
        Ok(())
    }

    fn validate_target(&self, state: &AppState, target: &Target) -> Result<()> {
        ensure!(
            now_ms() < target.authorization_expires_at_ms,
            "Local desktop approval expired"
        );
        ensure!(
            active_console()? == target.binding.windows_session_id(),
            "The approved console changed"
        );
        let current = state
            .agent_registry
            .active_for_session_at(target.binding.windows_session_id(), now_ms())
            .context("Capture Agent disconnected")?;
        ensure!(
            current.identity.logon_sid_hash == target.logon_hash,
            "The approved desktop logon changed"
        );
        state
            .agent_registry
            .resolve_exact(&target.binding, AgentCapability::Capture, now_ms())?;
        Ok(())
    }

    /// Drain only the exact resource authorized for this product session.
    pub(crate) async fn drain(
        &self,
        state: &AppState,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<MediaAccessUnit>> {
        let _operation = self.operation.lock().await;
        let target = self
            .targets
            .lock()
            .await
            .get(session_id)
            .cloned()
            .context("Capture target missing")?;
        self.validate_target(state, &target)?;
        ensure!(!target.closing, "Desktop cleanup is pending");
        let resource = target
            .resource
            .as_ref()
            .context("Capture resource not started")?;
        ensure!(
            now_ms() < resource.expires_at_ms
                && state
                    .session_authorizations
                    .allows_scope(session_id, RemotePermissionScope::ScreenView, now_ms())
                    .await,
            "Capture authorization expired or revoked"
        );
        let units = state
            .agent_media_ingress
            .lock()
            .await
            .drain_session(&session_id.0, limit);
        let mut source_bounds = resource.source_bounds;
        for unit in &units {
            ensure!(
                frame_matches_target(unit, session_id, &target.binding, resource.id),
                "Capture frame belongs to another resource or desktop"
            );
            let bounds = unit
                .source_bounds
                .filter(CaptureSourceBounds::is_valid)
                .context("Capture frame has no authenticated source geometry")?;
            ensure!(
                source_bounds.is_none_or(|previous| previous == bounds),
                "Captured display geometry changed"
            );
            source_bounds = Some(bounds);
        }
        if let Some(current) = self.targets.lock().await.get_mut(session_id) {
            if let Some(current_resource) = current
                .resource
                .as_mut()
                .filter(|current| current.id == resource.id)
            {
                current_resource.source_bounds = source_bounds;
            }
        }
        Ok(units)
    }

    pub(crate) async fn stop(&self, state: &AppState, session_id: &SessionId) {
        let _operation = self.operation.lock().await;
        self.stop_inner(state, session_id).await;
    }

    /// Retry one pending exact resource owner per reconciliation tick. Cleanup
    /// never creates a resource, chooses a replacement Agent, or resumes input.
    pub async fn retry_pending_cleanup(&self, state: &AppState) {
        let mut pending: Vec<_> = self
            .targets
            .lock()
            .await
            .iter()
            .filter(|(_, target)| {
                target.closing || target.input.values().any(|input| input.cleanup_pending)
            })
            .map(|(session, target)| (session.clone(), target.closing))
            .collect();
        pending.sort_by(|left, right| left.0 .0.cmp(&right.0 .0));
        if pending.is_empty() {
            return;
        }
        let cursor = self
            .cleanup_cursor
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (session, closing) = &pending[cursor % pending.len()];
        if *closing {
            self.stop(state, session).await;
        } else if let Err(reason) = self.stop_input(state, session).await {
            tracing::warn!("Desktop input cleanup retry remains pending: {reason:#}");
        }
    }

    async fn stop_inner(&self, state: &AppState, session_id: &SessionId) {
        // Freeze before sending cleanup. A lost reply never loses ownership of
        // a potentially active resource and cannot enable a replacement Agent.
        let target = {
            let mut targets = self.targets.lock().await;
            targets.get_mut(session_id).map(|target| {
                target.closing = true;
                for input in target.input.values_mut() {
                    input.cleanup_pending = true;
                }
                target.clone()
            })
        };
        state
            .agent_media_ingress
            .lock()
            .await
            .remove_session(&session_id.0);
        let Some(target) = target else {
            return;
        };
        for (scope, input) in &target.input {
            match self
                .stop_input_resource(state, session_id, input, target.authorization_expires_at_ms)
                .await
            {
                Ok(()) => {
                    if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                        current.input.remove(scope);
                    }
                }
                Err(reason) => {
                    tracing::warn!("Desktop input cleanup was not acknowledged: {reason:#}")
                }
            }
        }
        if let Some(resource) = &target.resource {
            match self
                .stop_capture_resource(
                    state,
                    session_id,
                    &target.binding,
                    resource,
                    target.authorization_expires_at_ms,
                )
                .await
            {
                Ok(()) => {
                    if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                        current.resource = None;
                    }
                }
                Err(reason) => {
                    tracing::warn!("Desktop capture cleanup was not acknowledged: {reason:#}")
                }
            }
        }
        let mut targets = self.targets.lock().await;
        if targets
            .get(session_id)
            .is_some_and(|current| current.resource.is_none() && current.input.is_empty())
        {
            targets.remove(session_id);
        }
    }

    async fn stop_capture_resource(
        &self,
        state: &AppState,
        session_id: &SessionId,
        binding: &AgentBinding,
        resource: &Resource,
        native_expiry: u64,
    ) -> Result<()> {
        // Cleanup was signed at startup against the original valid authority;
        // retries preserve that command identity even after expiry.
        let execute = resource
            .stop_command
            .lock()
            .await
            .clone()
            .context("Approved capture cleanup command missing")?;
        ensure!(
            execute.grant.claims.session_id == *session_id
                && execute.grant.claims.expires_at_ms <= native_expiry,
            "Capture cleanup owner changed"
        );
        let server = state
            .console_agent_server()
            .context("Capture Agent server unavailable")?;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            server.request_execute(binding, execute),
        )
        .await
        .context("Capture cleanup acknowledgement timed out")??;
        ensure!(
            matches!(
                result.outcome,
                CommandOutcome::Completed | CommandOutcome::AlreadyStopped
            ),
            "The approved desktop did not acknowledge capture cleanup"
        );
        Ok(())
    }

    /// Called only after the network receiver verifies its envelope, replay
    /// sequence, scope and current authoritative policy. The same sequence is
    /// retained at the exact desktop Agent and acknowledged by its input backend.
    pub(crate) async fn apply_input(
        &self,
        state: &AppState,
        session_id: &SessionId,
        scope: RemotePermissionScope,
        remote_sequence: u64,
        remote_expires_at_ms: u64,
        event: mrd_agent_ipc::InputEventPayload,
    ) -> Result<Option<mrd_agent_ipc::InputAck>> {
        let _operation = self.operation.lock().await;
        ensure!(remote_sequence != 0, "Signed remote input sequence invalid");
        let cleanup = event == mrd_agent_ipc::InputEventPayload::ReleaseAll;
        ensure!(
            cleanup || now_ms() < remote_expires_at_ms,
            "Signed remote input expired"
        );
        let input_scope = match scope {
            RemotePermissionScope::InputPointer => PermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard => PermissionScope::InputKeyboard,
            _ => anyhow::bail!("Signed input scope invalid"),
        };
        let mut target = self
            .targets
            .lock()
            .await
            .get(session_id)
            .cloned()
            .context("Input has no approved local desktop")?;
        self.validate_target(state, &target)?;
        ensure!(
            !target.closing && !target.input.values().any(|input| input.cleanup_pending),
            "Desktop input cleanup is pending"
        );
        if event != mrd_agent_ipc::InputEventPayload::ReleaseAll {
            ensure!(
                event.required_scope() == Some(input_scope),
                "Input payload differs from its signed scope"
            );
            ensure!(
                state
                    .session_authorizations
                    .allows_scope(session_id, scope, now_ms())
                    .await,
                "Current session grant denies this input"
            );
        }
        let snapshot = state
            .session_authorizations
            .snapshot(session_id)
            .await
            .context("Input authorization missing")?;
        ensure!(
            snapshot.role == RemoteSessionRole::Agent
                && snapshot.authorization_state == RemoteAuthorizationState::Granted,
            "Input requires a target grant"
        );
        if event == mrd_agent_ipc::InputEventPayload::ReleaseAll
            && !target.input.contains_key(&input_scope)
        {
            return Ok(None);
        }
        let event = map_pointer_to_capture_source(event, target.resource.as_ref())?;
        let server = state
            .console_agent_server()
            .context("Input Agent server unavailable")?;
        if !target.input.contains_key(&input_scope) {
            ensure!(
                snapshot.granted_scopes.contains(&scope),
                "Input was not approved"
            );
            let scopes: PermissionScopes = [input_scope].into_iter().collect();
            let grant = state
                .session_authorizations
                .active_grant(session_id)
                .await
                .context("Input grant missing")?;
            let binding = state.agent_registry.bind_active_session(
                target.binding.windows_session_id(),
                AgentCapability::Input,
                now_ms(),
            )?;
            ensure!(
                same_registration(&binding, &target.binding),
                "Input Agent differs from the approved desktop"
            );
            let input = InputResource {
                binding,
                id: random_id()?,
                start_grant_id: random_id()?,
                peer: PeerBinding {
                    device_id: snapshot.peer_device_id.clone(),
                    key_id: decode_peer_key(&snapshot.peer_key_id)?,
                },
                policy_revision: grant.policy_revision,
                scopes,
                expires_at_ms: grant.expires_at_ms.min(target.authorization_expires_at_ms),
                last_sequence: 0,
                cleanup_pending: false,
                stop_command: Arc::new(Mutex::new(None)),
            };
            let now = now_ms();
            let template = ExecuteGrantTemplate::for_binding(
                &input.binding,
                session_id.clone(),
                input.peer.clone(),
                input.scopes.clone(),
                input.policy_revision,
                DesktopKind::Default,
                now,
                now,
                input.expires_at_ms,
            )?;
            let execute = self.issuer()?.issue(
                random_id()?,
                input.start_grant_id,
                AgentCommand::StartInput {
                    resource_id: input.id,
                    input_scopes: input.scopes.clone(),
                },
                template.clone(),
            )?;
            *input.stop_command.lock().await = Some(self.issuer()?.issue(
                random_id()?,
                random_id()?,
                AgentCommand::StopInput {
                    resource_id: input.id,
                },
                template.with_scopes(PermissionScopes::new()),
            )?);
            // Retain cleanup ownership before dispatch; a timed-out reply may
            // still represent an already-created resource in the exact Agent.
            target.input.insert(input_scope, input.clone());
            self.targets
                .lock()
                .await
                .insert(session_id.clone(), target.clone());
            let started = server.request_execute(&input.binding, execute).await;
            if !matches!(started, Ok(ref result) if result.outcome == CommandOutcome::Completed) {
                self.cleanup_failed_input(
                    state,
                    session_id,
                    input_scope,
                    &input,
                    target.authorization_expires_at_ms,
                )
                .await;
                anyhow::bail!("The approved desktop rejected input startup");
            }
        }
        self.validate_target(state, &target)?;
        ensure!(
            cleanup
                || state
                    .session_authorizations
                    .allows_scope(session_id, scope, now_ms())
                    .await,
            "Input grant expired while awaiting the Agent"
        );
        let input = target
            .input
            .get_mut(&input_scope)
            .context("Input resource unavailable")?;
        // The authenticated outer sequence is independent per network lane.
        // Reserve before dispatch: an interrupted reply may still have applied
        // its input, so a future event must never reuse that local sequence.
        let sequence = input
            .last_sequence
            .checked_add(1)
            .context("Input sequence exhausted")?;
        ensure!(
            now_ms() < input.expires_at_ms,
            "Input authorization expired"
        );
        if let Some(scope) = event.required_scope() {
            ensure!(
                input.scopes.contains(&scope),
                "Input resource denies this operation"
            );
        }
        state
            .agent_registry
            .resolve_exact(&input.binding, AgentCapability::Input, now_ms())?;
        ensure!(
            cleanup || now_ms() < remote_expires_at_ms,
            "Signed remote input expired while awaiting the Agent"
        );
        input.last_sequence = sequence;
        let input = input.clone();
        self.targets
            .lock()
            .await
            .insert(session_id.clone(), target.clone());
        let result = server
            .request_input(
                &input.binding,
                mrd_agent_ipc::InputEventEnvelope {
                    request_token: 1,
                    session_id: session_id.clone(),
                    resource_id: input.id,
                    start_grant_id: input.start_grant_id,
                    sequence,
                    expires_at_ms: if cleanup {
                        remote_expires_at_ms
                    } else {
                        remote_expires_at_ms.min(input.expires_at_ms)
                    },
                    event,
                },
            )
            .await;
        if !matches!(result, Ok(ref ack) if ack.outcome == mrd_agent_ipc::InputAckOutcome::Applied)
        {
            self.cleanup_failed_input(
                state,
                session_id,
                input_scope,
                &input,
                target.authorization_expires_at_ms,
            )
            .await;
            anyhow::bail!("The approved desktop did not acknowledge this input");
        }
        let ack = result?;
        if let Err(reason) = self.validate_target(state, &target) {
            self.cleanup_failed_input(
                state,
                session_id,
                input_scope,
                &input,
                target.authorization_expires_at_ms,
            )
            .await;
            return Err(reason);
        }
        self.targets.lock().await.insert(session_id.clone(), target);
        Ok(Some(ack))
    }

    async fn cleanup_failed_input(
        &self,
        state: &AppState,
        session_id: &SessionId,
        scope: PermissionScope,
        input: &InputResource,
        native_expiry: u64,
    ) {
        if let Some(current) = self.targets.lock().await.get_mut(session_id) {
            if let Some(resource) = current.input.get_mut(&scope) {
                resource.cleanup_pending = true;
            }
        }
        match self
            .stop_input_resource(state, session_id, input, native_expiry)
            .await
        {
            Ok(()) => {
                if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                    current.input.remove(&scope);
                }
            }
            Err(reason) => tracing::warn!("Desktop input cleanup remains pending: {reason:#}"),
        }
    }

    pub(crate) async fn stop_input(&self, state: &AppState, session_id: &SessionId) -> Result<()> {
        let _operation = self.operation.lock().await;
        let target = self.targets.lock().await.get(session_id).cloned();
        if let Some(target) = target {
            let mut first_error = None;
            for (scope, input) in target.input.iter() {
                let result = self
                    .stop_input_resource(
                        state,
                        session_id,
                        &input,
                        target.authorization_expires_at_ms,
                    )
                    .await;
                match result {
                    Ok(()) => {
                        if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                            current.input.remove(scope);
                        }
                    }
                    Err(reason) => {
                        if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                            if let Some(input) = current.input.get_mut(scope) {
                                input.cleanup_pending = true;
                            }
                        }
                        first_error.get_or_insert(reason);
                    }
                }
            }
            if let Some(reason) = first_error {
                return Err(reason);
            }
        }
        Ok(())
    }

    async fn stop_input_resource(
        &self,
        state: &AppState,
        session_id: &SessionId,
        input: &InputResource,
        native_expiry: u64,
    ) -> Result<()> {
        // Pre-signed while the original authority was valid. Never create a
        // fresh validity interval after the authorization has expired.
        let execute = input
            .stop_command
            .lock()
            .await
            .clone()
            .context("Approved input cleanup command missing")?;
        ensure!(
            execute.grant.claims.session_id == *session_id
                && execute.grant.claims.expires_at_ms <= native_expiry,
            "Input cleanup owner changed"
        );
        let server = state
            .console_agent_server()
            .context("Input Agent server unavailable")?;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            server.request_execute(&input.binding, execute),
        )
        .await
        .context("Input cleanup acknowledgement timed out")??;
        ensure!(
            matches!(
                result.outcome,
                CommandOutcome::Completed | CommandOutcome::AlreadyStopped
            ),
            "The approved desktop did not acknowledge input cleanup"
        );
        Ok(())
    }
}

fn map_pointer_to_capture_source(
    event: mrd_agent_ipc::InputEventPayload,
    resource: Option<&Resource>,
) -> Result<mrd_agent_ipc::InputEventPayload> {
    let mrd_agent_ipc::InputEventPayload::MouseMove { x, y } = event else {
        return Ok(event);
    };
    let resource = resource.context("Pointer input requires a verified capture frame")?;
    let bounds = resource
        .source_bounds
        .filter(CaptureSourceBounds::is_valid)
        .context("Pointer input has no authenticated source geometry")?;
    ensure!(
        resource.profile.width > 0 && resource.profile.height > 0,
        "Capture profile geometry invalid"
    );
    let map = |value: i32, frame: u32, source: u32, origin: i32| -> i32 {
        let value = i64::from(value).clamp(0, i64::from(frame) - 1);
        let offset = if frame == 1 {
            0
        } else {
            value * (i64::from(source) - 1) / (i64::from(frame) - 1)
        };
        (i64::from(origin) + offset) as i32
    };
    Ok(mrd_agent_ipc::InputEventPayload::MouseMove {
        x: map(x, resource.profile.width, bounds.width, bounds.left),
        y: map(y, resource.profile.height, bounds.height, bounds.top),
    })
}

fn same_registration(left: &AgentBinding, right: &AgentBinding) -> bool {
    left.registration_id() == right.registration_id()
        && left.registration_epoch() == right.registration_epoch()
        && left.windows_session_id() == right.windows_session_id()
        && left.desktop_epoch() == right.desktop_epoch()
        && left.connection_id() == right.connection_id()
}

fn same_target(left: &Target, right: &Target) -> bool {
    left.binding == right.binding && left.logon_hash == right.logon_hash
}

fn frame_matches_target(
    unit: &MediaAccessUnit,
    session_id: &SessionId,
    binding: &AgentBinding,
    resource: [u8; 16],
) -> bool {
    unit.is_valid()
        && unit.session_id == session_id.0
        && unit.resource_id == resource
        && unit.codec == MediaCodec::H264
        && unit.context.registration_id == *binding.registration_id()
        && unit.context.registration_epoch == binding.registration_epoch()
        && unit.context.windows_session_id == binding.windows_session_id()
        && unit.context.desktop_epoch == binding.desktop_epoch()
}

fn grant_template(
    binding: &AgentBinding,
    session_id: &SessionId,
    resource: &Resource,
    now: u64,
    expires: u64,
) -> Result<ExecuteGrantTemplate> {
    let scopes: PermissionScopes = [PermissionScope::ScreenView].into_iter().collect();
    Ok(ExecuteGrantTemplate::for_binding(
        binding,
        session_id.clone(),
        resource.peer.clone(),
        scopes,
        resource.policy_revision,
        DesktopKind::Default,
        now,
        now,
        expires,
    )?)
}

pub(crate) fn active_console() -> Result<u32> {
    let session = unsafe { WTSGetActiveConsoleSessionId() };
    ensure!(
        session != 0 && session != u32::MAX,
        "No active local Windows desktop"
    );
    Ok(session)
}

fn random_id<const N: usize>() -> Result<[u8; N]> {
    use ring::rand::SecureRandom;
    let mut id = [0; N];
    ring::rand::SystemRandom::new()
        .fill(&mut id)
        .map_err(|_| anyhow!("Capture entropy unavailable"))?;
    ensure!(id != [0; N], "Capture entropy invalid");
    Ok(id)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn decode_peer_key(value: &str) -> Result<[u8; 32]> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Capture peer key invalid"
    );
    let mut key = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        key[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    ensure!(key != [0; 32], "Capture peer key invalid");
    Ok(key)
}

#[cfg(test)]
mod tests;
