use mrd_agent_ipc::{
    read_frame, write_frame, AgentCapability, AgentChallenge, AgentProtocolState, AgentStopping,
    AgentToService, CancelConsent, ConsentCancelReason, ConsentDecision, ConsentRequest,
    ConsentResult, PeerBinding, RegistrationProofVerifier, ServiceToAgent, StopAgent, StopReason,
    StoppingReason,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use mrd_session_agent::{
    consent::{
        ConsentAbortReason, ConsentBackend, ConsentBackendDecision, ConsentBackendFuture,
        ConsentPrompt,
    },
    runtime::{
        AgentClock, AgentExit, AgentRuntime, AgentRuntimeConfig, RegistrationSigner,
        RegistrationSigningError, SessionDescriptor, TrustedDesktopState,
        TrustedDesktopStateSource,
    },
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, DuplexStream},
    sync::{mpsc, oneshot, watch},
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

struct DefaultDesktop;

impl TrustedDesktopStateSource for DefaultDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        Some(TrustedDesktopState {
            desktop_epoch: 9,
            desktop_kind: mrd_agent_ipc::DesktopKind::Default,
        })
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
}

struct StartedPrompt {
    prompt: ConsentPrompt,
    abort: watch::Receiver<Option<ConsentAbortReason>>,
    respond: oneshot::Sender<ConsentBackendDecision>,
}

struct BackendState {
    started: mpsc::Sender<StartedPrompt>,
    visible: AtomicUsize,
    maximum_visible: AtomicUsize,
}

struct TestBackend {
    state: Arc<BackendState>,
    available: bool,
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
        self.available
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
        visible: AtomicUsize::new(0),
        maximum_visible: AtomicUsize::new(0),
    });
    (
        Arc::new(TestBackend {
            state: Arc::clone(&state),
            available,
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
    backend: Arc<TestBackend>,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
) {
    start_agent_with_environment(backend, Arc::new(DefaultDesktop), true).await
}

async fn start_agent_with_environment(
    backend: Arc<TestBackend>,
    desktop: Arc<dyn TrustedDesktopStateSource>,
    expect_consent_capability: bool,
) -> (
    JoinHandle<Result<AgentExit, mrd_session_agent::runtime::AgentRuntimeError>>,
    DuplexStream,
) {
    let (agent_stream, mut service_stream) = tokio::io::duplex(32 * 1024);
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
    .with_consent_backend(backend, desktop, EXECUTE_ISSUER_KEY_ID)
    .expect("valid consent manager configuration");
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
    (agent, service_stream)
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
