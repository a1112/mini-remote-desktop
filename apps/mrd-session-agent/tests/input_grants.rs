use mrd_agent_ipc::{
    read_frame, validate_execute_command, write_frame, AgentChallenge, AgentCommand,
    AgentProtocolState, AgentToService, CommandOutcome, ConsentDecision, ConsentRequest,
    DesktopKind, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, ExecuteGrantVerifier,
    ExecutionContext, GrantAudience, InputAckOutcome, InputButton, InputEventEnvelope,
    InputEventPayload, InputFailure, InputKey, InputRejection, PeerBinding,
    RegistrationProofVerifier, ServiceToAgent, StopAgent, StopReason,
};
use mrd_input::{InputError, InputEvent, InputInjector};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use mrd_session_agent::{
    capabilities::AgentCapabilities,
    consent::{ConsentBackend, ConsentBackendDecision, ConsentBackendFuture, ConsentPrompt},
    input::{InputBackend, InputResourceManager},
    runtime::{
        AgentClock, AgentExit, AgentRuntime, AgentRuntimeConfig, AuthorizedCommandExecutor,
        RegistrationSigner, RegistrationSigningError, SessionDescriptor, TrustedDesktopState,
        TrustedDesktopStateSource,
    },
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::watch;

const REGISTRATION_ID: [u8; 16] = [1; 16];
const RESOURCE_ID: [u8; 16] = [2; 16];
const OTHER_RESOURCE_ID: [u8; 16] = [3; 16];
const START_GRANT_ID: [u8; 32] = [4; 32];
const OTHER_START_GRANT_ID: [u8; 32] = [5; 32];
const ISSUER_KEY_ID: [u8; 32] = [6; 32];
const PEER_KEY_ID: [u8; 32] = [7; 32];

#[derive(Clone)]
struct SharedInjector {
    events: Arc<Mutex<Vec<InputEvent>>>,
    next_error: Arc<Mutex<Option<InputError>>>,
    available: bool,
}

impl SharedInjector {
    fn available(events: Arc<Mutex<Vec<InputEvent>>>) -> Self {
        Self {
            events,
            next_error: Arc::new(Mutex::new(None)),
            available: true,
        }
    }

    fn failing(events: Arc<Mutex<Vec<InputEvent>>>, error: InputError) -> Self {
        Self {
            events,
            next_error: Arc::new(Mutex::new(Some(error))),
            available: true,
        }
    }
}

impl InputInjector for SharedInjector {
    fn is_available(&self) -> bool {
        self.available
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        if let Some(error) = self.next_error.lock().expect("input error lock").take() {
            return Err(error);
        }
        self.events.lock().expect("input event lock").push(*event);
        Ok(())
    }
}

struct AcceptVerifier;

impl ExecuteGrantVerifier for AcceptVerifier {
    fn verify(&self, _issuer_key_id: &[u8; 32], _bytes: &[u8], _signature: &[u8; 64]) -> bool {
        true
    }
}

struct FixedClock;

impl AgentClock for FixedClock {
    fn now_ms(&self) -> u64 {
        1_500
    }
}

struct FixedSigner;

impl RegistrationSigner for FixedSigner {
    fn key_id(&self) -> [u8; 32] {
        [8; 32]
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; 64], RegistrationSigningError> {
        Ok(test_signature(message))
    }
}

struct FixedRegistrationVerifier;

impl RegistrationProofVerifier for FixedRegistrationVerifier {
    fn verify(&self, _key_id: &[u8; 32], bytes: &[u8], signature: &[u8; 64]) -> bool {
        test_signature(bytes) == *signature
    }
}

fn test_signature(message: &[u8]) -> [u8; 64] {
    let mut signature = [0_u8; 64];
    for (index, byte) in message.iter().enumerate() {
        signature[index % 64] ^= *byte;
    }
    signature[0] |= 1;
    signature
}

struct MutableDesktop {
    state: Arc<Mutex<TrustedDesktopState>>,
}

impl TrustedDesktopStateSource for MutableDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        self.state.lock().ok().map(|state| *state)
    }
}

struct NoopExecutor;

impl AuthorizedCommandExecutor for NoopExecutor {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::empty()
    }

    fn execute(&mut self, _command: mrd_agent_ipc::AuthorizedCommand) -> CommandOutcome {
        CommandOutcome::Rejected
    }
}

