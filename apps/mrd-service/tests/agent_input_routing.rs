use mrd_agent_ipc::{
    read_frame, write_frame, AgentCapability, AgentCapabilitySnapshot, AgentRegister,
    AgentRegistered, AgentToService, InputAck, InputAckOutcome, InputEventEnvelope,
    InputEventPayload, RegistrationProofVerifier, ServiceToAgent,
};
use mrd_proto::SessionId;
use mrd_service::agent_runtime::{
    AgentBinding, AgentCallerKind, AgentConnectionExit, AgentConnectionId, AgentRegistry,
    AgentRegistryError, AgentRequestError, AgentServer, AgentServerClock, AgentServerError,
    ChallengeMaterial, ChallengeMaterialSource, ExpectedAgentSession, ObservedAgentIdentity,
    ReplacementPolicy,
};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::DuplexStream;

const NOW_MS: u64 = 1_000;
const WINDOWS_SESSION_ID: u32 = 7;
const PROCESS_ID: u32 = 42;
const CONNECTION_BYTES: [u8; 16] = [1; 16];
const SID_HASH: [u8; 32] = [7; 32];
const AGENT_KEY_ID: [u8; 32] = [9; 32];

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
        let registry = Arc::new(AgentRegistry::with_challenge_source(Arc::new(
            DeterministicChallenges::default(),
        )));
        registry
            .expect_session_at(
                ExpectedAgentSession {
                    windows_session_id: WINDOWS_SESSION_ID,
                    logon_sid_hash: SID_HASH,
                    process_id: PROCESS_ID,
                    process_creation_time: u64::from(PROCESS_ID) * 10,
                    agent_key_id: AGENT_KEY_ID,
                    expires_at_ms: 20_000,
                    replacement_policy: ReplacementPolicy::RejectExisting,
                },
                Arc::new(AcceptProof),
                NOW_MS - 1,
            )
            .expect("install expected agent");
        let server = Arc::new(AgentServer::with_clock_and_request_timeout(
            Arc::clone(&registry),
            Arc::new(FixedClock),
            request_timeout,
        ));
        let connection_id = connection_id();
        let (service_stream, mut stream) = tokio::io::duplex(stream_capacity);
        let serving = tokio::spawn({
            let server = Arc::clone(&server);
            async move {
                server
                    .serve_connection(service_stream, connection_id, observed())
                    .await
            }
        });

        let register = register();
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
                windows_session_id: WINDOWS_SESSION_ID,
                revision: 1,
                desktop_epoch: 1,
                observed_at_ms: NOW_MS,
                capabilities: [AgentCapability::Input].into_iter().collect(),
            }),
        )
        .await
        .expect("send capabilities");

        let binding = loop {
            match registry.bind_active_session(WINDOWS_SESSION_ID, AgentCapability::Input, NOW_MS) {
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
    AgentConnectionId::from_bytes(CONNECTION_BYTES).expect("nonzero connection")
}

fn observed() -> ObservedAgentIdentity {
    ObservedAgentIdentity {
        caller_kind: AgentCallerKind::InteractiveUser,
        process_id: PROCESS_ID,
        process_creation_time: u64::from(PROCESS_ID) * 10,
        logon_sid_hash: SID_HASH,
        windows_session_id: WINDOWS_SESSION_ID,
    }
}

fn register() -> AgentRegister {
    AgentRegister {
        agent_instance_id: [PROCESS_ID as u8; 16],
        process_id: PROCESS_ID,
        process_creation_time: u64::from(PROCESS_ID) * 10,
        logon_sid_hash: SID_HASH,
        windows_session_id: WINDOWS_SESSION_ID,
        agent_key_id: AGENT_KEY_ID,
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
