use mrd_agent_ipc::{
    read_frame, write_frame, AgentCapability, AgentCapabilitySnapshot, AgentChallenge,
    AgentProtocolState, AgentStopping, AgentToService, AuthorizedCommand, CancelConsent,
    CommandOutcome, ConsentCancelReason, ConsentDecision, ConsentRequest, ConsentResult,
    ExecuteGrantVerifier, PeerBinding, RegistrationProofVerifier, ServiceToAgent, StopAgent,
    StopReason, StoppingReason,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use mrd_session_agent::{
    capabilities::AgentCapabilities,
    consent::{
        ConsentAbortReason, ConsentBackend, ConsentBackendDecision, ConsentBackendFuture,
        ConsentPrompt,
    },
    runtime::{
        AgentClock, AgentExit, AgentRuntime, AgentRuntimeConfig, AuthorizedCommandExecutor,
        RegistrationSigner, RegistrationSigningError, SessionDescriptor, TrustedDesktopState,
        TrustedDesktopStateSource,
    },
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, DuplexStream},
    sync::{mpsc, oneshot, watch, Notify},
    task::JoinHandle,
};

const AGENT_INSTANCE_ID: [u8; 16] = [1; 16];
const REGISTRATION_ID: [u8; 16] = [2; 16];
const CHALLENGE_ID: [u8; 16] = [3; 16];
const AGENT_KEY_ID: [u8; 32] = [4; 32];
const EXECUTE_ISSUER_KEY_ID: [u8; 32] = [5; 32];
const TEST_TIMEOUT: Duration = Duration::from_millis(500);

struct FixedClock;

impl AgentClock for FixedClock {
    fn now_ms(&self) -> u64 {
        1_500
    }
}

struct FixedSigner;

fn test_signature(message: &[u8]) -> [u8; 64] {
    let mut signature = [0_u8; 64];
    for (index, byte) in message.iter().enumerate() {
        signature[index % signature.len()] ^= *byte;
    }
    signature[0] |= 1;
    signature
}

impl RegistrationSigner for FixedSigner {
    fn key_id(&self) -> [u8; 32] {
        AGENT_KEY_ID
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; 64], RegistrationSigningError> {
        Ok(test_signature(message))
    }
}

struct FixedVerifier;

impl RegistrationProofVerifier for FixedVerifier {
    fn verify(&self, _agent_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool {
        test_signature(signing_bytes) == *signature
    }
}

struct RejectExecuteGrant;

impl ExecuteGrantVerifier for RejectExecuteGrant {
    fn verify(
        &self,
        _issuer_key_id: &[u8; 32],
        _signing_bytes: &[u8],
        _signature: &[u8; 64],
    ) -> bool {
        false
    }
}

struct EmptyExecutor;

impl AuthorizedCommandExecutor for EmptyExecutor {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::empty()
    }

    fn execute(&mut self, _command: AuthorizedCommand) -> CommandOutcome {
        CommandOutcome::Rejected
    }
}

struct DefaultDesktop;

fn stable_desktop_subscription() -> watch::Receiver<()> {
    static CHANGES: OnceLock<watch::Sender<()>> = OnceLock::new();
    CHANGES.get_or_init(|| watch::channel(()).0).subscribe()
}

impl TrustedDesktopStateSource for DefaultDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        Some(TrustedDesktopState {
            desktop_epoch: 9,
            desktop_kind: mrd_agent_ipc::DesktopKind::Default,
        })
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        stable_desktop_subscription()
    }
}

struct SecureDesktop;

impl TrustedDesktopStateSource for SecureDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        Some(TrustedDesktopState {
            desktop_epoch: 10,
            desktop_kind: mrd_agent_ipc::DesktopKind::Secure,
        })
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        stable_desktop_subscription()
    }
}

struct UnavailableDesktop;

impl TrustedDesktopStateSource for UnavailableDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        None
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        stable_desktop_subscription()
    }
}

struct WatchDesktop {
    state: Mutex<Option<TrustedDesktopState>>,
    changes: watch::Sender<()>,
}

impl WatchDesktop {
    fn new(state: TrustedDesktopState) -> Self {
        let (changes, _) = watch::channel(());
        Self {
            state: Mutex::new(Some(state)),
            changes,
        }
    }

    fn set(&self, state: Option<TrustedDesktopState>) {
        *self.state.lock().expect("desktop state") = state;
        self.changes.send_replace(());
    }
}

impl TrustedDesktopStateSource for WatchDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        *self.state.lock().expect("desktop state")
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }
}

struct SubscribeBaselineRaceDesktop {
    state: Mutex<TrustedDesktopState>,
    changes: watch::Sender<()>,
    raced: AtomicBool,
}

impl SubscribeBaselineRaceDesktop {
    fn new() -> Self {
        let (changes, _) = watch::channel(());
        Self {
            state: Mutex::new(TrustedDesktopState {
                desktop_epoch: 9,
                desktop_kind: mrd_agent_ipc::DesktopKind::Default,
            }),
            changes,
            raced: AtomicBool::new(false),
        }
    }
}

impl TrustedDesktopStateSource for SubscribeBaselineRaceDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        if !self.raced.swap(true, Ordering::SeqCst) {
            *self.state.lock().expect("desktop state") = TrustedDesktopState {
                desktop_epoch: 10,
                desktop_kind: mrd_agent_ipc::DesktopKind::Secure,
            };
            self.changes.send_replace(());
        }
        Some(*self.state.lock().expect("desktop state"))
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }
}

struct StartedPrompt {
    prompt: ConsentPrompt,
    abort: watch::Receiver<Option<ConsentAbortReason>>,
    respond: oneshot::Sender<ConsentBackendDecision>,
}