struct ReservedCapabilityClaimingExecutor {
    executions: Arc<AtomicUsize>,
}

impl AuthorizedCommandExecutor for ReservedCapabilityClaimingExecutor {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::from_implemented([
            mrd_agent_ipc::AgentCapability::Input,
            mrd_agent_ipc::AgentCapability::Consent,
        ])
    }

    fn execute(&mut self, _command: mrd_agent_ipc::AuthorizedCommand) -> CommandOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        CommandOutcome::Completed
    }
}

struct UnavailableConsentBackend;

impl ConsentBackend for UnavailableConsentBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn prompt(
        &self,
        _prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<mrd_session_agent::consent::ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        panic!("unavailable consent backend must not receive a prompt")
    }
}

struct ApproveRequestedScopes;

impl ConsentBackend for ApproveRequestedScopes {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<mrd_session_agent::consent::ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let approved = prompt.requested_scopes().clone();
        Box::pin(async move { ConsentBackendDecision::Approved(approved) })
    }
}

fn session_id() -> SessionId {
    SessionId("input-grant-session".to_owned())
}

fn other_session_id() -> SessionId {
    SessionId("other-input-grant-session".to_owned())
}

fn peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId("input-grant-peer".to_owned()),
        key_id: PEER_KEY_ID,
    }
}

fn scopes(values: &[PermissionScope]) -> PermissionScopes {
    values.iter().copied().collect()
}

fn context() -> ExecutionContext {
    ExecutionContext {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        session_id: session_id(),
        peer: peer(),
        policy_revision: 3,
        windows_session_id: 7,
        desktop_epoch: 11,
        desktop_kind: DesktopKind::Default,
        now_ms: 1_500,
        expected_issuer_key_id: ISSUER_KEY_ID,
        authorization_scopes: scopes(&[
            PermissionScope::InputPointer,
            PermissionScope::InputKeyboard,
        ]),
        authorization_expires_at_ms: 2_500,
    }
}

fn context_for(session_id: SessionId) -> ExecutionContext {
    ExecutionContext {
        session_id,
        ..context()
    }
}

fn authorized_start(
    resource_id: [u8; 16],
    grant_id: [u8; 32],
    input_scopes: PermissionScopes,
) -> mrd_agent_ipc::AuthorizedCommand {
    authorized_command(
        AgentCommand::StartInput {
            resource_id,
            input_scopes,
        },
        grant_id,
        &context(),
    )
}

fn authorized_command(
    command: AgentCommand,
    grant_id: [u8; 32],
    execution: &ExecutionContext,
) -> mrd_agent_ipc::AuthorizedCommand {
    let execute = execute_command(command, grant_id, execution);
    validate_execute_command(&execute, execution, &AcceptVerifier).expect("authorized command")
}

fn execute_command(
    command: AgentCommand,
    grant_id: [u8; 32],
    execution: &ExecutionContext,
) -> ExecuteCommand {
    let mut execute = ExecuteCommand {
        request_token: u64::from(grant_id[0]).max(1),
        command_id: [grant_id[0].wrapping_add(20); 16],
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id,
                registration_id: execution.registration_id,
                registration_epoch: execution.registration_epoch,
                session_id: execution.session_id.clone(),
                peer: execution.peer.clone(),
                scopes: command.required_scopes(),
                policy_revision: execution.policy_revision,
                windows_session_id: execution.windows_session_id,
                desktop_epoch: execution.desktop_epoch,
                desktop_kind: execution.desktop_kind,
                issued_at_ms: 1_000,
                not_before_ms: 1_000,
                expires_at_ms: 2_000,
                command_digest: [0; 32],
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: ISSUER_KEY_ID,
            signature: [9; 64],
        },
        command,
    };
    execute.grant.claims.command_digest = execute.command_digest();
    execute
}

fn event(
    resource_id: [u8; 16],
    start_grant_id: [u8; 32],
    sequence: u64,
    payload: InputEventPayload,
) -> InputEventEnvelope {
    InputEventEnvelope {
        request_token: sequence,
        session_id: session_id(),
        resource_id,
        start_grant_id,
        sequence,
        event: payload,
    }
}

