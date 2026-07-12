use mrd_agent_ipc::{
    encode_frame, read_frame, write_frame, AgentCapability, AgentChallenge, AgentCommand,
    AgentProtocolState, AgentStopping, AgentToService, AuthorizedCommand, CancelConsent,
    CommandOutcome, CommandResult, ConsentCancelReason, ConsentDecision, ConsentRequest,
    DesktopKind, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, ExecuteGrantVerifier,
    GrantAudience, PeerBinding, RegistrationProofVerifier, ServiceToAgent, StopAgent, StopReason,
    StoppingReason, AGENT_IPC_PROTOCOL_MAJOR, AGENT_IPC_PROTOCOL_MINOR,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::PermissionScope;
use mrd_session_agent::capabilities::AgentCapabilities;
use mrd_session_agent::consent::{
    ConsentBackend, ConsentBackendDecision, ConsentBackendFuture, ConsentPrompt,
};
use mrd_session_agent::runtime::{
    AgentClock, AgentExit, AgentRuntime, AgentRuntimeConfig, AgentRuntimeError,
    AuthorizedCommandExecutor, RegistrationSigner, RegistrationSigningError, SessionDescriptor,
    TrustedDesktopState, TrustedDesktopStateSource,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

const AGENT_INSTANCE_ID: [u8; 16] = [1; 16];
const REGISTRATION_ID: [u8; 16] = [2; 16];
const CHALLENGE_ID: [u8; 16] = [3; 16];
const AGENT_KEY_ID: [u8; 32] = [4; 32];
const GRANT_ISSUER_KEY_ID: [u8; 32] = [12; 32];
const TEST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(1);

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

struct FixedExecuteGrantVerifier;

impl ExecuteGrantVerifier for FixedExecuteGrantVerifier {
    fn verify(&self, issuer_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool {
        *issuer_key_id == GRANT_ISSUER_KEY_ID && test_signature(signing_bytes) == *signature
    }
}

struct MutableDesktopSource {
    state: Arc<RwLock<Option<TrustedDesktopState>>>,
    changes: watch::Sender<()>,
}

impl TrustedDesktopStateSource for MutableDesktopSource {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        self.state.read().ok().and_then(|state| *state)
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }
}

struct CountingCaptureExecutor {
    executions: Arc<AtomicUsize>,
}

impl AuthorizedCommandExecutor for CountingCaptureExecutor {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::from_implemented([AgentCapability::Capture])
    }

    fn execute(&mut self, command: AuthorizedCommand) -> CommandOutcome {
        match command.command() {
            AgentCommand::StartCapture {
                resource_id,
                display_id,
            } => {
                assert_eq!(resource_id, &[13; 16]);
                assert_eq!(*display_id, 0);
            }
            other => panic!("expected authorized start capture, got {other:?}"),
        }
        self.executions.fetch_add(1, Ordering::SeqCst);
        CommandOutcome::Completed
    }
}

struct ApproveScreenView;

impl ConsentBackend for ApproveScreenView {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        _prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<mrd_session_agent::consent::ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        Box::pin(async {
            ConsentBackendDecision::Approved(
                [mrd_session::PermissionScope::ScreenView]
                    .into_iter()
                    .collect(),
            )
        })
    }
}

fn execution_session_id() -> SessionId {
    SessionId("session-replay-test".to_owned())
}

fn execution_peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId("peer-replay-test".to_owned()),
        key_id: [11; 32],
    }
}

fn signed_start_capture(
    grant_id: [u8; 32],
    command_id: [u8; 16],
    desktop_epoch: u64,
) -> ExecuteCommand {
    let command = AgentCommand::StartCapture {
        resource_id: [13; 16],
        display_id: 0,
    };
    let scopes = command.required_scopes();
    let mut execute = ExecuteCommand {
        request_token: u64::from(command_id[0]).max(1),
        command_id,
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id,
                registration_id: REGISTRATION_ID,
                registration_epoch: 1,
                session_id: execution_session_id(),
                peer: execution_peer(),
                scopes,
                policy_revision: 1,
                windows_session_id: 7,
                desktop_epoch,
                desktop_kind: DesktopKind::Default,
                issued_at_ms: 1_000,
                not_before_ms: 1_000,
                expires_at_ms: 2_000,
                command_digest: [0; 32],
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: GRANT_ISSUER_KEY_ID,
            signature: [0; 64],
        },
        command,
    };
    execute.grant.claims.command_digest = execute.command_digest();
    execute.grant.signature = test_signature(&execute.grant.signing_bytes());
    execute
}

