use mrd_agent_ipc::{
    read_frame, write_frame, AgentCapability, AgentCapabilitySnapshot, AgentCommand, AgentRegister,
    AgentRegistered, AgentToService, CommandOutcome, CommandResult, ConsentDecision,
    ConsentRequest, ConsentResult, DesktopKind, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims,
    GrantAudience, InputAck, InputAckOutcome, InputEventEnvelope, InputEventPayload, PeerBinding,
    RegistrationProofVerifier, ServiceToAgent,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_service::agent_runtime::{
    AgentBinding, AgentCallerKind, AgentConnectionExit, AgentConnectionId, AgentRegistry,
    AgentRegistryError, AgentRequestError, AgentServer, AgentServerClock, AgentServerError,
    ChallengeMaterial, ChallengeMaterialSource, ExpectedAgentSession, ObservedAgentIdentity,
    ReplacementPolicy,
};
use mrd_session::{PermissionScope, PermissionScopes};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::DuplexStream;

const NOW_MS: u64 = 1_000;
const WINDOWS_SESSION_ID: u32 = 7;
const PROCESS_ID: u32 = 42;
const CONNECTION_BYTES: [u8; 16] = [1; 16];

#[derive(Default)]
struct DeterministicChallenges {
    next: AtomicU8,
}

impl ChallengeMaterialSource for DeterministicChallenges {
    fn next_material(&self) -> Result<ChallengeMaterial, AgentRegistryError> {
        let value = self.next.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        Ok(ChallengeMaterial {
            registration_id: [value; 16],
            challenge_id: [value.saturating_add(32); 16],
            challenge_nonce: [value.saturating_add(64); 32],
        })
    }
}

struct AcceptProof;

impl RegistrationProofVerifier for AcceptProof {
    fn verify(
        &self,
        _agent_key_id: &[u8; 32],
        _signing_bytes: &[u8],
        signature: &[u8; 64],
    ) -> bool {
        signature.iter().any(|byte| *byte != 0)
    }
}

struct FixedClock;

impl AgentServerClock for FixedClock {
    fn now_ms(&self) -> u64 {
        NOW_MS
    }
}

struct ConnectedAgent {
    registry: Arc<AgentRegistry>,
    server: Arc<AgentServer>,
    binding: AgentBinding,
    identity: mrd_agent_ipc::RegisteredAgentIdentity,
    stream: DuplexStream,
    serving: tokio::task::JoinHandle<
        Result<AgentConnectionExit, mrd_service::agent_runtime::AgentServerError>,
    >,
}

impl ConnectedAgent {
    async fn start(request_timeout: Duration) -> Self {
        Self::start_with_capacity(request_timeout, 32 * 1024).await
    }

    async fn start_with_capacity(request_timeout: Duration, stream_capacity: usize) -> Self {
        let (registry, server) = Self::shared_server(request_timeout);
        Self::connect_to(
            registry,
            server,
            WINDOWS_SESSION_ID,
            PROCESS_ID,
            1,
            stream_capacity,
            [AgentCapability::Consent, AgentCapability::Input]
                .into_iter()
                .collect(),
            ReplacementPolicy::RejectExisting,
        )
        .await
    }

    fn shared_server(request_timeout: Duration) -> (Arc<AgentRegistry>, Arc<AgentServer>) {
        let registry = Arc::new(AgentRegistry::with_challenge_source(Arc::new(
            DeterministicChallenges::default(),
        )));
        let server = Arc::new(AgentServer::with_clock_and_request_timeout(
            Arc::clone(&registry),
            Arc::new(FixedClock),
            request_timeout,
        ));
        (registry, server)
    }

    #[allow(clippy::too_many_arguments)]
    async fn connect_to(
        registry: Arc<AgentRegistry>,
        server: Arc<AgentServer>,
        windows_session_id: u32,
        process_id: u32,
        connection_value: u8,
        stream_capacity: usize,
        capabilities: BTreeSet<AgentCapability>,
        replacement_policy: ReplacementPolicy,
    ) -> Self {
        let logon_sid_hash = [windows_session_id as u8; 32];
        let agent_key_id = [process_id as u8; 32];
        registry
            .expect_session_at(
                ExpectedAgentSession {
                    windows_session_id,
                    logon_sid_hash,
                    process_id,
                    process_creation_time: u64::from(process_id) * 10,
                    agent_key_id,
                    expires_at_ms: 20_000,
                    replacement_policy,
                },
                Arc::new(AcceptProof),
                NOW_MS - 1,
            )
            .expect("install expected agent");
        let connection_id = connection_id_for(connection_value);
        let (service_stream, mut stream) = tokio::io::duplex(stream_capacity);
        let serving = tokio::spawn({
            let server = Arc::clone(&server);
            async move {
                server
                    .serve_connection(
                        service_stream,
                        connection_id,
                        observed_for(windows_session_id, process_id),
                    )
                    .await
            }
        });

        let register = register_for(windows_session_id, process_id);
        write_frame(
            &mut stream,
            &AgentToService::AgentRegister(register.clone()),
        )
        .await
        .expect("send register");
        let challenge = match read_frame::<_, ServiceToAgent>(&mut stream)
            .await
            .expect("read challenge")
            .message
        {
            ServiceToAgent::AgentChallenge(challenge) => challenge,
            other => panic!("expected challenge, got {other:?}"),
        };
        write_frame(
            &mut stream,
            &AgentToService::AgentRegistered(AgentRegistered {
                registration_id: challenge.registration_id,
                registration_epoch: challenge.registration_epoch,
                challenge_id: challenge.challenge_id,
                agent_instance_id: register.agent_instance_id,
                accepted_protocol_major: 1,
                accepted_protocol_minor: 0,
                signed_at_ms: NOW_MS,
                signature: [11; 64],
            }),
        )
        .await
        .expect("send registration proof");
        let identity = mrd_agent_ipc::RegisteredAgentIdentity {
            agent_instance_id: register.agent_instance_id,
            process_id: register.process_id,
            process_creation_time: register.process_creation_time,
            logon_sid_hash: register.logon_sid_hash,
            windows_session_id: register.windows_session_id,
            agent_key_id: register.agent_key_id,
            registration_id: challenge.registration_id,
            registration_epoch: challenge.registration_epoch,
            protocol_major: 1,
            protocol_minor: 0,
        };
        write_frame(
            &mut stream,
            &AgentToService::AgentCapabilitySnapshot(AgentCapabilitySnapshot {
                agent_instance_id: identity.agent_instance_id,
                registration_id: identity.registration_id,
                windows_session_id,
                revision: 1,
                desktop_epoch: 1,
                observed_at_ms: NOW_MS,
                capabilities,
            }),
        )
        .await
        .expect("send capabilities");

        let binding = loop {
            match registry.bind_active_session(windows_session_id, AgentCapability::Input, NOW_MS) {
                Ok(binding) => break binding,
                Err(_) => tokio::task::yield_now().await,
            }
        };
        Self {
            registry,
            server,
            binding,
            identity,
            stream,
            serving,
        }
    }

    async fn finish(self) -> AgentConnectionExit {
        drop(self.stream);
        self.serving
            .await
            .expect("server task")
            .expect("server connection")
    }
}

fn connection_id() -> AgentConnectionId {
    connection_id_for(CONNECTION_BYTES[0])
}

fn connection_id_for(value: u8) -> AgentConnectionId {
    AgentConnectionId::from_bytes([value; 16]).expect("nonzero connection")
}

fn observed_for(windows_session_id: u32, process_id: u32) -> ObservedAgentIdentity {
    ObservedAgentIdentity {
        caller_kind: AgentCallerKind::InteractiveUser,
        process_id,
        process_creation_time: u64::from(process_id) * 10,
        logon_sid_hash: [windows_session_id as u8; 32],
        windows_session_id,
    }
}

fn register_for(windows_session_id: u32, process_id: u32) -> AgentRegister {
    AgentRegister {
        agent_instance_id: [process_id as u8; 16],
        process_id,
        process_creation_time: u64::from(process_id) * 10,
        logon_sid_hash: [windows_session_id as u8; 32],
        windows_session_id,
        agent_key_id: [process_id as u8; 32],
        agent_nonce: [10; 32],
    }
}

fn input_event(sequence: u64) -> InputEventEnvelope {
    InputEventEnvelope {
        session_id: SessionId("input-session".to_string()),
        resource_id: [4; 16],
        start_grant_id: [5; 32],
        sequence,
        event: InputEventPayload::MouseMove { x: 10, y: 20 },
    }
}

fn input_ack(
    identity: &mrd_agent_ipc::RegisteredAgentIdentity,
    event: &InputEventEnvelope,
) -> InputAck {
    InputAck {
        registration_id: identity.registration_id,
        registration_epoch: identity.registration_epoch,
        session_id: event.session_id.clone(),
        resource_id: event.resource_id,
        start_grant_id: event.start_grant_id,
        sequence: event.sequence,
        event_commitment: event.commitment().expect("valid input commitment"),
        outcome: InputAckOutcome::Applied,
    }
}

fn permission_scopes(values: impl IntoIterator<Item = PermissionScope>) -> PermissionScopes {
    values.into_iter().collect()
}

fn consent_request(request_id: [u8; 16], windows_session_id: u32) -> ConsentRequest {
    ConsentRequest {
        request_id,
        session_id: SessionId("consent-session".to_string()),
        peer: PeerBinding {
            device_id: DeviceId("consent-peer".to_string()),
            key_id: [12; 32],
        },
        requested_scopes: permission_scopes([PermissionScope::ScreenView]),
        policy_revision: 1,
        windows_session_id,
        issued_at_ms: NOW_MS - 1,
        expires_at_ms: NOW_MS + 5_000,
    }
}

fn consent_result(request: &ConsentRequest) -> ConsentResult {
    ConsentResult {
        request_id: request.request_id,
        session_id: request.session_id.clone(),
        peer: request.peer.clone(),
        policy_revision: request.policy_revision,
        windows_session_id: request.windows_session_id,
        decision: ConsentDecision::Approved,
        approved_scopes: request.requested_scopes.clone(),
        decided_at_ms: NOW_MS,
    }
}

fn execute_command(command_id: [u8; 16]) -> ExecuteCommand {
    let command = AgentCommand::StopInput {
        resource_id: [13; 16],
    };
    let mut execute = ExecuteCommand {
        command_id,
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id: [14; 32],
                registration_id: [15; 16],
                registration_epoch: 1,
                session_id: SessionId("execute-session".to_string()),
                peer: PeerBinding {
                    device_id: DeviceId("execute-peer".to_string()),
                    key_id: [16; 32],
                },
                scopes: PermissionScopes::new(),
                policy_revision: 1,
                windows_session_id: WINDOWS_SESSION_ID,
                desktop_epoch: 1,
                desktop_kind: DesktopKind::Default,
                issued_at_ms: NOW_MS - 1,
                not_before_ms: NOW_MS - 1,
                expires_at_ms: NOW_MS + 5_000,
                command_digest: [17; 32],
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: [18; 32],
            signature: [19; 64],
        },
        command,
    };
    execute.grant.claims.command_digest = execute.command_digest();
    execute
}