struct BackendState {
    started: mpsc::Sender<StartedPrompt>,
    available: AtomicBool,
    visible: AtomicUsize,
    maximum_visible: AtomicUsize,
}

struct TestBackend {
    state: Arc<BackendState>,
}

struct VisiblePromptGuard {
    state: Arc<BackendState>,
}

impl Drop for VisiblePromptGuard {
    fn drop(&mut self) {
        self.state.visible.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ConsentBackend for TestBackend {
    fn is_available(&self) -> bool {
        self.state.available.load(Ordering::SeqCst)
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let visible = state.visible.fetch_add(1, Ordering::SeqCst) + 1;
            state.maximum_visible.fetch_max(visible, Ordering::SeqCst);
            let _guard = VisiblePromptGuard {
                state: Arc::clone(&state),
            };
            let (respond, decision) = oneshot::channel();
            if state
                .started
                .send(StartedPrompt {
                    prompt,
                    abort,
                    respond,
                })
                .await
                .is_err()
            {
                return ConsentBackendDecision::Dismissed;
            }
            decision.await.unwrap_or(ConsentBackendDecision::Cancelled)
        })
    }
}

struct BarrierBackendState {
    entered: Notify,
    dropped: AtomicBool,
}

struct BarrierBackend {
    state: Arc<BarrierBackendState>,
}

struct BarrierFutureGuard(Arc<BarrierBackendState>);

impl Drop for BarrierFutureGuard {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

impl ConsentBackend for BarrierBackend {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        _prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let _guard = BarrierFutureGuard(Arc::clone(&state));
            state.entered.notify_one();
            std::future::pending().await
        })
    }
}

#[derive(Clone, Copy)]
enum PanicMode {
    Constructor,
    Future,
}

struct PanicOnceBackend {
    mode: PanicMode,
    calls: AtomicUsize,
    fallback: Arc<TestBackend>,
}

struct PanicOnAbortBackend {
    calls: AtomicUsize,
    entered: Notify,
    fallback: Arc<TestBackend>,
}

struct AvailabilityPanicBackend;

impl ConsentBackend for PanicOnceBackend {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            match self.mode {
                PanicMode::Constructor => panic!("test consent constructor panic"),
                PanicMode::Future => {
                    return Box::pin(async { panic!("test consent future panic") });
                }
            }
        }
        self.fallback.prompt(prompt, abort)
    }
}

impl ConsentBackend for PanicOnAbortBackend {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        mut abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        if self.calls.fetch_add(1, Ordering::SeqCst) != 0 {
            return self.fallback.prompt(prompt, abort);
        }
        self.entered.notify_one();
        Box::pin(async move {
            while abort.borrow_and_update().is_none() {
                if abort.changed().await.is_err() {
                    return ConsentBackendDecision::Cancelled;
                }
            }
            panic!("test panic while closing consent surface")
        })
    }
}

impl ConsentBackend for AvailabilityPanicBackend {
    fn is_available(&self) -> bool {
        panic!("test availability panic")
    }

    fn prompt(
        &self,
        _prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        panic!("unavailable backend must not construct a prompt")
    }
}

fn backend() -> (
    Arc<TestBackend>,
    Arc<BackendState>,
    mpsc::Receiver<StartedPrompt>,
) {
    backend_with_availability(true)
}

fn backend_with_availability(
    available: bool,
) -> (
    Arc<TestBackend>,
    Arc<BackendState>,
    mpsc::Receiver<StartedPrompt>,
) {
    let (started, receiver) = mpsc::channel(32);
    let state = Arc::new(BackendState {
        started,
        available: AtomicBool::new(available),
        visible: AtomicUsize::new(0),
        maximum_visible: AtomicUsize::new(0),
    });
    (
        Arc::new(TestBackend {
            state: Arc::clone(&state),
        }),
        state,
        receiver,
    )
}

fn descriptor() -> SessionDescriptor {
    SessionDescriptor::new(AGENT_INSTANCE_ID, 4_242, 55, [6; 32], 7, [8; 32], 1)
        .expect("valid fixed descriptor")
}

async fn start_agent(
    backend: Arc<dyn ConsentBackend>,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
) {
    start_agent_with_environment(backend, Arc::new(DefaultDesktop), true).await
}

async fn start_agent_with_environment(
    backend: Arc<dyn ConsentBackend>,
    desktop: Arc<dyn TrustedDesktopStateSource>,
    expect_consent_capability: bool,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
) {
    let (agent, service, _) =
        start_agent_with_environment_and_snapshot(backend, desktop, expect_consent_capability)
            .await;
    (agent, service)
}

async fn start_agent_with_environment_and_snapshot(
    backend: Arc<dyn ConsentBackend>,
    desktop: Arc<dyn TrustedDesktopStateSource>,
    expect_consent_capability: bool,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
    AgentCapabilitySnapshot,
) {
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_millis(10),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid fixed runtime")
    .with_attended_authority(
        backend,
        Arc::new(RejectExecuteGrant),
        desktop,
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    )
    .expect("valid attended authority configuration");
    start_configured_agent(runtime, expect_consent_capability).await
}

async fn start_configured_agent(
    runtime: AgentRuntime,
    expect_consent_capability: bool,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
    AgentCapabilitySnapshot,
) {
    let (agent_stream, mut service_stream) = tokio::io::duplex(32 * 1024);
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    let mut registration = AgentProtocolState::new();
    registration
        .accept_register(register.clone())
        .expect("accept register");
    let challenge = AgentChallenge {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        challenge_id: CHALLENGE_ID,
        challenge_nonce: [10; 32],
        expected_agent_instance_id: register.agent_instance_id,
        expected_process_id: register.process_id,
        expected_process_creation_time: register.process_creation_time,
        expected_logon_sid_hash: register.logon_sid_hash,
        expected_windows_session_id: register.windows_session_id,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
    };
    registration
        .issue_challenge(challenge.clone())
        .expect("issue challenge");
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::AgentChallenge(challenge),
    )
    .await;
    let registered = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentRegistered(registered) => registered,
        other => panic!("expected registered proof, got {other:?}"),
    };
    registration
        .complete_registration(registered, 1_500, &FixedVerifier)
        .expect("verify registration proof");
    let capabilities = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected capability snapshot, got {other:?}"),
    };
    assert_eq!(
        capabilities
            .capabilities
            .contains(&AgentCapability::Consent),
        expect_consent_capability
    );
    (agent, service_stream, capabilities)
}