fn event_for_session(
    session_id: SessionId,
    resource_id: [u8; 16],
    start_grant_id: [u8; 32],
    sequence: u64,
    payload: InputEventPayload,
) -> InputEventEnvelope {
    InputEventEnvelope {
        session_id,
        ..event(resource_id, start_grant_id, sequence, payload)
    }
}

fn consent_request(request_token: u64, request_id: [u8; 16]) -> ConsentRequest {
    ConsentRequest {
        request_token,
        request_id,
        session_id: session_id(),
        peer: peer(),
        requested_scopes: scopes(&[
            PermissionScope::InputPointer,
            PermissionScope::InputKeyboard,
        ]),
        policy_revision: 3,
        windows_session_id: 7,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
        authorization_expires_at_ms: 2_500,
    }
}

async fn start_attended_input_runtime(
    injector: SharedInjector,
) -> (
    tokio::task::JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    tokio::io::DuplexStream,
) {
    start_attended_runtime(
        Arc::new(ApproveRequestedScopes),
        Box::new(NoopExecutor),
        Some(Box::new(InputResourceManager::new(injector))),
        true,
        true,
    )
    .await
}

async fn start_attended_runtime(
    backend: Arc<dyn ConsentBackend>,
    executor: Box<dyn AuthorizedCommandExecutor>,
    input: Option<Box<dyn InputBackend>>,
    expect_consent_capability: bool,
    expect_input_capability: bool,
) -> (
    tokio::task::JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    tokio::io::DuplexStream,
) {
    let desktop = Arc::new(Mutex::new(TrustedDesktopState {
        desktop_epoch: 11,
        desktop_kind: DesktopKind::Default,
    }));
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: SessionDescriptor::new([10; 16], 4_242, 55, [11; 32], 7, [12; 32], 11)
                .unwrap(),
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(1),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .unwrap()
    .with_attended_authority(
        backend,
        Arc::new(AcceptVerifier),
        Arc::new(MutableDesktop { state: desktop }),
        ISSUER_KEY_ID,
        executor,
    )
    .expect("valid attended authority configuration");
    let runtime = match input {
        Some(input) => runtime.with_input_backend(input),
        None => runtime,
    };
    let (agent_stream, mut service_stream) = tokio::io::duplex(32 * 1024);
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    let mut protocol = AgentProtocolState::new();
    protocol.accept_register(register.clone()).unwrap();
    let challenge = AgentChallenge {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        challenge_id: [13; 16],
        challenge_nonce: [14; 32],
        expected_agent_instance_id: register.agent_instance_id,
        expected_process_id: register.process_id,
        expected_process_creation_time: register.process_creation_time,
        expected_logon_sid_hash: register.logon_sid_hash,
        expected_windows_session_id: register.windows_session_id,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
    };
    protocol.issue_challenge(challenge.clone()).unwrap();
    write_frame(
        &mut service_stream,
        &ServiceToAgent::AgentChallenge(challenge),
    )
    .await
    .unwrap();
    let registered = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::AgentRegistered(registered) => registered,
        other => panic!("expected registration proof, got {other:?}"),
    };
    protocol
        .complete_registration(registered, 1_500, &FixedRegistrationVerifier)
        .unwrap();
    let capabilities = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected capabilities, got {other:?}"),
    };
    assert_eq!(
        capabilities
            .capabilities
            .contains(&mrd_agent_ipc::AgentCapability::Consent),
        expect_consent_capability
    );
    assert_eq!(
        capabilities
            .capabilities
            .contains(&mrd_agent_ipc::AgentCapability::Input),
        expect_input_capability
    );
    (agent, service_stream)
}