#[tokio::test]
async fn consent_request_returns_only_a_validated_correlated_result() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let request = consent_request([20; 16], WINDOWS_SESSION_ID);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });

    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read consent request")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    assert_eq!(delivered, request);
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(consent_result(&delivered)),
    )
    .await
    .expect("send consent result");

    let correlated = requesting
        .await
        .expect("request task")
        .expect("validated consent");
    assert_eq!(correlated.consent().request_id(), &request.request_id);
    assert_eq!(
        correlated.consent().windows_session_id(),
        WINDOWS_SESSION_ID
    );
    assert_eq!(
        correlated.binding().registration_id(),
        &agent.identity.registration_id
    );
    assert_eq!(
        correlated.binding().required_capability(),
        AgentCapability::Consent
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn execute_request_completes_only_for_the_exact_registration_and_command() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let execute = execute_command([21; 16]);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });

    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read execute request")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    assert_eq!(delivered, execute);
    let result = CommandResult {
        registration_id: agent.identity.registration_id,
        command_id: execute.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(result.clone()),
    )
    .await
    .expect("send command result");

    assert_eq!(requesting.await.expect("request task"), Ok(result));
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn consent_is_delivered_only_to_its_named_windows_session_and_connection() {
    let (registry, server) = ConnectedAgent::shared_server(Duration::from_secs(1));
    let capabilities = || {
        [AgentCapability::Consent, AgentCapability::Input]
            .into_iter()
            .collect()
    };
    let mut other = ConnectedAgent::connect_to(
        Arc::clone(&registry),
        Arc::clone(&server),
        WINDOWS_SESSION_ID,
        PROCESS_ID,
        1,
        32 * 1024,
        capabilities(),
        ReplacementPolicy::RejectExisting,
    )
    .await;
    let named_session = WINDOWS_SESSION_ID + 1;
    let mut named = ConnectedAgent::connect_to(
        registry,
        Arc::clone(&server),
        named_session,
        PROCESS_ID + 1,
        2,
        32 * 1024,
        capabilities(),
        ReplacementPolicy::RejectExisting,
    )
    .await;
    let request = consent_request([22; 16], named_session);
    let requesting = tokio::spawn({
        let server = Arc::clone(&server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });

    let delivered = match read_frame::<_, ServiceToAgent>(&mut named.stream)
        .await
        .expect("named session receives consent")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            read_frame::<_, ServiceToAgent>(&mut other.stream),
        )
        .await
        .is_err(),
        "another Windows session must not receive the prompt"
    );

    // Even a byte-for-byte matching result on another authenticated connection
    // cannot complete the named connection's pending request.
    write_frame(
        &mut other.stream,
        &AgentToService::ConsentResult(consent_result(&delivered)),
    )
    .await
    .expect("send result from wrong connection");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!requesting.is_finished());

    write_frame(
        &mut named.stream,
        &AgentToService::ConsentResult(consent_result(&delivered)),
    )
    .await
    .expect("send result from named connection");
    let correlated = requesting
        .await
        .expect("request task")
        .expect("correlated consent");
    assert_eq!(correlated.binding().windows_session_id(), named_session);
    assert_eq!(correlated.binding().connection_id(), connection_id_for(2));

    assert_eq!(other.finish().await, AgentConnectionExit::Disconnected);
    assert_eq!(named.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn mismatched_and_late_consent_results_cannot_complete_or_reuse_a_request() {
    let mut agent = ConnectedAgent::start(Duration::from_millis(30)).await;
    let request = consent_request([23; 16], WINDOWS_SESSION_ID);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });
    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read consent request")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    let mut mismatched = consent_result(&delivered);
    mismatched.session_id = SessionId("wrong-session".to_string());
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(mismatched),
    )
    .await
    .expect("send mismatched consent");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!requesting.is_finished());
    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Timeout)
    );

    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(consent_result(&delivered)),
    )
    .await
    .expect("send late consent");
    assert_eq!(
        agent.server.request_consent(request).await,
        Err(AgentRequestError::RetiredRequest)
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn expired_consent_and_missing_consent_capability_fail_before_delivery() {
    let expired_agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let mut expired = consent_request([24; 16], WINDOWS_SESSION_ID);
    expired.expires_at_ms = NOW_MS;
    assert_eq!(
        expired_agent.server.request_consent(expired).await,
        Err(AgentRequestError::InvalidConsent(
            mrd_agent_ipc::ConsentValidationError::Expired,
        ))
    );
    assert_eq!(
        expired_agent.finish().await,
        AgentConnectionExit::Disconnected
    );

    let (registry, server) = ConnectedAgent::shared_server(Duration::from_secs(1));
    let unavailable = ConnectedAgent::connect_to(
        registry,
        Arc::clone(&server),
        WINDOWS_SESSION_ID,
        PROCESS_ID,
        1,
        32 * 1024,
        [AgentCapability::Input].into_iter().collect(),
        ReplacementPolicy::RejectExisting,
    )
    .await;
    assert_eq!(
        server
            .request_consent(consent_request([25; 16], WINDOWS_SESSION_ID))
            .await,
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::CapabilityUnavailable,
        ))
    );
    assert_eq!(
        unavailable.finish().await,
        AgentConnectionExit::Disconnected
    );
}