fn resign(execute: &mut ExecuteCommand) {
    execute.grant.signature = test_signature(&execute.grant.signing_bytes());
}

async fn read_agent_message<R>(reader: &mut R) -> AgentToService
where
    R: AsyncRead + Unpin,
{
    tokio::time::timeout(
        TEST_MESSAGE_TIMEOUT,
        read_frame::<_, AgentToService>(reader),
    )
    .await
    .expect("agent message timeout")
    .expect("read agent message")
    .message
}

async fn send_service_message<W>(writer: &mut W, message: &ServiceToAgent)
where
    W: AsyncWrite + Unpin,
{
    tokio::time::timeout(TEST_MESSAGE_TIMEOUT, write_frame(writer, message))
        .await
        .expect("service message timeout")
        .expect("write service message");
}

async fn send_execute_and_read_result<W>(stream: &mut W, execute: ExecuteCommand) -> CommandResult
where
    W: AsyncRead + AsyncWrite + Unpin,
{
    let request_token = execute.request_token;
    send_service_message(stream, &ServiceToAgent::Execute(Box::new(execute))).await;
    loop {
        match read_agent_message(stream).await {
            AgentToService::CommandResult(result) => {
                assert_eq!(result.request_token, request_token);
                return result;
            }
            AgentToService::AgentCapabilitySnapshot(_) | AgentToService::AgentHeartbeat(_) => {}
            other => panic!("expected command result, got {other:?}"),
        }
    }
}

fn descriptor() -> SessionDescriptor {
    SessionDescriptor::new(AGENT_INSTANCE_ID, 4_242, 55, [6; 32], 7, [8; 32], 1)
        .expect("valid fixed descriptor")
}