async fn establish_pressed_input(service: &mut tokio::io::DuplexStream) {
    let first_consent = consent_request(20, [12; 16]);
    write_frame(
        &mut *service,
        &ServiceToAgent::ConsentRequest(first_consent.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut *service)
        .await
        .unwrap()
        .message
    {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, first_consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
        }
        other => panic!("expected first consent result, got {other:?}"),
    }

    let start = execute_command(
        AgentCommand::StartInput {
            resource_id: RESOURCE_ID,
            input_scopes: scopes(&[PermissionScope::InputKeyboard]),
        },
        START_GRANT_ID,
        &context(),
    );
    let start_token = start.request_token;
    write_frame(&mut *service, &ServiceToAgent::Execute(Box::new(start)))
        .await
        .unwrap();
    match read_frame::<_, AgentToService>(&mut *service)
        .await
        .unwrap()
        .message
    {
        AgentToService::CommandResult(result) => {
            assert_eq!(result.request_token, start_token);
            assert_eq!(result.outcome, CommandOutcome::Completed)
        }
        other => panic!("expected StartInput result, got {other:?}"),
    }

    let key_down = event(
        RESOURCE_ID,
        START_GRANT_ID,
        1,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    );
    write_frame(&mut *service, &ServiceToAgent::InputEvent(key_down.clone()))
        .await
        .unwrap();
    match read_frame::<_, AgentToService>(&mut *service)
        .await
        .unwrap()
        .message
    {
        AgentToService::InputAck(ack) => {
            assert_eq!(ack.request_token, key_down.request_token);
            assert_eq!(ack.outcome, InputAckOutcome::Applied);
        }
        other => panic!("expected input acknowledgment, got {other:?}"),
    }
}

#[test]
fn events_require_the_exact_start_resource_scope_and_live_context() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            scopes(&[PermissionScope::InputPointer]),
        ))
        .expect("start pointer input");

    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                1,
                InputEventPayload::MouseMove { x: 40, y: 50 },
            ),
            &context(),
        ),
        InputAckOutcome::Applied
    );
    assert_eq!(events.lock().expect("events").len(), 1);

    let wrong_scope = event(
        RESOURCE_ID,
        START_GRANT_ID,
        2,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    );
    assert_eq!(
        manager.handle(&wrong_scope, &context()),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );

    let mut wrong_session = event(
        RESOURCE_ID,
        START_GRANT_ID,
        2,
        InputEventPayload::MouseWheel { delta: 120 },
    );
    wrong_session.session_id = SessionId("other-session".to_owned());
    assert_eq!(
        manager.handle(&wrong_session, &context()),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );
    assert_eq!(
        manager.handle(
            &event(
                OTHER_RESOURCE_ID,
                START_GRANT_ID,
                2,
                InputEventPayload::MouseWheel { delta: 120 },
            ),
            &context(),
        ),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                OTHER_START_GRANT_ID,
                2,
                InputEventPayload::MouseWheel { delta: 120 },
            ),
            &context(),
        ),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );

    let mut wrong_registration = context();
    wrong_registration.registration_epoch += 1;
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                2,
                InputEventPayload::MouseWheel { delta: 120 },
            ),
            &wrong_registration,
        ),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );

    let mut stale_desktop = context();
    stale_desktop.desktop_epoch += 1;
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                2,
                InputEventPayload::MouseWheel { delta: 120 },
            ),
            &stale_desktop,
        ),
        InputAckOutcome::Rejected {
            reason: InputRejection::StaleDesktop,
        }
    );
    assert_eq!(events.lock().expect("events").len(), 1);
}

#[test]
fn resource_sequence_is_exactly_once_and_conflicts_are_rejected() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            scopes(&[PermissionScope::InputPointer]),
        ))
        .unwrap();
    let first = event(
        RESOURCE_ID,
        START_GRANT_ID,
        10,
        InputEventPayload::MouseMove { x: 1, y: 2 },
    );
    assert_eq!(manager.handle(&first, &context()), InputAckOutcome::Applied);
    assert_eq!(manager.handle(&first, &context()), InputAckOutcome::Applied);
    assert_eq!(events.lock().expect("events").len(), 1);

    let conflict = event(
        RESOURCE_ID,
        START_GRANT_ID,
        10,
        InputEventPayload::MouseMove { x: 9, y: 9 },
    );
    assert_eq!(
        manager.handle(&conflict, &context()),
        InputAckOutcome::Rejected {
            reason: InputRejection::Replay,
        }
    );
    let gap = event(
        RESOURCE_ID,
        START_GRANT_ID,
        12,
        InputEventPayload::MouseWheel { delta: -120 },
    );
    assert_eq!(manager.handle(&gap, &context()), InputAckOutcome::Applied);
    let late = event(
        RESOURCE_ID,
        START_GRANT_ID,
        11,
        InputEventPayload::MouseWheel { delta: 120 },
    );
    assert_eq!(
        manager.handle(&late, &context()),
        InputAckOutcome::Rejected {
            reason: InputRejection::Replay,
        }
    );
    assert_eq!(events.lock().expect("events").len(), 2);
}