#[tokio::test]
async fn aborting_consent_futures_frees_capacity_and_retires_each_request() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(5)).await;
    for value in 40..80_u8 {
        let request = consent_request([value; 16], WINDOWS_SESSION_ID);
        let requesting = tokio::spawn({
            let server = Arc::clone(&agent.server);
            let request = request.clone();
            async move { server.request_consent(request).await }
        });
        let delivered = tokio::time::timeout(
            Duration::from_millis(200),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("aborted requests must release pending capacity")
        .expect("read consent request");
        assert!(matches!(
            delivered.message,
            ServiceToAgent::ConsentRequest(_)
        ));
        requesting.abort();
        let _ = requesting.await;
        assert_eq!(
            agent.server.request_consent(request).await,
            Err(AgentRequestError::RetiredRequest)
        );
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_a_queued_execute_request_prevents_late_delivery() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 512).await;
    let mut requests = Vec::new();
    for value in 80..112_u8 {
        requests.push(tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move {
                server
                    .request_execute(&binding, execute_command([value; 16]))
                    .await
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    requests.last().expect("last request").abort();
    let last = requests.pop().expect("last request");
    let _ = last.await;

    let mut delivered_ids = Vec::new();
    loop {
        match tokio::time::timeout(
            Duration::from_millis(100),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        {
            Ok(Ok(frame)) => match frame.message {
                ServiceToAgent::Execute(execute) => delivered_ids.push(execute.command_id),
                other => panic!("expected queued execute request, got {other:?}"),
            },
            Ok(Err(error)) => panic!("failed reading queued execute: {error}"),
            Err(_) => break,
        }
    }
    assert_eq!(delivered_ids.len(), 31);
    assert!(!delivered_ids.contains(&[111; 16]));

    for request in requests {
        request.abort();
        let _ = request.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn mismatched_and_late_command_results_cannot_complete_or_reuse_a_command() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let execute = execute_command([112; 16]);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read execute request")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    let mut result = CommandResult {
        registration_id: agent.identity.registration_id,
        command_id: delivered.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    result.registration_id[0] ^= 1;
    write_frame(&mut agent.stream, &AgentToService::CommandResult(result))
        .await
        .expect("send mismatched result");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!requesting.is_finished());

    let result = CommandResult {
        registration_id: agent.identity.registration_id,
        command_id: delivered.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(result.clone()),
    )
    .await
    .expect("send matching result");
    assert_eq!(requesting.await.expect("request task"), Ok(result.clone()));
    write_frame(&mut agent.stream, &AgentToService::CommandResult(result))
        .await
        .expect("send late duplicate result");
    assert_eq!(
        agent.server.request_execute(&agent.binding, execute).await,
        Err(AgentRequestError::RetiredRequest)
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn timed_out_execute_is_retired_and_a_disconnect_closes_its_waiter() {
    let mut timed_out = ConnectedAgent::start(Duration::from_millis(20)).await;
    let execute = execute_command([115; 16]);
    let requesting = tokio::spawn({
        let server = Arc::clone(&timed_out.server);
        let binding = timed_out.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut timed_out.stream)
        .await
        .expect("read execute request");
    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Timeout)
    );
    assert_eq!(
        timed_out
            .server
            .request_execute(&timed_out.binding, execute)
            .await,
        Err(AgentRequestError::RetiredRequest)
    );
    assert_eq!(timed_out.finish().await, AgentConnectionExit::Disconnected);

    let mut disconnected = ConnectedAgent::start(Duration::from_secs(1)).await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&disconnected.server);
        let binding = disconnected.binding.clone();
        async move {
            server
                .request_execute(&binding, execute_command([116; 16]))
                .await
        }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut disconnected.stream)
        .await
        .expect("read execute request");
    drop(disconnected.stream);
    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Disconnected)
    );
    assert_eq!(
        disconnected
            .serving
            .await
            .expect("server task")
            .expect("server connection"),
        AgentConnectionExit::Disconnected
    );
}

#[tokio::test]
async fn queued_execute_revalidates_the_bound_capability_before_write() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 512).await;
    let mut blockers = Vec::new();
    for sequence in 500..508 {
        blockers.push(tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move { server.request_input(&binding, input_event(sequence)).await }
        }));
    }
    let target_execute = execute_command([113; 16]);
    let target = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = target_execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    agent
        .registry
        .record_capabilities(
            connection_id(),
            AgentCapabilitySnapshot {
                agent_instance_id: agent.identity.agent_instance_id,
                registration_id: agent.identity.registration_id,
                windows_session_id: WINDOWS_SESSION_ID,
                revision: 2,
                desktop_epoch: 1,
                observed_at_ms: NOW_MS,
                capabilities: [AgentCapability::Consent].into_iter().collect(),
            },
            NOW_MS,
        )
        .expect("remove bound input capability");

    let mut execute_was_delivered = false;
    while !target.is_finished() {
        let frame = tokio::time::timeout(
            Duration::from_millis(200),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("queued writes must make progress")
        .expect("read queued request");
        execute_was_delivered |= matches!(frame.message, ServiceToAgent::Execute(_));
    }
    assert_eq!(
        target.await.expect("target request"),
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::CapabilityUnavailable,
        ))
    );
    assert!(!execute_was_delivered);

    for blocker in blockers {
        blocker.abort();
        let _ = blocker.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn fast_replacement_never_retargets_a_persisted_execute_binding() {
    let (registry, server) = ConnectedAgent::shared_server(Duration::from_secs(5));
    let capabilities = || {
        [AgentCapability::Consent, AgentCapability::Input]
            .into_iter()
            .collect()
    };
    let mut first = ConnectedAgent::connect_to(
        Arc::clone(&registry),
        Arc::clone(&server),
        WINDOWS_SESSION_ID,
        PROCESS_ID,
        1,
        512,
        capabilities(),
        ReplacementPolicy::RejectExisting,
    )
    .await;
    let execute = execute_command([114; 16]);
    let requesting = tokio::spawn({
        let server = Arc::clone(&server);
        let binding = first.binding.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut first.stream)
        .await
        .expect("first generation receives request");

    let mut replacement = ConnectedAgent::connect_to(
        registry,
        Arc::clone(&server),
        WINDOWS_SESSION_ID,
        PROCESS_ID + 1,
        2,
        32 * 1024,
        capabilities(),
        ReplacementPolicy::ReplaceExisting {
            expected_registration_id: first.identity.registration_id,
            expected_registration_epoch: first.identity.registration_epoch,
        },
    )
    .await;
    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Revoked)
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            read_frame::<_, ServiceToAgent>(&mut replacement.stream),
        )
        .await
        .is_err(),
        "replacement must not inherit an old exact request"
    );

    drop(first.stream);
    let first_exit = first
        .serving
        .await
        .expect("first server task")
        .expect("first connection");
    assert!(matches!(
        first_exit,
        AgentConnectionExit::Disconnected | AgentConnectionExit::Stopped
    ));
    assert_eq!(
        replacement.finish().await,
        AgentConnectionExit::Disconnected
    );
}

