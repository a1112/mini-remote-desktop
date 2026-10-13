//! Consent and encoded capture bound to one exact local Windows desktop.
//! The resident never captures Session 0 or chooses a replacement Agent implicitly.

use crate::{
    agent_runtime::{
        AgentBinding, AgentRequestError, AgentRouteError, ExecuteGrantIssuer, ExecuteGrantTemplate,
        ObservedAgentIdentity,
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

#[derive(Debug, Clone, Copy, thiserror::Error)]
enum CaptureStartFailure {
    #[error("capture startup guard failed")]
    Guard,
    #[error("capture Agent request failed")]
    Request,
    #[error("capture Agent rejected resource")]
    AckRejected,
    #[error("capture Agent could not start resource")]
    AckFailed,
    #[error("capture Agent returned an unexpected outcome")]
    AckUnexpected,
    #[error("capture desktop recheck failed")]
    DesktopRecheck,
}

pub(crate) fn capture_failure_diagnostic(
    reason: &anyhow::Error,
) -> crate::wan_diagnostics::TargetInstallDiagnostic {
    use crate::wan_diagnostics::{
        TargetInstallCode as Code, TargetInstallDiagnostic, TargetInstallStage as Stage,
    };
    let mut code = match reason.downcast_ref::<CaptureStartFailure>() {
        Some(CaptureStartFailure::AckRejected) => Code::AckRejected,
        Some(CaptureStartFailure::AckFailed) => Code::AckFailed,
        Some(CaptureStartFailure::AckUnexpected) => Code::UnexpectedAck,
        _ => Code::GuardRejected,
    };
    if matches!(
        reason.downcast_ref::<CaptureStartFailure>(),
        Some(CaptureStartFailure::Request)
    ) {
        if let Some(request) = reason.downcast_ref::<AgentRequestError>() {
            code = agent_request_diagnostic(request);
        }
    } else if let Some(route) = reason.downcast_ref::<AgentRouteError>() {
        code = paired_route_diagnostic(*route);
    }
    TargetInstallDiagnostic::new(Stage::TargetMediaCaptureStart, code)
}

fn agent_request_diagnostic(
    request: &AgentRequestError,
) -> crate::wan_diagnostics::TargetInstallCode {
    use crate::wan_diagnostics::TargetInstallCode as Code;
    match request {
        AgentRequestError::Route(route) => paired_route_diagnostic(*route),
        AgentRequestError::InvalidPairedAuthority => Code::InvalidPairedAuthority,
        AgentRequestError::InvalidInput(_) => Code::InvalidInput,
        AgentRequestError::InvalidConsent(_) => Code::InvalidConsent,
        AgentRequestError::InvalidRenderAccessUnit => Code::InvalidRenderAccessUnit,
        AgentRequestError::ConnectionUnavailable => Code::ConnectionUnavailable,
        AgentRequestError::OutboundUnavailable => Code::OutboundUnavailable,
        AgentRequestError::DuplicateRequest => Code::DuplicateRequest,
        AgentRequestError::CapacityExhausted => Code::CapacityExhausted,
        AgentRequestError::Revoked => Code::Revoked,
        AgentRequestError::Disconnected => Code::Disconnected,
        AgentRequestError::Timeout => Code::Timeout,
        AgentRequestError::StateUnavailable => Code::StateUnavailable,
    }
}

fn capture_ack_failure(outcome: CommandOutcome) -> Option<CaptureStartFailure> {
    match outcome {
        CommandOutcome::Completed => None,
        CommandOutcome::Rejected => Some(CaptureStartFailure::AckRejected),
        CommandOutcome::Failed => Some(CaptureStartFailure::AckFailed),
        CommandOutcome::AlreadyStopped => Some(CaptureStartFailure::AckUnexpected),
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub(crate) enum PairedDesktopPreparationError {
    #[error("paired policy revalidation failed")]
    PolicyVerification,
    #[error("paired snapshot unavailable")]
    SnapshotUnavailable,
    #[error("paired active grant state required")]
    GrantState,
    #[error("paired grant unavailable")]
    GrantUnavailable,
    #[error("paired screen scope required")]
    MissingScreenScope,
    #[error("paired active console unavailable")]
    NoActiveConsole,
    #[error("paired Agent unavailable")]
    AgentUnavailable,
    #[error("paired Agent protocol unavailable")]
    ProtocolOutdated,
    #[error("paired Agent capture route failed")]
    CaptureRoute,
    #[error("paired grant expired")]
    DeadlineExpired,
    #[error("paired authority entropy unavailable")]
    EntropyUnavailable,
    #[error("paired peer key invalid")]
    InvalidPeerKey,
    #[error("paired issuer unavailable")]
    IssuerUnavailable,
    #[error("paired authority issuance failed")]
    AuthorityIssue,
    #[error("paired Agent server unavailable")]
    AgentServerUnavailable,
    #[error("paired target capacity exhausted")]
    SessionCapacity,
    #[error("paired target already exists")]
    DuplicateTarget,
    #[error("paired Agent request failed")]
    AgentRequest,
    #[error("paired Agent rejected authority")]
    AgentAckRejected,
    #[error("paired Agent returned an unexpected outcome")]
    AgentAckUnexpected,
    #[error("paired target disappeared")]
    TargetMissing,
    #[error("paired desktop changed after installation")]
    DesktopRecheck,
    #[error("paired policy changed after installation")]
    PolicyRecheck,
}

pub(crate) fn paired_failure_diagnostic(
    reason: &anyhow::Error,
) -> crate::wan_diagnostics::TargetInstallDiagnostic {
    use crate::wan_diagnostics::{
        TargetInstallCode as Code, TargetInstallDiagnostic, TargetInstallStage as Stage,
    };
    use PairedDesktopPreparationError::*;
    let Some(failure) = reason.downcast_ref::<PairedDesktopPreparationError>() else {
        return TargetInstallDiagnostic::new(Stage::PairedUnknown, Code::GuardRejected);
    };
    let (stage, mut code) = match failure {
        PolicyVerification => (Stage::PairedPolicy, Code::GuardRejected),
        SnapshotUnavailable => (Stage::PairedSnapshot, Code::StateUnavailable),
        GrantState => (Stage::PairedGrant, Code::GuardRejected),
        GrantUnavailable => (Stage::PairedGrant, Code::StateUnavailable),
        MissingScreenScope => (Stage::PairedScope, Code::GuardRejected),
        NoActiveConsole => (Stage::PairedConsole, Code::SessionUnavailable),
        AgentUnavailable => (Stage::PairedAgentSelection, Code::SessionUnavailable),
        ProtocolOutdated => (Stage::PairedAgentProtocol, Code::ProtocolVersionUnavailable),
        CaptureRoute => (Stage::PairedAgentRoute, Code::GuardRejected),
        DeadlineExpired => (Stage::PairedGrant, Code::Expired),
        EntropyUnavailable => (Stage::PairedEntropy, Code::StateUnavailable),
        InvalidPeerKey => (Stage::PairedPeerKey, Code::InvalidPeerKey),
        IssuerUnavailable => (Stage::PairedIssuer, Code::StateUnavailable),
        AuthorityIssue => (Stage::PairedAuthorityIssue, Code::InvalidAuthority),
        AgentServerUnavailable => (Stage::PairedServer, Code::ConnectionUnavailable),
        SessionCapacity => (Stage::PairedTargetInsert, Code::CapacityExhausted),
        DuplicateTarget => (Stage::PairedTargetInsert, Code::DuplicateRequest),
        AgentRequest => (Stage::PairedAgentRequest, Code::GuardRejected),
        AgentAckRejected => (Stage::PairedAck, Code::AckRejected),
        AgentAckUnexpected => (Stage::PairedAck, Code::UnexpectedAck),
        TargetMissing => (Stage::PairedTargetRead, Code::StateUnavailable),
        DesktopRecheck => (Stage::PairedDesktopRecheck, Code::GuardRejected),
        PolicyRecheck => (Stage::PairedPolicyRecheck, Code::GuardRejected),
    };
    if matches!(failure, CaptureRoute | DesktopRecheck) {
        if let Some(route) = reason.downcast_ref::<AgentRouteError>() {
            code = paired_route_diagnostic(*route);
        }
    } else if matches!(failure, AgentRequest) {
        if let Some(request) = reason.downcast_ref::<AgentRequestError>() {
            code = match request {
                AgentRequestError::Route(route) => paired_route_diagnostic(*route),
                AgentRequestError::InvalidPairedAuthority => Code::InvalidPairedAuthority,
                AgentRequestError::InvalidInput(_) => Code::InvalidInput,
                AgentRequestError::InvalidConsent(_) => Code::InvalidConsent,
                AgentRequestError::InvalidRenderAccessUnit => Code::InvalidRenderAccessUnit,
                AgentRequestError::ConnectionUnavailable => Code::ConnectionUnavailable,
                AgentRequestError::OutboundUnavailable => Code::OutboundUnavailable,
                AgentRequestError::DuplicateRequest => Code::DuplicateRequest,
                AgentRequestError::CapacityExhausted => Code::CapacityExhausted,
                AgentRequestError::Revoked => Code::Revoked,
                AgentRequestError::Disconnected => Code::Disconnected,
                AgentRequestError::Timeout => Code::Timeout,
                AgentRequestError::StateUnavailable => Code::StateUnavailable,
            };
        }
    }
    TargetInstallDiagnostic::new(stage, code)
}

fn paired_route_diagnostic(route: AgentRouteError) -> crate::wan_diagnostics::TargetInstallCode {
    use crate::wan_diagnostics::TargetInstallCode as Code;
    match route {
        AgentRouteError::SessionUnavailable => Code::SessionUnavailable,
        AgentRouteError::Unhealthy => Code::Unhealthy,
        AgentRouteError::CapabilityUnavailable => Code::CapabilityUnavailable,
        AgentRouteError::CapabilityBindingMismatch => Code::CapabilityBindingMismatch,
        AgentRouteError::BindingRevoked => Code::BindingRevoked,
        AgentRouteError::DesktopChanged => Code::DesktopChanged,
        AgentRouteError::StateUnavailable => Code::StateUnavailable,
        AgentRouteError::ProtocolVersionUnavailable => Code::ProtocolVersionUnavailable,
    }
}

// Only these fixed causes may cross the installed-product error boundary. The
// attached anyhow chain remains internal and is never returned to the UI.
#[derive(Debug, Clone, Copy, thiserror::Error)]
enum ConsoleConsentFailure {
    #[error("The confirmation request no longer exists. Start a new connection.")]
    RequestUnavailable,
    #[error("This confirmation request does not belong to this target desktop.")]
    WrongTarget,
    #[error("This confirmation request is no longer pending. Start a new connection.")]
    NotPending,
    #[error("The confirmation policy changed. Start a new connection.")]
    PolicyChanged,
    #[error("No active local Windows desktop is available.")]
    NoActiveDesktop,
    #[error("Confirm this request in the currently active local desktop.")]
    WrongConsole,
    #[error("The capture Agent has not registered in this desktop. Reopen Rdesk after the Agent is ready.")]
    AgentUnregistered,
    #[error(
        "The capture Agent belongs to another desktop logon. Reopen Rdesk in the active desktop."
    )]
    LogonChanged,
    #[error("Update the capture Agent before confirming this connection.")]
    ProtocolOutdated,
    #[error("Selected permissions exceed the connection request.")]
    ScopeExpansion,
    #[error("The capture Agent is not ready for this desktop.")]
    CaptureRoute,
    #[error("The desktop has reached its capture session limit.")]
    SessionLimit,
    #[error("Previous desktop cleanup is still pending. Retry after cleanup completes.")]
    CleanupPending,
    #[error("The target desktop changed. Start a new connection.")]
    TargetChanged,
    #[error("The confirmation deadline is unavailable. Start a new connection.")]
    DeadlineMissing,
    #[error("The confirmation request expired. Start a new connection.")]
    Expired,
    #[error("The native confirmation service is unavailable.")]
    NativeServerUnavailable,
    #[error("Native desktop confirmation could not be completed.")]
    NativeRequest,
    #[error("The native confirmation request is invalid. Start a new connection.")]
    InvalidNativeRequest,
    #[error("Native desktop confirmation was not approved.")]
    NativeNotApproved(mrd_agent_ipc::ConsentDecision),
    #[error("Native confirmation did not approve all selected permissions.")]
    IncompleteApproval,
}

/// A bounded diagnostic for the verified installed UI; never format the source
/// error chain, which may contain identity, platform, or request information.
pub(crate) fn consent_failure_message(reason: &anyhow::Error) -> &'static str {
    use ConsoleConsentFailure::*;
    let Some(failure) = reason.downcast_ref::<ConsoleConsentFailure>() else {
        return "Confirm this request in the active local desktop. The installed capture Agent must be running and unlocked.";
    };
    if matches!(failure, CaptureRoute | NativeRequest) {
        let route = reason
            .downcast_ref::<AgentRouteError>()
            .copied()
            .or_else(|| match reason.downcast_ref::<AgentRequestError>() {
                Some(AgentRequestError::Route(route)) => Some(*route),
                _ => None,
            });
        if let Some(route) = route {
            return match route {
                AgentRouteError::SessionUnavailable => "The capture Agent has not registered in this desktop. Reopen Rdesk after the Agent is ready.",
                AgentRouteError::Unhealthy => "The capture Agent heartbeat is stale. Wait for the Agent to become ready.",
                AgentRouteError::CapabilityUnavailable if matches!(failure, CaptureRoute) => "The capture Agent is not advertising screen capture for this desktop.",
                AgentRouteError::CapabilityUnavailable => "The capture Agent is not advertising native confirmation for this desktop.",
                AgentRouteError::ProtocolVersionUnavailable => "Update the capture Agent before confirming this connection.",
                AgentRouteError::BindingRevoked | AgentRouteError::DesktopChanged => "The target desktop changed. Start a new connection.",
                _ => "The capture Agent routing state is unavailable.",
            };
        }
        if let Some(request) = reason.downcast_ref::<AgentRequestError>() {
            return match request {
                AgentRequestError::Timeout => "Native desktop confirmation timed out. Start a new connection.",
                AgentRequestError::Revoked | AgentRequestError::Disconnected => "The capture Agent disconnected or changed during confirmation. Start a new connection.",
                AgentRequestError::InvalidConsent(mrd_agent_ipc::ConsentValidationError::Expired) => "The confirmation request expired. Start a new connection.",
                AgentRequestError::InvalidConsent(_) => "The native confirmation request or response is invalid. Start a new connection.",
                _ => "The native confirmation connection is unavailable. Retry after the Agent is ready.",
            };
        }
    }
    match failure {
        RequestUnavailable => "The confirmation request no longer exists. Start a new connection.",
        WrongTarget => "This confirmation request does not belong to this target desktop.",
        NotPending => "This confirmation request is no longer pending. Start a new connection.",
        PolicyChanged => "The confirmation policy changed. Start a new connection.",
        NoActiveDesktop => "No active local Windows desktop is available.",
        WrongConsole => "Confirm this request in the currently active local desktop.",
        AgentUnregistered => "The capture Agent has not registered in this desktop. Reopen Rdesk after the Agent is ready.",
        LogonChanged => "The capture Agent belongs to another desktop logon. Reopen Rdesk in the active desktop.",
        ProtocolOutdated => "Update the capture Agent before confirming this connection.",
        ScopeExpansion => "Selected permissions exceed the connection request.",
        CaptureRoute => "The capture Agent is not ready for this desktop.",
        SessionLimit => "The desktop has reached its capture session limit.",
        CleanupPending => "Previous desktop cleanup is still pending. Retry after cleanup completes.",
        TargetChanged => "The target desktop changed. Start a new connection.",
        DeadlineMissing => "The confirmation deadline is unavailable. Start a new connection.",
        Expired => "The confirmation request expired. Start a new connection.",
        NativeServerUnavailable => "The native confirmation service is unavailable.",
        NativeRequest => "Native desktop confirmation could not be completed.",
        InvalidNativeRequest => "The native confirmation request is invalid. Start a new connection.",
        NativeNotApproved(mrd_agent_ipc::ConsentDecision::Expired) => "Native desktop confirmation expired. Start a new connection.",
        NativeNotApproved(mrd_agent_ipc::ConsentDecision::Denied) => "Native desktop confirmation was denied.",
        NativeNotApproved(mrd_agent_ipc::ConsentDecision::Dismissed) => "Native desktop confirmation was dismissed or unavailable. Start a new connection.",
        NativeNotApproved(_) => "Native desktop confirmation was not approved.",
        IncompleteApproval => "Native confirmation did not approve all selected permissions.",
    }
}

#[derive(Clone)]
struct Target {
    binding: AgentBinding,
    logon_hash: [u8; 32],
    authorization_expires_at_ms: u64,
    resource: Option<Resource>,
    input: HashMap<PermissionScope, InputResource>,
    closing: bool,
    paired_authority_id: Option<[u8; 32]>,
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
            .context(ConsoleConsentFailure::RequestUnavailable)?;
        ensure!(
            snapshot.role == RemoteSessionRole::Agent
                && snapshot.access_mode == RemoteAccessMode::Attended,
            ConsoleConsentFailure::WrongTarget
        );
        ensure!(
            snapshot.authorization_state == RemoteAuthorizationState::AwaitingLocalConsent,
            ConsoleConsentFailure::NotPending
        );
        ensure!(
            snapshot.policy_revision == response.expected_policy_revision,
            ConsoleConsentFailure::PolicyChanged
        );
        let active = active_console().context(ConsoleConsentFailure::NoActiveDesktop)?;
        ensure!(
            caller.windows_session_id == active,
            ConsoleConsentFailure::WrongConsole
        );
        let observed = state
            .agent_registry
            .active_for_session_at(active, now_ms())
            .context(ConsoleConsentFailure::AgentUnregistered)?;
        ensure!(
            observed.identity.logon_sid_hash == caller.logon_sid_hash,
            ConsoleConsentFailure::LogonChanged
        );
        ensure!(
            observed.identity.protocol_minor
                >= mrd_agent_ipc::AGENT_IPC_INPUT_DEADLINE_PROTOCOL_MINOR,
            ConsoleConsentFailure::ProtocolOutdated
        );
        ensure!(
            response
                .approved_scopes
                .iter()
                .all(|scope| snapshot.requested_scopes.contains(scope)),
            ConsoleConsentFailure::ScopeExpansion
        );
        let binding = state
            .agent_registry
            .bind_active_session(active, AgentCapability::Capture, now_ms())
            .context(ConsoleConsentFailure::CaptureRoute)?;
        {
            let mut targets = self.targets.lock().await;
            targets.retain(|_, target| {
                target.resource.is_some()
                    || !target.input.is_empty()
                    || target.authorization_expires_at_ms > now_ms()
            });
            ensure!(
                targets.len() < MAX_CONSOLE_TARGETS || targets.contains_key(&response.session_id),
                ConsoleConsentFailure::SessionLimit
            );
            if let Some(existing) = targets.get(&response.session_id) {
                ensure!(
                    !existing.closing
                        && !existing.input.values().any(|input| input.cleanup_pending),
                    ConsoleConsentFailure::CleanupPending
                );
                ensure!(
                    existing.binding == binding && existing.logon_hash == caller.logon_sid_hash,
                    ConsoleConsentFailure::TargetChanged
                );
            }
        }
        let now = now_ms();
        let expires = snapshot
            .authorization_expires_at_ms
            .context(ConsoleConsentFailure::DeadlineMissing)?
            .min(now.saturating_add(15_000));
        ensure!(expires > now, ConsoleConsentFailure::Expired);
        let native = state
            .console_agent_server()
            .context(ConsoleConsentFailure::NativeServerUnavailable)?
            .request_consent(mrd_agent_ipc::ConsentRequest {
                request_token: 1,
                request_id: random_id().context(ConsoleConsentFailure::InvalidNativeRequest)?,
                session_id: response.session_id.clone(),
                peer: PeerBinding {
                    device_id: snapshot.peer_device_id.clone(),
                    key_id: decode_peer_key(&snapshot.peer_key_id)
                        .context(ConsoleConsentFailure::InvalidNativeRequest)?,
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
            .await
            .context(ConsoleConsentFailure::NativeRequest)?;
        ensure!(
            native.consent().decision() == mrd_agent_ipc::ConsentDecision::Approved,
            ConsoleConsentFailure::NativeNotApproved(native.consent().decision())
        );
        let approved: PermissionScopes = response
            .approved_scopes
            .iter()
            .copied()
            .map(PermissionScope::from)
            .collect();
        ensure!(
            native.consent().approved_scopes() == &approved,
            ConsoleConsentFailure::IncompleteApproval
        );
        let consent_binding = native.binding();
        ensure!(
            consent_binding.registration_id() == binding.registration_id()
                && consent_binding.registration_epoch() == binding.registration_epoch()
                && consent_binding.desktop_epoch() == binding.desktop_epoch()
                && consent_binding.windows_session_id() == binding.windows_session_id(),
            ConsoleConsentFailure::TargetChanged
        );
        let target = Target {
            binding,
            logon_hash: caller.logon_sid_hash,
            authorization_expires_at_ms: native.consent().authorization_expires_at_ms(),
            resource: None,
            input: HashMap::new(),
            closing: false,
            paired_authority_id: None,
        };
        self.validate_target(state, &target)
            .context(ConsoleConsentFailure::TargetChanged)?;
        let mut targets = self.targets.lock().await;
        targets.retain(|_, target| {
            target.resource.is_some()
                || !target.input.is_empty()
                || target.authorization_expires_at_ms > now_ms()
        });
        ensure!(
            targets.len() < MAX_CONSOLE_TARGETS || targets.contains_key(&response.session_id),
            ConsoleConsentFailure::SessionLimit
        );
        if let Some(existing) = targets.get(&response.session_id) {
            ensure!(
                !existing.closing && !existing.input.values().any(|input| input.cleanup_pending),
                ConsoleConsentFailure::CleanupPending
            );
            ensure!(
                same_target(existing, &target),
                ConsoleConsentFailure::TargetChanged
            );
        } else {
            targets.insert(response.session_id.clone(), target);
        }
        Ok(())
    }

    /// Install independent paired-policy authority after the exact backend grant.
    /// The caller holds the service authorization security gate across this await.
    pub(crate) async fn prepare_paired_session(
        &self,
        state: &AppState,
        session_id: &SessionId,
    ) -> Result<()> {
        let _operation = self.operation.lock().await;
        use PairedDesktopPreparationError as Failure;
        crate::ipc_server::paired_access::verify_session_policy_under_security_gate(
            state, session_id,
        )
        .await
        .map_err(|_| anyhow!(Failure::PolicyVerification))?;
        let now = now_ms();
        let snapshot = state
            .session_authorizations
            .snapshot_at(session_id, now)
            .await
            .context(Failure::SnapshotUnavailable)?;
        ensure!(
            snapshot.role == RemoteSessionRole::Agent
                && snapshot.authorization_state == RemoteAuthorizationState::Granted,
            Failure::GrantState
        );
        let grant = state
            .session_authorizations
            .active_grant(session_id)
            .await
            .context(Failure::GrantUnavailable)?;
        ensure!(
            grant
                .granted_scopes
                .contains(&RemotePermissionScope::ScreenView),
            Failure::MissingScreenScope
        );
        let active = active_console().context(Failure::NoActiveConsole)?;
        let observed = state
            .agent_registry
            .active_for_session_at(active, now)
            .context(Failure::AgentUnavailable)?;
        ensure!(
            observed.identity.protocol_minor
                >= mrd_agent_ipc::AGENT_IPC_PAIRED_SESSION_AUTHORITY_PROTOCOL_MINOR,
            Failure::ProtocolOutdated
        );
        let binding = state
            .agent_registry
            .bind_active_session(active, AgentCapability::Capture, now)
            .context(Failure::CaptureRoute)?;
        let expires = grant
            .expires_at_ms
            .min(now.saturating_add(mrd_agent_ipc::AGENT_PAIRED_AUTHORITY_MAX_LIFETIME_MS));
        ensure!(expires > now, Failure::DeadlineExpired);
        let authority_id = random_id().context(Failure::EntropyUnavailable)?;
        let authority = self
            .issuer()
            .context(Failure::IssuerUnavailable)?
            .issue_paired_authority(mrd_agent_ipc::PairedSessionAuthorityClaims {
                authority_id,
                registration_id: *binding.registration_id(),
                registration_epoch: binding.registration_epoch(),
                session_id: session_id.clone(),
                peer: PeerBinding {
                    device_id: snapshot.peer_device_id,
                    key_id: decode_peer_key(&snapshot.peer_key_id)
                        .context(Failure::InvalidPeerKey)?,
                },
                scopes: grant
                    .granted_scopes
                    .iter()
                    .copied()
                    .map(PermissionScope::from)
                    .collect(),
                policy_revision: grant.policy_revision,
                windows_session_id: active,
                desktop_epoch: binding.desktop_epoch(),
                desktop_kind: DesktopKind::Default,
                issued_at_ms: now,
                not_before_ms: now,
                expires_at_ms: expires,
                audience: mrd_agent_ipc::GrantAudience::SessionAgent,
            })
            .context(Failure::AuthorityIssue)?;
        let server = state
            .console_agent_server()
            .context(Failure::AgentServerUnavailable)?;
        {
            let mut targets = self.targets.lock().await;
            ensure!(
                targets.len() < MAX_CONSOLE_TARGETS,
                Failure::SessionCapacity
            );
            ensure!(!targets.contains_key(session_id), Failure::DuplicateTarget);
            targets.insert(
                session_id.clone(),
                Target {
                    binding: binding.clone(),
                    logon_hash: observed.identity.logon_sid_hash,
                    authorization_expires_at_ms: expires,
                    resource: None,
                    input: HashMap::new(),
                    closing: false,
                    paired_authority_id: Some(authority_id),
                },
            );
        }
        let result = server.request_paired_authority(&binding, authority).await;
        if !matches!(result, Ok(ref ack) if ack.outcome == mrd_agent_ipc::PairedSessionAuthorityOutcome::Installed)
        {
            self.finish_failed_paired_install(state, session_id, &result)
                .await;
            return match result {
                Err(reason) => Err(anyhow::Error::new(reason).context(Failure::AgentRequest)),
                Ok(ack)
                    if ack.outcome == mrd_agent_ipc::PairedSessionAuthorityOutcome::Rejected =>
                {
                    Err(anyhow!(Failure::AgentAckRejected))
                }
                Ok(_) => Err(anyhow!(Failure::AgentAckUnexpected)),
            };
        }
        let target = self
            .targets
            .lock()
            .await
            .get(session_id)
            .cloned()
            .context(Failure::TargetMissing)?;
        if let Err(reason) = self.validate_target(state, &target) {
            self.stop_inner(state, session_id).await;
            return Err(reason.context(Failure::DesktopRecheck));
        }
        if crate::ipc_server::paired_access::verify_session_policy_under_security_gate(
            state, session_id,
        )
        .await
        .is_err()
        {
            self.stop_inner(state, session_id).await;
            anyhow::bail!(Failure::PolicyRecheck);
        }
        Ok(())
    }

    async fn finish_failed_paired_install(
        &self,
        state: &AppState,
        session_id: &SessionId,
        result: &std::result::Result<
            mrd_agent_ipc::PairedSessionAuthorityResult,
            AgentRequestError,
        >,
    ) {
        if let Ok(ack) = result {
            if ack.outcome == mrd_agent_ipc::PairedSessionAuthorityOutcome::Rejected
                && &ack.session_id == session_id
            {
                if let Some(target) = self.targets.lock().await.get_mut(session_id) {
                    // Only an exact rejection of this still-empty installation
                    // proves that no authority needs a later revoke. Unknown
                    // outcomes retain the target until cleanup is confirmed.
                    if target.paired_authority_id == Some(ack.authority_id)
                        && target.binding.registration_id() == &ack.registration_id
                        && target.binding.registration_epoch() == ack.registration_epoch
                        && target.resource.is_none()
                        && target.input.is_empty()
                    {
                        target.paired_authority_id = None;
                    }
                }
            }
        }
        self.stop_inner(state, session_id).await;
    }

    /// Resolve the approved exact target, start its capture resource, and keep
    /// the grant metadata for a bounded, command-specific stop at cleanup.
    pub(crate) async fn start(
        &self,
        state: &AppState,
        session_id: &SessionId,
        profile: &MediaProfile,
    ) -> Result<()> {
        self.start_inner(state, session_id, profile)
            .await
            .map_err(|reason| {
                if reason.downcast_ref::<CaptureStartFailure>().is_some() {
                    reason
                } else {
                    reason.context(CaptureStartFailure::Guard)
                }
            })
    }

    async fn start_inner(
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
            .context("Session authority did not bind a target desktop")?;
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
                )
                .context(
                    capture_ack_failure(outcome.outcome)
                        .unwrap_or(CaptureStartFailure::AckUnexpected),
                )),
                Err(reason) => Err(anyhow!(reason)
                    .context("Capture Agent request failed")
                    .context(CaptureStartFailure::Request)),
            };
        }
        if let Err(error) = self.validate_target(state, &target) {
            self.stop_inner(state, session_id).await;
            return Err(error.context(CaptureStartFailure::DesktopRecheck));
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
        let resources_cleared = self
            .targets
            .lock()
            .await
            .get(session_id)
            .is_some_and(|current| current.resource.is_none() && current.input.is_empty());
        if resources_cleared {
            if let Some(authority_id) = target.paired_authority_id {
                let result = match state.console_agent_server() {
                    Some(server) => {
                        server
                            .request_revoke_paired_authority(
                                &target.binding,
                                authority_id,
                                session_id.clone(),
                            )
                            .await
                    }
                    None => Err(AgentRequestError::ConnectionUnavailable),
                };
                if matches!(result, Ok(ref ack) if ack.outcome == mrd_agent_ipc::PairedSessionAuthorityOutcome::Revoked)
                {
                    if let Some(current) = self.targets.lock().await.get_mut(session_id) {
                        current.paired_authority_id = None;
                    }
                } else {
                    tracing::warn!("Paired desktop authority cleanup was not acknowledged");
                }
            }
        }
        let mut targets = self.targets.lock().await;
        if targets.get(session_id).is_some_and(|current| {
            current.resource.is_none()
                && current.input.is_empty()
                && current.paired_authority_id.is_none()
        }) {
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