#[test]
fn one_start_grant_cannot_establish_a_second_resource() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(events));
    let pointer = scopes(&[PermissionScope::InputPointer]);
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            pointer.clone(),
        ))
        .unwrap();
    assert_eq!(
        manager.start(authorized_start(OTHER_RESOURCE_ID, START_GRANT_ID, pointer,)),
        Err(InputRejection::Replay)
    );
}

#[test]
fn release_all_releases_pressed_state_and_is_allowed_after_desktop_change() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            scopes(&[
                PermissionScope::InputPointer,
                PermissionScope::InputKeyboard,
            ]),
        ))
        .unwrap();
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                1,
                InputEventPayload::MouseButton {
                    button: InputButton::Left,
                    pressed: true,
                },
            ),
            &context(),
        ),
        InputAckOutcome::Applied
    );
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                2,
                InputEventPayload::Key {
                    key: InputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            ),
            &context(),
        ),
        InputAckOutcome::Applied
    );
    let mut changed = context();
    changed.desktop_epoch += 1;
    changed.desktop_kind = DesktopKind::Secure;
    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                3,
                InputEventPayload::ReleaseAll,
            ),
            &changed,
        ),
        InputAckOutcome::Applied
    );
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::MouseButton {
                button: mrd_input::InputButton::Left,
                pressed: true,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::MouseButton {
                button: mrd_input::InputButton::Left,
                pressed: false,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: false,
            },
        ]
    );
    manager.release_all().expect("disconnect cleanup");
    assert_eq!(events.lock().expect("events").len(), 4);
}

#[test]
fn stopping_one_resource_does_not_release_a_key_held_by_another() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    let keyboard = scopes(&[PermissionScope::InputKeyboard]);
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            keyboard.clone(),
        ))
        .unwrap();
    manager
        .start(authorized_start(
            OTHER_RESOURCE_ID,
            OTHER_START_GRANT_ID,
            keyboard,
        ))
        .unwrap();
    for (resource_id, grant_id) in [
        (RESOURCE_ID, START_GRANT_ID),
        (OTHER_RESOURCE_ID, OTHER_START_GRANT_ID),
    ] {
        assert_eq!(
            manager.handle(
                &event(
                    resource_id,
                    grant_id,
                    1,
                    InputEventPayload::Key {
                        key: InputKey::VirtualKey { code: 0x41 },
                        pressed: true,
                    },
                ),
                &context(),
            ),
            InputAckOutcome::Applied
        );
    }
    assert_eq!(events.lock().expect("events").len(), 1);

    assert_eq!(manager.stop(&RESOURCE_ID), InputAckOutcome::Applied);
    assert_eq!(events.lock().expect("events").len(), 1);
    assert_eq!(manager.stop(&OTHER_RESOURCE_ID), InputAckOutcome::Applied);
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: false,
            },
        ]
    );
}

#[test]
fn release_session_removes_only_the_target_sessions_resources() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    let pointer = scopes(&[PermissionScope::InputPointer]);
    let first_context = context();
    let second_context = context_for(other_session_id());
    manager
        .start(authorized_command(
            AgentCommand::StartInput {
                resource_id: RESOURCE_ID,
                input_scopes: pointer.clone(),
            },
            START_GRANT_ID,
            &first_context,
        ))
        .unwrap();
    manager
        .start(authorized_command(
            AgentCommand::StartInput {
                resource_id: OTHER_RESOURCE_ID,
                input_scopes: pointer,
            },
            OTHER_START_GRANT_ID,
            &second_context,
        ))
        .unwrap();

    assert_eq!(
        manager.handle(
            &event_for_session(
                other_session_id(),
                OTHER_RESOURCE_ID,
                OTHER_START_GRANT_ID,
                1,
                InputEventPayload::MouseMove { x: 3, y: 4 },
            ),
            &second_context,
        ),
        InputAckOutcome::Applied
    );

    manager
        .release_session(&session_id())
        .expect("targeted cleanup succeeds");

    assert_eq!(
        manager.handle(
            &event(
                RESOURCE_ID,
                START_GRANT_ID,
                1,
                InputEventPayload::MouseMove { x: 1, y: 2 },
            ),
            &first_context,
        ),
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        }
    );
    assert_eq!(
        manager.handle(
            &event_for_session(
                other_session_id(),
                OTHER_RESOURCE_ID,
                OTHER_START_GRANT_ID,
                2,
                InputEventPayload::MouseMove { x: 5, y: 6 },
            ),
            &second_context,
        ),
        InputAckOutcome::Applied
    );
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::MouseMove { x: 3, y: 4 },
            InputEvent::MouseMove { x: 5, y: 6 },
        ]
    );
}