#[tokio::test]
async fn input_request_routes_to_exact_binding_and_returns_correlated_ack() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let event = input_event(1);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });

    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read input request")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };
    assert_eq!(delivered, event);
    let ack = input_ack(&agent.identity, &event);
    write_frame(&mut agent.stream, &AgentToService::InputAck(ack.clone()))
        .await
        .expect("send input ack");

    assert_eq!(requesting.await.expect("request task"), Ok(ack));
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn mismatched_input_ack_cannot_complete_an_exact_request() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let event = input_event(2);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });
    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read input request")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };

    let mut wrong_generation = input_ack(&agent.identity, &delivered);
    wrong_generation.registration_id[0] ^= 1;
    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(wrong_generation),
    )
    .await
    .expect("send wrong-generation ack");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!requesting.is_finished());

    let mut wrong_commitment = input_ack(&agent.identity, &delivered);
    wrong_commitment.event_commitment[0] ^= 1;
    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(wrong_commitment),
    )
    .await
    .expect("send wrong-commitment ack");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!requesting.is_finished());

    let ack = input_ack(&agent.identity, &delivered);
    write_frame(&mut agent.stream, &AgentToService::InputAck(ack.clone()))
        .await
        .expect("send matching ack");
    assert_eq!(requesting.await.expect("request task"), Ok(ack));
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn one_way_compatibility_api_rejects_response_bearing_input() {
    let agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    assert!(matches!(
        agent
            .server
            .send_to_connection(connection_id(), ServiceToAgent::InputEvent(input_event(9)),),
        Err(AgentServerError::ResponseRequired)
    ));
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn input_request_waiter_closes_when_agent_disconnects() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        async move { server.request_input(&binding, input_event(3)).await }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read input request");
    drop(agent.stream);

    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Disconnected)
    );
    assert_eq!(
        agent
            .serving
            .await
            .expect("server task")
            .expect("server connection"),
        AgentConnectionExit::Disconnected
    );
}

