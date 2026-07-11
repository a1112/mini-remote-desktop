use mrd_agent_ipc::{
    read_frame, write_frame, AgentCapability, AgentCapabilitySnapshot, AgentEventContext,
    AgentHeartbeat, AgentRegister, AgentRegistered, AgentStopping, AgentToService,
    RegistrationError, RegistrationProofVerifier, ServiceToAgent, StopAgent, StopReason,
    StoppingReason,
};
use mrd_service::agent_runtime::{
    AgentCallerKind, AgentConnectionExit, AgentConnectionId, AgentHealth, AgentRegistry,
    AgentRegistryError, AgentServer, AgentServerClock, AgentServerError, ChallengeMaterial,
    ChallengeMaterialSource, ExpectedAgentSession, ObservedAgentIdentity, RegistrationOutcome,
    ReplacementPolicy, AGENT_HEARTBEAT_STALE_AFTER_MS,
};
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc,
};
use tokio::time::{sleep, Duration};

#[cfg(windows)]
use mrd_service::agent_runtime::{inspect_windows_process, WindowsAgentPipe};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;

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

struct RejectProof;

impl RegistrationProofVerifier for RejectProof {
    fn verify(
        &self,
        _agent_key_id: &[u8; 32],
        _signing_bytes: &[u8],
        _signature: &[u8; 64],
    ) -> bool {
        false
    }
}

struct StepClock(AtomicU64);