#[test]
fn release_session_keeps_a_shared_key_down_until_the_last_session_releases_it() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut manager = InputResourceManager::new(SharedInjector::available(Arc::clone(&events)));
    let keyboard = scopes(&[PermissionScope::InputKeyboard]);
    let first_context = context();
    let second_context = context_for(other_session_id());
    manager
        .start(authorized_command(
            AgentCommand::StartInput {
                resource_id: RESOURCE_ID,
                input_scopes: keyboard.clone(),
            },
            START_GRANT_ID,
            &first_context,
        ))
        .unwrap();
    manager
        .start(authorized_command(
            AgentCommand::StartInput {
                resource_id: OTHER_RESOURCE_ID,
                input_scopes: keyboard,
            },
            OTHER_START_GRANT_ID,
            &second_context,
        ))
        .unwrap();
    for (session, resource_id, grant_id, execution) in [
        (session_id(), RESOURCE_ID, START_GRANT_ID, &first_context),
        (
            other_session_id(),
            OTHER_RESOURCE_ID,
            OTHER_START_GRANT_ID,
            &second_context,
        ),
    ] {
        assert_eq!(
            manager.handle(
                &event_for_session(
                    session,
                    resource_id,
                    grant_id,
                    1,
                    InputEventPayload::Key {
                        key: InputKey::VirtualKey { code: 0x41 },
                        pressed: true,
                    },
                ),
                execution,
            ),
            InputAckOutcome::Applied
        );
    }
    assert_eq!(events.lock().expect("events").len(), 1);

    manager
        .release_session(&session_id())
        .expect("first session cleanup succeeds");
    assert_eq!(events.lock().expect("events").len(), 1);
    manager
        .release_session(&other_session_id())
        .expect("second session cleanup succeeds");
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: false,
            },
        ]
    );
}

#[test]
fn cleanup_attempts_every_pressed_transition_and_retains_failed_state_for_retry() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let injector = SharedInjector::available(Arc::clone(&events));
    let next_error = Arc::clone(&injector.next_error);
    let mut manager = InputResourceManager::new(injector);
    manager
        .start(authorized_start(
            RESOURCE_ID,
            START_GRANT_ID,
            scopes(&[PermissionScope::InputKeyboard]),
        ))
        .unwrap();
    for (sequence, code) in [(1, 0x41), (2, 0x42)] {
        assert_eq!(
            manager.handle(
                &event(
                    RESOURCE_ID,
                    START_GRANT_ID,
                    sequence,
                    InputEventPayload::Key {
                        key: InputKey::VirtualKey { code },
                        pressed: true,
                    },
                ),
                &context(),
            ),
            InputAckOutcome::Applied
        );
    }
    *next_error.lock().expect("error lock") = Some(InputError::UipiDenied);
    assert_eq!(manager.release_all(), Err(InputError::UipiDenied));
    assert_eq!(events.lock().expect("events").len(), 3);
    manager.release_all().expect("retry remaining release");
    assert_eq!(events.lock().expect("events").len(), 4);
}

#[test]
fn platform_failures_are_coarse_and_consume_the_sequence() {
    for (error, expected) in [
        (
            InputError::UipiDenied,
            InputAckOutcome::Failed {
                reason: InputFailure::Uipi,
            },
        ),
        (
            InputError::Platform("sensitive native detail".to_owned()),
            InputAckOutcome::Failed {
                reason: InputFailure::Platform,
            },
        ),
    ] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut manager =
            InputResourceManager::new(SharedInjector::failing(Arc::clone(&events), error));
        manager
            .start(authorized_start(
                RESOURCE_ID,
                START_GRANT_ID,
                scopes(&[PermissionScope::InputKeyboard]),
            ))
            .unwrap();
        let key = event(
            RESOURCE_ID,
            START_GRANT_ID,
            1,
            InputEventPayload::Key {
                key: InputKey::VirtualKey { code: 0x41 },
                pressed: true,
            },
        );
        assert_eq!(manager.handle(&key, &context()), expected);
        assert_eq!(manager.handle(&key, &context()), expected);
        assert!(events.lock().expect("events").is_empty());
    }
}