fn consent_request(seed: u8) -> ConsentRequest {
    ConsentRequest {
        request_token: u64::from(seed),
        request_id: [seed; 16],
        session_id: SessionId(format!("session-{seed}")),
        peer: PeerBinding {
            device_id: DeviceId(format!("peer-{seed}")),
            key_id: [seed.saturating_add(20); 32],
        },
        requested_scopes: [PermissionScope::ScreenView, PermissionScope::InputPointer]
            .into_iter()
            .collect(),
        policy_revision: u64::from(seed),
        windows_session_id: 7,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
        authorization_expires_at_ms: 2_500,
    }
}

async fn read_agent_message<R>(reader: &mut R) -> AgentToService
where
    R: AsyncRead + Unpin,
{
    tokio::time::timeout(TEST_TIMEOUT, read_frame::<_, AgentToService>(reader))
        .await
        .expect("agent message timeout")
        .expect("read agent message")
        .message
}

async fn send_service_message<W>(writer: &mut W, message: &ServiceToAgent)
where
    W: AsyncWrite + Unpin,
{
    tokio::time::timeout(TEST_TIMEOUT, write_frame(writer, message))
        .await
        .expect("service message timeout")
        .expect("write service message");
}

async fn stop_agent(
    agent: JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    service: &mut DuplexStream,
) {
    send_service_message(
        service,
        &ServiceToAgent::StopAgent(StopAgent {
            request_id: [99; 16],
            deadline_ms: 2_500,
            reason: StopReason::ServiceShutdown,
        }),
    )
    .await;
    loop {
        match read_agent_message(service).await {
            AgentToService::AgentStopping(AgentStopping { reason, .. }) => {
                assert_eq!(reason, StoppingReason::ServiceRequest);
                break;
            }
            AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected heartbeat or stopping, got {other:?}"),
        }
    }
    let exit = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("agent stop timeout")
        .expect("agent join")
        .expect("agent exit");
    assert_eq!(exit, AgentExit::StoppedByService);
}

async fn next_started(receiver: &mut mpsc::Receiver<StartedPrompt>) -> StartedPrompt {
    tokio::time::timeout(TEST_TIMEOUT, receiver.recv())
        .await
        .expect("backend start timeout")
        .expect("backend start channel closed")
}

async fn next_consent_result(service: &mut DuplexStream) -> ConsentResult {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match read_agent_message(service).await {
                AgentToService::ConsentResult(result) => return result,
                AgentToService::AgentHeartbeat(_) => {}
                other => panic!("expected consent result or heartbeat, got {other:?}"),
            }
        }
    })
    .await
    .expect("consent result timeout")
}

fn empty_scopes() -> PermissionScopes {
    PermissionScopes::new()
}