impl AgentServerClock for StepClock {
    fn now_ms(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}

fn registry() -> AgentRegistry {
    AgentRegistry::with_challenge_source(Arc::new(DeterministicChallenges::default()))
}

fn connection(value: u8) -> AgentConnectionId {
    AgentConnectionId::from_bytes([value; 16]).expect("nonzero connection")
}

fn observed(process_id: u32, windows_session_id: u32) -> ObservedAgentIdentity {
    ObservedAgentIdentity {
        caller_kind: AgentCallerKind::InteractiveUser,
        process_id,
        process_creation_time: u64::from(process_id) * 10,
        logon_sid_hash: SID_HASH,
        windows_session_id,
    }
}

fn register(process_id: u32, windows_session_id: u32) -> AgentRegister {
    AgentRegister {
        agent_instance_id: [process_id as u8; 16],
        process_id,
        process_creation_time: u64::from(process_id) * 10,
        logon_sid_hash: SID_HASH,
        windows_session_id,
        agent_key_id: AGENT_KEY_ID,
        agent_nonce: [10; 32],
    }
}

fn expect_session(
    registry: &AgentRegistry,
    process_id: u32,
    windows_session_id: u32,
    replacement_policy: ReplacementPolicy,
) {
    registry
        .expect_session_at(
            expected_session(process_id, windows_session_id, replacement_policy),
            Arc::new(AcceptProof),
            999,
        )
        .expect("expected session");
}

fn expected_session(
    process_id: u32,
    windows_session_id: u32,
    replacement_policy: ReplacementPolicy,
) -> ExpectedAgentSession {
    ExpectedAgentSession {
        windows_session_id,
        logon_sid_hash: SID_HASH,
        process_id,
        process_creation_time: u64::from(process_id) * 10,
        agent_key_id: AGENT_KEY_ID,
        expires_at_ms: 20_000,
        replacement_policy,
    }
}

fn capability(
    identity: &mrd_agent_ipc::RegisteredAgentIdentity,
    revision: u64,
) -> AgentCapabilitySnapshot {
    AgentCapabilitySnapshot {
        agent_instance_id: identity.agent_instance_id,
        registration_id: identity.registration_id,
        windows_session_id: identity.windows_session_id,
        revision,
        desktop_epoch: 1,
        observed_at_ms: 1_100,
        capabilities: [AgentCapability::Capture].into_iter().collect(),
    }
}

fn complete(
    registry: &AgentRegistry,
    connection_id: AgentConnectionId,
    register: AgentRegister,
    observed: ObservedAgentIdentity,
    now_ms: u64,
) -> RegistrationOutcome {
    let challenge = registry
        .begin_registration(connection_id, register.clone(), observed, now_ms)
        .expect("begin registration");
    let identity = registry
        .complete_registration(
            connection_id,
            AgentRegistered {
                registration_id: challenge.registration_id,
                registration_epoch: challenge.registration_epoch,
                challenge_id: challenge.challenge_id,
                agent_instance_id: register.agent_instance_id,
                accepted_protocol_major: 1,
                accepted_protocol_minor: 0,
                signed_at_ms: now_ms + 1,
                signature: [11; 64],
            },
            now_ms + 1,
        )
        .expect("complete registration");
    registry
        .activate_registration(connection_id, capability(&identity, 1), now_ms + 2)
        .expect("activate registration")
}

#[test]
fn expected_windows_session_registers_once_and_tracks_health() {
    let registry = registry();
    expect_session(&registry, 42, 7, ReplacementPolicy::RejectExisting);
    let outcome = complete(
        &registry,
        connection(1),
        register(42, 7),
        observed(42, 7),
        1_000,
    );
    assert!(outcome.replaced_connection.is_none());
    assert!(registry.is_registration_active(
        &outcome.identity.registration_id,
        outcome.identity.registration_epoch,
    ));

    registry
        .record_capabilities(connection(1), capability(&outcome.identity, 2), 1_100)
        .expect("capability snapshot");
    registry
        .record_heartbeat(
            connection(1),
            AgentHeartbeat {
                context: AgentEventContext {
                    registration_id: outcome.identity.registration_id,
                    registration_epoch: outcome.identity.registration_epoch,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence: 1,
                    observed_at_ms: 1_200,
                },
            },
            1_200,
        )
        .expect("heartbeat");
    let snapshot = registry
        .active_for_session_at(7, 1_200)
        .expect("active agent");
    assert_eq!(snapshot.last_event_sequence, 1);
    assert_eq!(snapshot.capabilities.revision, 2);
    assert_eq!(snapshot.health, AgentHealth::Healthy);

    let stale = registry
        .active_for_session_at(7, 1_200 + AGENT_HEARTBEAT_STALE_AFTER_MS + 1)
        .expect("stale agent remains addressable");
    assert_eq!(stale.health, AgentHealth::Unresponsive);
    let rolled_back_clock = registry
        .active_for_session_at(7, 1_199)
        .expect("agent remains addressable after clock rollback");
    assert_eq!(rolled_back_clock.health, AgentHealth::Unresponsive);
}

#[test]
fn wrong_or_untrusted_os_identity_is_rejected_before_challenge() {
    let registry = registry();
    expect_session(&registry, 42, 7, ReplacementPolicy::RejectExisting);

    let mut wrong_sid = observed(42, 7);
    wrong_sid.logon_sid_hash = [99; 32];
    assert_eq!(
        registry.begin_registration(connection(1), register(42, 7), wrong_sid, 1_000),
        Err(AgentRegistryError::ObservedIdentityMismatch)
    );
    assert_eq!(
        registry.begin_registration(connection(2), register(42, 8), observed(42, 8), 1_000),
        Err(AgentRegistryError::UnexpectedWindowsSession)
    );

    let mut wrong_key = register(42, 7);
    wrong_key.agent_key_id = [88; 32];
    assert_eq!(
        registry.begin_registration(connection(3), wrong_key, observed(42, 7), 1_000),
        Err(AgentRegistryError::ExpectedAgentKeyMismatch)
    );

    assert_eq!(
        registry.begin_registration(connection(4), register(43, 7), observed(43, 7), 1_000),
        Err(AgentRegistryError::ExpectedProcessMismatch)
    );

    for (index, caller_kind) in [AgentCallerKind::Anonymous, AgentCallerKind::Network]
        .into_iter()
        .enumerate()
    {
        let mut caller = observed(50 + index as u32, 7);
        caller.caller_kind = caller_kind;
        assert_eq!(
            registry.begin_registration(
                connection(10 + index as u8),
                register(50 + index as u32, 7),
                caller,
                1_000,
            ),
            Err(AgentRegistryError::UntrustedCaller)
        );
    }
}

#[test]
fn stale_challenge_is_consumed_and_cannot_be_retried() {
    let registry = registry();
    expect_session(&registry, 42, 7, ReplacementPolicy::RejectExisting);
    let register = register(42, 7);
    let challenge = registry
        .begin_registration(connection(1), register.clone(), observed(42, 7), 1_000)
        .unwrap();
    let proof = AgentRegistered {
        registration_id: challenge.registration_id,
        registration_epoch: challenge.registration_epoch,
        challenge_id: challenge.challenge_id,
        agent_instance_id: register.agent_instance_id,
        accepted_protocol_major: 1,
        accepted_protocol_minor: 0,
        signed_at_ms: challenge.expires_at_ms,
        signature: [11; 64],
    };
    assert_eq!(
        registry.complete_registration(connection(1), proof.clone(), challenge.expires_at_ms,),
        Err(AgentRegistryError::ChallengeExpired)
    );
    assert_eq!(
        registry.complete_registration(connection(1), proof, 1_001),
        Err(AgentRegistryError::NoPendingRegistration)
    );
}

#[test]
fn launcher_admission_owns_the_exact_proof_verifier() {
    let registry = registry();
    registry
        .expect_session_at(
            expected_session(42, 7, ReplacementPolicy::RejectExisting),
            Arc::new(RejectProof),
            1_000,
        )
        .unwrap();
    let register = register(42, 7);
    let challenge = registry
        .begin_registration(connection(1), register.clone(), observed(42, 7), 1_000)
        .unwrap();

    assert_eq!(
        registry.complete_registration(
            connection(1),
            AgentRegistered {
                registration_id: challenge.registration_id,
                registration_epoch: challenge.registration_epoch,
                challenge_id: challenge.challenge_id,
                agent_instance_id: register.agent_instance_id,
                accepted_protocol_major: 1,
                accepted_protocol_minor: 0,
                signed_at_ms: 1_001,
                signature: [11; 64],
            },
            1_001,
        ),
        Err(AgentRegistryError::Protocol(
            RegistrationError::InvalidSignature
        ))
    );
}

#[test]
fn expired_or_cancelled_launcher_admission_can_be_replaced() {
    let registry = registry();
    let first = expected_session(42, 7, ReplacementPolicy::RejectExisting);
    registry
        .expect_session_at(first.clone(), Arc::new(AcceptProof), 1_000)
        .unwrap();

    let mut second = expected_session(43, 7, ReplacementPolicy::RejectExisting);
    second.expires_at_ms = 30_000;
    registry
        .expect_session_at(second.clone(), Arc::new(AcceptProof), first.expires_at_ms)
        .expect("expired admission should be replaced atomically");
    assert!(registry.cancel_expected_session(&second).unwrap());
    assert!(!registry.cancel_expected_session(&first).unwrap());

    let mut third = expected_session(44, 7, ReplacementPolicy::RejectExisting);
    third.expires_at_ms = 40_000;
    registry
        .expect_session_at(third, Arc::new(AcceptProof), first.expires_at_ms + 1)
        .expect("cancelled admission should release its Windows session");
}

#[test]
fn duplicate_session_follows_explicit_replacement_policy() {
    let rejecting = registry();
    expect_session(&rejecting, 42, 7, ReplacementPolicy::RejectExisting);
    complete(
        &rejecting,
        connection(1),
        register(42, 7),
        observed(42, 7),
        1_000,
    );
    assert_eq!(
        rejecting.expect_session_at(
            expected_session(43, 7, ReplacementPolicy::RejectExisting),
            Arc::new(AcceptProof),
            1_100,
        ),
        Err(AgentRegistryError::ActiveSessionConflict)
    );

    let replacing = registry();
    expect_session(&replacing, 44, 7, ReplacementPolicy::RejectExisting);
    let first = complete(
        &replacing,
        connection(3),
        register(44, 7),
        observed(44, 7),
        2_000,
    );
    expect_session(
        &replacing,
        45,
        7,
        ReplacementPolicy::ReplaceExisting {
            expected_registration_id: first.identity.registration_id,
            expected_registration_epoch: first.identity.registration_epoch,
        },
    );
    let replacement_challenge = replacing
        .begin_registration(connection(4), register(45, 7), observed(45, 7), 2_100)
        .expect("replacement challenge");
    let replacement_identity = replacing
        .complete_registration(
            connection(4),
            AgentRegistered {
                registration_id: replacement_challenge.registration_id,
                registration_epoch: replacement_challenge.registration_epoch,
                challenge_id: replacement_challenge.challenge_id,
                agent_instance_id: [45; 16],
                accepted_protocol_major: 1,
                accepted_protocol_minor: 0,
                signed_at_ms: 2_101,
                signature: [11; 64],
            },
            2_101,
        )
        .expect("replacement proof");

    // Proof alone must not evict a healthy old agent.
    assert!(replacing.is_registration_active(
        &first.identity.registration_id,
        first.identity.registration_epoch,
    ));
    let old_lease = replacing
        .lease_for_session(7)
        .expect("old generation lease");
    assert!(!old_lease.is_revoked());

    let second = replacing
        .activate_registration(connection(4), capability(&replacement_identity, 1), 2_102)
        .expect("activate replacement");
    assert_eq!(second.replaced_connection, Some(connection(3)));
    assert!(old_lease.is_revoked());
    assert!(!replacing.is_registration_active(
        &first.identity.registration_id,
        first.identity.registration_epoch,
    ));
    assert!(replacing.is_registration_active(
        &second.identity.registration_id,
        second.identity.registration_epoch,
    ));

    // A late disconnect from the superseded connection cannot remove the new generation.
    assert!(replacing.disconnect(connection(3)).is_none());
    assert!(replacing.is_registration_active(
        &second.identity.registration_id,
        second.identity.registration_epoch,
    ));
}

#[test]
fn disconnect_invalidates_outstanding_registration_grants() {
    let registry = registry();
    expect_session(&registry, 42, 7, ReplacementPolicy::RejectExisting);
    let outcome = complete(
        &registry,
        connection(1),
        register(42, 7),
        observed(42, 7),
        1_000,
    );
    let lease = registry.lease_for_session(7).expect("execution lease");
    let invalidated = registry
        .disconnect(connection(1))
        .expect("disconnect active agent");
    assert_eq!(
        invalidated.registration_id,
        outcome.identity.registration_id
    );
    assert_eq!(
        invalidated.registration_epoch,
        outcome.identity.registration_epoch
    );
    assert!(!registry.is_registration_active(
        &outcome.identity.registration_id,
        outcome.identity.registration_epoch,
    ));
    assert!(lease.is_revoked());
    assert!(registry.active_for_session_at(7, 1_001).is_none());
}

#[test]
fn security_failure_revokes_agents_and_permanently_latches_registration_closed() {
    let registry = registry();
    expect_session(&registry, 42, 7, ReplacementPolicy::RejectExisting);
    let outcome = complete(
        &registry,
        connection(1),
        register(42, 7),
        observed(42, 7),
        1_000,
    );
    let lease = registry.lease_for_session(7).expect("active lease");

    let invalidated = registry.invalidate_all().expect("security invalidation");
    assert_eq!(invalidated.len(), 1);
    assert!(lease.is_revoked());
    assert!(!registry.is_registration_active(
        &outcome.identity.registration_id,
        outcome.identity.registration_epoch,
    ));
    assert_eq!(
        registry.expect_session_at(
            expected_session(43, 7, ReplacementPolicy::RejectExisting),
            Arc::new(AcceptProof),
            1_100,
        ),
        Err(AgentRegistryError::SecurityUnavailable)
    );
}

#[tokio::test]
async fn server_prioritizes_revocation_over_queued_commands_and_stops() {
    let registry = Arc::new(registry());
    expect_session(&registry, 52, 7, ReplacementPolicy::RejectExisting);
    let server = Arc::new(AgentServer::with_clock(
        Arc::clone(&registry),
        Arc::new(StepClock(AtomicU64::new(1_000))),
    ));
    let (service_stream, mut agent_stream) = tokio::io::duplex(32 * 1024);
    let connection_id = connection(22);
    let serving = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .serve_connection(service_stream, connection_id, observed(52, 7))
                .await
        }
    });

    let agent = tokio::spawn(async move {
        let register = register(52, 7);
        write_frame(
            &mut agent_stream,
            &AgentToService::AgentRegister(register.clone()),
        )
        .await
        .unwrap();
        let challenge = match read_frame::<_, ServiceToAgent>(&mut agent_stream)
            .await
            .unwrap()
            .message
        {
            ServiceToAgent::AgentChallenge(challenge) => challenge,
            other => panic!("expected challenge, got {other:?}"),
        };
        write_frame(
            &mut agent_stream,
            &AgentToService::AgentRegistered(AgentRegistered {
                registration_id: challenge.registration_id,
                registration_epoch: challenge.registration_epoch,
                challenge_id: challenge.challenge_id,
                agent_instance_id: register.agent_instance_id,
                accepted_protocol_major: 1,
                accepted_protocol_minor: 0,
                signed_at_ms: challenge.issued_at_ms,
                signature: [11; 64],
            }),
        )
        .await
        .unwrap();
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
            &mut agent_stream,
            &AgentToService::AgentCapabilitySnapshot(capability(&identity, 1)),
        )
        .await
        .unwrap();
        write_frame(
            &mut agent_stream,
            &AgentToService::AgentHeartbeat(AgentHeartbeat {
                context: AgentEventContext {
                    registration_id: identity.registration_id,
                    registration_epoch: identity.registration_epoch,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence: 1,
                    observed_at_ms: 1_003,
                },
            }),
        )
        .await
        .unwrap();

        let stop = match read_frame::<_, ServiceToAgent>(&mut agent_stream)
            .await
            .unwrap()
            .message
        {
            ServiceToAgent::StopAgent(stop) => stop,
            other => panic!("expected stop, got {other:?}"),
        };
        assert_eq!(stop.reason, StopReason::PolicyChange);
        assert_eq!(stop.request_id, identity.registration_id);
        write_frame(
            &mut agent_stream,
            &AgentToService::AgentStopping(AgentStopping {
                context: AgentEventContext {
                    registration_id: identity.registration_id,
                    registration_epoch: identity.registration_epoch,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence: 2,
                    observed_at_ms: 1_004,
                },
                reason: StoppingReason::ServiceRequest,
            }),
        )
        .await
        .unwrap();
    });

    for _ in 0..100 {
        if registry
            .active_for_session_at(7, 1_010)
            .is_some_and(|snapshot| snapshot.last_event_sequence == 1)
        {
            break;
        }
        sleep(Duration::from_millis(2)).await;
    }
    let lease = registry.lease_for_session(7).expect("registered agent");
    server
        .send_to_connection(
            connection_id,
            ServiceToAgent::StopAgent(StopAgent {
                request_id: [1; 16],
                deadline_ms: 5_000,
                reason: StopReason::ServiceShutdown,
            }),
        )
        .expect("queue stop");
    assert_eq!(
        registry
            .invalidate_all()
            .expect("revoke active agent")
            .len(),
        1
    );
    assert!(matches!(
        server.send_to_connection(
            connection_id,
            ServiceToAgent::StopAgent(StopAgent {
                request_id: [2; 16],
                deadline_ms: 5_000,
                reason: StopReason::ServiceShutdown,
            }),
        ),
        Err(AgentServerError::ConnectionUnavailable)
    ));

    agent.await.unwrap();
    assert_eq!(
        serving.await.unwrap().unwrap(),
        AgentConnectionExit::Stopped
    );
    assert!(lease.is_revoked());
    assert!(registry.active_for_session_at(7, 1_020).is_none());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_private_pipe_observes_the_real_local_process_identity() {
    let current_process = inspect_windows_process(std::process::id()).expect("current process");
    let pipe_name = format!(
        r"\\.\pipe\mrd-agent-registration-test-{}-{}",
        std::process::id(),
        StepClock(AtomicU64::new(1)).now_ms(),
    );
    let mut pipe = WindowsAgentPipe::create_for_process(&pipe_name, &current_process)
        .expect("protected first pipe instance");
    assert!(
        WindowsAgentPipe::create_for_process(&pipe_name, &current_process).is_err(),
        "the protected endpoint must reject a second pipe instance"
    );
    let client = ClientOptions::new()
        .open(&pipe_name)
        .expect("same-logon local client");
    pipe.connect().await.expect("connect private pipe");
    let peer = pipe
        .inspect_peer()
        .expect("inspect pipe and process tokens");

    assert_eq!(
        peer.identity().caller_kind,
        AgentCallerKind::InteractiveUser
    );
    assert_eq!(peer.identity().process_id, std::process::id());
    assert_ne!(peer.identity().process_creation_time, 0);
    assert_ne!(peer.identity().windows_session_id, 0);
    assert!(peer.holds_process_object());
    drop(client);
}