#[tokio::test]
async fn agent_registers_reports_capabilities_heartbeats_and_stops_cleanly() {
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
    .expect("valid fixed runtime");
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .expect("read register")
        .message
    {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    assert_eq!(register.process_id, 4_242);
    assert_eq!(register.logon_sid_hash, [6; 32]);
    assert_eq!(register.windows_session_id, 7);
    assert_eq!(register.agent_key_id, AGENT_KEY_ID);
    let mut registration_state = AgentProtocolState::new();
    registration_state
        .accept_register(register.clone())
        .expect("service accepts immutable register");

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
    registration_state
        .issue_challenge(challenge.clone())
        .expect("service issues bound challenge");
    write_frame(
        &mut service_stream,
        &ServiceToAgent::AgentChallenge(challenge),
    )
    .await
    .expect("send challenge");

    let registered = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .expect("read registered proof")
        .message
    {
        AgentToService::AgentRegistered(registered) => registered,
        other => panic!("expected registered proof, got {other:?}"),
    };
    assert_eq!(registered.registration_id, REGISTRATION_ID);
    assert_eq!(registered.challenge_id, CHALLENGE_ID);
    assert_eq!(registered.accepted_protocol_major, AGENT_IPC_PROTOCOL_MAJOR);
    assert_eq!(registered.accepted_protocol_minor, AGENT_IPC_PROTOCOL_MINOR);
    let identity = registration_state
        .complete_registration(registered, 1_500, &FixedVerifier)
        .expect("service verifies signed registration transcript");
    assert_eq!(identity.windows_session_id, 7);

    let snapshot = match read_frame::<_, AgentToService>(&mut service_stream)
        .await
        .expect("read capability snapshot")
        .message
    {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected capabilities, got {other:?}"),
    };
    assert_eq!(snapshot.agent_instance_id, AGENT_INSTANCE_ID);
    assert_eq!(snapshot.registration_id, REGISTRATION_ID);
    assert_eq!(snapshot.windows_session_id, 7);
    assert_eq!(snapshot.revision, 1);
    assert!(snapshot.capabilities.is_empty());

    send_service_message(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(ConsentRequest {
            request_token: 1,
            request_id: [12; 16],
            session_id: SessionId("consent-session".to_owned()),
            peer: PeerBinding {
                device_id: DeviceId("consent-peer".to_owned()),
                key_id: [13; 32],
            },
            requested_scopes: [PermissionScope::ScreenView].into_iter().collect(),
            policy_revision: 1,
            windows_session_id: 7,
            issued_at_ms: 1_000,
            expires_at_ms: 2_000,
            authorization_expires_at_ms: 2_500,
        }),
    )
    .await;
    let mut heartbeat_sequence = None;
    loop {
        match read_agent_message(&mut service_stream).await {
            AgentToService::ConsentResult(result) => {
                assert_eq!(result.request_token, 1);
                assert_eq!(result.request_id, [12; 16]);
                assert_eq!(result.decision, ConsentDecision::Dismissed);
                assert!(result.approved_scopes.is_empty());
                break;
            }
            AgentToService::AgentHeartbeat(heartbeat) => {
                heartbeat_sequence = Some(heartbeat.context.sequence);
            }
            other => panic!("expected consent result or heartbeat, got {other:?}"),
        }
    }

    let unauthorized = signed_start_capture([60; 32], [61; 16], 1);
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::Execute(Box::new(unauthorized)),
    )
    .await;
    loop {
        match read_agent_message(&mut service_stream).await {
            AgentToService::CommandResult(result) => {
                assert_eq!(result.outcome, CommandOutcome::Rejected);
                break;
            }
            AgentToService::AgentHeartbeat(heartbeat) => {
                heartbeat_sequence = Some(heartbeat.context.sequence);
            }
            other => panic!("expected rejected command or heartbeat, got {other:?}"),
        }
    }

    send_service_message(
        &mut service_stream,
        &ServiceToAgent::CancelConsent(CancelConsent {
            request_token: 1,
            request_id: [12; 16],
            session_id: SessionId("consent-session".to_owned()),
            reason: ConsentCancelReason::CallerAborted,
        }),
    )
    .await;
    let heartbeat_after_cancel = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentHeartbeat(heartbeat) => heartbeat.context.sequence,
        other => panic!("expected heartbeat after safe cancel consume, got {other:?}"),
    };
    assert!(heartbeat_after_cancel > 0);

    let heartbeat_sequence = match heartbeat_sequence {
        Some(sequence) => sequence,
        None => {
            let heartbeat = tokio::time::timeout(
                Duration::from_millis(250),
                read_frame::<_, AgentToService>(&mut service_stream),
            )
            .await
            .expect("heartbeat timeout")
            .expect("read heartbeat");
            match heartbeat.message {
                AgentToService::AgentHeartbeat(heartbeat) => heartbeat.context.sequence,
                other => panic!("expected heartbeat, got {other:?}"),
            }
        }
    };

    let stop_frame = encode_frame(&ServiceToAgent::StopAgent(StopAgent {
        request_id: [11; 16],
        deadline_ms: 2_500,
        reason: StopReason::ServiceShutdown,
    }))
    .expect("encode fragmented stop");
    service_stream
        .write_all(&stop_frame[..3])
        .await
        .expect("send partial stop header");
    tokio::time::sleep(Duration::from_millis(35)).await;
    service_stream
        .write_all(&stop_frame[3..])
        .await
        .expect("finish fragmented stop");

    let stopping = loop {
        match read_frame::<_, AgentToService>(&mut service_stream)
            .await
            .expect("read heartbeat or stopping")
            .message
        {
            AgentToService::AgentHeartbeat(heartbeat) => {
                assert!(heartbeat.context.sequence > heartbeat_sequence);
            }
            AgentToService::AgentStopping(AgentStopping { context, reason }) => {
                break (context, reason);
            }
            other => panic!("expected heartbeat or stopping, got {other:?}"),
        }
    };
    assert_eq!(stopping.1, StoppingReason::ServiceRequest);
    assert!(stopping.0.sequence > heartbeat_sequence);

    let exit = tokio::time::timeout(Duration::from_secs(1), agent)
        .await
        .expect("agent stop timeout")
        .expect("agent task join")
        .expect("agent exit");
    assert_eq!(exit, AgentExit::StoppedByService);
}