#[tokio::test]
async fn pending_prompt_does_not_block_heartbeats() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(1);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;

    let prompt = next_started(&mut started).await;
    assert_eq!(prompt.prompt.session_id(), &request.session_id);
    assert_eq!(prompt.prompt.peer(), &request.peer);
    assert_eq!(prompt.prompt.requested_scopes(), &request.requested_scopes);
    match read_agent_message(&mut service).await {
        AgentToService::AgentHeartbeat(_) => {}
        other => panic!("prompt must stay pending; expected heartbeat, got {other:?}"),
    }

    prompt
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("complete prompt");
    loop {
        match read_agent_message(&mut service).await {
            AgentToService::ConsentResult(result) => {
                assert_eq!(result.request_id, request.request_id);
                assert_eq!(result.decision, ConsentDecision::Dismissed);
                assert_eq!(result.approved_scopes, empty_scopes());
                break;
            }
            AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected consent result or heartbeat, got {other:?}"),
        }
    }
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn stop_agent_finishes_while_backend_never_resolves() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(consent_request(2)),
    )
    .await;
    let mut prompt = next_started(&mut started).await;

    send_service_message(
        &mut service,
        &ServiceToAgent::StopAgent(StopAgent {
            request_id: [98; 16],
            deadline_ms: 2_500,
            reason: StopReason::ServiceShutdown,
        }),
    )
    .await;
    tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed())
        .await
        .expect("backend abort notification timeout")
        .expect("backend abort sender dropped");
    assert_eq!(
        *prompt.abort.borrow_and_update(),
        Some(ConsentAbortReason::RuntimeStopping)
    );
    loop {
        match read_agent_message(&mut service).await {
            AgentToService::AgentStopping(_) => break,
            AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected heartbeat or stopping, got {other:?}"),
        }
    }
    let exit = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("agent must not wait for backend")
        .expect("agent join")
        .expect("agent exit");
    assert_eq!(exit, AgentExit::StoppedByService);
    assert_eq!(state.visible.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn manager_never_has_more_than_one_visible_prompt() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(3);
    let second_request = consent_request(4);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(second_request.clone()),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "second prompt must wait until the first surface closes"
    );
    assert_eq!(state.visible.load(Ordering::SeqCst), 1);

    first
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("complete first prompt");
    let second = next_started(&mut started).await;
    assert_eq!(second.prompt.session_id(), &second_request.session_id);
    assert_eq!(state.maximum_visible.load(Ordering::SeqCst), 1);
    second
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("complete second prompt");

    let mut completed = 0;
    while completed < 2 {
        match read_agent_message(&mut service).await {
            AgentToService::ConsentResult(result) => {
                assert!(
                    result.request_id == first_request.request_id
                        || result.request_id == second_request.request_id
                );
                completed += 1;
            }
            AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected consent result or heartbeat, got {other:?}"),
        }
    }
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn exact_active_cancel_tombstones_before_backend_closes() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(5);
    let second_request = consent_request(6);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let mut first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(second_request.clone()),
    )
    .await;
    send_service_message(
        &mut service,
        &ServiceToAgent::CancelConsent(CancelConsent {
            request_token: first_request.request_token,
            request_id: first_request.request_id,
            session_id: first_request.session_id.clone(),
            reason: ConsentCancelReason::CallerAborted,
        }),
    )
    .await;

    tokio::time::timeout(TEST_TIMEOUT, first.abort.changed())
        .await
        .expect("active cancel notification timeout")
        .expect("active cancel watch closed");
    assert_eq!(
        *first.abort.borrow_and_update(),
        Some(ConsentAbortReason::Service(
            ConsentCancelReason::CallerAborted
        ))
    );
    let cancelled = next_consent_result(&mut service).await;
    assert_eq!(cancelled.request_id, first_request.request_id);
    assert_eq!(cancelled.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "closing active prompt must retain the sole visible slot"
    );
    assert_eq!(state.visible.load(Ordering::SeqCst), 1);

    first
        .respond
        .send(ConsentBackendDecision::Cancelled)
        .expect("confirm first surface closed");
    let second = next_started(&mut started).await;
    assert_eq!(second.prompt.session_id(), &second_request.session_id);
    second
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close second prompt");
    let second_result = next_consent_result(&mut service).await;
    assert_eq!(second_result.request_id, second_request.request_id);
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn duplicate_exact_cancel_preserves_first_terminal_and_closing_slot() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(79);
    let queued_request = consent_request(80);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let mut first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(queued_request.clone()),
    )
    .await;
    let exact = |reason| {
        ServiceToAgent::CancelConsent(CancelConsent {
            request_token: first_request.request_token,
            request_id: first_request.request_id,
            session_id: first_request.session_id.clone(),
            reason,
        })
    };
    send_service_message(&mut service, &exact(ConsentCancelReason::CallerAborted)).await;
    tokio::time::timeout(TEST_TIMEOUT, first.abort.changed())
        .await
        .expect("first cancel notification timeout")
        .expect("first cancel watch closed");
    assert_eq!(
        *first.abort.borrow_and_update(),
        Some(ConsentAbortReason::Service(
            ConsentCancelReason::CallerAborted
        ))
    );
    let first_terminal = next_consent_result(&mut service).await;
    assert_eq!(first_terminal.decision, ConsentDecision::Dismissed);

    send_service_message(&mut service, &exact(ConsentCancelReason::PolicyChanged)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(40), first.abort.changed())
            .await
            .is_err(),
        "duplicate cancel must not overwrite the first abort reason"
    );
    assert_eq!(
        *first.abort.borrow(),
        Some(ConsentAbortReason::Service(
            ConsentCancelReason::CallerAborted
        ))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), next_consent_result(&mut service))
            .await
            .is_err(),
        "duplicate cancel must not emit another terminal result"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "duplicate cancel must preserve the closing active slot"
    );

    first
        .respond
        .send(ConsentBackendDecision::Cancelled)
        .expect("confirm first surface closed");
    let queued = next_started(&mut started).await;
    assert_eq!(queued.prompt.session_id(), &queued_request.session_id);
    queued
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close queued prompt");
    assert_eq!(
        next_consent_result(&mut service).await.request_id,
        queued_request.request_id
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn exact_queued_cancel_never_reaches_backend() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(7);
    let queued_request = consent_request(8);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(queued_request.clone()),
    )
    .await;
    send_service_message(
        &mut service,
        &ServiceToAgent::CancelConsent(CancelConsent {
            request_token: queued_request.request_token,
            request_id: queued_request.request_id,
            session_id: queued_request.session_id.clone(),
            reason: ConsentCancelReason::SessionClosed,
        }),
    )
    .await;
    let cancelled = next_consent_result(&mut service).await;
    assert_eq!(cancelled.request_id, queued_request.request_id);
    assert_eq!(cancelled.decision, ConsentDecision::Dismissed);

    first
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close first prompt");
    let first_result = next_consent_result(&mut service).await;
    assert_eq!(first_result.request_id, first_request.request_id);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "cancelled queued prompt must never be shown"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn wrong_cancel_identity_is_ignored() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(9);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let mut prompt = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::CancelConsent(CancelConsent {
            request_token: request.request_token + 1,
            request_id: request.request_id,
            session_id: request.session_id.clone(),
            reason: ConsentCancelReason::PolicyChanged,
        }),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(40), prompt.abort.changed())
            .await
            .is_err(),
        "wrong transport token must not abort the active prompt"
    );

    prompt
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("complete unaffected prompt");
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Denied);
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn deadline_expires_active_and_late_approval_cannot_replace_tombstone() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let mut expiring = consent_request(10);
    expiring.expires_at_ms = 1_540;
    let following = consent_request(11);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(expiring.clone()),
    )
    .await;
    let mut first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(following.clone()),
    )
    .await;

    let expired = next_consent_result(&mut service).await;
    assert_eq!(expired.request_id, expiring.request_id);
    assert_eq!(expired.decision, ConsentDecision::Expired);
    tokio::time::timeout(TEST_TIMEOUT, first.abort.changed())
        .await
        .expect("deadline abort notification timeout")
        .expect("deadline abort watch closed");
    assert_eq!(
        *first.abort.borrow_and_update(),
        Some(ConsentAbortReason::PromptExpired)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "deadline enters closing and must retain the visible slot"
    );

    first
        .respond
        .send(ConsentBackendDecision::Approved(
            expiring.requested_scopes.clone(),
        ))
        .expect("return deliberately late approval");
    let second = next_started(&mut started).await;
    assert_eq!(second.prompt.session_id(), &following.session_id);

    let mut replay = expiring.clone();
    replay.request_token = 99;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    assert_eq!(cached.request_token, replay.request_token);
    assert_eq!(cached.request_id, expiring.request_id);
    assert_eq!(cached.decision, ConsentDecision::Expired);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "cached expired replay and late approval must not open another prompt"
    );

    second
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close following prompt");
    let following_result = next_consent_result(&mut service).await;
    assert_eq!(following_result.request_id, following.request_id);
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn earliest_queued_deadline_expires_without_becoming_visible() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(12);
    let mut expiring_queued = consent_request(13);
    expiring_queued.expires_at_ms = 1_530;
    let third_request = consent_request(14);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let first = next_started(&mut started).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(expiring_queued.clone()),
    )
    .await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(third_request.clone()),
    )
    .await;

    let expired = next_consent_result(&mut service).await;
    assert_eq!(expired.request_id, expiring_queued.request_id);
    assert_eq!(expired.decision, ConsentDecision::Expired);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), started.recv())
            .await
            .is_err(),
        "queued expiry must not invoke the backend"
    );

    first
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close first prompt");
    let third = next_started(&mut started).await;
    assert_eq!(third.prompt.session_id(), &third_request.session_id);
    third
        .respond
        .send(ConsentBackendDecision::Dismissed)
        .expect("close third prompt");
    let mut remaining = [first_request.request_id, third_request.request_id]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    while !remaining.is_empty() {
        remaining.remove(&next_consent_result(&mut service).await.request_id);
    }
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn pending_capacity_overflow_is_dismissed_without_backend_call() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    for seed in 20..52 {
        send_service_message(
            &mut service,
            &ServiceToAgent::ConsentRequest(consent_request(seed)),
        )
        .await;
    }
    let first = next_started(&mut started).await;
    assert_eq!(state.visible.load(Ordering::SeqCst), 1);
    let overflow = consent_request(52);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(overflow.clone()),
    )
    .await;

    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, overflow.request_id);
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "overflow request must not invoke the backend"
    );
    stop_agent(agent, &mut service).await;
    drop(first);
}