#[tokio::test]
async fn runtime_fresh_consent_releases_old_session_input_before_approval() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (agent, mut service_stream) =
        start_attended_input_runtime(SharedInjector::available(Arc::clone(&events))).await;

    let first_consent = consent_request(20, [12; 16]);
    write_frame(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(first_consent.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, first_consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
        }
        other => panic!("expected first consent result, got {other:?}"),
    }

    let start = execute_command(
        AgentCommand::StartInput {
            resource_id: RESOURCE_ID,
            input_scopes: scopes(&[PermissionScope::InputKeyboard]),
        },
        START_GRANT_ID,
        &context(),
    );
    let start_token = start.request_token;
    write_frame(
        &mut service_stream,
        &ServiceToAgent::Execute(Box::new(start)),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::CommandResult(result) => {
            assert_eq!(result.request_token, start_token);
            assert_eq!(result.outcome, CommandOutcome::Completed)
        }
        other => panic!("expected StartInput result, got {other:?}"),
    }

    let key_down = event(
        RESOURCE_ID,
        START_GRANT_ID,
        1,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    );
    write_frame(
        &mut service_stream,
        &ServiceToAgent::InputEvent(key_down.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::InputAck(ack) => {
            assert_eq!(ack.request_token, key_down.request_token);
            assert_eq!(ack.outcome, InputAckOutcome::Applied);
            assert_eq!(ack.event_commitment, key_down.commitment().unwrap());
        }
        other => panic!("expected input acknowledgment, got {other:?}"),
    }
    assert_eq!(events.lock().expect("events").len(), 1);

    let mut cached_replay = first_consent.clone();
    cached_replay.request_token = 22;
    write_frame(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(cached_replay.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_token, cached_replay.request_token);
            assert_eq!(result.decision, ConsentDecision::Approved);
            assert_eq!(
                events.lock().expect("events").len(),
                1,
                "cached approval must not release the current input generation"
            );
        }
        other => panic!("expected cached consent result, got {other:?}"),
    }

    let event_after_cached_replay = event(
        RESOURCE_ID,
        START_GRANT_ID,
        2,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    );
    write_frame(
        &mut service_stream,
        &ServiceToAgent::InputEvent(event_after_cached_replay.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::InputAck(ack) => assert_eq!(ack.outcome, InputAckOutcome::Applied),
        other => panic!("expected input acknowledgment after cached replay, got {other:?}"),
    }

    let replacement_consent = consent_request(21, [13; 16]);
    write_frame(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(replacement_consent.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, replacement_consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
            assert_eq!(
                events.lock().expect("events").as_slice(),
                &[
                    InputEvent::Key {
                        key: mrd_input::InputKey::VirtualKey(0x41),
                        pressed: true,
                    },
                    InputEvent::Key {
                        key: mrd_input::InputKey::VirtualKey(0x41),
                        pressed: false,
                    },
                ],
                "old input must be released before fresh approval is visible"
            );
        }
        other => panic!("expected replacement consent result, got {other:?}"),
    }

    let old_resource_event = event(
        RESOURCE_ID,
        START_GRANT_ID,
        3,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x42 },
            pressed: true,
        },
    );
    write_frame(
        &mut service_stream,
        &ServiceToAgent::InputEvent(old_resource_event.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::InputAck(ack) => {
            assert_eq!(ack.request_token, old_resource_event.request_token);
            assert_eq!(
                ack.outcome,
                InputAckOutcome::Rejected {
                    reason: InputRejection::Grant,
                }
            );
        }
        other => panic!("expected cleanup acknowledgment, got {other:?}"),
    }
    assert_eq!(events.lock().expect("events").len(), 2);

    write_frame(
        &mut service_stream,
        &ServiceToAgent::StopAgent(StopAgent {
            request_id: [15; 16],
            deadline_ms: 2_000,
            reason: StopReason::ServiceShutdown,
        }),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .unwrap()
        .message
    {
        AgentToService::AgentStopping(_) => {}
        other => panic!("expected stopping event, got {other:?}"),
    }
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: false,
            },
        ]
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), agent)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        AgentExit::StoppedByService
    );
}