#[tokio::test]
async fn input_request_waiter_closes_when_exact_generation_is_revoked() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        async move { server.request_input(&binding, input_event(4)).await }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read input request");
    agent.registry.disconnect(connection_id());

    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Revoked)
    );
    drop(agent.stream);
    let exit = agent
        .serving
        .await
        .expect("server task")
        .expect("server connection");
    assert!(matches!(
        exit,
        AgentConnectionExit::Disconnected | AgentConnectionExit::Stopped
    ));
}

#[tokio::test]
async fn input_request_waiter_closes_at_the_bounded_timeout() {
    let mut agent = ConnectedAgent::start(Duration::from_millis(20)).await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        async move { server.request_input(&binding, input_event(5)).await }
    });
    let _ = read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read input request");

    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Timeout)
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn timed_out_input_correlation_is_retired_before_a_late_ack_or_retry() {
    let mut agent = ConnectedAgent::start(Duration::from_millis(20)).await;
    let event = input_event(6);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });
    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read first input request")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };
    assert_eq!(
        requesting.await.expect("request task"),
        Err(AgentRequestError::Timeout)
    );

    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(input_ack(&agent.identity, &delivered)),
    )
    .await
    .expect("send late ack");
    assert_eq!(
        agent.server.request_input(&agent.binding, event).await,
        Err(AgentRequestError::RetiredRequest)
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_input_request_futures_releases_pending_capacity() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(5)).await;
    for sequence in 100..140 {
        let requesting = tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move { server.request_input(&binding, input_event(sequence)).await }
        });
        let delivered = tokio::time::timeout(
            Duration::from_millis(200),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("cancelled requests must not exhaust pending capacity")
        .expect("read input request");
        assert!(matches!(delivered.message, ServiceToAgent::InputEvent(_)));
        requesting.abort();
        let _ = requesting.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_a_queued_input_request_prevents_late_delivery() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 512).await;
    let mut requests = Vec::new();
    for sequence in 200..232 {
        requests.push(tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move { server.request_input(&binding, input_event(sequence)).await }
        }));
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    requests.last().expect("last request").abort();
    let last = requests.pop().expect("last request");
    let _ = last.await;

    let mut delivered_sequences = Vec::new();
    loop {
        match tokio::time::timeout(
            Duration::from_millis(100),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        {
            Ok(Ok(frame)) => match frame.message {
                ServiceToAgent::InputEvent(event) => delivered_sequences.push(event.sequence),
                other => panic!("expected queued input event, got {other:?}"),
            },
            Ok(Err(error)) => panic!("failed reading queued input: {error}"),
            Err(_) => break,
        }
    }
    assert_eq!(delivered_sequences.len(), 31);
    assert!(!delivered_sequences.contains(&231));

    for request in requests {
        request.abort();
        let _ = request.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn queued_input_revalidates_the_exact_desktop_before_write() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 512).await;
    let mut blockers = Vec::new();
    for sequence in 300..308 {
        blockers.push(tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move { server.request_input(&binding, input_event(sequence)).await }
        }));
    }
    let target_event = input_event(399);
    let target = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = target_event.clone();
        async move { server.request_input(&binding, event).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    agent
        .registry
        .record_capabilities(
            connection_id(),
            AgentCapabilitySnapshot {
                agent_instance_id: agent.identity.agent_instance_id,
                registration_id: agent.identity.registration_id,
                windows_session_id: WINDOWS_SESSION_ID,
                revision: 2,
                desktop_epoch: 2,
                observed_at_ms: NOW_MS + 1,
                capabilities: [AgentCapability::Input].into_iter().collect(),
            },
            NOW_MS + 1,
        )
        .expect("advance desktop generation");

    let mut delivered_sequences = Vec::new();
    while !target.is_finished() {
        let frame = tokio::time::timeout(
            Duration::from_millis(200),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("queued writes must make progress")
        .expect("read queued input");
        match frame.message {
            ServiceToAgent::InputEvent(event) => delivered_sequences.push(event.sequence),
            other => panic!("expected queued input event, got {other:?}"),
        }
    }
    assert_eq!(
        target.await.expect("target request"),
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::DesktopChanged,
        ))
    );
    assert!(!delivered_sequences.contains(&399));

    for blocker in blockers {
        blocker.abort();
        let _ = blocker.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}