#[tokio::test]
async fn wrong_windows_session_is_dismissed_without_backend_call() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let mut request = consent_request(53);
    request.windows_session_id = 8;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;

    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "wrong-session request must not invoke the backend"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn inactive_request_is_expired_without_backend_call() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let mut request = consent_request(54);
    request.expires_at_ms = 1_400;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;

    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Expired);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "inactive request must not invoke the backend"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn completed_duplicate_is_cached_without_second_backend_call() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(55);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    next_started(&mut started)
        .await
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("complete original prompt");
    let original = next_consent_result(&mut service).await;
    assert_eq!(original.decision, ConsentDecision::Denied);

    let mut replay = request.clone();
    replay.request_token = 155;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    assert_eq!(cached.request_token, replay.request_token);
    assert_eq!(cached.request_id, request.request_id);
    assert_eq!(cached.decision, ConsentDecision::Denied);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "completed duplicate must be served from the tombstone"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn semantic_replay_conflict_closes_and_aborts_blocked_backend() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(56);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let mut prompt = next_started(&mut started).await;
    let mut conflicting = request;
    conflicting.request_token += 100;
    conflicting.policy_revision += 1;
    send_service_message(&mut service, &ServiceToAgent::ConsentRequest(conflicting)).await;

    tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed())
        .await
        .expect("conflict abort timeout")
        .expect("conflict abort watch closed");
    assert_eq!(
        *prompt.abort.borrow_and_update(),
        Some(ConsentAbortReason::ServiceDisconnected)
    );
    let error = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("conflict shutdown timeout")
        .expect("agent join")
        .expect_err("semantic replay conflict must close the connection");
    assert!(matches!(
        error,
        mrd_session_agent::runtime::AgentRuntimeError::ConsentStateUnavailable
    ));
    assert_eq!(state.visible.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn service_disconnect_aborts_and_joins_blocked_backend() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(consent_request(57)),
    )
    .await;
    let mut prompt = next_started(&mut started).await;
    drop(service);

    tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed())
        .await
        .expect("disconnect abort timeout")
        .expect("disconnect abort watch closed");
    assert_eq!(
        *prompt.abort.borrow_and_update(),
        Some(ConsentAbortReason::ServiceDisconnected)
    );
    let exit = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("disconnect shutdown timeout")
        .expect("agent join")
        .expect("agent exit");
    assert_eq!(exit, AgentExit::ServiceDisconnected);
    assert_eq!(state.visible.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn each_service_cancel_reason_is_forwarded_exactly() {
    for (index, reason) in [
        ConsentCancelReason::CallerAborted,
        ConsentCancelReason::TimedOut,
        ConsentCancelReason::SessionClosed,
        ConsentCancelReason::PolicyChanged,
    ]
    .into_iter()
    .enumerate()
    {
        let (backend, _state, mut started) = backend();
        let (agent, mut service) = start_agent(backend).await;
        let request = consent_request(60 + u8::try_from(index).expect("four reasons"));
        send_service_message(
            &mut service,
            &ServiceToAgent::ConsentRequest(request.clone()),
        )
        .await;
        let mut prompt = next_started(&mut started).await;
        send_service_message(
            &mut service,
            &ServiceToAgent::CancelConsent(CancelConsent {
                request_token: request.request_token,
                request_id: request.request_id,
                session_id: request.session_id.clone(),
                reason,
            }),
        )
        .await;
        tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed())
            .await
            .expect("service cancel notification timeout")
            .expect("service cancel watch closed");
        assert_eq!(
            *prompt.abort.borrow_and_update(),
            Some(ConsentAbortReason::Service(reason))
        );
        prompt
            .respond
            .send(ConsentBackendDecision::Cancelled)
            .expect("close cancelled surface");
        let result = next_consent_result(&mut service).await;
        assert_eq!(result.request_id, request.request_id);
        stop_agent(agent, &mut service).await;
    }
}