#[tokio::test]
async fn runtime_cleanup_failure_withholds_approval_and_fails_closed() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let injector = SharedInjector::available(Arc::clone(&events));
    let next_error = Arc::clone(&injector.next_error);
    let (agent, mut service) = start_attended_input_runtime(injector).await;
    establish_pressed_input(&mut service).await;
    *next_error.lock().expect("input error lock") = Some(InputError::Platform(
        "sensitive native cleanup detail".to_owned(),
    ));

    let replacement = consent_request(21, [13; 16]);
    write_frame(&mut service, &ServiceToAgent::ConsentRequest(replacement))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if next_error.lock().expect("input error lock").is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("targeted cleanup attempt timeout");
    let late_old_event = event(
        RESOURCE_ID,
        START_GRANT_ID,
        2,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x42 },
            pressed: true,
        },
    );
    let _ = write_frame(&mut service, &ServiceToAgent::InputEvent(late_old_event)).await;

    let next_frame = tokio::time::timeout(
        Duration::from_secs(1),
        read_frame::<_, AgentToService>(&mut service),
    )
    .await
    .expect("agent must fail-stop promptly");
    assert!(
        next_frame.is_err(),
        "fresh Approved must not be sent after targeted cleanup fails"
    );
    let error = tokio::time::timeout(Duration::from_secs(1), agent)
        .await
        .expect("agent failure timeout")
        .expect("agent join")
        .expect_err("cleanup failure must close the agent connection");
    assert!(matches!(
        error,
        mrd_session_agent::runtime::AgentRuntimeError::InputCleanupFailed
    ));
    assert_eq!(error.to_string(), "session input cleanup failed");
    assert!(!error
        .to_string()
        .contains("sensitive native cleanup detail"));
    assert_eq!(
        events.lock().expect("events").as_slice(),
        &[
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(0x41),
                pressed: false,
            },
        ],
        "outer shutdown must retry the failed targeted release"
    );
}

#[tokio::test]
async fn generic_executor_cannot_claim_or_execute_input_without_input_backend() {
    let executions = Arc::new(AtomicUsize::new(0));
    let (agent, mut service) = start_attended_runtime(
        Arc::new(ApproveRequestedScopes),
        Box::new(ReservedCapabilityClaimingExecutor {
            executions: Arc::clone(&executions),
        }),
        None,
        true,
        false,
    )
    .await;

    let consent = consent_request(30, [30; 16]);
    write_frame(
        &mut service,
        &ServiceToAgent::ConsentRequest(consent.clone()),
    )
    .await
    .unwrap();
    match read_frame::<_, AgentToService>(&mut service)
        .await
        .unwrap()
        .message
    {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
        }
        other => panic!("expected consent result, got {other:?}"),
    }

    let start = execute_command(
        AgentCommand::StartInput {
            resource_id: RESOURCE_ID,
            input_scopes: scopes(&[PermissionScope::InputKeyboard]),
        },
        START_GRANT_ID,
        &context(),
    );
    write_frame(&mut service, &ServiceToAgent::Execute(Box::new(start)))
        .await
        .unwrap();
    match read_frame::<_, AgentToService>(&mut service)
        .await
        .unwrap()
        .message
    {
        AgentToService::CommandResult(result) => {
            assert_eq!(result.outcome, CommandOutcome::Rejected)
        }
        other => panic!("expected rejected StartInput result, got {other:?}"),
    }
    assert_eq!(executions.load(Ordering::SeqCst), 0);
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

#[tokio::test]
async fn generic_executor_cannot_spoof_unavailable_consent_capability() {
    let executions = Arc::new(AtomicUsize::new(0));
    let (agent, service) = start_attended_runtime(
        Arc::new(UnavailableConsentBackend),
        Box::new(ReservedCapabilityClaimingExecutor {
            executions: Arc::clone(&executions),
        }),
        None,
        false,
        false,
    )
    .await;
    assert_eq!(executions.load(Ordering::SeqCst), 0);
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
