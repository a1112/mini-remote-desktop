use mrd_agent_ipc::{
    decode_frame, read_frame, write_frame, AgentCapability, AgentCapabilitySnapshot, AgentCommand,
    AgentRegister, AgentRegistered, AgentToService, CancelConsent, CommandOutcome, CommandResult,
    ConsentCancelReason, ConsentDecision, ConsentRequest, ConsentResult, DesktopKind,
    ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, GrantAudience, InputAck, InputAckOutcome,
    InputEventEnvelope, InputEventPayload, PeerBinding, RegistrationProofVerifier, ServiceToAgent,
    AGENT_IPC_PROTOCOL_MINOR,
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
use tokio::io::{AsyncReadExt, DuplexStream};

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
        Self::connect_to_with_minor(
            registry,
            server,
            windows_session_id,
            process_id,
            connection_value,
            stream_capacity,
            capabilities,
            replacement_policy,
            AGENT_IPC_PROTOCOL_MINOR,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn connect_to_with_minor(
        registry: Arc<AgentRegistry>,
        server: Arc<AgentServer>,
        windows_session_id: u32,
        process_id: u32,
        connection_value: u8,
        stream_capacity: usize,
        capabilities: BTreeSet<AgentCapability>,
        replacement_policy: ReplacementPolicy,
        protocol_minor: u16,
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
                accepted_protocol_minor: protocol_minor,
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
            protocol_minor,
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
        request_token: 0,
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
        request_token: event.request_token,
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
        request_token: 0,
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
        authorization_expires_at_ms: NOW_MS + 10_000,
    }
}

fn consent_result(request: &ConsentRequest) -> ConsentResult {
    ConsentResult {
        request_token: request.request_token,
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
    execute_command_for(
        command_id,
        AgentCommand::StopInput {
            resource_id: [13; 16],
        },
    )
}

fn execute_command_for(command_id: [u8; 16], command: AgentCommand) -> ExecuteCommand {
    let mut execute = ExecuteCommand {
        request_token: 0,
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
    assert_ne!(delivered.request_token, 0);
    let mut expected = request.clone();
    expected.request_token = delivered.request_token;
    assert_eq!(delivered, expected);
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
    assert_ne!(delivered.request_token, 0);
    let mut expected = execute.clone();
    expected.request_token = delivered.request_token;
    assert_eq!(delivered, expected);
    let result = CommandResult {
        request_token: delivered.request_token,
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
async fn mismatched_and_late_consent_results_cannot_complete_a_retried_request() {
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
    let cancel = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read timed-out consent cancellation")
        .message
    {
        ServiceToAgent::CancelConsent(cancel) => cancel,
        other => panic!("expected consent cancellation, got {other:?}"),
    };
    assert_eq!(cancel.request_token, delivered.request_token);
    assert_eq!(cancel.reason, ConsentCancelReason::TimedOut);
    let retrying = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });
    let retry = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read retried consent")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    assert_ne!(delivered.request_token, retry.request_token);
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(consent_result(&retry)),
    )
    .await
    .expect("answer retried consent");
    retrying.await.expect("retry task").expect("retry result");
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
async fn consent_cancel_contract_rejects_an_agent_negotiated_at_minor_one() {
    let (registry, server) = ConnectedAgent::shared_server(Duration::from_secs(1));
    let legacy = ConnectedAgent::connect_to_with_minor(
        registry,
        Arc::clone(&server),
        WINDOWS_SESSION_ID,
        PROCESS_ID,
        1,
        32 * 1024,
        [AgentCapability::Consent, AgentCapability::Input]
            .into_iter()
            .collect(),
        ReplacementPolicy::RejectExisting,
        1,
    )
    .await;

    assert_eq!(
        server
            .request_consent(consent_request([26; 16], WINDOWS_SESSION_ID))
            .await,
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::ProtocolVersionUnavailable,
        ))
    );
    assert_eq!(legacy.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_consent_futures_frees_pending_capacity() {
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
        let cancel = tokio::time::timeout(
            Duration::from_millis(200),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("aborted delivered consent must be cancelled")
        .expect("read consent cancel");
        assert!(matches!(
            cancel.message,
            ServiceToAgent::CancelConsent(CancelConsent {
                reason: ConsentCancelReason::CallerAborted,
                ..
            })
        ));
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_a_delivered_consent_queues_exact_cancel_cleanup() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(5)).await;
    let request = consent_request([79; 16], WINDOWS_SESSION_ID);
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });
    let delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read delivered consent")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };

    requesting.abort();
    let _ = requesting.await;
    let cancel = match tokio::time::timeout(
        Duration::from_millis(200),
        read_frame::<_, ServiceToAgent>(&mut agent.stream),
    )
    .await
    .expect("delivered consent abort must queue cancellation")
    .expect("read consent cancellation")
    .message
    {
        ServiceToAgent::CancelConsent(cancel) => cancel,
        other => panic!("expected consent cancellation, got {other:?}"),
    };
    assert_eq!(
        cancel,
        CancelConsent {
            request_token: delivered.request_token,
            request_id: delivered.request_id,
            session_id: delivered.session_id,
            reason: ConsentCancelReason::CallerAborted,
        }
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn aborting_a_partially_written_consent_hard_closes_without_appending_cancel() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 64).await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&agent.server);
        async move {
            server
                .request_consent(consent_request([78; 16], WINDOWS_SESSION_ID))
                .await
        }
    });

    let mut partial = vec![0_u8; 16];
    tokio::time::timeout(
        Duration::from_millis(200),
        agent.stream.read_exact(&mut partial),
    )
    .await
    .expect("consent frame must begin writing")
    .expect("read partial consent frame");
    requesting.abort();
    let _ = requesting.await;
    tokio::time::timeout(
        Duration::from_millis(200),
        agent.stream.read_to_end(&mut partial),
    )
    .await
    .expect("partial cancellation must hard-close the stream")
    .expect("drain partial stream");
    assert!(
        decode_frame::<ServiceToAgent>(&partial).is_err(),
        "a CancelConsent frame must not be appended to a partial request frame"
    );
    let cancel_tag = br#""type":"cancel_consent""#;
    assert!(
        !partial
            .windows(cancel_tag.len())
            .any(|window| window == cancel_tag),
        "the hard-closed stream must not contain a serialized CancelConsent"
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
async fn aborting_a_queued_consent_sends_neither_prompt_nor_cancel() {
    let mut agent = ConnectedAgent::start_with_capacity(Duration::from_secs(5), 512).await;
    let mut blockers = Vec::new();
    for value in 80..111_u8 {
        blockers.push(tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            async move {
                server
                    .request_execute(&binding, execute_command([value; 16]))
                    .await
            }
        }));
    }
    let target_id = [111; 16];
    let target = tokio::spawn({
        let server = Arc::clone(&agent.server);
        async move {
            server
                .request_consent(consent_request(target_id, WINDOWS_SESSION_ID))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    target.abort();
    let _ = target.await;

    let mut execute_count = 0;
    loop {
        match tokio::time::timeout(
            Duration::from_millis(100),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        {
            Ok(Ok(frame)) => match frame.message {
                ServiceToAgent::Execute(_) => execute_count += 1,
                ServiceToAgent::ConsentRequest(request) if request.request_id == target_id => {
                    panic!("queued aborted consent prompt was delivered")
                }
                ServiceToAgent::CancelConsent(cancel) if cancel.request_id == target_id => {
                    panic!("never-delivered consent was cancelled")
                }
                other => panic!("unexpected queued frame: {other:?}"),
            },
            Ok(Err(error)) => panic!("failed reading queued requests: {error}"),
            Err(_) => break,
        }
    }
    assert_eq!(execute_count, 31);

    for blocker in blockers {
        blocker.abort();
        let _ = blocker.await;
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn consent_cancel_is_never_retargeted_to_a_replacement_generation() {
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
        32 * 1024,
        capabilities(),
        ReplacementPolicy::RejectExisting,
    )
    .await;
    let requesting = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request_consent(consent_request([112; 16], WINDOWS_SESSION_ID))
                .await
        }
    });
    assert!(matches!(
        read_frame::<_, ServiceToAgent>(&mut first.stream)
            .await
            .expect("first generation receives prompt")
            .message,
        ServiceToAgent::ConsentRequest(_)
    ));

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
            Duration::from_millis(50),
            read_frame::<_, ServiceToAgent>(&mut replacement.stream),
        )
        .await
        .is_err(),
        "replacement must not receive an earlier generation's consent cancel"
    );

    drop(first.stream);
    let _ = first.serving.await;
    assert_eq!(
        replacement.finish().await,
        AgentConnectionExit::Disconnected
    );
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
async fn mismatched_and_late_command_results_cannot_complete_another_attempt() {
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
        request_token: delivered.request_token,
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
        request_token: delivered.request_token,
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
    let retrying = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let retry = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read retried command")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    assert_ne!(delivered.request_token, retry.request_token);
    let retry_result = CommandResult {
        request_token: retry.request_token,
        registration_id: agent.identity.registration_id,
        command_id: retry.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(retry_result.clone()),
    )
    .await
    .expect("complete retried command");
    assert_eq!(retrying.await.expect("retry task"), Ok(retry_result));
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn timed_out_execute_can_retry_and_a_disconnect_closes_its_waiter() {
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
    let retrying = tokio::spawn({
        let server = Arc::clone(&timed_out.server);
        let binding = timed_out.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let retry = match read_frame::<_, ServiceToAgent>(&mut timed_out.stream)
        .await
        .expect("read retried execute")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    let retry_result = CommandResult {
        request_token: retry.request_token,
        registration_id: timed_out.identity.registration_id,
        command_id: retry.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut timed_out.stream,
        &AgentToService::CommandResult(retry_result.clone()),
    )
    .await
    .expect("complete retried execute");
    assert_eq!(retrying.await.expect("retry task"), Ok(retry_result));
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
async fn more_than_retired_capacity_successes_do_not_exhaust_request_correlation() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(5)).await;
    for sequence in 1..=4_100_u64 {
        let event = input_event(sequence);
        let requesting = tokio::spawn({
            let server = Arc::clone(&agent.server);
            let binding = agent.binding.clone();
            let event = event.clone();
            async move { server.request_input(&binding, event).await }
        });
        let delivered = match tokio::time::timeout(
            Duration::from_secs(1),
            read_frame::<_, ServiceToAgent>(&mut agent.stream),
        )
        .await
        .expect("successful traffic must not saturate correlation state")
        .expect("read input request")
        .message
        {
            ServiceToAgent::InputEvent(event) => event,
            other => panic!("expected input event, got {other:?}"),
        };
        assert_ne!(delivered.request_token, 0);
        let ack = input_ack(&agent.identity, &delivered);
        write_frame(&mut agent.stream, &AgentToService::InputAck(ack.clone()))
            .await
            .expect("send input ack");
        assert_eq!(requesting.await.expect("request task"), Ok(ack));
    }
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn late_tokens_cannot_complete_reused_input_consent_or_execute_semantics() {
    let mut agent = ConnectedAgent::start(Duration::from_secs(1)).await;

    let event = input_event(4_200);
    let first_input = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });
    let first_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read first input")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };
    let first_ack = input_ack(&agent.identity, &first_delivered);
    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(first_ack.clone()),
    )
    .await
    .expect("ack first input");
    assert_eq!(
        first_input.await.expect("first input task"),
        Ok(first_ack.clone())
    );

    let second_input = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });
    let second_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read second input")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };
    assert_ne!(
        first_delivered.request_token,
        second_delivered.request_token
    );
    write_frame(&mut agent.stream, &AgentToService::InputAck(first_ack))
        .await
        .expect("send late input ack");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!second_input.is_finished());
    let second_ack = input_ack(&agent.identity, &second_delivered);
    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(second_ack.clone()),
    )
    .await
    .expect("ack second input");
    assert_eq!(
        second_input.await.expect("second input task"),
        Ok(second_ack)
    );

    let request = consent_request([117; 16], WINDOWS_SESSION_ID);
    let first_consent = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });
    let first_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read first consent")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    let first_result = consent_result(&first_delivered);
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(first_result.clone()),
    )
    .await
    .expect("answer first consent");
    first_consent
        .await
        .expect("first consent task")
        .expect("first consent result");

    let second_consent = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let request = request.clone();
        async move { server.request_consent(request).await }
    });
    let second_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read second consent")
        .message
    {
        ServiceToAgent::ConsentRequest(request) => request,
        other => panic!("expected consent request, got {other:?}"),
    };
    assert_ne!(
        first_delivered.request_token,
        second_delivered.request_token
    );
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(first_result),
    )
    .await
    .expect("send late consent result");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!second_consent.is_finished());
    write_frame(
        &mut agent.stream,
        &AgentToService::ConsentResult(consent_result(&second_delivered)),
    )
    .await
    .expect("answer second consent");
    second_consent
        .await
        .expect("second consent task")
        .expect("second consent result");

    let execute = execute_command([118; 16]);
    let first_execute = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let first_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read first execute")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    let first_result = CommandResult {
        request_token: first_delivered.request_token,
        registration_id: agent.identity.registration_id,
        command_id: first_delivered.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(first_result.clone()),
    )
    .await
    .expect("complete first execute");
    assert_eq!(
        first_execute.await.expect("first execute task"),
        Ok(first_result.clone())
    );

    let second_execute = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let execute = execute.clone();
        async move { server.request_execute(&binding, execute).await }
    });
    let second_delivered = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read second execute")
        .message
    {
        ServiceToAgent::Execute(execute) => *execute,
        other => panic!("expected execute request, got {other:?}"),
    };
    assert_ne!(
        first_delivered.request_token,
        second_delivered.request_token
    );
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(first_result),
    )
    .await
    .expect("send late command result");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!second_execute.is_finished());
    let second_result = CommandResult {
        request_token: second_delivered.request_token,
        registration_id: agent.identity.registration_id,
        command_id: second_delivered.command_id,
        outcome: CommandOutcome::Completed,
        completed_at_ms: NOW_MS,
    };
    write_frame(
        &mut agent.stream,
        &AgentToService::CommandResult(second_result.clone()),
    )
    .await
    .expect("complete second execute");
    assert_eq!(
        second_execute.await.expect("second execute task"),
        Ok(second_result)
    );

    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
}