#[tokio::test]
async fn unavailable_backend_does_not_publish_consent_or_receive_prompt() {
    let (backend, _state, mut started) = backend_with_availability(false);
    let (agent, mut service) =
        start_agent_with_environment(backend, Arc::new(DefaultDesktop), false).await;
    let request = consent_request(70);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "unavailable backend must not receive a prompt"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn nondefault_desktop_does_not_publish_consent_or_receive_prompt() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) =
        start_agent_with_environment(backend, Arc::new(SecureDesktop), false).await;
    let request = consent_request(71);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "secure desktop must not invoke the consent backend"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn desktop_watch_immediately_publishes_revision_two_without_secure_capabilities() {
    let (backend, _state, _started) = backend();
    let desktop = Arc::new(WatchDesktop::new(TrustedDesktopState {
        desktop_epoch: 9,
        desktop_kind: mrd_agent_ipc::DesktopKind::Default,
    }));
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid runtime")
    .with_attended_authority(
        backend,
        Arc::new(RejectExecuteGrant),
        desktop.clone(),
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    )
    .expect("attended authority");
    let (agent, mut service, first) = start_configured_agent(runtime, true).await;
    assert_eq!(first.revision, 1);

    desktop.set(Some(TrustedDesktopState {
        desktop_epoch: 10,
        desktop_kind: mrd_agent_ipc::DesktopKind::Secure,
    }));

    let changed = match read_agent_message(&mut service).await {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected immediate capability revision, got {other:?}"),
    };
    assert_eq!(changed.revision, 2);
    assert_eq!(changed.desktop_epoch, 10);
    assert!(!changed.capabilities.contains(&AgentCapability::Input));
    assert!(!changed.capabilities.contains(&AgentCapability::Consent));

    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn subscribe_before_baseline_does_not_lose_a_racing_desktop_change() {
    let (backend, _state, _started) = backend();
    let desktop = Arc::new(SubscribeBaselineRaceDesktop::new());
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid runtime")
    .with_attended_authority(
        backend,
        Arc::new(RejectExecuteGrant),
        desktop,
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    )
    .expect("attended authority");
    let (agent, mut service, baseline) = start_configured_agent(runtime, false).await;
    assert_eq!(baseline.desktop_epoch, 10);
    assert!(!baseline.capabilities.contains(&AgentCapability::Consent));

    let observed = match read_agent_message(&mut service).await {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected retained desktop notification, got {other:?}"),
    };
    assert_eq!(observed.revision, 2);
    assert_eq!(observed.desktop_epoch, 10);
    assert!(!observed.capabilities.contains(&AgentCapability::Input));
    assert!(!observed.capabilities.contains(&AgentCapability::Consent));
    stop_agent(agent, &mut service).await;
}

#[test]
fn attended_authority_cannot_be_replaced_by_builder_order() {
    let (first_backend, _state, _started) = backend();
    let (second_backend, _state, _started) = backend();
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_millis(10),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid fixed runtime")
    .with_attended_authority(
        first_backend,
        Arc::new(RejectExecuteGrant),
        Arc::new(DefaultDesktop),
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    )
    .expect("first atomic authority installs");
    let replacement = runtime.with_attended_authority(
        second_backend,
        Arc::new(RejectExecuteGrant),
        Arc::new(SecureDesktop),
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    );
    assert!(matches!(
        replacement,
        Err(mrd_session_agent::runtime::AgentRuntimeError::InvalidConfiguration)
    ));
}

#[tokio::test]
async fn empty_approval_is_normalized_and_cached_as_dismissed() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(72);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    next_started(&mut started)
        .await
        .respond
        .send(ConsentBackendDecision::Approved(PermissionScopes::new()))
        .expect("return empty approval");
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(result.approved_scopes.is_empty());

    let mut replay = request;
    replay.request_token += 100;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    assert_eq!(cached.request_token, replay.request_token);
    assert_eq!(cached.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "normalized empty approval must be tombstoned"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn scope_escalation_is_normalized_and_cached_as_dismissed() {
    let (backend, _state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(73);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    next_started(&mut started)
        .await
        .respond
        .send(ConsentBackendDecision::Approved(
            [PermissionScope::InputKeyboard].into_iter().collect(),
        ))
        .expect("return escalated approval");
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    assert!(result.approved_scopes.is_empty());

    let mut replay = request;
    replay.request_token += 100;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    assert_eq!(cached.request_token, replay.request_token);
    assert_eq!(cached.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "scope escalation must be tombstoned"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn capability_and_heartbeat_share_the_consent_desktop_epoch() {
    let (backend, _state, _started) = backend();
    let (agent, mut service, snapshot) =
        start_agent_with_environment_and_snapshot(backend, Arc::new(DefaultDesktop), true).await;
    assert_eq!(snapshot.desktop_epoch, 9);
    let heartbeat = match read_agent_message(&mut service).await {
        AgentToService::AgentHeartbeat(heartbeat) => heartbeat,
        other => panic!("expected heartbeat after snapshot, got {other:?}"),
    };
    assert_eq!(heartbeat.context.desktop_epoch, snapshot.desktop_epoch);
    assert_eq!(heartbeat.context.registration_id, snapshot.registration_id);
    assert_eq!(
        heartbeat.context.windows_session_id,
        snapshot.windows_session_id
    );
    send_service_message(
        &mut service,
        &ServiceToAgent::StopAgent(StopAgent {
            request_id: [97; 16],
            deadline_ms: 2_500,
            reason: StopReason::ServiceShutdown,
        }),
    )
    .await;
    let stopping = loop {
        match read_agent_message(&mut service).await {
            AgentToService::AgentStopping(stopping) => break stopping,
            AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected heartbeat or stopping, got {other:?}"),
        }
    };
    assert_eq!(stopping.context.desktop_epoch, snapshot.desktop_epoch);
    assert_eq!(
        tokio::time::timeout(TEST_TIMEOUT, agent)
            .await
            .expect("agent stop timeout")
            .expect("agent join")
            .expect("agent exit"),
        AgentExit::StoppedByService
    );
}

#[tokio::test]
async fn unavailable_atomic_authority_desktop_fails_before_capabilities() {
    let (backend, _state, _started) = backend();
    let (agent_stream, mut service) = tokio::io::duplex(32 * 1024);
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_millis(10),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid fixed runtime")
    .with_attended_authority(
        backend,
        Arc::new(RejectExecuteGrant),
        Arc::new(UnavailableDesktop),
        EXECUTE_ISSUER_KEY_ID,
        Box::new(EmptyExecutor),
    )
    .expect("valid attended authority configuration");
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_agent_message(&mut service).await {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    send_service_message(
        &mut service,
        &ServiceToAgent::AgentChallenge(AgentChallenge {
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            challenge_id: CHALLENGE_ID,
            challenge_nonce: [10; 32],
            expected_agent_instance_id: register.agent_instance_id,
            expected_process_id: register.process_id,
            expected_process_creation_time: register.process_creation_time,
            expected_logon_sid_hash: register.logon_sid_hash,
            expected_windows_session_id: register.windows_session_id,
            issued_at_ms: 1_000,
            expires_at_ms: 2_000,
        }),
    )
    .await;
    assert!(matches!(
        read_agent_message(&mut service).await,
        AgentToService::AgentRegistered(_)
    ));

    let error = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("unavailable desktop must close before capabilities")
        .expect("agent join")
        .expect_err("the single authority desktop source must fail closed");
    assert!(matches!(
        error,
        mrd_session_agent::runtime::AgentRuntimeError::DesktopStateUnavailable
    ));
    let mut trailing = [0_u8; 1];
    assert_eq!(
        tokio::time::timeout(
            TEST_TIMEOUT,
            tokio::io::AsyncReadExt::read(&mut service, &mut trailing)
        )
        .await
        .expect("agent close timeout")
        .expect("read agent close"),
        0
    );
}

#[tokio::test]
async fn completed_replay_survives_backend_becoming_unavailable() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(74);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    next_started(&mut started)
        .await
        .respond
        .send(ConsentBackendDecision::Approved(
            [PermissionScope::ScreenView].into_iter().collect(),
        ))
        .expect("complete original prompt");
    let original = next_consent_result(&mut service).await;
    state.available.store(false, Ordering::SeqCst);

    let mut replay = request;
    replay.request_token += 100;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    let mut expected = original;
    expected.request_token = replay.request_token;
    assert_eq!(cached, expected);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "cached replay must not consult the unavailable backend"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn pending_semantic_conflict_survives_backend_becoming_unavailable() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let request = consent_request(75);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let mut prompt = next_started(&mut started).await;
    state.available.store(false, Ordering::SeqCst);
    let mut conflict = request;
    conflict.request_token += 100;
    conflict.peer.key_id[0] ^= 1;
    send_service_message(&mut service, &ServiceToAgent::ConsentRequest(conflict)).await;

    tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed())
        .await
        .expect("semantic conflict abort timeout")
        .expect("semantic conflict abort watch closed");
    assert_eq!(
        *prompt.abort.borrow_and_update(),
        Some(ConsentAbortReason::ServiceDisconnected)
    );
    let error = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("semantic conflict shutdown timeout")
        .expect("agent join")
        .expect_err("semantic conflict must close while backend unavailable");
    assert!(matches!(
        error,
        mrd_session_agent::runtime::AgentRuntimeError::ConsentStateUnavailable
    ));
}

#[tokio::test]
async fn queued_prompt_checks_availability_only_when_promoted() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    let first_request = consent_request(76);
    let queued_request = consent_request(77);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(first_request.clone()),
    )
    .await;
    let first = next_started(&mut started).await;
    state.available.store(false, Ordering::SeqCst);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(queued_request.clone()),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(40), next_consent_result(&mut service))
            .await
            .is_err(),
        "queued request must not be dismissed before reaching the visible slot"
    );

    first
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("close active surface");
    let first_result = next_consent_result(&mut service).await;
    let unavailable_result = next_consent_result(&mut service).await;
    assert_eq!(first_result.request_id, first_request.request_id);
    assert_eq!(first_result.decision, ConsentDecision::Denied);
    assert_eq!(unavailable_result.request_id, queued_request.request_id);
    assert_eq!(unavailable_result.decision, ConsentDecision::Dismissed);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "unavailable queued request must never invoke the backend"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn first_unavailable_dismissal_is_tombstoned() {
    let (backend, state, mut started) = backend_with_availability(false);
    let (agent, mut service) =
        start_agent_with_environment(backend, Arc::new(DefaultDesktop), false).await;
    let request = consent_request(78);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let first = next_consent_result(&mut service).await;
    assert_eq!(first.decision, ConsentDecision::Dismissed);
    state.available.store(true, Ordering::SeqCst);

    let mut replay = request;
    replay.request_token += 100;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(replay.clone()),
    )
    .await;
    let cached = next_consent_result(&mut service).await;
    let mut expected = first;
    expected.request_token = replay.request_token;
    assert_eq!(cached, expected);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), started.recv())
            .await
            .is_err(),
        "unavailable dismissal replay must stay cached after recovery"
    );
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn aborting_runtime_drops_the_active_backend_future() {
    let (backend, state, mut started) = backend();
    let (agent, mut service) = start_agent(backend).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(consent_request(83)),
    )
    .await;
    let mut prompt = next_started(&mut started).await;
    assert_eq!(state.visible.load(Ordering::SeqCst), 1);

    agent.abort();
    let join_error = tokio::time::timeout(TEST_TIMEOUT, agent)
        .await
        .expect("aborted runtime join timeout")
        .expect_err("runtime task must report cancellation");
    assert!(join_error.is_cancelled());
    tokio::time::timeout(TEST_TIMEOUT, async {
        while state.visible.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("backend future must be dropped with runtime");
    let _ = tokio::time::timeout(TEST_TIMEOUT, prompt.abort.changed()).await;
    assert_eq!(
        *prompt.abort.borrow(),
        Some(ConsentAbortReason::RuntimeStopping)
    );
    assert!(prompt
        .respond
        .send(ConsentBackendDecision::Approved(
            [PermissionScope::ScreenView].into_iter().collect(),
        ))
        .is_err());
}

#[tokio::test]
async fn startup_barrier_future_does_not_block_heartbeat_or_stop() {
    let state = Arc::new(BarrierBackendState {
        entered: Notify::new(),
        dropped: AtomicBool::new(false),
    });
    let backend = Arc::new(BarrierBackend {
        state: Arc::clone(&state),
    });
    let (agent, mut service) = start_agent(backend).await;
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(consent_request(84)),
    )
    .await;
    tokio::time::timeout(TEST_TIMEOUT, state.entered.notified())
        .await
        .expect("backend future was never polled");
    assert!(matches!(
        read_agent_message(&mut service).await,
        AgentToService::AgentHeartbeat(_)
    ));
    stop_agent(agent, &mut service).await;
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !state.dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stopping runtime must drop the startup-barrier future");
}

