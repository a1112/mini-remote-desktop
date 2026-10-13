use super::*;
use crate::consent::{ConsentBackendFuture, ConsentPrompt};
use crate::input::InputResourceManager;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use mrd_agent_ipc::{
    AgentCapability, AgentCommand, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, GrantAudience,
    InputEventEnvelope, InputEventPayload, InputKey, PairedSessionAuthority,
    PairedSessionAuthorityClaims,
};
use mrd_proto::DeviceId;
use mrd_session::PermissionScope;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

struct Clock(AtomicU64);
impl AgentClock for Clock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
struct NoRegistration;
impl RegistrationSigner for NoRegistration {
    fn key_id(&self) -> [u8; 32] {
        [1; 32]
    }
    fn sign(&self, _: &[u8]) -> Result<[u8; 64], RegistrationSigningError> {
        Ok([1; 64])
    }
}
struct NoPrompt;
impl ConsentBackend for NoPrompt {
    fn is_available(&self) -> bool {
        true
    }
    fn prompt(
        &self,
        _: ConsentPrompt,
        _: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        panic!("paired authority must never create native consent")
    }
}
struct Pinned(SigningKey);
impl ExecuteGrantVerifier for Pinned {
    fn verify(&self, issuer: &[u8; 32], bytes: &[u8], signature: &[u8; 64]) -> bool {
        *issuer
            == mrd_agent_ipc::derive_execute_grant_issuer_key_id(&self.0.verifying_key().to_bytes())
            && self
                .0
                .verifying_key()
                .verify(bytes, &Signature::from_bytes(signature))
                .is_ok()
    }
}
struct Desktop {
    state: Mutex<TrustedDesktopState>,
    wake: watch::Sender<()>,
}
impl TrustedDesktopStateSource for Desktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        Some(*self.state.lock().unwrap())
    }
    fn subscribe(&self) -> watch::Receiver<()> {
        self.wake.subscribe()
    }
}
struct Capture(Arc<Mutex<HashSet<SessionId>>>);
impl AuthorizedCommandExecutor for Capture {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::from_implemented([AgentCapability::Capture])
    }
    fn execute(&mut self, command: AuthorizedCommand) -> CommandOutcome {
        if matches!(command.command(), AgentCommand::StartCapture { .. }) {
            self.0
                .lock()
                .unwrap()
                .insert(command.grant().claims().session_id.clone());
        }
        CommandOutcome::Completed
    }
    fn revoke_session(&mut self, session: &SessionId) -> bool {
        self.0.lock().unwrap().remove(session);
        true
    }
}
struct Injector(Arc<Mutex<Vec<mrd_input::InputEvent>>>);
impl mrd_input::InputInjector for Injector {
    fn is_available(&self) -> bool {
        true
    }
    fn inject(&mut self, event: &mrd_input::InputEvent) -> Result<(), mrd_input::InputError> {
        self.0.lock().unwrap().push(*event);
        Ok(())
    }
}

struct DiagnosticCapture {
    unit: Option<crate::media::EncodedMediaAccessUnit>,
}

struct NoDiagnosticRender;
impl crate::render::RenderAdapter for NoDiagnosticRender {
    fn is_available(&self) -> bool {
        false
    }

    fn start(&mut self, _: &crate::media::MediaResource, _: &SessionId) -> bool {
        panic!("capture diagnostic must not start rendering")
    }

    fn push_access_unit(
        &mut self,
        _: &crate::media::MediaResource,
        _: &mrd_agent_ipc::RenderAccessUnit,
    ) -> bool {
        panic!("capture diagnostic must not render media")
    }

    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        panic!("capture diagnostic must not stop rendering")
    }
}

impl crate::capture::CaptureAdapter for DiagnosticCapture {
    fn is_available(&self) -> bool {
        true
    }

    fn start(&mut self, resource: &crate::media::MediaResource, session: &SessionId) -> bool {
        resource.session_id() == session
    }