#[tokio::test]
async fn execute_replays_are_cached_and_semantic_conflicts_close_the_agent() {
    let (agent_stream, mut service_stream) = tokio::io::duplex(32 * 1024);
    let executions = Arc::new(AtomicUsize::new(0));
    let desktop_state = Arc::new(RwLock::new(Some(TrustedDesktopState {
        desktop_epoch: 3,
        desktop_kind: DesktopKind::Default,
    })));
    let (desktop_changes, _) = watch::channel(());
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid fixed runtime")
    .with_attended_authority(
        Arc::new(ApproveScreenView),
        Arc::new(FixedExecuteGrantVerifier),
        Arc::new(MutableDesktopSource {
            state: Arc::clone(&desktop_state),
            changes: desktop_changes,
        }),
        GRANT_ISSUER_KEY_ID,
        Box::new(CountingCaptureExecutor {
            executions: Arc::clone(&executions),
        }),
    )
    .expect("valid attended authority configuration");
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    let mut registration_state = AgentProtocolState::new();
    registration_state
        .accept_register(register.clone())
        .expect("service accepts immutable register");
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
    registration_state
        .issue_challenge(challenge.clone())
        .expect("service issues bound challenge");
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::AgentChallenge(challenge),
    )
    .await;

    let registered = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentRegistered(registered) => registered,
        other => panic!("expected registered proof, got {other:?}"),
    };
    registration_state
        .complete_registration(registered, 1_500, &FixedVerifier)
        .expect("service verifies signed registration transcript");

    let snapshot = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
        other => panic!("expected capabilities, got {other:?}"),
    };
    assert_eq!(snapshot.registration_id, REGISTRATION_ID);
    assert!(snapshot.capabilities.contains(&AgentCapability::Capture));
    assert!(snapshot.capabilities.contains(&AgentCapability::Consent));

    let before_consent = signed_start_capture([44; 32], [45; 16], 3);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, before_consent)
            .await
            .outcome,
        CommandOutcome::Rejected
    );
    assert_eq!(executions.load(Ordering::SeqCst), 0);

    let consent = ConsentRequest {
        request_token: 12,
        request_id: [12; 16],
        session_id: execution_session_id(),
        peer: execution_peer(),
        requested_scopes: [mrd_session::PermissionScope::ScreenView]
            .into_iter()
            .collect(),
        policy_revision: 1,
        windows_session_id: 7,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
        authorization_expires_at_ms: 2_500,
    };
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(consent.clone()),
    )
    .await;
    match read_agent_message(&mut service_stream).await {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
        }
        other => panic!("expected consent result, got {other:?}"),
    }

    let mut wrong_session = signed_start_capture([30; 32], [31; 16], 3);
    wrong_session.grant.claims.session_id = SessionId("untrusted-session".to_owned());
    resign(&mut wrong_session);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, wrong_session)
            .await
            .outcome,
        CommandOutcome::Rejected
    );

    let mut wrong_peer = signed_start_capture([32; 32], [33; 16], 3);
    wrong_peer.grant.claims.peer.key_id = [99; 32];
    resign(&mut wrong_peer);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, wrong_peer)
            .await
            .outcome,
        CommandOutcome::Rejected
    );

    let mut wrong_policy = signed_start_capture([34; 32], [35; 16], 3);
    wrong_policy.grant.claims.policy_revision += 1;
    resign(&mut wrong_policy);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, wrong_policy)
            .await
            .outcome,
        CommandOutcome::Rejected
    );

    let mut wrong_desktop = signed_start_capture([36; 32], [37; 16], 3);
    wrong_desktop.grant.claims.desktop_kind = DesktopKind::Secure;
    resign(&mut wrong_desktop);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, wrong_desktop)
            .await
            .outcome,
        CommandOutcome::Rejected
    );
    assert_eq!(executions.load(Ordering::SeqCst), 0);

    *desktop_state.write().expect("desktop state lock") = Some(TrustedDesktopState {
        desktop_epoch: 4,
        desktop_kind: DesktopKind::Secure,
    });
    let stale_after_switch = signed_start_capture([38; 32], [39; 16], 3);
    assert_eq!(
        send_execute_and_read_result(&mut service_stream, stale_after_switch)
            .await
            .outcome,
        CommandOutcome::Rejected
    );
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    *desktop_state.write().expect("desktop state lock") = Some(TrustedDesktopState {
        desktop_epoch: 5,
        desktop_kind: DesktopKind::Default,
    });

    let first = signed_start_capture([20; 32], [21; 16], 5);
    let revoked = send_execute_and_read_result(&mut service_stream, first.clone()).await;
    assert_eq!(revoked.outcome, CommandOutcome::Rejected);
    assert_eq!(executions.load(Ordering::SeqCst), 0);

    let mut replacement_consent = consent;
    replacement_consent.request_token = 13;
    replacement_consent.request_id = [13; 16];
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::ConsentRequest(replacement_consent.clone()),
    )
    .await;
    match read_agent_message(&mut service_stream).await {
        AgentToService::ConsentResult(result) => {
            assert_eq!(result.request_id, replacement_consent.request_id);
            assert_eq!(result.decision, ConsentDecision::Approved);
        }
        other => panic!("expected replacement consent result, got {other:?}"),
    }

    let first_result = send_execute_and_read_result(&mut service_stream, first.clone()).await;
    assert_eq!(first_result.command_id, [21; 16]);
    assert_eq!(first_result.outcome, CommandOutcome::Completed);
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let exact_replay = send_execute_and_read_result(&mut service_stream, first.clone()).await;
    assert_eq!(exact_replay, first_result);
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let new_grant_same_command = signed_start_capture([22; 32], [21; 16], 5);
    let command_id_replay =
        send_execute_and_read_result(&mut service_stream, new_grant_same_command).await;
    assert_eq!(command_id_replay, first_result);
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let conflicting_command = signed_start_capture([20; 32], [23; 16], 5);
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::Execute(Box::new(conflicting_command)),
    )
    .await;

    let runtime_error = tokio::time::timeout(TEST_MESSAGE_TIMEOUT, agent)
        .await
        .expect("agent conflict shutdown timeout")
        .expect("agent task join")
        .expect_err("semantic replay conflict must fail closed");
    assert!(matches!(runtime_error, AgentRuntimeError::ReplayConflict));
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let mut trailing = [0_u8; 1];
    let read = tokio::time::timeout(TEST_MESSAGE_TIMEOUT, service_stream.read(&mut trailing))
        .await
        .expect("agent connection close timeout")
        .expect("read agent connection close");
    assert_eq!(
        read, 0,
        "agent must close the private stream on replay conflict"
    );
}