async fn assert_panicking_backend_does_not_wedge(mode: PanicMode, first_seed: u8) {
    let (fallback, _state, mut started) = backend();
    let backend = Arc::new(PanicOnceBackend {
        mode,
        calls: AtomicUsize::new(0),
        fallback,
    });
    let (agent, mut service) = start_agent(backend).await;
    let failed = consent_request(first_seed);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(failed.clone()),
    )
    .await;
    let failed_result = next_consent_result(&mut service).await;
    assert_eq!(failed_result.request_id, failed.request_id);
    assert_eq!(failed_result.decision, ConsentDecision::Dismissed);

    let following = consent_request(first_seed + 1);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(following.clone()),
    )
    .await;
    next_started(&mut started)
        .await
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("complete prompt after backend failure");
    let following_result = next_consent_result(&mut service).await;
    assert_eq!(following_result.request_id, following.request_id);
    assert_eq!(following_result.decision, ConsentDecision::Denied);
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn panicking_prompt_constructor_is_dismissed_and_next_progresses() {
    assert_panicking_backend_does_not_wedge(PanicMode::Constructor, 85).await;
}

#[tokio::test]
async fn panicking_prompt_future_is_dismissed_and_next_progresses() {
    assert_panicking_backend_does_not_wedge(PanicMode::Future, 87).await;
}