#[tokio::test]
async fn execute_command_capability_must_match_the_persisted_binding() {
    let agent = ConnectedAgent::start(Duration::from_secs(1)).await;
    let consent_binding = agent
        .registry
        .bind_active_session(WINDOWS_SESSION_ID, AgentCapability::Consent, NOW_MS)
        .expect("bind consent capability");
    let start_input = execute_command_for(
        [119; 16],
        AgentCommand::StartInput {
            resource_id: [20; 16],
            input_scopes: permission_scopes([PermissionScope::InputPointer]),
        },
    );
    assert_eq!(
        agent
            .server
            .request_execute(&consent_binding, start_input)
            .await,
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::CapabilityBindingMismatch,
        ))
    );

    let start_capture = execute_command_for(
        [120; 16],
        AgentCommand::StartCapture {
            resource_id: [21; 16],
            display_id: 1,
        },
    );
    assert_eq!(
        agent
            .server
            .request_execute(&agent.binding, start_capture)
            .await,
        Err(AgentRequestError::Route(
            mrd_service::agent_runtime::AgentRouteError::CapabilityBindingMismatch,
        ))
    );
    assert_eq!(agent.finish().await, AgentConnectionExit::Disconnected);
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
    assert_ne!(delivered.request_token, 0);
    let mut expected = event.clone();
    expected.request_token = delivered.request_token;
    assert_eq!(delivered, expected);
    let ack = input_ack(&agent.identity, &delivered);
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
async fn timed_out_input_uses_a_new_token_and_ignores_the_late_ack() {
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
    let retrying = tokio::spawn({
        let server = Arc::clone(&agent.server);
        let binding = agent.binding.clone();
        let event = event.clone();
        async move { server.request_input(&binding, event).await }
    });
    let retry = match read_frame::<_, ServiceToAgent>(&mut agent.stream)
        .await
        .expect("read retried input")
        .message
    {
        ServiceToAgent::InputEvent(event) => event,
        other => panic!("expected input event, got {other:?}"),
    };
    assert_ne!(delivered.request_token, retry.request_token);
    let retry_ack = input_ack(&agent.identity, &retry);
    write_frame(
        &mut agent.stream,
        &AgentToService::InputAck(retry_ack.clone()),
    )
    .await
    .expect("ack retried input");
    assert_eq!(retrying.await.expect("retry task"), Ok(retry_ack));
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