    fn poll_encoded(&mut self, _: &[u8; 16]) -> Option<crate::media::EncodedMediaAccessUnit> {
        self.unit.take()
    }

    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        self.unit = None;
        true
    }
}
struct Fixture {
    runtime: AgentRuntime,
    identity: RegisteredAgentIdentity,
    signing: SigningKey,
    authority: PairedSessionAuthority,
    clock: Arc<Clock>,
    desktop: Arc<Desktop>,
    captures: Arc<Mutex<HashSet<SessionId>>>,
    events: Arc<Mutex<Vec<mrd_input::InputEvent>>>,
    writer: OutboundWriter,
    reader: tokio::io::DuplexStream,
}
impl Fixture {
    fn new() -> Self {
        let signing = SigningKey::from_bytes(&[19; 32]);
        let issuer =
            mrd_agent_ipc::derive_execute_grant_issuer_key_id(&signing.verifying_key().to_bytes());
        let clock = Arc::new(Clock(AtomicU64::new(1500)));
        let desktop = Arc::new(Desktop {
            state: Mutex::new(TrustedDesktopState {
                desktop_epoch: 6,
                desktop_kind: DesktopKind::Default,
            }),
            wake: watch::channel(()).0,
        });
        let captures = Arc::new(Mutex::new(HashSet::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let runtime = AgentRuntime::new(
            AgentRuntimeConfig {
                session: SessionDescriptor::new([1; 16], 1, 1, [1; 32], 1, [1; 32], 6).unwrap(),
                heartbeat_interval: Duration::from_secs(30),
                handshake_timeout: Duration::from_secs(1),
            },
            clock.clone(),
            Arc::new(NoRegistration),
        )
        .unwrap()
        .with_attended_authority(
            Arc::new(NoPrompt),
            Arc::new(Pinned(signing.clone())),
            desktop.clone(),
            issuer,
            Box::new(Capture(captures.clone())),
        )
        .unwrap()
        .with_input_backend(Box::new(InputResourceManager::new(Injector(
            events.clone(),
        ))));
        let identity = RegisteredAgentIdentity {
            agent_instance_id: [1; 16],
            process_id: 1,
            process_creation_time: 1,
            logon_sid_hash: [1; 32],
            windows_session_id: 1,
            agent_key_id: [1; 32],
            registration_id: [2; 16],
            registration_epoch: 3,
            protocol_major: 1,
            protocol_minor: 9,
        };
        let mut authority = PairedSessionAuthority {
            claims: PairedSessionAuthorityClaims {
                authority_id: [1; 32],
                registration_id: [2; 16],
                registration_epoch: 3,
                session_id: SessionId("paired".into()),
                peer: PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [4; 32],
                },
                scopes: [
                    PermissionScope::ScreenView,
                    PermissionScope::InputPointer,
                    PermissionScope::InputKeyboard,
                ]
                .into_iter()
                .collect(),
                policy_revision: 5,
                windows_session_id: 1,
                desktop_epoch: 6,
                desktop_kind: DesktopKind::Default,
                issued_at_ms: 1000,
                not_before_ms: 1000,
                expires_at_ms: 2000,
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: issuer,
            signature: [0; 64],
        };
        authority.signature = signing.sign(&authority.signing_bytes()).to_bytes();
        let (stream, reader) = tokio::io::duplex(65536);
        let writer = OutboundWriter::spawn(stream);
        Self {
            runtime,
            identity,
            signing,
            authority,
            clock,
            desktop,
            captures,
            events,
            writer,
            reader,
        }
    }
    async fn reply(&mut self) -> AgentToService {
        mrd_agent_ipc::read_frame(&mut self.reader)
            .await
            .unwrap()
            .message
    }
    async fn install(&mut self) -> PairedSessionAuthorityOutcome {
        self.runtime
            .handle_paired_install(
                &self.writer,
                &self.identity,
                InstallPairedSessionAuthority {
                    request_token: 1,
                    authority: self.authority.clone(),
                },
            )
            .unwrap();
        match self.reply().await {
            AgentToService::PairedSessionAuthorityResult(result) => result.outcome,
            other => panic!("unexpected {other:?}"),
        }
    }
    fn execute(&self, seed: u8, command: AgentCommand) -> ExecuteCommand {
        let claims = &self.authority.claims;
        let mut execute = ExecuteCommand {
            request_token: u64::from(seed),
            command_id: [seed; 16],
            command,
            grant: ExecuteGrant {
                claims: ExecuteGrantClaims {
                    grant_id: [seed; 32],
                    registration_id: claims.registration_id,
                    registration_epoch: claims.registration_epoch,
                    session_id: claims.session_id.clone(),
                    peer: claims.peer.clone(),
                    scopes: claims.scopes.clone(),
                    policy_revision: claims.policy_revision,
                    windows_session_id: claims.windows_session_id,
                    desktop_epoch: claims.desktop_epoch,
                    desktop_kind: claims.desktop_kind,
                    issued_at_ms: 1000,
                    not_before_ms: 1000,
                    expires_at_ms: claims.expires_at_ms,
                    command_digest: [0; 32],
                    audience: GrantAudience::SessionAgent,
                },
                issuer_key_id: self.authority.issuer_key_id,
                signature: [0; 64],
            },
        };
        self.resign(&mut execute);
        execute
    }
    fn resign(&self, execute: &mut ExecuteCommand) {
        execute.grant.claims.command_digest = execute.command_digest();
        execute.grant.signature = self.signing.sign(&execute.grant.signing_bytes()).to_bytes();
    }
    async fn run_execute(&mut self, execute: ExecuteCommand) -> CommandOutcome {
        self.runtime
            .handle_execute(&self.writer, &self.identity, &execute)
            .await
            .unwrap();
        match self.reply().await {
            AgentToService::CommandResult(result) => result.outcome,
            other => panic!("unexpected {other:?}"),
        }
    }
    fn input(&self, sequence: u64) -> InputEventEnvelope {
        InputEventEnvelope {
            request_token: sequence,
            session_id: self.authority.claims.session_id.clone(),
            resource_id: [12; 16],
            start_grant_id: [12; 32],
            sequence,
            expires_at_ms: 1900,
            event: InputEventPayload::Key {
                key: InputKey::VirtualKey { code: 65 },
                pressed: true,
            },
        }
    }
    async fn run_input(&mut self, input: InputEventEnvelope) -> InputAckOutcome {
        self.runtime
            .handle_input(&self.writer, &self.identity, input)
            .await
            .unwrap();
        match self.reply().await {
            AgentToService::InputAck(ack) => ack.outcome,
            other => panic!("unexpected {other:?}"),
        }
    }
    async fn start_resources(&mut self) {
        assert_eq!(
            self.install().await,
            PairedSessionAuthorityOutcome::Installed
        );
        let capture = self.execute(
            11,
            AgentCommand::StartCapture {
                resource_id: [11; 16],
                display_id: 0,
                profile: None,
            },
        );
        assert_eq!(self.run_execute(capture).await, CommandOutcome::Completed);
        let input = self.execute(
            12,
            AgentCommand::StartInput {
                resource_id: [12; 16],
                input_scopes: [PermissionScope::InputKeyboard].into_iter().collect(),
            },
        );
        assert_eq!(self.run_execute(input).await, CommandOutcome::Completed);
        assert_eq!(
            self.run_input(self.input(1)).await,
            InputAckOutcome::Applied
        );
        assert_eq!(self.events.lock().unwrap().len(), 1);
    }
}

async fn capture_diagnostic_fixture(seed: u8, payload: Vec<u8>) -> Fixture {
    let mut f = Fixture::new();
    f.authority.claims.session_id =
        SessionId(format!("c4b10cca-eac1-4bbf-980a-9338bb5784{seed:02x}"));
    f.authority.signature = f.signing.sign(&f.authority.signing_bytes()).to_bytes();
    let unit = crate::media::EncodedMediaAccessUnit::new(
        [11; 16],
        f.authority.claims.session_id.clone(),
        1,
        1,
        true,
        payload,
    )
    .unwrap()
    .with_source_bounds(mrd_agent_ipc::CaptureSourceBounds {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
    })
    .unwrap();
    f.runtime.authority.as_mut().unwrap().executor = Box::new(crate::media::MediaExecutor::new(
        DiagnosticCapture { unit: Some(unit) },
        NoDiagnosticRender,
    ));
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Installed);
    let start = f.execute(
        11,
        AgentCommand::StartCapture {
            resource_id: [11; 16],
            display_id: 0,
            profile: None,
        },
    );
    assert_eq!(f.run_execute(start).await, CommandOutcome::Completed);
    f
}

fn capture_diagnostic_assert_record(f: &Fixture, cause: &str) {
    let records = crate::capture_diagnostics::take_test_records(&f.authority.claims.session_id);
    assert_eq!(
        records.len(),
        1,
        "first fatal pump cause must be observable"
    );
    assert_eq!(records[0], format!(
        "{{\"event\":\"agent_capture_failed\",\"agent_pid\":{},\"session_id\":\"{}\",\"cause\":\"{cause}\"}}",
        std::process::id(), f.authority.claims.session_id.0
    ));
}

#[tokio::test]
async fn capture_diagnostic_runtime_binding_rejection_keeps_original_media_error() {
    let mut f = capture_diagnostic_fixture(1, vec![1]).await;
    f.clock.0.store(2000, Ordering::SeqCst);
    assert!(matches!(
        f.runtime.pump_capture_access_units(&f.writer, &f.identity),
        Err(AgentRuntimeError::MediaCaptureFailed)
    ));
    capture_diagnostic_assert_record(&f, "capture_binding_rejected");
}

#[tokio::test]
async fn capture_diagnostic_runtime_invalid_ipc_unit_keeps_original_media_error() {
    let mut f = capture_diagnostic_fixture(2, vec![]).await;
    assert!(matches!(
        f.runtime.pump_capture_access_units(&f.writer, &f.identity),
        Err(AgentRuntimeError::MediaCaptureFailed)
    ));
    capture_diagnostic_assert_record(&f, "capture_ipc_unit_invalid");
}

#[tokio::test]
async fn capture_diagnostic_runtime_outbound_failure_keeps_original_outbound_error() {
    let mut f = capture_diagnostic_fixture(3, vec![1]).await;
    f.writer.abort_and_join().await;
    assert!(matches!(
        f.runtime.pump_capture_access_units(&f.writer, &f.identity),
        Err(AgentRuntimeError::OutboundUnavailable)
    ));
    capture_diagnostic_assert_record(&f, "capture_outbound_unavailable");
}

#[tokio::test]
async fn capture_diagnostic_runtime_valid_unit_preserves_binding_and_geometry_without_event() {
    let mut f = capture_diagnostic_fixture(4, vec![1]).await;
    f.runtime
        .pump_capture_access_units(&f.writer, &f.identity)
        .unwrap();
    let AgentToService::MediaAccessUnit(unit) = f.reply().await else {
        panic!("valid capture unit must reach the original authenticated writer");
    };
    assert_eq!(unit.session_id, f.authority.claims.session_id.0);
    assert_eq!(unit.resource_id, [11; 16]);
    assert_eq!(unit.context.registration_id, f.identity.registration_id);
    assert_eq!(
        unit.context.registration_epoch,
        f.identity.registration_epoch
    );
    assert_eq!(unit.context.desktop_epoch, 6);
    assert_eq!(unit.source_bounds.unwrap().width, 1920);
    assert_eq!(unit.payload, vec![1]);
    assert!(
        crate::capture_diagnostics::take_test_records(&f.authority.claims.session_id).is_empty()
    );
}

#[tokio::test]
async fn capture_diagnostic_runtime_stopped_resource_is_not_pumped_or_reported() {
    let mut f = capture_diagnostic_fixture(5, vec![]).await;
    let stop = f.execute(
        12,
        AgentCommand::StopCapture {
            resource_id: [11; 16],
        },
    );
    assert_eq!(f.run_execute(stop).await, CommandOutcome::Completed);
    f.runtime
        .pump_capture_access_units(&f.writer, &f.identity)
        .unwrap();
    assert!(
        crate::capture_diagnostics::take_test_records(&f.authority.claims.session_id).is_empty()
    );
}

#[tokio::test]
async fn paired_runtime_installs_without_consent_and_exact_revoke_releases_capture_and_pressed_keys(
) {
    let mut f = Fixture::new();
    f.start_resources().await;
    let request = RevokePairedSessionAuthority {
        request_token: 13,
        authority_id: [1; 32],
        registration_id: [2; 16],
        registration_epoch: 3,
        session_id: f.authority.claims.session_id.clone(),
    };
    f.runtime
        .handle_paired_revoke(&f.writer, &f.identity, request.clone())
        .unwrap();
    assert!(
        matches!(f.reply().await,AgentToService::PairedSessionAuthorityResult(result) if result.outcome==PairedSessionAuthorityOutcome::Revoked)
    );
    assert!(f.captures.lock().unwrap().is_empty());
    assert_eq!(f.events.lock().unwrap().len(), 2, "held key is released");
    assert!(matches!(
        f.run_input(f.input(2)).await,
        InputAckOutcome::Rejected { .. }
    ));
    let execute = f.execute(
        14,
        AgentCommand::StartCapture {
            resource_id: [14; 16],
            display_id: 0,
            profile: None,
        },
    );
    assert_eq!(f.run_execute(execute).await, CommandOutcome::Rejected);
    f.runtime
        .handle_paired_revoke(&f.writer, &f.identity, request)
        .unwrap();
    assert!(
        matches!(f.reply().await,AgentToService::PairedSessionAuthorityResult(result) if result.outcome==PairedSessionAuthorityOutcome::Revoked)
    );
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Rejected);
}