#[tokio::test]
async fn backend_failure_while_closing_releases_slot_for_next_prompt() {
    let (fallback, _state, mut started) = backend();
    let backend = Arc::new(PanicOnAbortBackend {
        calls: AtomicUsize::new(0),
        entered: Notify::new(),
        fallback,
    });
    let (agent, mut service) = start_agent(backend.clone()).await;
    let first = consent_request(89);
    let second = consent_request(90);
    send_service_message(&mut service, &ServiceToAgent::ConsentRequest(first.clone())).await;
    tokio::time::timeout(TEST_TIMEOUT, backend.entered.notified())
        .await
        .expect("first backend constructor did not run");
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(second.clone()),
    )
    .await;
    send_service_message(
        &mut service,
        &ServiceToAgent::CancelConsent(CancelConsent {
            request_token: first.request_token,
            request_id: first.request_id,
            session_id: first.session_id.clone(),
            reason: ConsentCancelReason::CallerAborted,
        }),
    )
    .await;
    let cancelled = next_consent_result(&mut service).await;
    assert_eq!(cancelled.request_id, first.request_id);
    assert_eq!(cancelled.decision, ConsentDecision::Dismissed);

    let promoted = next_started(&mut started).await;
    assert_eq!(promoted.prompt.session_id(), &second.session_id);
    promoted
        .respond
        .send(ConsentBackendDecision::Denied)
        .expect("complete promoted prompt");
    let second_result = next_consent_result(&mut service).await;
    assert_eq!(second_result.request_id, second.request_id);
    assert_eq!(second_result.decision, ConsentDecision::Denied);
    stop_agent(agent, &mut service).await;
}

#[tokio::test]
async fn availability_panic_is_fail_closed_for_snapshot_and_request() {
    let (agent, mut service) = start_agent_with_environment(
        Arc::new(AvailabilityPanicBackend),
        Arc::new(DefaultDesktop),
        false,
    )
    .await;
    let request = consent_request(91);
    send_service_message(
        &mut service,
        &ServiceToAgent::ConsentRequest(request.clone()),
    )
    .await;
    let result = next_consent_result(&mut service).await;
    assert_eq!(result.request_id, request.request_id);
    assert_eq!(result.decision, ConsentDecision::Dismissed);
    stop_agent(agent, &mut service).await;
}