#[tokio::test]
async fn mismatched_registration_challenge_fails_before_capabilities() {
    let (agent_stream, mut service_stream) = tokio::io::duplex(8 * 1024);
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: descriptor(),
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_millis(250),
        },
        Arc::new(FixedClock),
        Arc::new(FixedSigner),
    )
    .expect("valid fixed runtime");
    let agent = tokio::spawn(runtime.run(agent_stream));

    let register = match read_agent_message(&mut service_stream).await {
        AgentToService::AgentRegister(register) => register,
        other => panic!("expected register, got {other:?}"),
    };
    send_service_message(
        &mut service_stream,
        &ServiceToAgent::AgentChallenge(AgentChallenge {
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            challenge_id: CHALLENGE_ID,
            challenge_nonce: [10; 32],
            expected_agent_instance_id: register.agent_instance_id,
            expected_process_id: register.process_id,
            expected_process_creation_time: register.process_creation_time,
            expected_logon_sid_hash: register.logon_sid_hash,
            expected_windows_session_id: register.windows_session_id + 1,
            issued_at_ms: 1_000,
            expires_at_ms: 2_000,
        }),
    )
    .await;

    let runtime_error = tokio::time::timeout(TEST_MESSAGE_TIMEOUT, agent)
        .await
        .expect("invalid challenge shutdown timeout")
        .expect("agent task join")
        .expect_err("mismatched challenge must fail closed");
    assert!(matches!(runtime_error, AgentRuntimeError::InvalidChallenge));

    let mut trailing = [0_u8; 1];
    let read = tokio::time::timeout(TEST_MESSAGE_TIMEOUT, service_stream.read(&mut trailing))
        .await
        .expect("agent connection close timeout")
        .expect("read agent connection close");
    assert_eq!(read, 0, "no capability snapshot may follow a bad challenge");
}