#[tokio::test]
async fn paired_runtime_rejects_correctly_signed_cross_session_peer_policy_and_scope_commands() {
    let mut f = Fixture::new();
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Installed);
    for change in 0..6 {
        let mut execute = f.execute(
            20 + change,
            AgentCommand::StartCapture {
                resource_id: [20 + change; 16],
                display_id: 0,
                profile: None,
            },
        );
        match change {
            0 => execute.grant.claims.session_id.0.push('x'),
            1 => execute.grant.claims.peer.device_id.0.push('x'),
            2 => execute.grant.claims.peer.key_id[0] ^= 1,
            3 => execute.grant.claims.policy_revision += 1,
            4 => execute.grant.claims.registration_epoch += 1,
            _ => {
                execute
                    .grant
                    .claims
                    .scopes
                    .insert(PermissionScope::ClipboardRead);
            }
        }
        f.resign(&mut execute);
        assert_eq!(f.run_execute(execute).await, CommandOutcome::Rejected);
    }
    assert!(f.captures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paired_runtime_expiry_desktop_unknown_and_disconnect_clean_resources() {
    for cause in 0..3 {
        let mut f = Fixture::new();
        f.start_resources().await;
        match cause {
            0 => {
                f.clock.0.store(2000, Ordering::SeqCst);
                assert_eq!(
                    f.runtime.reconcile_due_authority().unwrap(),
                    vec![f.authority.claims.session_id.clone()]
                );
            }
            1 => {
                let state = TrustedDesktopState {
                    desktop_epoch: 7,
                    desktop_kind: DesktopKind::Unknown,
                };
                *f.desktop.state.lock().unwrap() = state;
                f.runtime
                    .reconcile_desktop_authority(&f.writer, state)
                    .unwrap();
            }
            _ => f
                .runtime
                .terminal_cleanup(ConsentAbortReason::ServiceDisconnected)
                .await
                .unwrap(),
        }
        assert!(f.captures.lock().unwrap().is_empty());
        assert_eq!(f.events.lock().unwrap().len(), 2);
        assert!(f
            .runtime
            .resolve_session_binding(&f.authority.claims.session_id, f.clock.now_ms())
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn paired_runtime_old_minor_and_unknown_revoke_fail_closed() {
    let mut f = Fixture::new();
    f.identity.protocol_minor = 8;
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Rejected);
    f.identity.protocol_minor = 9;
    f.start_resources().await;
    let request = RevokePairedSessionAuthority {
        request_token: 13,
        authority_id: [99; 32],
        registration_id: [2; 16],
        registration_epoch: 3,
        session_id: f.authority.claims.session_id.clone(),
    };
    f.runtime
        .handle_paired_revoke(&f.writer, &f.identity, request)
        .unwrap();
    assert!(
        matches!(f.reply().await,AgentToService::PairedSessionAuthorityResult(result) if result.outcome==PairedSessionAuthorityOutcome::Rejected)
    );
    assert_eq!(f.events.lock().unwrap().len(), 1);
    assert!(!f.captures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paired_runtime_old_idempotent_revoke_does_not_release_new_same_session_resources() {
    let mut f = Fixture::new();
    f.start_resources().await;
    let old = RevokePairedSessionAuthority {
        request_token: 13,
        authority_id: [1; 32],
        registration_id: [2; 16],
        registration_epoch: 3,
        session_id: f.authority.claims.session_id.clone(),
    };
    f.runtime
        .handle_paired_revoke(&f.writer, &f.identity, old.clone())
        .unwrap();
    let _ = f.reply().await;
    f.authority.claims.authority_id = [9; 32];
    f.authority.signature = f.signing.sign(&f.authority.signing_bytes()).to_bytes();
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Installed);
    let capture = f.execute(
        21,
        AgentCommand::StartCapture {
            resource_id: [21; 16],
            display_id: 0,
            profile: None,
        },
    );
    assert_eq!(f.run_execute(capture).await, CommandOutcome::Completed);
    f.runtime
        .handle_paired_revoke(&f.writer, &f.identity, old)
        .unwrap();
    assert!(
        matches!(f.reply().await,AgentToService::PairedSessionAuthorityResult(result) if result.outcome==PairedSessionAuthorityOutcome::Revoked)
    );
    assert!(
        !f.captures.lock().unwrap().is_empty(),
        "old replay cannot release the new authority's capture"
    );
}

#[tokio::test]
async fn paired_runtime_never_converts_attended_request_into_approval_or_replaces_its_origin() {
    let mut f = Fixture::new();
    assert_eq!(f.install().await, PairedSessionAuthorityOutcome::Installed);
    let claims = f.authority.claims.clone();
    let request = ConsentRequest {
        request_token: 33,
        request_id: [33; 16],
        session_id: claims.session_id.clone(),
        peer: claims.peer.clone(),
        requested_scopes: claims.scopes.clone(),
        policy_revision: claims.policy_revision,
        windows_session_id: 1,
        issued_at_ms: 1000,
        expires_at_ms: 1800,
        authorization_expires_at_ms: 2000,
    };
    f.runtime
        .handle_managed_consent(&f.writer, &f.identity, request)
        .await
        .unwrap();
    assert!(
        matches!(f.reply().await,AgentToService::ConsentResult(result) if result.decision==ConsentDecision::Dismissed)
    );
    assert!(!f
        .runtime
        .authority
        .as_ref()
        .unwrap()
        .manager
        .has_active_prompt());
    assert!(matches!(
        f.runtime
            .resolve_session_binding(&claims.session_id, 1500)
            .unwrap()
            .unwrap()
            .origin,
        AuthorityOrigin::Paired { .. }
    ));
}

#[tokio::test]
async fn paired_production_loop_monotonic_expiry_releases_capture_before_long_heartbeat() {
    let mut f = Fixture::new();
    f.authority.claims.expires_at_ms = 1700;
    f.authority.signature = f.signing.sign(&f.authority.signing_bytes()).to_bytes();
    let execute = f.execute(
        41,
        AgentCommand::StartCapture {
            resource_id: [41; 16],
            display_id: 0,
            profile: None,
        },
    );
    let (stream, mut service) = tokio::io::duplex(65536);
    let agent = tokio::spawn(f.runtime.run(stream));
    let register = match mrd_agent_ipc::read_frame::<_, AgentToService>(&mut service)
        .await
        .unwrap()
        .message
    {
        AgentToService::AgentRegister(register) => register,
        other => panic!("unexpected {other:?}"),
    };
    let challenge = AgentChallenge {
        registration_id: [2; 16],
        registration_epoch: 3,
        challenge_id: [3; 16],
        challenge_nonce: [3; 32],
        expected_agent_instance_id: register.agent_instance_id,
        expected_process_id: register.process_id,
        expected_process_creation_time: register.process_creation_time,
        expected_logon_sid_hash: register.logon_sid_hash,
        expected_windows_session_id: register.windows_session_id,
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    write_frame(&mut service, &ServiceToAgent::AgentChallenge(challenge))
        .await
        .unwrap();
    assert!(matches!(
        mrd_agent_ipc::read_frame::<_, AgentToService>(&mut service)
            .await
            .unwrap()
            .message,
        AgentToService::AgentRegistered(_)
    ));
    assert!(matches!(
        mrd_agent_ipc::read_frame::<_, AgentToService>(&mut service)
            .await
            .unwrap()
            .message,
        AgentToService::AgentCapabilitySnapshot(_)
    ));
    write_frame(
        &mut service,
        &ServiceToAgent::InstallPairedSessionAuthority(Box::new(InstallPairedSessionAuthority {
            request_token: 40,
            authority: f.authority,
        })),
    )
    .await
    .unwrap();
    loop {
        match mrd_agent_ipc::read_frame::<_, AgentToService>(&mut service)
            .await
            .unwrap()
            .message
        {
            AgentToService::AgentHeartbeat(_) => {}
            AgentToService::PairedSessionAuthorityResult(result) => {
                assert_eq!(result.outcome, PairedSessionAuthorityOutcome::Installed);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    write_frame(&mut service, &ServiceToAgent::Execute(Box::new(execute)))
        .await
        .unwrap();
    loop {
        match mrd_agent_ipc::read_frame::<_, AgentToService>(&mut service)
            .await
            .unwrap()
            .message
        {
            AgentToService::AgentHeartbeat(_) => {}
            AgentToService::CommandResult(result) => {
                assert_eq!(result.outcome, CommandOutcome::Completed);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(!f.captures.lock().unwrap().is_empty());
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.captures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.clock.now_ms(),
        1500,
        "no wall-clock advance or 30-second heartbeat was required"
    );
    drop(service);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), agent)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        AgentExit::ServiceDisconnected
    );
}
